//! Manual subprocess fixture for native activation; it never launches a UI.
#[cfg(unix)]
#[path = "../tests/support/peer.rs"]
mod peer;

#[cfg(unix)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = args.next().ok_or("missing root")?.into();
    let label = args.next().ok_or("missing label")?;
    let log = args.next().ok_or("missing log")?.into();
    let gate = args.next().map(Into::into);
    peer::run(root, label, log, gate)
}

#[cfg(not(unix))]
fn main() {}
