use super::*;
use acheflow_media_api::access::ProfileRequest;

#[component]
pub(super) fn Identity(project: Signal<Workspace>) -> Element {
    let mut state = use_context::<AccessState>();
    let clipboard = try_consume_context::<crate::live_client::LiveClient>();
    let endpoint = use_memo(move || crate::media::server_url(&project.read(), "/api/access"));
    let scope = use_memo(move || {
        state.session.read().as_ref().map(|s| {
            (
                s.server_id.clone(),
                s.user.id.clone(),
                s.device.id.clone(),
                s.device.permissions.clone(),
            )
        })
    });
    let mut name = use_signal(String::new);
    let mut dirty = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut error = use_signal(String::new);
    let mut devices = use_signal(Vec::<Device>::new);
    let mut code = use_signal(|| None::<String>);
    let mut show = use_signal(|| false);
    let mut copied = use_signal(|| false);
    use_effect(move || {
        let _ = (endpoint(), scope());
        name.set(String::new());
        dirty.set(false);
        busy.set(false);
        error.set(String::new());
        code.set(None);
        show.set(false);
        copied.set(false);
    });
    use_effect(move || {
        let session = state.session.read().clone();
        if !*dirty.peek() {
            name.set(session.map(|s| s.user.name).unwrap_or_default());
        }
    });
    use_resource(move || {
        let target = endpoint();
        let identity = scope();
        async move {
            devices.set(vec![]);
            let (Ok(target), Some(identity)) = (target, identity) else {
                return;
            };
            loop {
                let result = get::<Devices>(&format!("{target}/my-devices")).await;
                if endpoint.peek().as_ref() != Ok(&target)
                    || scope.peek().as_ref() != Some(&identity)
                {
                    return;
                }
                match result {
                    Ok(result) => devices.set(result.devices),
                    Err(e) => error.set(e),
                }
                crate::scene_preview::pause(5000).await;
            }
        }
    });
    let save = move |_: ()| {
        if *busy.peek() {
            return;
        }
        let target = endpoint();
        let identity = scope();
        let candidate = name.peek().clone();
        busy.set(true);
        error.set(String::new());
        spawn(async move {
            let result = async {
                let target = target.clone()?;
                decode::<Session>(
                    request(reqwest::Method::PUT, &format!("{target}/profile"))?
                        .json(&ProfileRequest {
                            name: candidate.clone(),
                        })
                        .timeout(std::time::Duration::from_secs(10))
                        .send()
                        .await
                        .map_err(|_| "Cannot save your name.")?,
                )
                .await
            }
            .await;
            if endpoint.peek().as_ref() != target.as_ref() || *scope.peek() != identity {
                return;
            }
            match result {
                Ok(session) => {
                    if *name.peek() == candidate {
                        name.set(session.user.name.clone());
                        dirty.set(false);
                    }
                    state.session.set(Some(session));
                }
                Err(e) => error.set(e),
            }
            busy.set(false);
        });
    };
    let link = move |_: ()| {
        if *busy.peek() {
            return;
        }
        let target = endpoint();
        let identity = scope();
        let permissions = identity.as_ref().map(|s| s.3.clone()).unwrap_or_default();
        busy.set(true);
        error.set(String::new());
        spawn(async move {
            let result = async {
                let target = target.clone()?;
                decode::<acheflow_media_api::access::Invitation>(
                    request(reqwest::Method::POST, &format!("{target}/link-invitations"))?
                        .json(&InviteRequest { permissions })
                        .timeout(std::time::Duration::from_secs(10))
                        .send()
                        .await
                        .map_err(|_| "Cannot create a device link.")?,
                )
                .await
            }
            .await;
            if endpoint.peek().as_ref() != target.as_ref() || *scope.peek() != identity {
                return;
            }
            match result {
                Ok(invitation) => {
                    code.set(Some(invitation.code));
                    show.set(false);
                    copied.set(false);
                }
                Err(e) => error.set(e),
            }
            busy.set(false);
        });
    };
    let mut remove = move |id: String| {
        if *busy.peek() {
            return;
        }
        let target = endpoint();
        let identity = scope();
        busy.set(true);
        error.set(String::new());
        spawn(async move {
            let result = async {
                let target = target.clone()?;
                let response = request(reqwest::Method::DELETE, &format!("{target}/devices/{id}"))?
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                    .await
                    .map_err(|_| "Cannot remove this device.")?;
                if !response.status().is_success() {
                    return Err("Cannot remove this device.".to_owned());
                }
                Ok::<_, String>(())
            }
            .await;
            if endpoint.peek().as_ref() != target.as_ref() || *scope.peek() != identity {
                return;
            }
            match result {
                Ok(()) => devices.write().retain(|d| d.id != id),
                Err(e) => error.set(e),
            }
            busy.set(false);
        });
    };
    let session = state.session.read().clone();
    let Some(session) = session else {
        return rsx! {};
    };
    rsx! {section {class:"access-card access-identity",
        div {class:"access-card-heading",span {class:"access-eyebrow","YOUR IDENTITY"}h3 {"Your profile & devices"}p {"Link your own devices to keep the same identity. Each device keeps its own permissions."}}
        div {class:"access-connect-form",
            label {class:"field",span {"Your name"}input {r#type:"text",maxlength:80,value:name(),oninput:move|e|{dirty.set(true);name.set(e.value());}}}
            ActionButton {class:"access-text-button",disabled:busy() || !dirty() || name.read().trim().is_empty(),onclick:save,"Save name"}
        }
        for device in devices.read().clone() {div {class:"access-device-row",span {class:"access-device-avatar","aria-hidden":"true","◈"}div {class:"access-device-copy",strong {"{device.name}"}p {"{role_name(&device.permissions)}"}if device.id==session.device.id {span {class:"access-this-device","This device"}}}
            if device.id!=session.device.id {ActionButton {class:"access-revoke",disabled:busy(),onclick:move |_|remove(device.id.clone()),"Remove"}}
        }}
        if let Some(value)=code.read().clone() {
            div {class:"access-invitation-ready",h4 {"Connect your other device"}p {"Enter this code on your own phone or computer. It links to your profile with this device’s current access."}
                div {class:"access-invitation-code",input {r#type:if show(){"text"}else{"password"},readonly:true,value,"aria-label":"One-time device link code"}
                    ActionButton {class:"access-text-button",onclick:move |_|show.set(!show()),if show(){"Hide"}else{"Show"}}
                }
                ActionButton {class:"primary",onclick:move |_|{let value=code.peek().clone();if let Some(value)=value {spawn(async move {if let Some(client)=clipboard {if client.copy_text(value).await {copied.set(true);}}else{error.set("Select the code to copy it.".into());}});}},if copied(){"Copied ✓"}else{"Copy device link"}}
                p {class:"access-footnote","Single use · 10 minutes. Your linked devices share your identity and future personal account access. Only use this for devices you own."}
                ActionButton {class:"access-text-button",onclick:move |_|code.set(None),"Close"}
            }
        } else {
            div {class:"access-invite-footer",ActionButton {id:"access-link-device",class:"primary",disabled:busy() || session.device.permissions.is_empty(),onclick:link,"Link my other device"}p {"Same profile · this device’s current access"}}
        }
        if !error.read().is_empty() {div {class:"access-error",role:"alert","{error}"}}
    }}
}
