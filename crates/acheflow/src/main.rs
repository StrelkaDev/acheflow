//! One production executable; server mode never initializes the desktop engine.
use acheflow_extension_api::forwarding::ForwardRecipient;
use acheflow_launcher::{ActivationError, Command, Request};
use std::{ffi::OsString, process::ExitCode};

#[cfg(any(target_os = "macos", test))]
mod bootstrap;
#[cfg(target_os = "macos")]
mod delivery;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod native_events;

fn run() -> Result<(), String> {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    #[cfg(target_os = "macos")]
    let os_launch = args.is_empty();
    if args.first().is_some_and(|arg| arg == "--server") {
        args.remove(0);
        return server(args, None);
    }
    let client_only = args.first().is_some_and(|arg| arg == "--client");
    if client_only {
        args.remove(0);
    }
    let request = match acheflow_launcher::parse_args(args).map_err(|e| e.to_string())? {
        Command::Help => {
            println!(
                "Acheflow\n  acheflow [--client] [--show]\n  acheflow --server [server options]\n  acheflow --forward <URI or HTTPS URL>\n  acheflow --forward-stdin\n  acheflow --version\n\nURI recipients: acheflow://client/fwd/<extension>/<action>/...\n                acheflow://server/fwd/<extension>/<action>/...\nServer options: acheflow --server --help"
            );
            return Ok(());
        }
        Command::Version => {
            println!("Acheflow {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Command::Run(request) => request,
        Command::ReadStdin => {
            acheflow_launcher::read_link(std::io::stdin().lock()).map_err(|e| e.to_string())?
        }
    };
    let (recipient, request) = request.route().map_err(|e| e.to_string())?;
    match recipient {
        ForwardRecipient::Client => {
            #[cfg(target_os = "macos")]
            {
                macos::run(request, client_only, os_launch)
            }
            #[cfg(not(target_os = "macos"))]
            {
                acheflow_ui::launch_native_request(request)
            }
        }
        ForwardRecipient::Server if client_only => Err(ActivationError::InvalidLink.to_string()),
        ForwardRecipient::Server => server(vec![], Some(request)),
    }
}

fn server(args: Vec<OsString>, request: Option<Request>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let args = if request.is_some() {
        macos::activation_server_args()?
    } else {
        args
    };
    let args = [OsString::from("acheflow-server")].into_iter().chain(args);
    acheflow_server::run_from(args, request).map_err(|e| e.to_string())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
