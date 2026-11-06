#![cfg(unix)]

use std::{
    io::Write,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Server {
    child: Child,
    directory: PathBuf,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
fn port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test]
async fn cold_server_uri_and_warm_handoff_never_start_a_client_or_leak_payloads() {
    let directory = std::env::temp_dir().join(format!("dl13-dispatch-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let http = port();
    let mut rtmp = port();
    while rtmp == http {
        rtmp = port();
    }
    std::fs::write(directory.join("server.json"), format!(r#"{{"http_host":"127.0.0.1","http_port":{http},"rtmp_host":"127.0.0.1","rtmp_port":{rtmp}}}"#)).unwrap();
    let command = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_acheflow"));
        command
            .arg("--forward-stdin")
            .env(
                "ACHEFLOW_EXTENSIONS_DIR",
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../extensions"),
            )
            .env("ACHEFLOW_RUNTIME_DIR", directory.join("run"))
            .env("ACHEFLOW_CONFIG", directory.join("server.json"))
            .env("ACHEFLOW_FFMPEG", "/nonexistent/ffmpeg")
            .env("ACHEFLOW_MCP_PORT", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        command
    };
    let mut child = command().spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"acheflow://server/fwd/sample/increment?value=private-marker")
        .unwrap();
    let mut server = Server {
        child,
        directory: directory.clone(),
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    let info = loop {
        assert!(
            server.child.try_wait().unwrap().is_none(),
            "Cold server dispatch exited"
        );
        if let Some(info) = acheflow_client::server::inspect_local(http).await.unwrap() {
            break info;
        }
        assert!(Instant::now() < deadline, "Cold server never became ready");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(info.process_id, server.child.id());
    assert!(directory.join("run/server.sock").exists());
    assert!(!directory.join("run/client.sock").exists());
    // The server is intentionally unconfigured. A warm link must reach it and
    // return NotReady, instead of starting another server or accepting a guest.
    let mut warm = command().spawn().unwrap();
    warm.stdin
        .take()
        .unwrap()
        .write_all(b"https://app.acheflow.com/server/fwd/sample/increment?value=private-marker")
        .unwrap();
    let output = warm.wait_with_output().unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("not ready"),
        "Unexpected fixed diagnostic: {error}"
    );
    assert!(!error.contains("private-marker"));
    assert!(server.child.try_wait().unwrap().is_none());
    assert!(!directory.join("run/client.sock").exists());
}
