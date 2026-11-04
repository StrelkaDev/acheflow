//! Desktop mailbox for idle sessions and callbacks while the browser is open.
use super::*;
use acheflow_extension_api::forwarding::{ForwardLink, ForwardMode, ForwardRecipient};
use acheflow_launcher::{Activation, ActivationError, Request};
use std::{
    cell::{Cell, RefCell},
    future::Future,
    pin::Pin,
};
use tokio::sync::{Mutex, mpsc};

#[derive(Clone)]
pub(super) struct Mailbox(Rc<Inner>);
struct Inner {
    sender: mpsc::Sender<Routed>,
    receiver: Mutex<mpsc::Receiver<Routed>>,
    current: RefCell<Option<Current>>,
    error: Signal<String>,
    initializing: Cell<bool>,
}
struct Current {
    owner: Owner,
    epoch: u64,
    instance: String,
}
struct Routed {
    activation: Activation,
    owner: Owner,
    epoch: u64,
    instance: Option<String>,
}
#[derive(Clone)]
pub(super) struct Context {
    signals: PanelSignals,
    owner: Owner,
    epoch: u64,
}
impl Context {
    pub(super) fn new(signals: PanelSignals, owner: Owner, epoch: u64) -> Self {
        Self {
            signals,
            owner,
            epoch,
        }
    }
    fn live(&self, id: &str) -> bool {
        self.signals.identity.peek().clone() == self.owner
            && *self.signals.restart.peek() == self.epoch
            && self
                .signals
                .client
                .project
                .peek()
                .client_extensions
                .get(id)
                .is_some_and(|s| s.enabled)
    }
}
impl Mailbox {
    pub(super) fn new(error: Signal<String>) -> Self {
        let (sender, receiver) = mpsc::channel(8);
        Self(Rc::new(Inner {
            sender,
            receiver: Mutex::new(receiver),
            current: RefCell::new(None),
            error,
            initializing: Cell::new(true),
        }))
    }
    pub(super) fn begin_initialization(&self) {
        self.0.initializing.set(true);
    }
    pub(super) fn finish_initialization(&self) {
        self.0.initializing.set(false);
    }
    fn enqueue(&self, activation: Activation, owner: Owner, epoch: u64) {
        let instance = self
            .0
            .current
            .borrow()
            .as_ref()
            .filter(|c| c.owner == owner && c.epoch == epoch)
            .map(|c| c.instance.clone());
        if let Err(error) = self.0.sender.try_send(Routed {
            activation,
            owner,
            epoch,
            instance,
        }) {
            let (request, error) = match error {
                mpsc::error::TrySendError::Full(r) => (r, ActivationError::Busy),
                mpsc::error::TrySendError::Closed(r) => (r, ActivationError::NotReady),
            };
            request.activation.complete(Err(error));
        }
    }
    fn activate(&self, context: &Context, instance: &str) {
        *self.0.current.borrow_mut() = Some(Current {
            owner: context.owner.clone(),
            epoch: context.epoch,
            instance: instance.into(),
        });
    }
    fn deliver(
        &self,
        request: Routed,
        session: &mut LocalSession,
        context: &Context,
    ) -> (Vec<Value>, bool) {
        let Routed {
            activation,
            owner,
            epoch,
            instance,
        } = request;
        if !activation.is_live() {
            activation.complete(Err(ActivationError::Expired));
            return (vec![], false);
        }
        if owner != context.owner
            || epoch != context.epoch
            || !context.live(&session.manifest.id)
            || instance
                .as_deref()
                .is_some_and(|id| id != session.instance_id())
        {
            activation.complete(Err(ActivationError::NotReady));
            return (vec![], false);
        }
        let Request::Forward { url } = &activation.request else {
            activation.complete(Ok(()));
            return (vec![], false);
        };
        let callback = ForwardLink::parse_for(url, ForwardRecipient::Client)
            .ok()
            .and_then(|l| {
                l.declaration(&session.manifest)
                    .ok()
                    .map(|d| d.mode == ForwardMode::Callback)
            })
            .unwrap_or(false);
        let expected = instance.unwrap_or_else(|| session.instance_id().to_owned());
        match session.forward(&owner.0, &expected, url, &environment(), now()) {
            Ok(effects) => {
                let mut notice = self.0.error;
                notice.set(String::new());
                activation.complete(Ok(()));
                (effects, callback)
            }
            Err(error) => {
                let mut notice = self.0.error;
                notice.set(error);
                activation.complete(Err(ActivationError::Rejected));
                (vec![], false)
            }
        }
    }
}

