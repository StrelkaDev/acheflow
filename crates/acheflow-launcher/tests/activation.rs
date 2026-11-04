#![cfg(unix)]

use acheflow_extension_api::forwarding::ForwardRecipient;
use acheflow_launcher::{
    ActivationError, HANDOFF_TIMEOUT, INBOX_CAPACITY, Request, Startup, start,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::{MetadataExt, PermissionsExt, symlink},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

#[path = "support/peer.rs"]
mod peer;

#[test]
fn activation_peer_process() -> Result<(), Box<dyn std::error::Error>> {
    let Some(root) = std::env::var_os("ACHEFLOW_ACTIVATION_TEST_ROOT") else {
        return Ok(());
    };
    let label = std::env::var("ACHEFLOW_ACTIVATION_TEST_LABEL")?;
    let log = std::env::var_os("ACHEFLOW_ACTIVATION_TEST_LOG").ok_or("missing fixture log")?;
    let gate = std::env::var_os("ACHEFLOW_ACTIVATION_TEST_GATE").map(Into::into);
    peer::run(root.into(), label, log.into(), gate)
}

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Path::new("/tmp").join(format!(
            "af-activation-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        Self(root)
    }
    fn runtime(&self) -> PathBuf {
        self.0.join("run")
    }
    fn log(&self) -> PathBuf {
        self.0.join("events")
    }
    fn lines(&self) -> Vec<String> {
        fs::read_to_string(self.log())
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Peer(Child);
impl Peer {
    fn spawn(root: &Directory, label: &str, gate: Option<&Path>) -> Self {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "activation_peer_process", "--nocapture"])
            .env("ACHEFLOW_ACTIVATION_TEST_ROOT", root.runtime())
            .env("ACHEFLOW_ACTIVATION_TEST_LABEL", label)
            .env("ACHEFLOW_ACTIVATION_TEST_LOG", root.log())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some(gate) = gate {
            command.env("ACHEFLOW_ACTIVATION_TEST_GATE", gate);
        } else {
            command.env_remove("ACHEFLOW_ACTIVATION_TEST_GATE");
        }
        Self(command.spawn().unwrap())
    }
    fn finish(&mut self) {
        if let Some(stdin) = self.0.stdin.as_mut() {
            let _ = stdin.write_all(b"exit\n");
        }
        wait_until(|| self.0.try_wait().unwrap().is_some(), "peer did not stop");
        assert!(self.0.wait().unwrap().success());
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_until(mut condition: impl FnMut() -> bool, message: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "{message}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn primary(root: &Path) -> acheflow_launcher::Primary {
    match start(ForwardRecipient::Client, Request::Show, root).unwrap() {
        Startup::Primary(primary) => primary,
        Startup::HandedOff => panic!("a fresh test directory cannot have another primary"),
    }
}

fn connect(root: &Path) -> UnixStream {
    let stream = UnixStream::connect(root.join("client.sock")).unwrap();
    stream
        .set_read_timeout(Some(HANDOFF_TIMEOUT + Duration::from_secs(2)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    stream
}
fn send_frame(stream: &mut UnixStream, bytes: &[u8]) {
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(bytes).unwrap();
}
fn wire(label: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "version":1,"recipient":"client",
        "request":{"type":"forward","url":format!("https://client.acheflow.com/fwd/example/open?request={label}")}
    })).unwrap()
}
fn read_reply(stream: &mut UnixStream) -> Result<(), ActivationError> {
    let mut length = [0; 4];
    stream.read_exact(&mut length).unwrap();
    let length = u32::from_be_bytes(length) as usize;
    assert!(length <= 512);
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).unwrap();
    let reply: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(reply["version"], 1);
    serde_json::from_value(reply["result"].clone()).unwrap()
}

#[test]
fn warm_requests_reach_one_real_primary_process() {
    let root = Directory::new();
    let mut first = Peer::spawn(&root, "first", None);
    wait_until(
        || {
            root.lines()
                .iter()
                .any(|line| line.starts_with("primary first "))
        },
        "primary did not start",
    );
    for label in ["second", "third", "fourth"] {
        let mut follower = Peer::spawn(&root, label, None);
        wait_until(
            || follower.0.try_wait().unwrap().is_some(),
            "follower did not hand off",
        );
        assert!(follower.0.wait().unwrap().success(), "follower failed");
    }
    let lines = root.lines();
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("primary "))
            .count(),
        1
    );
    for label in ["first", "second", "third", "fourth"] {
        assert_eq!(
            lines
                .iter()
                .filter(|line| **line == format!("request {label}"))
                .count(),
            1
        );
    }
    first.finish();
}

