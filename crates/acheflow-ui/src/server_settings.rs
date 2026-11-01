use acheflow_media_api::server::{ServerSettings, ServerSettingsStatus};
use dioxus_native::prelude::*;

async fn request(settings: Option<&ServerSettings>) -> Result<ServerSettingsStatus, String> {
    let url = crate::media::server_url("/api/server/settings")?;
    let client = reqwest::Client::new();
    let request = if let Some(settings) = settings {
        client.put(url).json(settings)
    } else {
        client.get(url)
    };
    let response = request
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Cannot reach server settings: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let error = response
            .json::<acheflow_media_api::live::ApiError>()
            .await
            .ok();
        return Err(error
            .map(|e| e.error)
            .unwrap_or_else(|| format!("Settings API: {status}")));
    }
    response
        .json()
        .await
        .map_err(|e| format!("Invalid settings response: {e}"))
}

#[component]
pub fn ServerSettingsPanel() -> Element {
    let mut status = use_signal(|| None::<ServerSettingsStatus>);
    let mut draft = use_signal(ServerSettings::default);
    let mut http_port = use_signal(String::new);
    let mut rtmp_port = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut notice = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let mut revision = use_signal(|| 0u32);
    let _load = use_resource(move || {
        let _ = revision();
        async move {
            busy.set(true);
            error.set(None);
            match request(None).await {
                Ok(value) => {
                    http_port.set(value.saved.http_port.to_string());
                    rtmp_port.set(value.saved.rtmp_port.to_string());
                    draft.set(value.saved.clone());
                    status.set(Some(value));
                }
                Err(message) => error.set(Some(message)),
            }
            busy.set(false);
        }
    });
    let save = move |_| {
        if busy() {
            return;
        }
        let mut settings = draft.peek().clone();
        let ports = (
            http_port.peek().parse::<u16>(),
            rtmp_port.peek().parse::<u16>(),
        );
        let (Ok(http), Ok(rtmp)) = ports else {
            error.set(Some("Enter ports from 1 to 65535.".into()));
            return;
        };
        settings.http_port = http;
        settings.rtmp_port = rtmp;
        if let Err(message) = settings.validate() {
            error.set(Some(message));
            return;
        }
        busy.set(true);
        error.set(None);
        notice.set(None);
        spawn(async move {
            match request(Some(&settings)).await {
                Ok(value) => {
                    status.set(Some(value));
                    draft.set(settings);
                    notice.set(Some("Saved. Restart the server to apply changes.".into()));
                }
                Err(message) => error.set(Some(message)),
            }
            busy.set(false);
        });
    };
    let settings = draft.read().clone();
    rsx! {
        section {class:"panel server-settings",
            div {class:"panel-heading",h2 {"Server listeners & TLS"}button {disabled:busy(),onclick:move |_|*revision.write()+=1,"Reload"}}
            div {class:"padded",
                p {"Settings apply after restarting the server. Launch arguments and environment variables take priority."}
                if let Some(current)=status.read().as_ref() {
                    p {class:"form-help","Active HTTP(S): {current.active.http_host}:{current.active.http_port} · RTMP(S): {current.active.rtmp_host}:{current.active.rtmp_port}"}
                    if current.restart_required {p {class:"form-help",role:"status","Saved settings differ from the running server. Restart required."}}
                    if !current.overrides.is_empty() {p {class:"form-help","Active overrides: ",{current.overrides.join(", ")},". Remove the corresponding launch options to use saved values."}}
                }
            }
            if status.read().is_some() {
                div {
                    fieldset {disabled:busy(),class:"server-fields",
                        h3 {class:"padded","HTTP / HTTPS"}
                        div {class:"form-grid",
                            SettingField {label:"HTTP listen host",value:settings.http_host,change:move |value|draft.write().http_host=value}
                            SettingField {label:"HTTP port",kind:"number",value:http_port(),change:move |value|http_port.set(value)}
                            SettingField {label:"HTTPS certificate chain (PEM)",value:settings.http_tls.cert,change:move |value|draft.write().http_tls.cert=value}
                            SettingField {label:"HTTPS private key (PEM)",value:settings.http_tls.key,change:move |value|draft.write().http_tls.key=value}
                        }
                        h3 {class:"padded","RTMP / RTMPS listener"}
                        div {class:"form-grid",
                            SettingField {label:"RTMP listen host",value:settings.rtmp_host,change:move |value|draft.write().rtmp_host=value}
                            SettingField {label:"RTMP port",kind:"number",value:rtmp_port(),change:move |value|rtmp_port.set(value)}
                            SettingField {label:"RTMPS certificate chain (PEM)",value:settings.rtmp_tls.cert,change:move |value|draft.write().rtmp_tls.cert=value}
                            SettingField {label:"RTMPS private key (PEM)",value:settings.rtmp_tls.key,change:move |value|draft.write().rtmp_tls.key=value}
                        }
                        div {class:"padded",p {class:"form-help","Use file paths on the server. Supply both certificate and key to enable TLS on that listener; leave both empty for HTTP / RTMP."}
                            button {r#type:"button",class:"primary",disabled:busy(),onclick:save,if busy(){"Saving…"}else{"Save server settings"}}
                        }
                    }
                }
            } else if busy(){p {class:"padded",role:"status","Loading server settings…"}}
            if let Some(message)=error(){p {class:"padded field-error",role:"alert","{message}"}}
            if let Some(message)=notice(){p {class:"padded",role:"status","{message}"}}
        }
    }
}

#[component]
fn SettingField(
    label: &'static str,
    value: String,
    change: EventHandler<String>,
    #[props(default = "text")] kind: &'static str,
) -> Element {
    // Keep the native input in charge while typing; echoing each queued input
    // value back into Blitz can overwrite keystrokes that arrived after it.
    let mut text = use_signal(|| value.clone());
    let mut displayed = use_signal(|| value.clone());
    let mut applied = use_signal(|| value.clone());
    use_effect(use_reactive((&value,), move |(value,)| {
        if value != *applied.peek() {
            text.set(value.clone());
            displayed.set(value.clone());
            applied.set(value);
        }
    }));
    rsx! {label {class:"field",span {"{label}"}input {r#type:"text",inputmode:if kind=="number" {"numeric"} else {"text"},value:{let _=displayed();text.peek().clone()},
        oninput:move |event|text.set(event.value()),
        onblur:move |_|change.call(text.peek().clone()),
        onkeydown:move |event| {
            if event.key()==Key::Named(NamedKey::Enter) {event.prevent_default();change.call(text.peek().clone());}
        }
    }}}
}