pub(super) fn register(id: String, signals: PanelSignals) {
    let native = signals.native.peek().clone();
    let callback = Callback::new({
        let id = id.clone();
        move |activation: Activation| {
            let owner = (signals.identity)();
            if !signals
                .client
                .project
                .peek()
                .client_extensions
                .get(&id)
                .is_some_and(|s| s.enabled)
            {
                activation.complete(Err(ActivationError::Disabled));
                return;
            }
            if (signals.busy)() {
                native.enqueue(activation, owner, (signals.restart)());
                return;
            }
            let mut sessions = signals.sessions;
            let session = sessions.write().remove(&(owner.0.clone(), id.clone()));
            if let Some(session) = session {
                native.enqueue(activation, owner, (signals.restart)());
                super::start_operation(id.clone(), session, vec![], signals);
            } else if native.0.initializing.get() {
                native.enqueue(activation, owner, (signals.restart)());
            } else {
                let mut error = signals.error;
                error.set(ActivationError::NotReady.to_string());
                activation.complete(Err(ActivationError::NotReady));
            }
        }
    });
    let lease = use_hook(move || {
        Rc::new(RefCell::new(
            try_consume_context::<crate::native_activation::NativeRoutes>()
                .map(|routes| routes.register(id, callback)),
        ))
    });
    use_drop(move || {
        lease.borrow_mut().take();
    });
}

type BrowserWait = Pin<Box<dyn Future<Output = Result<Value, String>>>>;
#[cfg(all(test, unix))]
#[path = "native_tests.rs"]
mod tests;

async fn open_browser(session: &LocalSession, effect: &Value) -> Result<BrowserWait, String> {
    let url = acheflow_client::transport::allowed_url(
        effect["url"].as_str().ok_or("Missing browser URL.")?,
        &session.manifest,
        false,
    )?;
    let callback = acheflow_client::browser::Callback::bind().await?;
    callback.open(url.as_str())?;
    Ok(Box::pin(callback.receive()))
}

async fn run_effect(
    effect: Value,
    session: &mut LocalSession,
    panel: &mut Option<ExtensionPanel>,
    browser: &mut Option<BrowserWait>,
    browser_capture: &mut Value,
) -> Result<Option<Value>, String> {
    match effect["type"].as_str() {
        Some("panel") => {
            *panel = Some(
                serde_json::from_value(effect["panel"].clone())
                    .map_err(|_| "Invalid local panel.")?,
            );
            Ok(None)
        }
        Some("http") => Ok(Some(session.http(&effect).await)),
        Some("browser") => {
            if browser.is_some() {
                return Err("Only one browser authorization can be pending.".into());
            }
            match open_browser(session, &effect).await {
                Ok(wait) => {
                    *browser_capture = effect["capture"].clone();
                    *browser = Some(wait);
                    Ok(None)
                }
                Err(_) => Ok(Some(browser_interrupted())),
            }
        }
        Some("status" | "failure") => Ok(None),
        _ => Err("Unsupported local effect.".into()),
    }
}

fn browser_interrupted() -> Value {
    json!({"type":"local_browser","payload":null,"error":"Browser authorization interrupted."})
}

pub(super) async fn drive(
    mut session: LocalSession,
    effects: Vec<Value>,
    mailbox: Mailbox,
    context: Context,
) -> Result<(LocalSession, Option<ExtensionPanel>), String> {
    let mut inbox = mailbox.0.receiver.lock().await;
    if !context.live(&session.manifest.id) {
        session.stop();
        return Err("The local extension context changed.".into());
    }
    mailbox.activate(&context, session.instance_id());
    let mut pending = VecDeque::from(effects);
    let mut panel = None;
    let mut browser = None::<BrowserWait>;
    let mut browser_capture = Value::Null;
    let mut turns = 0;
    loop {
        if pending.len() > 512 {
            return Err("Local panel event queue exceeded.".into());
        }
        if !context.live(&session.manifest.id) {
            session.stop();
            return Err("The local extension context changed.".into());
        }
        if let Ok(request) = inbox.try_recv() {
            let (effects, callback) = mailbox.deliver(request, &mut session, &context);
            if callback {
                browser = None;
            }
            pending.extend(effects);
        }
        let input = match pending.pop_front() {
            Some(effect) => {
                turns += 1;
                if turns > 256 {
                    return Err("Local panel event queue exceeded.".into());
                }
                let Some(input) = run_effect(
                    effect,
                    &mut session,
                    &mut panel,
                    &mut browser,
                    &mut browser_capture,
                )
                .await?
                else {
                    continue;
                };
                input
            }
            None => {
                // A rejected/no-op request must not strand later queued links.
                if !inbox.is_empty() {
                    continue;
                }
                let Some(wait) = browser.as_mut() else {
                    return Ok((session, panel));
                };
                tokio::select! {
                    response=wait=>{
                        browser=None;
                        match response{
                            Ok(mut payload)=>{session.capture(&mut payload,&browser_capture)?;json!({"type":"local_browser","payload":payload})},
                            Err(_)=>browser_interrupted(),
                        }
                    },
                    Some(request)=inbox.recv()=>{
                        let(effects,callback)=mailbox.deliver(request,&mut session,&context);
                        if callback{browser=None;}
                        pending.extend(effects);continue;
                    },
                }
            }
        };
        if !context.live(&session.manifest.id) {
            session.stop();
            return Err("The local extension context changed.".into());
        }
        pending.extend(session.step(input, &environment(), now())?);
    }
}
