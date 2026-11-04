//! Minimal desktop URI adapter. Payloads never enter a shell or child arguments.
use acheflow_extension_api::forwarding::ForwardRecipient;
use acheflow_launcher::{ActivationError, Request};
use std::{
    io::Write,
    process::{Command, ExitCode, Stdio},
};

fn run() -> Result<(), ActivationError> {
    let args = std::env::args_os()
        .skip(1)
        .map(|argument| {
            argument
                .into_string()
                .map_err(|_| ActivationError::InvalidArguments)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (recipient, executable) = match args.first().map(String::as_str) {
        Some("client") => (ForwardRecipient::Client, "acheflow-ui"),
        Some("server") => (ForwardRecipient::Server, "acheflow-server"),
        _ => return Err(ActivationError::InvalidArguments),
    };
    if args.len() != 2 {
        return Err(ActivationError::InvalidArguments);
    }
    let Request::Forward { url } = (Request::Forward {
        url: args[1].clone(),
    })
    .normalize(recipient)?
    else {
        return Err(ActivationError::InvalidArguments);
    };
    let executable = std::env::current_exe()
        .map_err(|_| ActivationError::Unavailable)?
        .parent()
        .ok_or(ActivationError::Unavailable)?
        .join(executable);
    let mut child = Command::new(executable)
        .arg("--forward-stdin")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ActivationError::Unavailable)?;
    let result = child
        .stdin
        .take()
        .ok_or(ActivationError::Unavailable)
        .and_then(|mut input| {
            input
                .write_all(url.as_bytes())
                .map_err(|_| ActivationError::Unavailable)
        });
    if let Err(error) = result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    if child
        .wait()
        .map_err(|_| ActivationError::OutcomeUnknown)?
        .success()
    {
        Ok(())
    } else {
        Err(ActivationError::Rejected)
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(ActivationError::InvalidArguments) => {
            eprintln!("Use acheflow-link <client|server> <forwarding URL>.");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("Acheflow link: {error}");
            ExitCode::FAILURE
        }
    }
}
