//! Forward only server-declared events through a private live-instance inbox.
use crate::control::ControlHost;
use acheflow_extension_api::forwarding::{ForwardLink, ForwardMode, ForwardRecipient};
use serde_json::json;

pub async fn deliver(
    host: &ControlHost,
    owner: &str,
    instance: &str,
    raw_url: &str,
    request_id: &str,
    is_live: &(impl Fn() -> bool + Send + Sync),
) -> Result<(), String> {
    if instance.is_empty() || instance.len() > 128 {
        return Err("invalid_argument: forwarding instance".into());
    }
    let link =
        ForwardLink::parse_for(raw_url, ForwardRecipient::Server).map_err(|e| e.to_string())?;
    let installed = host
        .installed
        .get(&link.extension_id)
        .ok_or("not_found: forwarding extension")?;
    if link.declaration(&installed.manifest)?.mode != ForwardMode::Event {
        return Err("forbidden: callbacks belong to clients".into());
    }
    if !host.extension_is_live(owner, &link.extension_id, instance) {
        return Err("unavailable: forwarding extension is not live".into());
    }
    let payload = link.parameters().map_err(|e| e.to_string())?;
    let running = host.guests.lock().await;
    if !is_live() {
        return Err("unavailable: request expired".into());
    }
    let guest = running
        .get(&link.extension_id)
        .ok_or("unavailable: forwarding instance stopped")?;
    guest.forward(
        owner,
        instance,
        json!({"type":"forwarded","request_id":request_id,"action":link.action,"tail":link.tail,"payload":payload}),
    )
}
