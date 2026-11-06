//! The native event loop must be able to stop its embedded server without a signal.
use std::{ffi::OsString, sync::mpsc, time::Duration};

fn port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test]
async fn embedding_owner_cancels_and_joins_the_running_backend() {
    let http = port();
    let mut rtmp = port();
    while http == rtmp {
        rtmp = port();
    }
    let args: Vec<OsString> = [
        "acheflow-server".to_owned(),
        "--listen".into(),
        "127.0.0.1".into(),
        "--port".into(),
        http.to_string(),
        "--rtmp-listen".into(),
        "127.0.0.1".into(),
        "--rtmp-port".into(),
        rtmp.to_string(),
        "--config".into(),
        std::env::temp_dir()
            .join(format!("acheflow-embedded-{}.json", std::process::id()))
            .to_string_lossy()
            .into_owned(),
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    let shutdown = acheflow_server::Shutdown::new();
    let cancel = shutdown.clone();
    let (sender, completed) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let result = acheflow_server::run_with_shutdown(args, None, cancel)
            .map_err(|error| error.to_string());
        let _ = sender.send(result);
    });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if acheflow_client::server::inspect_local(http)
            .await
            .unwrap()
            .is_some()
        {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            shutdown.cancel();
            panic!("Embedded backend did not become ready");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    shutdown.cancel();
    completed
        .recv_timeout(Duration::from_secs(12))
        .unwrap()
        .unwrap();
    worker.join().unwrap();
    assert!(std::net::TcpListener::bind(("127.0.0.1", http)).is_ok());
    assert!(std::net::TcpListener::bind(("127.0.0.1", rtmp)).is_ok());
}
