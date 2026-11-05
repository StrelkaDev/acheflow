use super::{LaunchConfig, Phase, Reply, Request, ServerError, Status, io, logs::Logs};
use acheflow_media_api::server::ServerInfo;
use std::{
    process::ExitStatus,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    process::{Child, ChildStdin},
    sync::{mpsc, watch},
    task::JoinHandle,
    time::{Instant, interval, timeout},
};

struct Running {
    child: Child,
    input: Option<ChildStdin>,
    output: Vec<JoinHandle<()>>,
    config: LaunchConfig,
    instance: String,
    deadline: Instant,
    pending: Option<Reply>,
    forced_stop: bool,
    released: bool,
}
impl Running {
    async fn launch(mut config: LaunchConfig, logs: &Logs) -> Result<Self, ServerError> {
        io::prepare(&mut config).await?;
        let mut random = [0; 16];
        getrandom::getrandom(&mut random).map_err(|_| ServerError::SpawnFailed)?;
        let instance: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let mut child = io::spawn(&config, &instance)?;
        let input = child.stdin.take();
        let mut output = vec![];
        if let Some(stdout) = child.stdout.take() {
            let logs = logs.clone();
            output.push(tokio::spawn(
                async move { logs.drain("stdout", stdout).await },
            ));
        }
        if let Some(stderr) = child.stderr.take() {
            let logs = logs.clone();
            output.push(tokio::spawn(
                async move { logs.drain("stderr", stderr).await },
            ));
        }
        Ok(Self {
            child,
            input,
            output,
            instance,
            deadline: Instant::now() + config.startup_timeout,
            config,
            pending: None,
            forced_stop: false,
            released: false,
        })
    }
    fn matches(&self, info: &ServerInfo) -> bool {
        info.version == env!("CARGO_PKG_VERSION")
            && Some(info.process_id) == self.child.id()
            && info.managed_instance.as_ref() == Some(&self.instance)
            && info.http_address == self.config.address()
            && info.rtmp_address
                == std::net::SocketAddr::from(([127, 0, 0, 1], self.config.rtmp_port))
    }
    fn complete(&mut self, result: Result<Status, ServerError>) {
        if let Some(reply) = self.pending.take() {
            let _ = reply.send(result);
        }
    }
    async fn stop(&mut self) -> Result<ExitStatus, ServerError> {
        // Pipe EOF is private to this exact child and works without signal/unsafe APIs.
        self.input = None;
        match timeout(Duration::from_secs(12), self.child.wait()).await {
            Ok(result) => result.map_err(|_| ServerError::StopFailed),
            Err(_) => {
                self.forced_stop = true;
                self.child
                    .start_kill()
                    .map_err(|_| ServerError::StopFailed)?;
                timeout(Duration::from_secs(2), self.child.wait())
                    .await
                    .map_err(|_| ServerError::StopFailed)?
                    .map_err(|_| ServerError::StopFailed)
            }
        }
    }
    async fn finish_logs(&mut self) {
        for mut task in self.output.drain(..) {
            if timeout(Duration::from_millis(100), &mut task)
                .await
                .is_err()
            {
                task.abort();
            }
        }
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        if !self.released {
            let _ = self.child.start_kill();
        }
        for task in &self.output {
            task.abort();
        }
    }
}

pub(super) async fn run(
    mut commands: mpsc::Receiver<Request>,
    status: watch::Sender<Status>,
    logs: Logs,
    client: reqwest::Client,
    keep_running: Arc<AtomicBool>,
) {
    let mut active: Option<Running> = None;
    let mut tick = interval(Duration::from_millis(300));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            request = commands.recv() => match request {
                Some(Request::Start(config, reply)) => {
                    if active.is_some() { let _ = reply.send(Err(ServerError::AlreadyRunning)); continue; }
                    logs.clear();
                    status.send_replace(Status { phase: Phase::Starting, endpoint: Some(format!("http://{}", config.address())), ..Default::default() });
                    match Running::launch(config, &logs).await {
                        Ok(mut running) => {
                            status.send_modify(|status| status.pid = running.child.id());
                            running.pending = Some(reply);
                            active = Some(running);
                        }
                        Err(error) => {
                            status.send_modify(|status| { status.phase = Phase::Failed; status.error = Some(error.clone()); });
                            let _ = reply.send(Err(error));
                        }
                    }
                }
                Some(Request::Stop(reply)) => {
                    let result = stop(&mut active, &status).await;
                    let _ = reply.send(result);
                }
                Some(Request::Shutdown(reply)) => {
                    let result = exit(&mut active, &status, &client, keep_running.load(Ordering::Acquire)).await;
                    let _ = reply.send(result);
                    break;
                }
                None => { let _ = exit(&mut active, &status, &client, keep_running.load(Ordering::Acquire)).await; break; }
            },
            _ = tick.tick(), if active.is_some() => {
                if let Some(running) = active.as_mut() {
                    let result = check(running, &client, &status).await;
                    if let Err(error) = result {
                        let cleanup = running.stop().await;
                        running.finish_logs().await;
                        running.complete(Err(error.clone()));
                        let code = cleanup.as_ref().ok().and_then(ExitStatus::code);
                        let forced = running.forced_stop;
                        if cleanup.is_ok() {
                            active = None;
                            status.send_modify(|status| { status.pid = None; status.exit_code = code; status.forced_stop = forced; });
                        }
                        status.send_modify(|status| { status.phase = Phase::Failed; status.error = Some(error); });
                    } else if running.child.id().is_none() {
                        running.finish_logs().await;
                        active = None;
                    }
                }
            }
        }
    }
}

