//! Launch selection precedes native surfaces, UI state and owned media workers.
use acheflow_extension_api::forwarding::ForwardRecipient;
use acheflow_launcher::{ActivationError, Request};

pub struct Selection {
    pub recipient: ForwardRecipient,
    pub request: Request,
}

pub struct Bootstrap {
    initial: Option<Request>,
    client_only: bool,
    waiting_for_activation: bool,
}

impl Bootstrap {
    pub fn new(initial: Request, client_only: bool) -> Self {
        Self {
            initial: Some(initial),
            client_only,
            waiting_for_activation: true,
        }
    }

    /// A nondefault OS launch must supply its activation before a UI is created.
    /// This also handles platforms delivering the URI after launch completion.
    pub fn launch_finished(&mut self, default_launch: bool) {
        self.waiting_for_activation = !default_launch;
    }

    pub fn activate(&mut self, request: Request) -> Result<Selection, ActivationError> {
        // AppKit may request an untitled window even when explicit CLI input
        // already carries a client URL. A fallback Show must not discard it.
        let request = match (&request, &self.initial) {
            (Request::Show, Some(initial @ Request::Forward { .. })) => initial.clone(),
            _ => request,
        };
        let (recipient, request) = self.route(request)?;
        self.initial = None;
        Ok(Selection { recipient, request })
    }

    pub fn route(&self, request: Request) -> Result<(ForwardRecipient, Request), ActivationError> {
        let (recipient, request) = request.route()?;
        if self.client_only && recipient != ForwardRecipient::Client {
            return Err(ActivationError::InvalidLink);
        }
        Ok((recipient, request))
    }

    pub fn default_selection(&mut self) -> Result<Option<Selection>, ActivationError> {
        if self.waiting_for_activation {
            return Ok(None);
        }
        let Some(request) = self.initial.take() else {
            return Ok(None);
        };
        self.activate(request).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nondefault_launch_waits_for_a_late_server_uri_without_selecting_ui() {
        let mut bootstrap = Bootstrap::new(Request::Show, false);
        assert!(bootstrap.default_selection().unwrap().is_none());
        bootstrap.launch_finished(false);
        assert!(bootstrap.default_selection().unwrap().is_none());
        let selection = bootstrap
            .activate(Request::Forward {
                url: "acheflow://server/fwd/example/open?value=a%2Bb".into(),
            })
            .unwrap();
        assert_eq!(selection.recipient, ForwardRecipient::Server);
        assert!(bootstrap.default_selection().unwrap().is_none());
    }

    #[test]
    fn default_launch_selects_client_once_and_explicit_client_rejects_server() {
        let mut bootstrap = Bootstrap::new(Request::Show, true);
        assert!(
            bootstrap
                .activate(Request::Forward {
                    url: "acheflow://server/fwd/example/open".into()
                })
                .is_err()
        );
        bootstrap.launch_finished(true);
        assert_eq!(
            bootstrap.default_selection().unwrap().unwrap().recipient,
            ForwardRecipient::Client
        );
        assert!(bootstrap.default_selection().unwrap().is_none());
    }

    #[test]
    fn explicit_client_guard_also_applies_after_startup() {
        let mut bootstrap = Bootstrap::new(Request::Show, true);
        bootstrap.launch_finished(true);
        bootstrap.default_selection().unwrap().unwrap();
        assert!(
            bootstrap
                .route(Request::Forward {
                    url: "acheflow://server/fwd/example/open".into(),
                })
                .is_err()
        );
        assert!(
            bootstrap
                .route(Request::Forward {
                    url: "acheflow://client/fwd/example/open".into(),
                })
                .is_ok()
        );
    }

    #[test]
    fn native_untitled_or_reopen_event_can_resolve_a_nondefault_launch() {
        let mut bootstrap = Bootstrap::new(Request::Show, false);
        bootstrap.launch_finished(false);
        assert!(bootstrap.default_selection().unwrap().is_none());
        assert_eq!(
            bootstrap.activate(Request::Show).unwrap().recipient,
            ForwardRecipient::Client
        );
    }

    #[test]
    fn untitled_event_preserves_the_explicit_initial_client_url() {
        let mut bootstrap = Bootstrap::new(
            Request::Forward {
                url: "acheflow://client/fwd/example/open?value=kept".into(),
            },
            false,
        );
        bootstrap.launch_finished(true);
        let selection = bootstrap.activate(Request::Show).unwrap();
        let Request::Forward { url } = selection.request else {
            panic!("Initial URL was discarded")
        };
        assert_eq!(url, "https://app.acheflow.com/fwd/example/open?value=kept");
        assert!(bootstrap.default_selection().unwrap().is_none());
    }

    #[test]
    fn initial_uri_selects_its_recipient_and_invalid_links_do_not_consume_startup() {
        let mut bootstrap = Bootstrap::new(
            Request::Forward {
                url: "acheflow://client/fwd/example/open".into(),
            },
            false,
        );
        assert!(
            bootstrap
                .activate(Request::Forward {
                    url: "acheflow://unknown/fwd/example/open".into()
                })
                .is_err()
        );
        bootstrap.launch_finished(true);
        assert_eq!(
            bootstrap.default_selection().unwrap().unwrap().recipient,
            ForwardRecipient::Client
        );
    }
}