#[test]
fn simultaneous_cold_processes_elect_exactly_one_primary() {
    let root = Directory::new();
    let gate = root.0.join("start");
    let mut peers: Vec<_> = (0..6)
        .map(|n| Peer::spawn(&root, &format!("peer{n}"), Some(&gate)))
        .collect();
    fs::write(gate, b"go").unwrap();
    wait_until(
        || {
            root.lines()
                .iter()
                .filter(|line| line.starts_with("request "))
                .count()
                == 6
        },
        "concurrent requests were not received",
    );
    wait_until(
        || {
            peers
                .iter_mut()
                .map(|peer| peer.0.try_wait().unwrap().is_some())
                .filter(|finished| *finished)
                .count()
                == 5
        },
        "followers did not exit",
    );
    let lines = root.lines();
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("primary "))
            .count(),
        1,
        "{lines:?}"
    );
    for n in 0..6 {
        assert_eq!(
            lines
                .iter()
                .filter(|line| **line == format!("request peer{n}"))
                .count(),
            1
        );
    }
    for peer in &mut peers {
        peer.finish();
    }
}

#[test]
fn a_killed_primary_releases_the_lock_and_its_stale_socket_is_recovered() {
    let root = Directory::new();
    let mut killed = Peer::spawn(&root, "killed", None);
    wait_until(
        || root.lines().iter().any(|line| line == "request killed"),
        "first request did not arrive",
    );
    killed.0.kill().unwrap();
    killed.0.wait().unwrap();
    assert!(
        root.runtime().join("client.sock").exists(),
        "SIGKILL should leave the socket path"
    );
    let mut replacement = Peer::spawn(&root, "replacement", None);
    wait_until(
        || {
            root.lines()
                .iter()
                .any(|line| line == "request replacement")
        },
        "replacement did not recover the socket",
    );
    assert_eq!(
        root.lines()
            .iter()
            .filter(|line| line.starts_with("primary "))
            .count(),
        2
    );
    replacement.finish();
}

