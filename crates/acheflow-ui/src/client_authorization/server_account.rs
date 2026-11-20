//! Save an independently authorized server account; device accounts stay separate.
use super::*;
use crate::{server_access, server_settings::ActionButton};
use acheflow_media_api::{
    access::Permission as P, server_provider::*, server_workspace::WorkspaceState,
};

#[component]
pub(super) fn ServerAccount(
    id: String,
    #[props(default)] on_saved: Callback<()>,
    #[props(default = true)] offer_authorization: bool,
) -> Element {
    let client = use_context::<crate::control_client::ControlClient>();
    let sessions = use_context::<Signal<Sessions>>();
    let mut expanded = use_signal(|| false);
    let mut channel = use_signal(String::new);
    let mut trusted = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut message = use_signal(String::new);
    let mut error = use_signal(String::new);
    let scope = use_memo({
        let id = id.clone();
        move || {
            let project = client.project.read();
            (
                project.id.clone(),
                project.server_url.clone(),
                project
                    .client_extensions
                    .get(&id)
                    .map(|s| s.config.clone())
                    .unwrap_or_default(),
                server_access::identity(),
            )
        }
    });
    use_effect(move || {
        let _ = scope();
        expanded.set(false);
        trusted.set(false);
        channel.set(String::new());
        message.set(String::new());
        error.set(String::new());
        busy.set(false);
    });
    let permitted = server_access::allows(P::ExtensionsConfigure);
    let can_execute = server_access::allows(P::ExtensionsRun);
    let owner = scope();
    let key = (owner.0.clone(), owner.1.clone(), id.clone());
    let account = sessions
        .read()
        .get(&key)
        .and_then(|s| s.credential_revision().map(|_| s.account_id()));
    let binding = client.project.read().server_integrations.get(&id).cloned();
    let channels: Vec<_> = client
        .state
        .read()
        .as_ref()
        .map(|s| {
            s.channels
                .iter()
                .filter(|c| c.extension_id == id)
                .map(|c| (c.id.clone(), c.label.clone()))
                .collect()
        })
        .unwrap_or_default();
    let connect = {
        let id = id.clone();
        move |_: ()| {
            if *busy.peek() || !*trusted.peek() {
                return;
            }
            let actor = scope();
            let source = sessions
                .peek()
                .get(&(actor.0.clone(), actor.1.clone(), id.clone()))
                .and_then(|s| s.credential_revision());
            let credentials = sessions
                .peek()
                .get(&(actor.0.clone(), actor.1.clone(), id.clone()))
                .ok_or("Authorize an account first.")
                .and_then(|s| {
                    s.server_credentials(&actor.0)
                        .map_err(|_| "Finish account authorization first.")
                });
            let credentials = match credentials {
                Ok(value) => value,
                Err(e) => {
                    error.set(e.into());
                    return;
                }
            };
            let target = crate::media::server_url(&client.project.peek(), "/api/access");
            let selected = channel.peek().trim().to_owned();
            let id = id.clone();
            busy.set(true);
            error.set(String::new());
            message.set(String::new());
            spawn(async move {
                let result = async {
                let target = target?;
                require_secure(&target)?;
                let workspace_url = target.trim_end_matches("/access").to_owned()+"/workspace";
                let workspace = server_access::get::<WorkspaceState>(&workspace_url).await?;
                if workspace.summary.as_ref().map(|w| &w.id) != Some(&actor.0) { return Err("Open the shared server workspace first.".into()); }
                if *scope.peek() != actor || sessions.peek().get(&(actor.0.clone(),actor.1.clone(),id.clone())).and_then(|s|s.credential_revision()) != source { return Err("The account or server changed. Try again.".into()); }
                server_access::decode::<ServerAccountConnected>(server_access::request(reqwest::Method::POST,&format!("{target}/server-accounts/{id}"))?
                    .json(&ConnectServerAccount { workspace_id:actor.0.clone(),expected_revision:workspace.revision,trust_server:true,expected_account:credentials.account_id,channel_id:selected,config:actor.2.clone(),credentials:credentials.secrets })
                    .timeout(std::time::Duration::from_secs(40)).send().await.map_err(|_| "Connection could not finish. Check the server integration status before retrying.")?).await
            }.await;
                if *scope.peek() != actor {
                    return;
                }
                match result {
                    Ok(connected) => {
                        let mut client = client;
                        client
                            .project
                            .write()
                            .server_integrations
                            .insert(id.clone(), connected.binding);
                        trusted.set(false);
                        on_saved.call(());
                        message.set("Server account saved. Shared chat collection starts independently of video output.".into());
                    }
                    Err(e) => error.set(e),
                }
                busy.set(false);
            });
        }
    };
    let configure = Callback::new({
        let id = id.clone();
        move |enabled: bool| {
            let actor = scope();
            let id = id.clone();
            busy.set(true);
            error.set(String::new());
            spawn(async move {
                let result = async {
                    let base = crate::media::server_url(&client.project.peek(), "/api/access")?;
                    require_secure(&base)?;
                    let workspace = server_access::get::<WorkspaceState>(
                        &(base.trim_end_matches("/access").to_owned() + "/workspace"),
                    )
                    .await?;
                    if *scope.peek() != actor {
                        return Err("Server settings changed.".into());
                    }
                    server_access::decode::<ServerAccountConnected>(
                        server_access::request(
                            reqwest::Method::PUT,
                            &format!("{base}/server-accounts/{id}"),
                        )?
                        .json(&ConfigureServerIntegration {
                            workspace_id: actor.0.clone(),
                            expected_revision: workspace.revision,
                            enabled,
                            config: actor.2.clone(),
                        })
                        .timeout(std::time::Duration::from_secs(15))
                        .send()
                        .await
                        .map_err(|_| "Cannot update server integration.")?,
                    )
                    .await
                }
                .await;
                if *scope.peek() != actor {
                    return;
                }
                match result {
                    Ok(connected) => {
                        let mut client = client;
                        client
                            .project
                            .write()
                            .server_integrations
                            .insert(id, connected.binding);
                        message.set("Server integration settings saved.".into());
                    }
                    Err(reason) => error.set(reason),
                }
                busy.set(false);
            });
        }
    });
    let disconnect = {
        let id = id.clone();
        move |_: ()| {
            if *busy.peek() {
                return;
            }
            let actor = scope();
            let target = crate::media::server_url(&client.project.peek(), "/api/access");
            let id = id.clone();
            busy.set(true);
            error.set(String::new());
            message.set(String::new());
            spawn(async move {
                let result = async {
                let target = target?; require_secure(&target)?;
                let workspace = server_access::get::<WorkspaceState>(&(target.trim_end_matches("/access").to_owned()+"/workspace")).await?;
                if *scope.peek() != actor || workspace.summary.as_ref().map(|w|&w.id)!=Some(&actor.0) { return Err("The workspace changed. Try again.".into()); }
                let response = server_access::request(reqwest::Method::DELETE,&format!("{target}/server-accounts/{id}"))?
                    .json(&DisconnectServerAccount { workspace_id:actor.0.clone(),expected_revision:workspace.revision })
                    .timeout(std::time::Duration::from_secs(15)).send().await.map_err(|_| "Cannot disconnect the server account.")?;
                if !response.status().is_success() { return Err("Cannot disconnect the server account. Check its status before retrying.".into()); }
                Ok::<_,String>(())
            }.await;
                if *scope.peek() != actor {
                    return;
                }
                match result {
                    Ok(()) => message.set("Server credentials removed.".into()),
                    Err(e) => error.set(e),
                }
                busy.set(false);
            });
        }
    };
    if !permitted {
        return rsx! {};
    }
    rsx! {section {class:"server-account-connect",
        if offer_authorization {h4 {"Save server authorization"}
        if !expanded() {
            p {class:"form-help","Save this independently authorized account on the server to collect shared chat."}
            if account.is_none() {p {class:"form-help","Sign in to the server account above, then save it here."}}
            ActionButton {id:"server-account-review",class:"access-text-button",disabled:busy() || !can_execute || account.is_none(),expanded:Some(false),onclick:move |_|expanded.set(true),"Connect this account to the server…"}
        }
        if expanded() {
        p {class:"form-help","The server will verify and store a copy of your account credentials, including refresh tokens and application secrets needed for renewal. Its administrator can access them. Only connect to a server you trust."}
        if let Some(account)=account {p {class:"form-help","Selected server account: {account}"}
            label {class:"field",span {"Shared chat channel ID"}input {r#type:"text",maxlength:128,value:channel(),disabled:busy(),oninput:move|e|channel.set(e.value())}}
            if !channels.is_empty() {div {class:"row",for (value,label) in channels {ActionButton {class:"access-text-button",disabled:busy(),onclick:move |_|channel.set(value.clone()),"{label}"}}}}
            p {class:"form-help","Use the exact channel ID shown by the extension. Only chat reading is granted to the server; sending and moderation require separate permission."}
            ActionButton {id:"server-account-trust",class:if trusted(){"access-permission selected"}else{"access-permission"},pressed:Some(trusted()),disabled:busy(),onclick:move |_|trusted.set(!trusted()),span {class:"access-check",if trusted(){"✓"}}span {"I trust this server to store and use this account."}}
            ActionButton {id:"server-account-submit",class:"primary",disabled:busy() || !trusted() || channel.read().trim().is_empty(),onclick:connect,if busy(){"Checking account…"}else{"Connect account to server"}}
        } else {p {class:"form-help","Sign in to the server account above, then save it here."}}
        ActionButton {class:"access-text-button",disabled:busy(),onclick:move |_|{expanded.set(false);trusted.set(false);},"Cancel"}
        }
        }
        if let Some(binding)=binding {div {class:"server-account-current",h3{"Collection"}p {class:"form-help","{binding.account_id} · Channel {binding.channel_id}"}
            div{class:"extension-action-row",ActionButton{class:"primary",disabled:busy(),onclick:move |_|configure.call(binding.enabled),"Save settings"}
            ActionButton{disabled:busy()||!can_execute,onclick:move |_|configure.call(!binding.enabled),if binding.enabled{"Stop collecting"}else{"Start collecting"}}ActionButton {class:"access-text-button",disabled:busy()||!can_execute,onclick:disconnect,"Disconnect server account"}}}}
        if !message.read().is_empty() {p {class:"form-help",role:"status","{message}"}}
        if !error.read().is_empty() {p {class:"field-error",role:"alert","{error}"}}
    }}
}
pub(super) fn require_secure(endpoint: &str) -> Result<(), String> {
    let url = url::Url::parse(endpoint).map_err(|_| "Invalid server address.")?;
    if url.scheme() == "https"
        || (url.scheme() == "http"
            && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
    {
        Ok(())
    } else {
        Err("Use trusted HTTPS before sending account credentials to a remote server.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::require_secure;
    #[test]
    fn credentials_require_https_except_loopback_development() {
        for address in [
            "https://studio.example/api/access",
            "http://localhost:21295/api/access",
            "http://127.0.0.1:21295/api/access",
            "http://[::1]:21295/api/access",
        ] {
            assert!(require_secure(address).is_ok(), "{address}");
        }
        for address in [
            "http://192.0.2.1:21295/api/access",
            "http://studio.example/api/access",
            "file:///studio",
            "garbage",
        ] {
            assert!(require_secure(address).is_err(), "{address}");
        }
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use acheflow_media_api::access::{Device, Session, User};
    use blitz_dom::{Document, DocumentConfig};
    use dioxus_native::DioxusDocument;
    use std::{cell::Cell, rc::Rc};
    #[derive(Default)]
    struct Harness {
        access: Cell<Option<server_access::AccessState>>,
    }
    fn app() -> Element {
        let harness = use_context::<Rc<Harness>>();
        let project = use_signal(|| {
            let mut p = crate::project::Workspace {
                id: "workspace".into(),
                server_url: "https://studio.example".into(),
                ..Default::default()
            };
            p.client_extensions
                .entry("vklive".into())
                .or_default()
                .enabled = true;
            p
        });
        let access = server_access::AccessState {
            session: use_signal(|| {
                Some(Session {
                    server_id: "server".into(),
                    user: User {
                        id: "user".into(),
                        name: "Participant".into(),
                    },
                    device: Device {
                        id: "device".into(),
                        user_id: "user".into(),
                        name: "Desktop".into(),
                        permissions: P::ALL.into(),
                        expires_at_ms: u64::MAX,
                    },
                })
            }),
            error: use_signal(String::new),
        };
        use_context_provider(|| access);
        harness.access.set(Some(access));
        let client = crate::control_client::ControlClient {
            project,
            state: use_signal(|| {
                Some(acheflow_media_api::control::ControlSnapshot {
                    channels: vec![acheflow_extension_api::integration::Channel {
                        extension_id: "vklive".into(),
                        id: "channel".into(),
                        label: "Fixture channel".into(),
                        capabilities: vec!["chat.read".into()],
                        constraints: Default::default(),
                    }],
                    ..Default::default()
                })
            }),
            error: use_signal(|| None),
            extension_errors: use_signal(Default::default),
            busy: use_signal(|| false),
            sync_conflict: use_signal(|| false),
            request_generation: use_signal(|| 0),
        };
        use_context_provider(|| client);
        let sessions: Signal<Sessions> = use_signal(|| {
            let catalog = acheflow_client::catalog::ClientCatalog::bundled();
            let installed = &catalog.installed["vklive"];
            let mut session = LocalSession::new(
                installed.manifest.clone(),
                &installed.module,
                BTreeMap::new(),
            )
            .unwrap();
            session.bind_workspace("workspace").unwrap();
            session
                .set_secret("access_token", "synthetic-secret-must-never-render".into())
                .unwrap();
            session
                .step(
                    json!({"type":"local_server_authorize"}),
                    &json!({"browser":false}),
                    1,
                )
                .unwrap();
            session.step(json!({"type":"local_http","id":"auth:validate","status":200,"payload":{"data":{"user":{"id":123}}}}),&json!({"browser":false}),2).unwrap();
            BTreeMap::from([(
                (
                    "workspace".into(),
                    "https://studio.example".into(),
                    "vklive".into(),
                ),
                session,
            )])
        });
        use_context_provider(|| sessions);
        rsx! {ServerAccount {id:"vklive"}}
    }
    fn settle(doc: &mut DioxusDocument) {
        for _ in 0..40 {
            if !doc.poll(None) {
                doc.inner.borrow_mut().resolve(0.);
                return;
            }
        }
        panic!("account UI did not settle");
    }
    fn activate(doc: &mut DioxusDocument, selector: &str) {
        use blitz_traits::events::{BlitzKeyEvent, KeyState, UiEvent};
        let id = doc
            .inner
            .borrow()
            .query_selector(selector)
            .unwrap()
            .unwrap();
        doc.inner.borrow_mut().set_focus_to(id);
        let mut event = BlitzKeyEvent {
            key: Key::Named(NamedKey::Enter),
            code: Code::Enter,
            modifiers: Modifiers::empty(),
            location: Location::Standard,
            is_auto_repeating: false,
            is_composing: false,
            state: KeyState::Pressed,
            text: None,
        };
        doc.handle_ui_event(UiEvent::KeyDown(event.clone()));
        event.state = KeyState::Released;
        doc.handle_ui_event(UiEvent::KeyUp(event));
        settle(doc);
    }
    #[tokio::test]
    async fn copying_requires_review_channel_and_explicit_trust_and_never_renders_credentials() {
        let harness = Rc::new(Harness::default());
        let mut doc = DioxusDocument::new(
            VirtualDom::new(app).with_root_context(harness.clone()),
            DocumentConfig::default(),
        );
        doc.initial_build();
        settle(&mut doc);
        assert!(
            doc.inner
                .borrow()
                .query_selector("#server-account-submit")
                .unwrap()
                .is_none()
        );
        activate(&mut doc, "#server-account-review");
        assert!(
            doc.inner
                .borrow()
                .root_element()
                .outer_html()
                .contains("administrator can access")
        );
        assert!(
            doc.inner
                .borrow()
                .query_selector("#server-account-submit[disabled]")
                .unwrap()
                .is_some()
        );
        activate(&mut doc, ".server-account-connect .row button");
        assert!(
            doc.inner
                .borrow()
                .query_selector("#server-account-submit[disabled]")
                .unwrap()
                .is_some()
        );
        activate(&mut doc, "#server-account-trust");
        assert!(
            doc.inner
                .borrow()
                .query_selector("#server-account-submit[disabled]")
                .unwrap()
                .is_none()
        );
        assert!(
            !doc.inner
                .borrow()
                .root_element()
                .outer_html()
                .contains("synthetic-secret-must-never-render")
        );
        let mut access = harness.access.get().unwrap();
        access
            .session
            .write()
            .as_mut()
            .unwrap()
            .device
            .permissions
            .clear();
        settle(&mut doc);
        assert!(
            doc.inner
                .borrow()
                .query_selector(".server-account-connect")
                .unwrap()
                .is_none()
        );
    }
}
