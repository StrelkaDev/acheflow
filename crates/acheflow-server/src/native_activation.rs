//! Explicit same-user native ingress; all effects pass through control commands.
use crate::control::ControlHost;
#[cfg(any(unix, test))]
use crate::control::now_ms;
use acheflow_extension_api::forwarding::ForwardRecipient;
#[cfg(any(unix, test))]
use acheflow_extension_api::forwarding::{ForwardLink, ForwardMode};
use acheflow_launcher::{ActivationError, Request};
#[cfg(any(unix, test))]
use acheflow_media_api::control::{ControlAction, ControlRequest};
use std::sync::Arc;

pub enum Launch {
    Serve(Option<Receiver>),
    #[cfg(unix)]
    HandedOff,
}
pub struct Receiver {
    #[cfg(unix)]
    primary: acheflow_launcher::Primary,
}

pub async fn prepare(args: &crate::settings::Args) -> Result<Launch, ActivationError> {
    if !args.native_links && !args.forward_stdin {
        return Ok(Launch::Serve(None));
    }
    let request = if args.forward_stdin {
        acheflow_launcher::read_link(std::io::stdin().lock())?
    } else {
        Request::Show
    };
    let request = request.normalize(ForwardRecipient::Server)?;
    #[cfg(unix)]
    {
        let directory = acheflow_launcher::default_directory()?;
        let startup = tokio::task::spawn_blocking(move || {
            acheflow_launcher::start(ForwardRecipient::Server, request, &directory)
        })
        .await
        .map_err(|_| ActivationError::Unavailable)??;
        match startup {
            acheflow_launcher::Startup::Primary(primary) => {
                Ok(Launch::Serve(Some(Receiver { primary })))
            }
            acheflow_launcher::Startup::HandedOff => Ok(Launch::HandedOff),
        }
    }
    #[cfg(not(unix))]
    Err(ActivationError::Unsupported)
}

impl Receiver {
    pub async fn run(self, host: Arc<ControlHost>) {
        #[cfg(unix)]
        {
            let Some(mut inbox) = self.primary.inbox().take() else {
                return;
            };
            loop {
                let activation = tokio::select! {
                    _ = host.runtime.shutdown.cancelled() => break,
                    request = inbox.recv() => match request {
                        Some(request) => request,
                        None => break,
                    },
                };
                let result = match &activation.request {
                    _ if !activation.is_live() => Err(ActivationError::Expired),
                    Request::Show => Ok(()),
                    Request::Forward { url } => tokio::select! {
                        _ = host.runtime.shutdown.cancelled() => Err(ActivationError::OutcomeUnknown),
                        result = dispatch(&host, url, || {
                            activation.is_live() && !host.runtime.shutdown.is_cancelled()
                        }) => result,
                    },
                };
                if let Err(error) = result {
                    // Fixed host diagnostics never include provider payloads.
                    eprintln!("Native server link: {error}");
                }
                activation.complete(result);
            }
        }
        #[cfg(not(unix))]
        let _ = host;
    }
}

#[cfg(any(unix, test))]
pub(super) async fn dispatch(
    host: &Arc<ControlHost>,
    url: &str,
    is_live: impl Fn() -> bool + Send + Sync,
) -> Result<(), ActivationError> {
    if !is_live() {
        return Err(ActivationError::Expired);
    }
    let request = request(host, url)?;
    host.execute_while(request, &is_live)
        .await
        .map(|_| ())
        .map_err(|_| {
            if is_live() {
                ActivationError::Rejected
            } else {
                ActivationError::Expired
            }
        })
}

#[cfg(any(unix, test))]
fn request(host: &ControlHost, url: &str) -> Result<ControlRequest, ActivationError> {
    let link = ForwardLink::parse_for(url, ForwardRecipient::Server)
        .map_err(|_| ActivationError::InvalidLink)?;
    let installed = host
        .installed
        .get(&link.extension_id)
        .ok_or(ActivationError::NotFound)?;
    if link
        .declaration(&installed.manifest)
        .map_err(|_| ActivationError::Rejected)?
        .mode
        != ForwardMode::Event
    {
        return Err(ActivationError::Rejected);
    }
    let state = host.snapshot(0, 1);
    let workspace_id = state.workspace_id.ok_or(ActivationError::NotReady)?;
    if !state
        .extension_settings
        .get(&link.extension_id)
        .is_some_and(|settings| settings.enabled)
    {
        return Err(ActivationError::Disabled);
    }
    let instance_id = state
        .extensions
        .iter()
        .find(|status| status.id == link.extension_id)
        .and_then(|status| status.instance_id.clone())
        .ok_or(ActivationError::NotReady)?;
    Ok(ControlRequest {
        workspace_id,
        expected_revision: state.revision,
        request_id: format!(
            "{}:{}",
            now_ms(),
            acheflow_media_api::workspace::new_token()
        ),
        action: ControlAction::ForwardExtension {
            url: url.into(),
            instance_id,
        },
    })
}
