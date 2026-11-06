use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn help_and_versions_do_not_initialize_native_receivers_or_gui() {
    let root = std::env::temp_dir().join(format!("dl13-cli-{}", std::process::id()));
    assert!(!root.exists());
    for args in [
        vec!["--help"],
        vec!["--version"],
        vec!["--client", "--version"],
        vec!["--server", "--version"],
        vec!["--server", "--help"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_acheflow"))
            .args(args)
            .env("ACHEFLOW_RUNTIME_DIR", &root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!root.exists());
    }
}

#[test]
fn invalid_and_wrong_recipient_links_fail_without_exposing_callback_payloads() {
    for (args, url) in [
        (
            vec!["--forward-stdin"],
            "acheflow-client://fwd/example/open?code=private-marker",
        ),
        (
            vec!["--forward-stdin"],
            "https://server.acheflow.com/fwd/example/open?code=private-marker",
        ),
        (
            vec!["--forward-stdin"],
            "acheflow://client/fwd/example/open/%2Fsecret?code=private-marker",
        ),
        (
            vec!["--client", "--forward-stdin"],
            "acheflow://server/fwd/example/open?code=private-marker",
        ),
        (
            vec!["--server", "--forward-stdin"],
            "acheflow://client/fwd/example/oauth/callback?code=private-marker",
        ),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_acheflow"))
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(url.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("private-marker"));
        assert!(!String::from_utf8_lossy(&output.stdout).contains("private-marker"));
    }
}