async fn exit(
    active: &mut Option<Running>,
    status: &watch::Sender<Status>,
    client: &reqwest::Client,
    keep: bool,
) -> Result<Status, ServerError> {
    if keep
        && let Some(running) = active.as_mut()
        && running.pending.is_none()
    {
        if let Ok(info) = detach(running, client).await {
            running.released = true;
            running.input = None;
            running.finish_logs().await;
            status.send_modify(|status| {
                status.phase = Phase::Detached;
                status.pid = None;
                status.info = Some(info);
            });
            if let Some(mut released) = active.take() {
                // Reap while this runtime lives. Runtime shutdown may drop this
                // future; its disarmed guard intentionally leaves the child alive.
                tokio::spawn(async move {
                    let _ = released.child.wait().await;
                });
            }
            return Ok(status.borrow().clone());
        }
        stop(active, status).await?;
        status.send_modify(|status| status.error = Some(ServerError::DetachFailed));
        return Err(ServerError::DetachFailed);
    }
    stop(active, status).await
}

async fn detach(
    running: &mut Running,
    client: &reqwest::Client,
) -> Result<ServerInfo, ServerError> {
    timeout(Duration::from_secs(2), async {
        running
            .input
            .as_mut()
            .ok_or(ServerError::DetachFailed)?
            .write_all(b"D")
            .await
            .map_err(|_| ServerError::DetachFailed)?;
        loop {
            if let Some(info) = io::info(client, running.config.address()).await
                && running.matches(&info)
                && !info.desktop_attached
            {
                return Ok(info);
            }
            if running
                .child
                .try_wait()
                .map_err(|_| ServerError::DetachFailed)?
                .is_some()
            {
                return Err(ServerError::DetachFailed);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| ServerError::DetachFailed)?
}

async fn check(
    running: &mut Running,
    client: &reqwest::Client,
    status: &watch::Sender<Status>,
) -> Result<(), ServerError> {
    if exited(running, status)? {
        return Ok(());
    }
    let info = if running.pending.is_some() {
        if Instant::now() >= running.deadline {
            return Err(ServerError::StartupTimeout);
        }
        tokio::time::timeout_at(running.deadline, io::info(client, running.config.address()))
            .await
            .map_err(|_| ServerError::StartupTimeout)?
    } else {
        io::info(client, running.config.address()).await
    };
    if exited(running, status)? {
        return Ok(());
    }
    if let Some(info) = info {
        if !running.matches(&info) {
            return Err(ServerError::ReadinessMismatch);
        }
        status.send_modify(|status| {
            status.phase = Phase::Ready;
            status.info = Some(info);
            status.error = None;
        });
        running.complete(Ok(status.borrow().clone()));
    } else if running.pending.is_some() {
        if Instant::now() >= running.deadline {
            return Err(ServerError::StartupTimeout);
        }
    } else {
        status.send_modify(|status| status.phase = Phase::Unavailable);
    }
    Ok(())
}

fn exited(running: &mut Running, status: &watch::Sender<Status>) -> Result<bool, ServerError> {
    if let Some(exit) = running
        .child
        .try_wait()
        .map_err(|_| ServerError::ProcessExited)?
    {
        status.send_modify(|status| {
            status.phase = Phase::Exited;
            status.pid = None;
            status.exit_code = exit.code();
            status.error = Some(ServerError::ProcessExited);
        });
        running.complete(Err(ServerError::ProcessExited));
        return Ok(true);
    }
    Ok(false)
}

async fn stop(
    active: &mut Option<Running>,
    status: &watch::Sender<Status>,
) -> Result<Status, ServerError> {
    if let Some(running) = active.as_mut() {
        status.send_modify(|status| status.phase = Phase::Stopping);
        let result = match running.stop().await {
            Ok(result) => result,
            Err(error) => {
                status.send_modify(|status| {
                    status.phase = Phase::Failed;
                    status.error = Some(error.clone());
                });
                return Err(error);
            }
        };
        running.finish_logs().await;
        running.complete(Err(ServerError::ProcessExited));
        status.send_modify(|status| {
            status.exit_code = result.code();
            status.forced_stop = running.forced_stop;
        });
        *active = None;
    }
    status.send_modify(|status| {
        status.phase = Phase::Stopped;
        status.pid = None;
        status.error = None;
    });
    Ok(status.borrow().clone())
}
