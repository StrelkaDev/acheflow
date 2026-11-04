//! Main-thread routing for desktop activation; browser navigation is separate.
use acheflow_client::catalog::ClientCatalog;
use acheflow_extension_api::forwarding::{ForwardLink, ForwardRecipient};
use acheflow_launcher::{Activation, ActivationError, Inbox, Request};
use dioxus_native::prelude::*;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    rc::{Rc, Weak},
    sync::Arc,
};

type Routes = BTreeMap<String, (Rc<()>, Callback<Activation>)>;
#[derive(Clone, Default)]
pub(crate) struct NativeRoutes(Rc<RefCell<Routes>>);
pub(crate) struct RouteLease {
    routes: Weak<RefCell<Routes>>,
    id: String,
    token: Rc<()>,
}
impl Drop for RouteLease {
    fn drop(&mut self) {
        if let Some(routes) = self.routes.upgrade() {
            let mut routes = routes.borrow_mut();
            if routes
                .get(&self.id)
                .is_some_and(|(token, _)| Rc::ptr_eq(token, &self.token))
            {
                routes.remove(&self.id);
            }
        }
    }
}
impl NativeRoutes {
    pub(crate) fn register(&self, id: String, callback: Callback<Activation>) -> RouteLease {
        let token = Rc::new(());
        self.0
            .borrow_mut()
            .insert(id.clone(), (token.clone(), callback));
        RouteLease {
            routes: Rc::downgrade(&self.0),
            id,
            token,
        }
    }
}

pub(crate) fn provide(
    project: Signal<crate::project::Workspace>,
    mut page: Signal<&'static str>,
) -> Signal<String> {
    let routes = use_hook(NativeRoutes::default);
    use_context_provider(|| routes.clone());
    let mut notice = use_signal(String::new);
    let inbox = use_hook(try_consume_context::<Arc<Inbox>>);
    let catalog = use_context::<Rc<ClientCatalog>>();
    let window = use_hook(try_consume_context::<Arc<dyn winit::window::Window>>);
    use_future(move || {
        let inbox = inbox.clone();
        let routes = routes.clone();
        let catalog = catalog.clone();
        let window = window.clone();
        async move {
            let Some(mut inbox) = inbox.and_then(|i| i.take()) else {
                return;
            };
            while let Some(activation) = inbox.recv().await {
                if !activation.is_live() {
                    activation.complete(Err(ActivationError::Expired));
                    continue;
                }
                if let Some(window) = &window {
                    window.set_visible(true);
                    window.set_minimized(false);
                    window.focus_window();
                }
                let Request::Forward { url } = &activation.request else {
                    activation.complete(Ok(()));
                    continue;
                };
                page.set("Extensions");
                let link = match ForwardLink::parse_for(url, ForwardRecipient::Client) {
                    Ok(link) => link,
                    Err(_) => {
                        notice.set(ActivationError::InvalidLink.to_string());
                        activation.complete(Err(ActivationError::InvalidLink));
                        continue;
                    }
                };
                let error = match catalog.installed.get(&link.extension_id) {
                    None => Some(ActivationError::NotFound),
                    Some(installed) if link.declaration(&installed.manifest).is_err() => {
                        Some(ActivationError::Rejected)
                    }
                    Some(_)
                        if !project
                            .peek()
                            .client_extensions
                            .get(&link.extension_id)
                            .is_some_and(|s| s.enabled) =>
                    {
                        Some(ActivationError::Disabled)
                    }
                    _ => None,
                };
                if let Some(error) = error {
                    notice.set(error.to_string());
                    activation.complete(Err(error));
                    continue;
                }
                let callback = routes
                    .0
                    .borrow()
                    .get(&link.extension_id)
                    .map(|(_, callback)| *callback);
                if let Some(callback) = callback {
                    notice.set(String::new());
                    callback.call(activation);
                } else {
                    notice.set(ActivationError::NotReady.to_string());
                    activation.complete(Err(ActivationError::NotReady));
                }
            }
        }
    });
    notice
}

#[component]
pub(crate) fn Notice(mut value: Signal<String>) -> Element {
    rsx! {if !value().is_empty(){div{class:"panel activation-notice",role:"alert",span{"{value}"}button{r#type:"button","aria-label":"Dismiss link message",onclick:move|_|value.set(String::new()),"Dismiss"}}}}
}
