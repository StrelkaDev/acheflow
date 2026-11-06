//! Native activation transport. No provider credentials or OAuth policy live here.
use acheflow_extension_api::forwarding::{ForwardLink, ForwardRecipient, MAX_FORWARD_URL_BYTES};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fmt,
    io::Read,
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::{Primary, Startup, default_directory, start};

pub const HANDOFF_TIMEOUT: Duration = Duration::from_secs(5);
pub const INBOX_CAPACITY: usize = 32;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Show,
    Forward { url: String },
}
impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Show => "Show",
            Self::Forward { .. } => "Forward(<redacted>)",
        })
    }
}
impl Request {
    /// Normalize OS-only scheme aliases before entering the canonical IPC API.
    pub fn normalize(self, recipient: ForwardRecipient) -> Result<Self, ActivationError> {
        let Self::Forward { url } = self else {
            return Ok(self);
        };
        if url.len() > MAX_FORWARD_URL_BYTES {
            return Err(ActivationError::InvalidLink);
        }
        let scheme = match recipient {
            ForwardRecipient::Client => "acheflow-client://",
            ForwardRecipient::Server => "acheflow-server://",
        };
        let url = match url.strip_prefix(scheme) {
            Some(route) => format!("{}/{route}", recipient.origin()),
            None => url,
        };
        let request = Self::Forward { url };
        request.validate(recipient)?;
        Ok(request)
    }
    pub fn validate(&self, recipient: ForwardRecipient) -> Result<(), ActivationError> {
        if let Self::Forward { url } = self {
            ForwardLink::parse_for(url, recipient).map_err(|_| ActivationError::InvalidLink)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationError {
    InvalidArguments,
    InvalidLink,
    UnsafeRuntime,
    Unavailable,
    Busy,
    Expired,
    NotFound,
    Disabled,
    NotReady,
    Rejected,
    OutcomeUnknown,
    Unsupported,
}
impl fmt::Display for ActivationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidArguments => {
                "Use --forward <URL>, --forward-stdin, --show, --help or --version."
            }
            Self::InvalidLink => "Invalid Acheflow forwarding address for this application.",
            Self::UnsafeRuntime => {
                "The native activation directory or endpoint is not private and owned by this user."
            }
            Self::Unavailable => "The running Acheflow instance is unavailable.",
            Self::Busy => "Acheflow's activation inbox is full; try again.",
            Self::Expired => "The activation request expired before delivery.",
            Self::NotFound => "The extension is missing or has conflicting installation IDs.",
            Self::Disabled => "The extension is disabled in the active workspace.",
            Self::NotReady => "The extension is not ready in the active workspace.",
            Self::Rejected => "The extension rejected this link; see Acheflow for details.",
            Self::OutcomeUnknown => {
                "Acheflow did not confirm delivery; the request may have been received."
            }
            Self::Unsupported => "Native activation is not supported on this platform yet.",
        })
    }
}
impl std::error::Error for ActivationError {}

/// An activation is never Debug/Serialize: it may contain a callback credential.
pub struct Activation {
    pub request: Request,
    reply: Option<oneshot::Sender<Result<(), ActivationError>>>,
    deadline: Instant,
}
impl Activation {
    pub fn is_live(&self) -> bool {
        Instant::now() < self.deadline && self.reply.as_ref().is_none_or(|r| !r.is_closed())
    }
    pub fn complete(self, result: Result<(), ActivationError>) {
        if let Some(reply) = self.reply {
            let _ = reply.send(result);
        }
    }
    #[cfg(unix)]
    fn initial(request: Request) -> Self {
        Self {
            request,
            reply: None,
            deadline: Instant::now() + Duration::from_secs(30),
        }
    }
}

pub struct Inbox(Mutex<Option<mpsc::Receiver<Activation>>>);
impl Inbox {
    pub fn take(&self) -> Option<mpsc::Receiver<Activation>> {
        self.0.lock().ok()?.take()
    }
}