#[test]
fn unsafe_paths_are_rejected_without_deleting_or_rewriting_user_files() {
    let root = Directory::new();
    let public = root.0.join("public");
    fs::create_dir(&public).unwrap();
    fs::set_permissions(&public, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(matches!(
        start(ForwardRecipient::Client, Request::Show, &public),
        Err(ActivationError::UnsafeRuntime)
    ));

    let private = root.runtime();
    fs::create_dir(&private).unwrap();
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
    let sentinel = root.0.join("keep");
    fs::write(&sentinel, b"private-user-content").unwrap();
    let alias = root.0.join("alias");
    symlink(&private, &alias).unwrap();
    assert!(matches!(
        start(ForwardRecipient::Client, Request::Show, &alias),
        Err(ActivationError::UnsafeRuntime)
    ));
    for name in ["client.lock", "client.sock", ".c.new"] {
        let path = private.join(name);
        symlink(&sentinel, &path).unwrap();
        assert!(matches!(
            start(ForwardRecipient::Client, Request::Show, &private),
            Err(ActivationError::UnsafeRuntime)
        ));
        assert!(
            fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        fs::remove_file(path).unwrap();
        assert_eq!(fs::read(&sentinel).unwrap(), b"private-user-content");
    }
    let socket = private.join("client.sock");
    fs::write(&socket, b"not-a-socket").unwrap();
    assert!(matches!(
        start(ForwardRecipient::Client, Request::Show, &private),
        Err(ActivationError::UnsafeRuntime)
    ));
    assert_eq!(fs::read(&socket).unwrap(), b"not-a-socket");
    fs::remove_file(socket).unwrap();
    let staging = private.join(".c.new");
    fs::write(&staging, b"not-a-staging-socket").unwrap();
    assert!(matches!(
        start(ForwardRecipient::Client, Request::Show, &private),
        Err(ActivationError::UnsafeRuntime)
    ));
    assert_eq!(fs::read(&staging).unwrap(), b"not-a-staging-socket");
    fs::remove_file(staging).unwrap();
    let lock = private.join("client.lock");
    fs::set_permissions(&lock, fs::Permissions::from_mode(0o666)).unwrap();
    assert!(matches!(
        start(ForwardRecipient::Client, Request::Show, &private),
        Err(ActivationError::UnsafeRuntime)
    ));
}

#[test]
fn only_a_private_endpoint_is_published_and_owned_abandoned_staging_is_recovered() {
    let root = Directory::new();
    let runtime = root.runtime();
    fs::create_dir(&runtime).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    let staging = runtime.join(".c.new");
    let abandoned = UnixListener::bind(&staging).unwrap();
    // Models termination between bind and the private chmod/atomic publication.
    fs::set_permissions(&staging, fs::Permissions::from_mode(0o777)).unwrap();
    drop(abandoned);

    let primary = primary(&runtime);
    assert!(!staging.exists());
    let user = rustix::process::geteuid().as_raw();
    for (path, mode) in [
        (runtime.clone(), 0o700),
        (runtime.join("client.sock"), 0o600),
        (runtime.join("client.lock"), 0o600),
    ] {
        let metadata = fs::symlink_metadata(path).unwrap();
        assert_eq!(metadata.uid(), user);
        assert_eq!(metadata.permissions().mode() & 0o777, mode);
    }
    drop(primary);
    assert!(!runtime.join("client.sock").exists());
}

#[test]
fn malformed_oversized_and_wrong_target_frames_never_reach_the_inbox() {
    let root = Directory::new();
    let primary = primary(&root.runtime());
    let mut inbox = primary.inbox().take().unwrap();
    assert!(matches!(
        inbox.blocking_recv().unwrap().request,
        Request::Show
    ));
    for bytes in [
        b"not json".to_vec(),
        serde_json::to_vec(&json!({"version":2,"recipient":"client","request":{"type":"show"}}))
            .unwrap(),
        serde_json::to_vec(&json!({"version":1,"recipient":"server","request":{"type":"show"}}))
            .unwrap(),
    ] {
        let mut stream = connect(&root.runtime());
        send_frame(&mut stream, &bytes);
        assert_eq!(read_reply(&mut stream), Err(ActivationError::Rejected));
    }
    let mut wrong = connect(&root.runtime());
    send_frame(&mut wrong, &serde_json::to_vec(&json!({"version":1,"recipient":"client","request":{"type":"forward","url":"https://server.acheflow.com/fwd/example/open"}})).unwrap());
    assert_eq!(read_reply(&mut wrong), Err(ActivationError::InvalidLink));
    let mut oversized = connect(&root.runtime());
    oversized.write_all(&u32::MAX.to_be_bytes()).unwrap();
    assert_eq!(read_reply(&mut oversized), Err(ActivationError::Rejected));
    assert!(matches!(
        inbox.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
}

#[test]
fn stalled_peers_are_bounded_and_pending_requests_are_revoked_on_shutdown() {
    let root = Directory::new();
    let primary = primary(&root.runtime());
    let mut inbox = primary.inbox().take().unwrap();
    inbox.blocking_recv().unwrap().complete(Ok(()));
    let mut peers = Vec::new();
    for n in 0..16 {
        let mut stream = connect(&root.runtime());
        send_frame(&mut stream, &wire(&format!("held{n}")));
        peers.push(stream);
    }
    wait_until(|| inbox.len() == 16, "held requests did not arrive");
    assert!(inbox.len() <= INBOX_CAPACITY);
    let mut excess = connect(&root.runtime());
    send_frame(&mut excess, &wire("excess"));
    assert_eq!(read_reply(&mut excess), Err(ActivationError::Busy));
    let held = inbox.blocking_recv().unwrap();
    assert!(held.is_live());
    drop(primary);
    assert!(
        !held.is_live(),
        "shutdown must revoke a delivery that has not run"
    );
    held.complete(Ok(()));
    assert!(!root.runtime().join("client.sock").exists());
}

#[test]
fn expired_and_dropped_delivery_replies_never_report_success() {
    let root = Directory::new();
    let primary = primary(&root.runtime());
    let mut inbox = primary.inbox().take().unwrap();
    inbox.blocking_recv().unwrap().complete(Ok(()));

    let mut dropped = connect(&root.runtime());
    send_frame(&mut dropped, &wire("dropped"));
    drop(inbox.blocking_recv().unwrap());
    assert_eq!(
        read_reply(&mut dropped),
        Err(ActivationError::OutcomeUnknown)
    );

    let mut expired = connect(&root.runtime());
    send_frame(&mut expired, &wire("expired"));
    let activation = inbox.blocking_recv().unwrap();
    std::thread::sleep(HANDOFF_TIMEOUT + Duration::from_millis(100));
    assert!(!activation.is_live());
    activation.complete(Ok(()));
    assert_eq!(
        read_reply(&mut expired),
        Err(ActivationError::OutcomeUnknown)
    );
}
