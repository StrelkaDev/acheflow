//! Exercise the production executable, not a native substitute for the server.
use acheflow_client::server::{LaunchConfig, LocalServer, Phase, ServerError};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "af-owned-{}-{} with spaces;$",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
fn config(directory: &Directory) -> LaunchConfig {
    let http = free_port();
    let mut rtmp = free_port();
    while rtmp == http {
        rtmp = free_port();
    }
    LaunchConfig {
        binary: PathBuf::from(env!("CARGO_BIN_EXE_acheflow-server")),
        directory: directory.0.clone(),
        http_port: http,
        rtmp_port: rtmp,
        startup_timeout: Duration::from_secs(15),
    }
}

#[tokio::test]
async fn missing_binary_is_reported_without_starting_a_process() {
    let directory = Directory::new();
    let mut server = LocalServer::new().unwrap();
    let config = LaunchConfig {
        binary: directory.0.join("missing"),
        directory: directory.0.clone(),
        http_port: 21295,
        rtmp_port: 1935,
        startup_timeout: Duration::from_secs(1),
    };
    assert_eq!(
        server.start(config).await.unwrap_err(),
        ServerError::BinaryMissing
    );
    assert_eq!(server.status().pid, None);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn production_server_becomes_ready_stops_and_restarts() {
    let directory = Directory::new();
    let config = config(&directory);
    std::fs::write(
        directory.0.join("acheflow-server.json"),
        r#"{"http_host":"0.0.0.0","http_port":1,"rtmp_host":"0.0.0.0","rtmp_port":2}"#,
    )
    .unwrap();
    let mut server = LocalServer::new().unwrap();
    let ready = server.start(config.clone()).await.unwrap();
    assert_eq!(ready.phase, Phase::Ready);
    assert_eq!(
        ready.info.as_ref().unwrap().version,
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(ready.info.as_ref().unwrap().process_id, ready.pid.unwrap());
    let old_instance = ready.info.unwrap().managed_instance.unwrap();
    assert_eq!(
        server.start(config.clone()).await.unwrap_err(),
        ServerError::AlreadyRunning
    );
    server.stop().await.unwrap();
    assert_eq!(server.status().phase, Phase::Stopped);
    assert!(!server.status().forced_stop);
    assert!(server.status().pid.is_none());
    let ready = server.restart(config).await.unwrap();
    assert_ne!(ready.info.unwrap().managed_instance.unwrap(), old_instance);
    server.stop().await.unwrap();
    assert!(
        server
            .diagnostics()
            .iter()
            .any(|line| line.contains("Acheflow listening"))
    );
}

#[tokio::test]
async fn occupied_http_or_ingest_ports_never_take_ownership() {
    for ingest in [false, true] {
        let directory = Directory::new();
        let mut config = config(&directory);
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        if ingest {
            config.rtmp_port = port;
        } else {
            config.http_port = port;
        }
        let mut server = LocalServer::new().unwrap();
        assert_eq!(
            server.start(config).await.unwrap_err(),
            ServerError::PortInUse(port)
        );
        server.stop().await.unwrap();
        assert!(std::net::TcpStream::connect(listener.local_addr().unwrap()).is_ok());
        assert!(server.status().pid.is_none());
    }
}

#[tokio::test]
async fn dropping_owner_stops_the_production_child_without_another_ui_turn() {
    let directory = Directory::new();
    let mut server = LocalServer::new().unwrap();
    server.start(config(&directory)).await.unwrap();
    let mut status = server.subscribe();
    drop(server);
    tokio::time::timeout(Duration::from_secs(15), async {
        while status.borrow().pid.is_some() {
            status.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(status.borrow().phase, Phase::Stopped);
}

#[cfg(unix)]
#[tokio::test]
async fn production_crash_is_reported_without_signalling_any_external_process() {
    let directory = Directory::new();
    let mut server = LocalServer::new().unwrap();
    let ready = server.start(config(&directory)).await.unwrap();
    let mut status = server.subscribe();
    assert!(
        std::process::Command::new("/bin/kill")
            .args(["-KILL", &ready.pid.unwrap().to_string()])
            .status()
            .unwrap()
            .success()
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while status.borrow().phase != Phase::Exited {
            status.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(status.borrow().error, Some(ServerError::ProcessExited));
    assert!(status.borrow().pid.is_none());
    server.stop().await.unwrap();
}

#[cfg(unix)]
fn executable(directory: &Directory, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let binary = directory.0.join("server fixture;$");
    std::fs::write(&binary, format!("#!/usr/bin/python3\nimport sys, os\nif '--version' in sys.argv:\n print('acheflow-server {}')\n sys.exit(0)\n{}\n", env!("CARGO_PKG_VERSION"), body)).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    binary
}

#[cfg(unix)]
#[tokio::test]
async fn timeout_drains_large_unbroken_logs_and_reaps_the_child() {
    let directory = Directory::new();
    let mut config = config(&directory);
    config.binary = executable(
        &directory,
        "sys.stderr.write('diagnostic' * 100000)\nsys.stderr.flush()\nsys.stdin.buffer.read(1)",
    );
    config.startup_timeout = Duration::from_millis(700);
    let mut server = LocalServer::new().unwrap();
    assert_eq!(
        server.start(config).await.unwrap_err(),
        ServerError::StartupTimeout
    );
    assert_eq!(server.status().phase, Phase::Failed);
    assert!(server.status().pid.is_none());
    let logs = server.diagnostics();
    assert!(!logs.is_empty());
    assert!(logs.len() <= 128);
    assert!(logs.iter().map(String::len).sum::<usize>() <= 64 * 1024);
    server.stop().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn incompatible_binary_is_rejected_before_launch() {
    let directory = Directory::new();
    let binary = executable(&directory, "raise Exception('must not run')");
    let text = std::fs::read_to_string(&binary)
        .unwrap()
        .replace(env!("CARGO_PKG_VERSION"), "999.0.0");
    std::fs::write(&binary, text).unwrap();
    let mut config = config(&directory);
    config.binary = binary;
    let mut server = LocalServer::new().unwrap();
    assert_eq!(
        server.start(config).await.unwrap_err(),
        ServerError::VersionMismatch
    );
    assert!(server.status().pid.is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn cancelled_start_remains_owned_and_can_be_stopped_immediately() {
    let directory = Directory::new();
    let mut config = config(&directory);
    config.binary = executable(&directory, "sys.stdin.buffer.read(1)");
    let mut server = LocalServer::new().unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), server.start(config))
            .await
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(5), server.stop())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(server.status().phase, Phase::Stopped);
    assert!(server.status().pid.is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn readiness_from_the_wrong_instance_is_rejected_and_cleaned_up() {
    let directory = Directory::new();
    let mut config = config(&directory);
    config.binary = executable(&directory, r#"
import json, threading
from http.server import BaseHTTPRequestHandler, HTTPServer
port = int(sys.argv[sys.argv.index('--port') + 1])
rtmp = int(sys.argv[sys.argv.index('--rtmp-port') + 1])
class Handler(BaseHTTPRequestHandler):
 def do_GET(self):
  payload = json.dumps(dict(protocol_version=1, version='VERSION', process_id=os.getpid(), managed_instance='0'*32, http_address='127.0.0.1:'+str(port), rtmp_address='127.0.0.1:'+str(rtmp))).encode()
  self.send_response(200)
  self.send_header('Content-Length', str(len(payload)))
  self.end_headers()
  self.wfile.write(payload)
 def log_message(self, *args): pass
http = HTTPServer(('127.0.0.1', port), Handler)
threading.Thread(target=http.serve_forever, daemon=True).start()
sys.stdin.buffer.read(1)
http.shutdown()
"#.replace("VERSION", env!("CARGO_PKG_VERSION")).as_str());
    let mut server = LocalServer::new().unwrap();
    assert_eq!(
        server.start(config).await.unwrap_err(),
        ServerError::ReadinessMismatch
    );
    assert!(server.status().pid.is_none());
}

#[tokio::test]
async fn a_second_manager_can_inspect_but_never_stop_the_first_managers_server() {
    let directory = Directory::new();
    let config = config(&directory);
    let mut owner = LocalServer::new().unwrap();
    owner.start(config.clone()).await.unwrap();
    let mut other = LocalServer::new().unwrap();
    let observed = acheflow_client::server::inspect_local(config.http_port)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(observed.process_id, owner.status().pid.unwrap());
    assert!(other.status().pid.is_none());
    assert_eq!(
        other.start(config.clone()).await.unwrap_err(),
        ServerError::PortInUse(config.http_port)
    );
    other.stop().await.unwrap();
    assert_eq!(owner.status().phase, Phase::Ready);
    assert!(
        acheflow_client::server::inspect_local(config.http_port)
            .await
            .unwrap()
            .is_some()
    );
    owner.stop().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn an_uncooperative_child_is_force_killed_and_reaped_after_the_grace_period() {
    let directory = Directory::new();
    let mut config = config(&directory);
    config.binary = executable(&directory, "import time\ntime.sleep(60)");
    config.startup_timeout = Duration::from_millis(700);
    let mut server = LocalServer::new().unwrap();
    let result = tokio::time::timeout(Duration::from_secs(17), server.start(config))
        .await
        .unwrap();
    assert_eq!(result.unwrap_err(), ServerError::StartupTimeout);
    assert!(server.status().forced_stop);
    assert!(server.status().pid.is_none());
}