#[derive(Debug)]
pub enum Command {
    Run(Request),
    ReadStdin,
    Help,
    Version,
}
pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Command, ActivationError> {
    let args = args
        .into_iter()
        .map(|a| {
            a.into_string()
                .map_err(|_| ActivationError::InvalidArguments)
        })
        .collect::<Result<Vec<_>, _>>()?;
    match args.as_slice() {
        [] => Ok(Command::Run(Request::Show)),
        [flag] if flag == "--show" => Ok(Command::Run(Request::Show)),
        [flag] if flag == "--help" || flag == "-h" => Ok(Command::Help),
        [flag] if flag == "--version" || flag == "-V" => Ok(Command::Version),
        [flag] if flag == "--forward-stdin" => Ok(Command::ReadStdin),
        [flag, url] if flag == "--forward" => {
            Ok(Command::Run(Request::Forward { url: url.clone() }))
        }
        [url] if url.starts_with("https://") || url.starts_with("acheflow-") => {
            Ok(Command::Run(Request::Forward { url: url.clone() }))
        }
        _ => Err(ActivationError::InvalidArguments),
    }
}
pub fn read_link(reader: impl Read) -> Result<Request, ActivationError> {
    let mut bytes = vec![];
    reader
        .take(MAX_FORWARD_URL_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ActivationError::InvalidLink)?;
    if bytes.len() > MAX_FORWARD_URL_BYTES {
        return Err(ActivationError::InvalidLink);
    }
    let url = String::from_utf8(bytes).map_err(|_| ActivationError::InvalidLink)?;
    Ok(Request::Forward { url })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn os_schemes_preserve_payloads_and_cannot_change_recipient_or_authority() {
        for (recipient, scheme) in [
            (ForwardRecipient::Client, "acheflow-client"),
            (ForwardRecipient::Server, "acheflow-server"),
        ] {
            let suffix = "fwd/example/oauth/callback?code=a%2Bb&state=x%3Dy#extra=opaque";
            let request = Request::Forward {
                url: format!("{scheme}://{suffix}"),
            }
            .normalize(recipient)
            .unwrap();
            let Request::Forward { url } = request else {
                panic!("expected forwarding");
            };
            assert_eq!(url, format!("{}/{suffix}", recipient.origin()));
            for invalid in [
                "acheflow-other://fwd/example/open",
                "acheflow-client://evil@fwd/example/open",
                "acheflow-client://fwd:80/example/open",
                "acheflow-client://fwd/example/open/../other",
                "acheflow-client://fwd/example/open/%2Fsecret",
                "acheflow-client://fwd/example/open?bad=%0a",
                "acheflow-client://fwd/example/open\n",
            ] {
                assert!(
                    Request::Forward {
                        url: invalid.into()
                    }
                    .normalize(recipient)
                    .is_err()
                );
            }
            let other = match recipient {
                ForwardRecipient::Client => "acheflow-server",
                ForwardRecipient::Server => "acheflow-client",
            };
            assert!(
                Request::Forward {
                    url: format!("{other}://{suffix}")
                }
                .normalize(recipient)
                .is_err()
            );
        }
    }
    #[test]
    fn arguments_and_stdin_preserve_the_sensitive_address_without_diagnostics() {
        let url = "https://app.acheflow.com/fwd/example/oauth/callback?code=private-value";
        let request = read_link(url.as_bytes()).unwrap();
        request.validate(ForwardRecipient::Client).unwrap();
        assert!(!format!("{request:?}").contains("private-value"));
        let command = parse_args([OsString::from("--forward"), url.into()]).unwrap();
        assert!(!format!("{command:?}").contains("private-value"));
        assert!(parse_args(["--forward".into(), url.into(), "extra".into()]).is_err());
        assert!(read_link(vec![b'a'; MAX_FORWARD_URL_BYTES + 1].as_slice()).is_err());
        assert!(read_link([255u8].as_slice()).is_err());
        assert!(
            read_link(format!("{url}\n").as_bytes())
                .unwrap()
                .validate(ForwardRecipient::Client)
                .is_err()
        );
    }
}
