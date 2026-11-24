//! Durable account/channel consent, separate from temporary live connections.
use super::*;
use crate::server_settings::ActionButton;
use acheflow_extension_api::client_execution::{Capability, CapabilityGrant};

#[component]
pub(super) fn WorkspacePermissions() -> Element {
    let mut expanded = use_signal(|| true);
    let client = use_context::<ControlClient>();
    let current = client
        .state
        .read()
        .as_ref()
        .filter(|s| s.workspace_id.as_deref() == Some(client.project.read().id.as_str()))
        .cloned();
    rsx! {section {class:"panel workspace-capabilities",
        div {class:"panel-heading",
            div {h2 {"Channel permissions"} p {class:"form-help","Saved for this workspace. New broadcasts reuse the same permissions."}}
            ActionButton {expanded:Some(expanded()),onclick:move |_| expanded.set(!expanded()),if expanded() {"Hide permissions"} else {"Review permissions"}}
        }
        div {class:"padded permission-list",hidden:!expanded(),
            if let Some(state) = current {
                if state.client_capabilities.is_empty() {
                    div {class:"permission-empty",h3 {"No separate channel rules"} p {class:"form-help","Connected server accounts use the features enabled in their integration settings and authorized by the platform."}}
                }
                for grant in state.client_capabilities {
                    PermissionCard {key:"{grant.provider}:{grant.account_id}:{grant.channel_id}",grant}
                }
            } else {p {class:"form-help","Open the shared server workspace to manage channel access."}}
        }
    }}
}

#[component]
fn PermissionCard(grant: CapabilityGrant) -> Element {
    let client = use_context::<ControlClient>();
    let catalog = use_context::<Rc<ClientCatalog>>();
    let mut details = use_signal(|| false);
    let provider = catalog.installed.get(&grant.provider);
    let name = provider
        .map(|i| i.manifest.name.clone())
        .unwrap_or_else(|| grant.provider.clone());
    let snapshot = client.state.read();
    let channel = snapshot.as_ref().and_then(|s| {
        s.channels
            .iter()
            .find(|c| c.extension_id == grant.provider && c.id == grant.channel_id)
    });
    let connected = snapshot.as_ref().is_some_and(|s| {
        s.client_executors.iter().any(|e| {
            e.connections.iter().any(|c| {
                c.provider == grant.provider
                    && c.account_id == grant.account_id
                    && c.channel_id == grant.channel_id
            })
        })
    });
    let revoked = grant.capabilities.is_empty();
    let label = if revoked {
        "Access revoked"
    } else if connected {
        "Connected"
    } else {
        "Waiting for connection"
    };
    let title = channel
        .map(|c| c.label.clone())
        .unwrap_or_else(|| "Saved channel access".into());
    let stream = channel.and_then(|c| c.stream_id.clone());
    let disabled = (client.busy)()
        || !crate::server_access::allows(
            acheflow_media_api::access::Permission::ExtensionsConfigure,
        );
    let send_supported =
        provider.is_some_and(|i| i.manifest.subscriptions.iter().any(|s| s == "chat.send"));
    let identity = grant.clone();
    rsx! {article {class:"permission-card",
        div {class:"permission-heading",div {h3 {"{name}"}p {class:"permission-channel","{title}"}}span {class:if revoked {"badge permission-revoked"} else if connected {"badge account-connected"} else {"badge"},"{label}"}}
        div {class:"permission-controls",role:"group","aria-label":format!("{} channel permissions",name),
            for (capability, label) in [(Capability::ChatRead,"Read chat"),(Capability::AudienceRead,"Read audience"),(Capability::ChatSend,"Send chat")] {
                if capability != Capability::ChatSend || send_supported || grant.capabilities.contains(&capability) {
                    {let active=grant.capabilities.contains(&capability);let identity=grant.clone();
                    rsx! {ActionButton {
                        class:if capability==Capability::ChatSend && !active {"permission-toggle allow-chat-send"} else {"permission-toggle"},
                        pressed:Some(active),disabled,
                        aria_label:Some(if capability==Capability::ChatSend && !active {format!("Allow sending to {} / {}",grant.provider,grant.channel_id)} else {format!("{} for {} / {}",label,grant.provider,grant.channel_id)}),
                        onclick:move |_| update(client,&identity,Some(capability)),
                        span {class:"permission-check","aria-hidden":"true",if active {"✓"} else {""}}
                        "{label}"
                    }}}
                }
            }
        }
        if revoked {p {class:"form-help","Connections cannot restore access. Choose permissions above to allow this channel again."}}
        div {class:"permission-footer",
            ActionButton {class:"permission-details",expanded:Some(details()),onclick:move |_|details.set(!details()),if details() {"Hide account details"} else {"Account details"}}
            ActionButton {class:"permission-revoke",disabled:disabled||revoked,aria_label:Some(format!("Revoke access to {} / {}",grant.provider,grant.channel_id)),onclick:move |_|update(client,&identity,None),"Revoke access"}
        }
        if details() {dl {class:"permission-identity",dt {"Account"}dd {"{grant.account_id}"}dt {"Channel"}dd {"{grant.channel_id}"}
            if let Some(stream)=stream {dt {"Current broadcast"}dd {"{stream}"}}
        }}
    }}
}

fn update(client: ControlClient, identity: &CapabilityGrant, capability: Option<Capability>) {
    let Some(snapshot) = client.state.peek().clone() else {
        return;
    };
    if snapshot.workspace_id.as_deref() != Some(client.project.peek().id.as_str()) {
        return;
    }
    let mut grants = snapshot.client_capabilities;
    let Some(grant) = grants.iter_mut().find(|g| {
        g.provider == identity.provider
            && g.account_id == identity.account_id
            && g.channel_id == identity.channel_id
    }) else {
        return;
    };
    if let Some(capability) = capability {
        if !grant.capabilities.remove(&capability) {
            grant.capabilities.insert(capability);
        }
    } else {
        grant.capabilities.clear();
    }
    client.command(ControlAction::SetClientCapabilities { grants });
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
