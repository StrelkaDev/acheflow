//! Owned local server lifecycle. It never adopts or signals an existing server.
use acheflow_media_api::server::ServerInfo;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, watch};

mod io;
mod logs;
mod worker;

#[derive(Clone, Debug)]
pub struct LaunchConfig {
    pub binary: PathBuf,
    /// Explicit mode arguments; independent of the executable's filename.
    pub server_args: Vec<std::ffi::OsString>,
    /// Stable, explicit application-data directory; never the caller's cwd.
    pub directory: PathBuf,
    pub http_port: u16,
    pub rtmp_port: u16,
    /// Opt in to HTTP/RTMP connections on all IPv4 interfaces.
    pub allow_network: bool,
    pub startup_timeout: Duration,
}
impl LaunchConfig {
    pub fn bundled(directory: PathBuf) -> Result<Self, ServerError> {
        Ok(Self {
            binary: bundled_binary(
                &std::env::current_exe().map_err(|_| ServerError::BinaryMissing)?,
            )?,
            server_args: if cfg!(unix) {
                vec!["--server".into(), "--native-links".into()]
            } else {
                vec!["--server".into()]
            },
            directory,
            http_port: 21295,
            rtmp_port: 1935,
            allow_network: false,
            startup_timeout: Duration::from_secs(30),
        })
    }
    fn validate(&self) -> Result<(), ServerError> {
        if !self.binary.is_absolute() || !self.directory.is_absolute() {
            return Err(ServerError::InvalidConfig);
        }
        if [self.http_port, self.rtmp_port].contains(&0) || self.http_port == self.rtmp_port {
            return Err(ServerError::InvalidConfig);
        }
        if !(Duration::from_millis(100)..=Duration::from_secs(60)).contains(&self.startup_timeout) {
            return Err(ServerError::InvalidConfig);
        }
        Ok(())
    }
    fn address(&self) -> std::net::SocketAddr {
        ([127, 0, 0, 1], self.http_port).into()
    }
    fn listen_ip(&self) -> std::net::Ipv4Addr {
        if self.allow_network {
            std::net::Ipv4Addr::UNSPECIFIED
        } else {
            std::net::Ipv4Addr::LOCALHOST
        }
    }
}

/// Installed clients reuse their own executable; development UI uses its sibling.
pub fn bundled_binary(client_executable: &Path) -> Result<PathBuf, ServerError> {
    if !client_executable.is_absolute() {
        return Err(ServerError::InvalidConfig);
    }
    let name = if cfg!(windows) {
        "acheflow.exe"
    } else {
        "acheflow"
    };
    let path = if client_executable
        .file_name()
        .is_some_and(|file| file == name)
    {
        client_executable.to_owned()
    } else {
        client_executable
            .parent()
            .ok_or(ServerError::BinaryMissing)?
            .join(name)
    };
    path.canonicalize().map_err(|_| ServerError::BinaryMissing)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Stopped,
    Starting,
    Ready,
    Unavailable,
    Stopping,
    Exited,
    Failed,
    Detached,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub phase: Phase,
    /// Present only while this manager owns a live Child, never inferred from HTTP.
    pub pid: Option<u32>,
    pub endpoint: Option<String>,
    pub info: Option<ServerInfo>,
    pub exit_code: Option<i32>,
    /// Graceful pipe shutdown exceeded its deadline and the owned child was killed.
    pub forced_stop: bool,
    pub error: Option<ServerError>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerError {
    InvalidConfig,
    BinaryMissing,
    VersionProbeFailed,
    VersionMismatch,
    DirectoryUnavailable,
    PortInUse(u16),
    SpawnFailed,
    StartupTimeout,
    ReadinessMismatch,
    ProcessExited,
    AlreadyRunning,
    StopFailed,
    ManagerClosed,
    DetachFailed,
}
impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::InvalidConfig => "Invalid local server paths, ports or startup timeout",
            Self::BinaryMissing => "Bundled server executable is missing",
            Self::VersionProbeFailed => "Cannot read bundled server version",
            Self::VersionMismatch => "Client and bundled server versions do not match",
            Self::DirectoryUnavailable => "Cannot create server data directory",
            Self::PortInUse(port) => return write!(f, "Local port {port} is already in use"),
            Self::SpawnFailed => "Cannot start bundled server",
            Self::StartupTimeout => "Server API did not become ready before the deadline",
            Self::ReadinessMismatch => {
                "Server API does not belong to the launched compatible child"
            }
            Self::ProcessExited => "Server process exited",
            Self::AlreadyRunning => "This manager already owns a server process",
            Self::StopFailed => "Cannot stop owned server process",
            Self::ManagerClosed => "Server lifecycle worker is unavailable",
            Self::DetachFailed => "Could not confirm that the server can run independently",
        };
        f.write_str(message)
    }
}
impl std::error::Error for ServerError {}

type Reply = oneshot::Sender<Result<Status, ServerError>>;
enum Request {
    Start(LaunchConfig, Reply),
    Stop(Reply),
    Shutdown(Reply),
}

/// One process owner, usable from a desktop settings context. Dropping the handle
/// closes its command channel and asks the worker to gracefully stop its child.
pub struct LocalServer {
    commands: mpsc::Sender<Request>,
    status: watch::Receiver<Status>,
    logs: logs::Logs,
    keep_running: Arc<AtomicBool>,
}
impl LocalServer {
    /// Must be called inside a Tokio runtime.
    pub fn new() -> Result<Self, ServerError> {
        let client = io::client()?;
        let (commands, rx) = mpsc::channel(4);
        let (tx, status) = watch::channel(Status::default());
        let logs = logs::Logs::default();
        let keep_running = Arc::new(AtomicBool::new(false));
        tokio::spawn(worker::run(
            rx,
            tx,
            logs.clone(),
            client,
            keep_running.clone(),
        ));
        Ok(Self {
            commands,
            status,
            logs,
            keep_running,
        })
    }
    pub fn status(&self) -> Status {
        self.status.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }
    pub fn diagnostics(&self) -> Vec<String> {
        self.logs.snapshot()
    }
    pub async fn start(&self, config: LaunchConfig) -> Result<Status, ServerError> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Request::Start(config, reply))
            .await
            .map_err(|_| ServerError::ManagerClosed)?;
        rx.await.map_err(|_| ServerError::ManagerClosed)?
    }
    pub async fn stop(&self) -> Result<Status, ServerError> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Request::Stop(reply))
            .await
            .map_err(|_| ServerError::ManagerClosed)?;
        rx.await.map_err(|_| ServerError::ManagerClosed)?
    }
    pub async fn restart(&self, config: LaunchConfig) -> Result<Status, ServerError> {
        self.stop().await?;
        self.start(config).await
    }
    pub fn set_keep_running_on_exit(&self, enabled: bool) {
        self.keep_running.store(enabled, Ordering::Release);
    }
    /// Await this before shutting down the runtime. Explicit stop/restart always
    /// stop the owned child regardless of the exit preference.
    pub async fn shutdown(&self) -> Result<Status, ServerError> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Request::Shutdown(reply))
            .await
            .map_err(|_| ServerError::ManagerClosed)?;
        rx.await.map_err(|_| ServerError::ManagerClosed)?
    }
}

/// Read-only discovery for an external local server. This never grants ownership.
pub async fn inspect_local(port: u16) -> Result<Option<ServerInfo>, ServerError> {
    if port == 0 {
        return Err(ServerError::InvalidConfig);
    }
    Ok(io::info(&io::client()?, ([127, 0, 0, 1], port).into()).await)
}
