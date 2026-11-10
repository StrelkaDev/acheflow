//! Real guest/activation boundaries, exercised without launching a browser.
use super::*;
use acheflow_extension_api::forwarding::ForwardAction;
use acheflow_launcher::{Primary, Startup};
use blitz_dom::{Document, DocumentConfig};
use dioxus_native::DioxusDocument;
use std::{
    cell::Cell,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context as TaskContext, Poll},
    time::{Duration, Instant},
};

struct Harness {
    provider: String,
    signals: Cell<Option<PanelSignals>>,
    notice: Cell<Option<Signal<String>>>,
    page: Cell<Option<Signal<&'static str>>>,
}
fn app() -> Element {
    let harness = use_context::<Rc<Harness>>();
    let provider = harness.provider.clone();
    let project = use_signal(|| {
        let mut project = crate::project::Workspace {
            id: "native-mailbox-workspace".into(),
            ..Default::default()
        };
        let settings = project
            .client_extensions
            .entry(provider.clone())
            .or_default();
        settings.enabled = true;
        settings
            .config
            .insert("client_id".into(), "public-id".into());
        project
    });
    let client = crate::control_client::ControlClient {
        project,
        state: use_signal(|| None),
        error: use_signal(|| None),
        extension_errors: use_signal(BTreeMap::new),
        busy: use_signal(|| false),
        sync_conflict: use_signal(|| false),
    };
    use_context_provider(|| Rc::new(acheflow_client::catalog::ClientCatalog::bundled()));
    let page = use_signal(|| "Overview");
    let notice = crate::native_activation::provide(project, page);
    let error = use_signal(String::new);
    let identity = use_memo(move || {
        let p = project.read();
        (
            p.id.clone(),
            p.server_url.clone(),
            p.client_extensions[&provider].config.clone(),
        )
    });
    let signals = PanelSignals {
        client,
        sessions: use_signal(Sessions::new),
        panel: use_signal(|| None),
        error,
        busy: use_signal(|| false),
        drafts: use_signal(BTreeMap::new),
        task: use_signal(|| None),
        restart: use_signal(|| 0),
        identity,
        native: use_signal(|| Mailbox::new(error)),
    };
    harness.signals.set(Some(signals));
    harness.notice.set(Some(notice));
    harness.page.set(Some(page));
    rsx! {
        crate::native_activation::Notice {value:notice}
        if let Some(panel) = signals.panel.read().as_ref() {
            for widget in panel.widgets.clone() {
                {render_widget(widget, harness.provider.clone(), signals, Callback::new(|_| {}))}
            }
        }
    }
}

