use super::{LaunchConfig, ServerError};
use acheflow_media_api::server::{SERVER_PROTOCOL_VERSION, ServerInfo};
use std::{net::SocketAddr, process::Stdio, time::Duration};
use tokio::{
    io::AsyncReadExt,
    process::{Child, Command},
    time::timeout,
};

pub(super) fn client() -> Result<reqwest::Client, ServerError> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_millis(400))
        .build()
        .map_err(|_| ServerError::InvalidConfig)
}
pub(super) async fn info(client: &reqwest::Client, address: SocketAddr) -> Option<ServerInfo> {
    let mut response = client
        .get(format!("http://{address}/api/server/info"))
        .send()
        .await
        .ok()?;
    if response.status() != reqwest::StatusCode::OK {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if body.len() + chunk.len() > 4096 {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    let info: ServerInfo = serde_json::from_slice(&body).ok()?;
    if info.protocol_version != SERVER_PROTOCOL_VERSION
        || info.version.len() > 64
        || info
            .managed_instance
            .as_ref()
            .is_some_and(|id| id.len() != 32)
    {
        return None;
    }
    Some(info)
}

fn command(config: &LaunchConfig) -> Command {
    let mut command = Command::new(&config.binary);
    command.args(&config.server_args);
    // Listener/config credentials from a standalone invocation must not silently
    // change the owned child's configuration. FFmpeg/PATH remain available.
    for name in [
        "ACHEFLOW_CONFIG",
        "ACHEFLOW_HTTP_ADDR",
        "ACHEFLOW_RTMP_ADDR",
        "ACHEFLOW_HTTP_CERT",
        "ACHEFLOW_HTTP_KEY",
        "ACHEFLOW_RTMP_CERT",
        "ACHEFLOW_RTMP_KEY",
        "ACHEFLOW_DESKTOP_INSTANCE",
        "ACHEFLOW_MCP_TOKEN",
        "ACHEFLOW_MCP_READ_TOKEN",
        "ACHEFLOW_MCP_EXECUTORS",
        "ACHEFLOW_MCP_PORT",
    ] {
        command.env_remove(name);
    }
    command
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}

pub(super) async fn prepare(config: &mut LaunchConfig) -> Result<(), ServerError> {
    config.validate()?;
    let binary = config.binary.clone();
    config.binary = tokio::task::spawn_blocking(move || {
        let binary = binary
            .canonicalize()
            .map_err(|_| ServerError::BinaryMissing)?;
        if !binary.is_file() {
            return Err(ServerError::BinaryMissing);
        }
        Ok(binary)
    })
    .await
    .map_err(|_| ServerError::BinaryMissing)??;
    let mut child = command(config)
        .arg("--version")
        .spawn()
        .map_err(|_| ServerError::VersionProbeFailed)?;
    let stdout = child.stdout.take().ok_or(ServerError::VersionProbeFailed)?;
    let mut bytes = Vec::new();
    let mut stdout = stdout.take(257);
    let result = timeout(Duration::from_secs(3), async {
        let (status, read) = tokio::join!(child.wait(), stdout.read_to_end(&mut bytes));
        read.map_err(|_| ServerError::VersionProbeFailed)?;
        if !status
            .map_err(|_| ServerError::VersionProbeFailed)?
            .success()
            || bytes.len() > 256
        {
            return Err(ServerError::VersionProbeFailed);
        }
        if bytes != format!("acheflow-server {}\n", env!("CARGO_PKG_VERSION")).as_bytes() {
            return Err(ServerError::VersionMismatch);
        }
        Ok(())
    })
    .await;
    match result {
        Ok(result) => result?,
        Err(_) => {
            let _ = child.kill().await;
            return Err(ServerError::VersionProbeFailed);
        }
    }
    for port in [config.http_port, config.rtmp_port] {
        let connection = timeout(
            Duration::from_millis(200),
            tokio::net::TcpStream::connect(("127.0.0.1", port)),
        )
        .await;
        if !matches!(connection, Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused)
        {
            return Err(ServerError::PortInUse(port));
        }
    }
    let directory = config.directory.clone();
    tokio::task::spawn_blocking(move || std::fs::create_dir_all(directory))
        .await
        .map_err(|_| ServerError::DirectoryUnavailable)?
        .map_err(|_| ServerError::DirectoryUnavailable)?;
    Ok(())
}

pub(super) fn spawn(config: &LaunchConfig, instance: &str) -> Result<Child, ServerError> {
    command(config)
        // Running has an explicit drop guard, disarmed only after detach ACK.
        .kill_on_drop(false)
        .current_dir(&config.directory)
        .args(["--desktop-managed", "--config"])
        .arg(config.directory.join("acheflow-server.json"))
        .args(["--listen", "127.0.0.1", "--port"])
        .arg(config.http_port.to_string())
        .args(["--rtmp-listen", "127.0.0.1", "--rtmp-port"])
        .arg(config.rtmp_port.to_string())
        .env("ACHEFLOW_DESKTOP_INSTANCE", instance)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| ServerError::SpawnFailed)
}
