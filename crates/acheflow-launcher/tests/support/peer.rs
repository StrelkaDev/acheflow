// Shared subprocess fixture; callers supply only owned test runtime paths.
use acheflow_extension_api::forwarding::ForwardRecipient;
use acheflow_launcher::{ActivationError, Request, Startup, start};
use std::{
    fs::OpenOptions,
    io::{BufRead, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

pub fn run(
    root: PathBuf,
    label: String,
    log: PathBuf,
    gate: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(gate) = gate {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !gate.exists() {
            if Instant::now() >= deadline {
                return Err("start gate timed out".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let record = |line: String| -> std::io::Result<()> {
        // Each fixture record is a single append write and contains no URL/token.
        let mut file = OpenOptions::new().create(true).append(true).open(&log)?;
        file.write_all(format!("{line}\n").as_bytes())
    };
    let request = Request::Forward {
        url: format!("https://app.acheflow.com/fwd/example/open?request={label}"),
    };
    match start(ForwardRecipient::Client, request, &root) {
        Ok(Startup::HandedOff) => {
            record(format!("handed {label}"))?;
        }
        Ok(Startup::Primary(primary)) => {
            record(format!("primary {label} {}", std::process::id()))?;
            let mut inbox = primary.inbox().take().ok_or("inbox already taken")?;
            let (exit, mut stopped) = tokio::sync::oneshot::channel();
            std::thread::spawn(move || {
                let _ = std::io::stdin().lock().lines().next();
                let _ = exit.send(());
            });
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(async {
                loop {
                    tokio::select! {
                        _ = &mut stopped => break,
                        activation = inbox.recv() => {
                            let Some(activation) = activation else { break };
                            if !activation.is_live() {
                                activation.complete(Err(ActivationError::Expired));
                                continue;
                            }
                            let received = match &activation.request {
                                Request::Show => "show".to_string(),
                                Request::Forward { url } => url.split("request=").nth(1).unwrap_or("missing").to_string(),
                            };
                            if record(format!("request {received}")).is_err() {
                                activation.complete(Err(ActivationError::Rejected));
                                break;
                            }
                            activation.complete(Ok(()));
                        }
                    }
                }
            });
        }
        Err(error) => {
            record(format!("error {label} {error:?}"))?;
            return Err(error.into());
        }
    }
    Ok(())
}