struct Fixture {
    doc: DioxusDocument,
    harness: Rc<Harness>,
}
impl Fixture {
    fn new(provider: &str) -> Self {
        Self::with_inbox(provider, None)
    }
    fn with_inbox(provider: &str, inbox: Option<std::sync::Arc<acheflow_launcher::Inbox>>) -> Self {
        let harness = Rc::new(Harness {
            provider: provider.into(),
            signals: Cell::new(None),
            notice: Cell::new(None),
            page: Cell::new(None),
        });
        let mut vdom = VirtualDom::new(app).with_root_context(harness.clone());
        if let Some(inbox) = inbox {
            vdom = vdom.with_root_context(inbox);
        }
        let mut doc = DioxusDocument::new(vdom, DocumentConfig::default());
        doc.initial_build();
        let mut result = Self { doc, harness };
        result.settle();
        result
    }
    fn signals(&self) -> PanelSignals {
        self.harness.signals.get().unwrap()
    }
    fn scope<R>(&self, f: impl FnOnce() -> R) -> R {
        self.doc.vdom.in_scope(ScopeId::APP, f)
    }
    fn settle(&mut self) {
        for _ in 0..40 {
            if !self.doc.poll(None) {
                return;
            }
        }
        panic!("native mailbox fixture did not settle");
    }
    fn context(&self) -> Context {
        self.scope(|| {
            let signals = self.signals();
            Context::new(
                signals,
                signals.identity.peek().clone(),
                *signals.restart.peek(),
            )
        })
    }
    fn mailbox(&self) -> Mailbox {
        self.signals().native.peek().clone()
    }
    fn wait_peer(
        &mut self,
        peer: std::thread::JoinHandle<Result<Startup, ActivationError>>,
    ) -> Result<Startup, ActivationError> {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !peer.is_finished() {
            self.settle();
            assert!(
                Instant::now() < deadline,
                "the root activation router did not reply"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        self.settle();
        peer.join().unwrap()
    }
    fn session(&self) -> (LocalSession, String) {
        let catalog = acheflow_client::catalog::ClientCatalog::bundled();
        let installed = &catalog.installed[&self.harness.provider];
        let mut manifest = installed.manifest.clone();
        manifest.forwarding.push(ForwardAction {
            id: "open".into(),
            mode: ForwardMode::Event,
        });
        let owner = self.context().owner;
        let mut session = LocalSession::new(manifest, &installed.module, owner.2).unwrap();
        session.bind_workspace(&owner.0).unwrap();
        let state = connect(&mut session);
        (session, state)
    }
}

fn connect(session: &mut LocalSession) -> String {
    let environment =
        || json!({"browser":false,"callback_uri":"http://127.0.0.1:51234/oauth/callback"});
    session
        .step(json!({"type":"local_start"}), &environment(), now())
        .unwrap();
    if session.manifest.id == "youtube" {
        session
            .set_secret("client_secret", "fixture-local-secret".into())
            .unwrap();
    }
    let effects = session
        .step(
            json!({"type":"local_action","action_id":"connect"}),
            &environment(),
            now(),
        )
        .unwrap();
    let browser = effects.iter().find(|e| e["type"] == "browser").unwrap();
    url::Url::parse(browser["url"].as_str().unwrap())
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "state")
        .unwrap()
        .1
        .into_owned()
}
fn callback(provider: &str, state: &str) -> String {
    format!(
        "https://app.acheflow.com/fwd/{provider}/oauth/callback?state={state}&code=native-code-marker&access_token=native-token-marker&token_type=bearer&expires_in=3600"
    )
}

struct Source {
    primary: Option<Primary>,
    receiver: Option<mpsc::Receiver<Activation>>,
    path: PathBuf,
}
impl Source {
    fn new(request: Request) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let base = if cfg!(target_os = "macos") {
            PathBuf::from("/private/tmp")
        } else {
            std::env::temp_dir()
        };
        let path = base.join(format!(
            "af-ui-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let Startup::Primary(primary) =
            acheflow_launcher::start(ForwardRecipient::Client, request, &path).unwrap()
        else {
            panic!("fresh test runtime unexpectedly handed off");
        };
        Self {
            primary: Some(primary),
            receiver: None,
            path,
        }
    }
    fn inbox(&self) -> std::sync::Arc<acheflow_launcher::Inbox> {
        self.primary.as_ref().unwrap().inbox()
    }
    fn next(&mut self) -> Activation {
        if self.receiver.is_none() {
            self.receiver = self.inbox().take();
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Ok(request) = self.receiver.as_mut().unwrap().try_recv() {
                return request;
            }
            assert!(Instant::now() < deadline, "activation did not arrive");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn peer(&self, address: String) -> std::thread::JoinHandle<Result<Startup, ActivationError>> {
        let path = self.path.clone();
        std::thread::spawn(move || {
            acheflow_launcher::start(
                ForwardRecipient::Client,
                Request::Forward { url: address },
                &path,
            )
        })
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        self.primary.take();
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn rejected_request_does_not_strand_a_following_public_event() {
    let fixture = Fixture::new("youtube");
    let (session, _) = fixture.session();
    let context = fixture.context();
    let mailbox = fixture.mailbox();
    mailbox.activate(&context, session.instance_id());
    let mut bad = Source::new(Request::Forward {
        url: callback("youtube", "wrong-state"),
    });
    let mut good = Source::new(Request::Forward {
        url: "https://app.acheflow.com/fwd/youtube/open?public=event".into(),
    });
    mailbox.enqueue(bad.next(), context.owner.clone(), context.epoch);
    mailbox.enqueue(good.next(), context.owner.clone(), context.epoch);
    let mut future = Box::pin(drive(session, vec![], mailbox.clone(), context));
    let result = fixture.scope(|| {
        let mut cx = TaskContext::from_waker(futures_util::task::noop_waker_ref());
        future.as_mut().poll(&mut cx)
    });
    let Poll::Ready(Ok((_session, panel))) = result else {
        panic!("queued local events should finish without browser or network I/O");
    };
    assert!(
        panel.is_some(),
        "the valid Event after a rejected callback was stranded"
    );
}

fn route(mailbox: &Mailbox, activation: Activation, context: &Context) -> Routed {
    mailbox.enqueue(activation, context.owner.clone(), context.epoch);
    mailbox.0.receiver.try_lock().unwrap().try_recv().unwrap()
}

fn assert_provider_effect(provider: &str, effects: &[Value]) {
    let id = if provider == "youtube" {
        "auth:token"
    } else {
        "auth:validate"
    };
    assert!(
        effects
            .iter()
            .any(|effect| effect["type"] == "http" && effect["id"] == id)
    );
    let output = serde_json::to_string(effects).unwrap();
    assert!(!output.contains("native-code-marker"));
    assert!(!output.contains("native-token-marker"));
}

#[test]
fn two_real_guests_accept_only_their_queued_callback_and_acknowledge_the_sender() {
    for provider in ["twitch", "youtube"] {
        let fixture = Fixture::new(provider);
        let (mut session, state) = fixture.session();
        let context = fixture.context();
        let mailbox = fixture.mailbox();
        mailbox.activate(&context, session.instance_id());
        let mut source = Source::new(Request::Show);
        source.next().complete(Ok(()));
        let peer = source.peer(callback(provider, &state));
        let routed = route(&mailbox, source.next(), &context);
        let (effects, accepted_callback) =
            fixture.scope(|| mailbox.deliver(routed, &mut session, &context));
        assert!(accepted_callback);
        assert_provider_effect(provider, &effects);
        assert!(matches!(peer.join().unwrap(), Ok(Startup::HandedOff)));
    }
}

#[test]
fn a_rejected_nonce_keeps_the_legitimate_pending_request_and_recovers_its_notice() {
    for provider in ["twitch", "youtube"] {
        let fixture = Fixture::new(provider);
        let (mut session, state) = fixture.session();
        let context = fixture.context();
        let mailbox = fixture.mailbox();
        mailbox.activate(&context, session.instance_id());
        let mut source = Source::new(Request::Show);
        source.next().complete(Ok(()));
        let wrong = source.peer(callback(provider, "unrelated-request"));
        let routed = route(&mailbox, source.next(), &context);
        let (effects, accepted) = fixture.scope(|| mailbox.deliver(routed, &mut session, &context));
        assert!(!accepted && effects.is_empty());
        assert!(matches!(
            wrong.join().unwrap(),
            Err(ActivationError::Rejected)
        ));
        assert!(!fixture.signals().error.peek().is_empty());

        let correct = source.peer(callback(provider, &state));
        let routed = route(&mailbox, source.next(), &context);
        let (effects, accepted) = fixture.scope(|| mailbox.deliver(routed, &mut session, &context));
        assert!(accepted);
        assert_provider_effect(provider, &effects);
        assert!(fixture.signals().error.peek().is_empty());
        assert!(matches!(correct.join().unwrap(), Ok(Startup::HandedOff)));
    }
}

#[test]
fn queued_requests_cannot_migrate_to_another_workspace_epoch_or_instance() {
    for change in ["workspace", "epoch", "instance"] {
        let mut fixture = Fixture::new("youtube");
        let (mut session, old_state) = fixture.session();
        let context = fixture.context();
        let mailbox = fixture.mailbox();
        mailbox.activate(&context, session.instance_id());
        let mut source = Source::new(Request::Show);
        source.next().complete(Ok(()));
        let stale = source.peer(callback("youtube", &old_state));
        let routed = route(&mailbox, source.next(), &context);

        match change {
            "workspace" => {
                let mut project = fixture.signals().client.project;
                fixture.scope(|| project.write().id = "replacement-workspace".into());
                fixture.settle();
                session = fixture.session().0;
            }
            "epoch" => {
                let mut restart = fixture.signals().restart;
                fixture.scope(|| restart.set(1));
                fixture.settle();
                connect(&mut session);
            }
            "instance" => {
                connect(&mut session);
            }
            _ => unreachable!(),
        }
        let current = fixture.context();
        mailbox.activate(&current, session.instance_id());
        let (effects, accepted) = fixture.scope(|| mailbox.deliver(routed, &mut session, &current));
        assert!(
            !accepted && effects.is_empty(),
            "stale {change} received guest work"
        );
        assert!(matches!(
            stale.join().unwrap(),
            Err(ActivationError::NotReady)
        ));

        let fresh_state = connect(&mut session);
        mailbox.activate(&current, session.instance_id());
        let fresh = source.peer(callback("youtube", &fresh_state));
        let routed = route(&mailbox, source.next(), &current);
        let (effects, accepted) = fixture.scope(|| mailbox.deliver(routed, &mut session, &current));
        assert!(accepted);
        assert_provider_effect("youtube", &effects);
        assert!(matches!(fresh.join().unwrap(), Ok(Startup::HandedOff)));
    }
}

#[test]
fn disabled_live_context_refuses_delivery_before_invoking_the_guest() {
    let mut fixture = Fixture::new("youtube");
    let (mut session, state) = fixture.session();
    let context = fixture.context();
    let mailbox = fixture.mailbox();
    mailbox.activate(&context, session.instance_id());
    let mut source = Source::new(Request::Show);
    source.next().complete(Ok(()));
    let peer = source.peer(callback("youtube", &state));
    let routed = route(&mailbox, source.next(), &context);
    let mut project = fixture.signals().client.project;
    fixture.scope(|| {
        project
            .write()
            .client_extensions
            .get_mut("youtube")
            .unwrap()
            .enabled = false
    });
    fixture.settle();
    let (effects, accepted) = fixture.scope(|| mailbox.deliver(routed, &mut session, &context));
    assert!(!accepted && effects.is_empty());
    assert!(matches!(
        peer.join().unwrap(),
        Err(ActivationError::NotReady)
    ));
}

#[test]
fn replacing_a_route_then_dropping_its_old_lease_keeps_the_new_recipient() {
    let source = Source::new(Request::Show);
    let mut fixture = Fixture::with_inbox("youtube", Some(source.inbox()));
    let routes = fixture.scope(consume_context::<crate::native_activation::NativeRoutes>);
    let old_calls = Rc::new(Cell::new(0));
    let new_calls = Rc::new(Cell::new(0));
    let old = fixture.scope(|| {
        let calls = old_calls.clone();
        routes.register(
            "youtube".into(),
            Callback::new(move |activation: Activation| {
                calls.set(calls.get() + 1);
                activation.complete(Ok(()));
            }),
        )
    });
    let replacement = fixture.scope(|| {
        let calls = new_calls.clone();
        routes.register(
            "youtube".into(),
            Callback::new(move |activation: Activation| {
                calls.set(calls.get() + 1);
                activation.complete(Ok(()));
            }),
        )
    });
    drop(old);
    let peer = source.peer(
        "https://app.acheflow.com/fwd/youtube/oauth/callback?state=registry-unit-only".into(),
    );
    assert!(matches!(fixture.wait_peer(peer), Ok(Startup::HandedOff)));
    assert_eq!(old_calls.get(), 0);
    assert_eq!(new_calls.get(), 1);
    drop(replacement);
}

#[test]
fn root_router_displays_unknown_and_disabled_recipient_errors_without_the_url() {
    let source = Source::new(Request::Show);
    let mut fixture = Fixture::with_inbox("youtube", Some(source.inbox()));
    for (address, expected) in [
        (
            "https://app.acheflow.com/fwd/missing/open?private=native-url-marker",
            ActivationError::NotFound,
        ),
        (
            "https://app.acheflow.com/fwd/youtube/oauth/callback?state=native-url-marker",
            ActivationError::Disabled,
        ),
    ] {
        if expected == ActivationError::Disabled {
            let mut project = fixture.signals().client.project;
            fixture.scope(|| {
                project
                    .write()
                    .client_extensions
                    .get_mut("youtube")
                    .unwrap()
                    .enabled = false
            });
            fixture.settle();
        }
        let peer = source.peer(address.into());
        assert!(matches!(fixture.wait_peer(peer), Err(error) if error == expected));
        assert_eq!(*fixture.harness.page.get().unwrap().peek(), "Extensions");
        let inner = fixture.doc.inner.borrow();
        let alert = inner.query_selector("[role='alert']").unwrap().unwrap();
        let text = inner.get_node(alert).unwrap().text_content();
        assert!(text.contains(&expected.to_string()));
        assert!(!text.contains("native-url-marker"));
        assert!(!text.contains("https://"));
    }
}

#[test]
fn saved_local_secret_remains_available_when_an_empty_replacement_is_submitted() {
    let mut fixture = Fixture::new("youtube");
    let (mut session, _) = fixture.session();
    // Finish the guest's validation using captured credentials, without a provider request.
    session
        .capture(
            &mut json!({"access_token":"fixture-token"}),
            &json!({"access_token":"/access_token"}),
        )
        .unwrap();
    session.step(json!({"type":"local_http","id":"auth:validate","status":200,"payload":{"aud":"public-id","expires_in":3600,"scope":"https://www.googleapis.com/auth/youtube.force-ssl"}}), &json!({"browser":false}), now()).unwrap();
    assert!(session.has_secret("client_secret"));
    // A new authorization lifecycle must reuse the committed application secret.
    session
        .step(
            json!({"type":"local_stop"}),
            &json!({"browser":false}),
            now(),
        )
        .unwrap();
    session
        .step(
            json!({"type":"local_start"}),
            &json!({"browser":false}),
            now(),
        )
        .unwrap();
    assert!(session.has_secret("client_secret"));
    fixture.scope(|| {
        let mut signals = fixture.signals();
        let key = session_key(&signals.identity.peek(), "youtube");
        signals.sessions.write().insert(key, session);
        signals.panel.set(Some(ExtensionPanel {
            extension_id: "youtube".into(),
            title: "Account authorization".into(),
            widgets: vec![PanelWidget::SecretInput {
                id: "client_secret".into(),
                label: "Application secret".into(),
            }],
        }));
    });
    fixture.settle();
    {
        let document = fixture.doc.inner.borrow();
        let note = document
            .query_selector(".local-secret-saved")
            .unwrap()
            .expect("saved credentials have a visible status");
        assert!(
            document
                .get_node(note)
                .unwrap()
                .text_content()
                .contains("Saved locally")
        );
        let input = document
            .query_selector("input[type='password']")
            .unwrap()
            .unwrap();
        assert!(
            !document
                .get_node(input)
                .unwrap()
                .element_data()
                .unwrap()
                .attrs()
                .iter()
                .any(|a| a.value.contains("fixture-local-secret"))
        );
    }
    fixture.scope(|| {
        let mut signals = fixture.signals();
        signals
            .drafts
            .write()
            .insert("client_secret".into(), String::new());
        assert!(required_inputs_ready(
            &["client_secret".into()],
            "youtube",
            signals
        ));
        dispatch("youtube", "noop".into(), signals);
        assert!(
            signals
                .sessions
                .read()
                .get(&session_key(&signals.identity.peek(), "youtube"))
                .unwrap()
                .has_secret("client_secret")
        );
    });
    fixture.settle();
}
