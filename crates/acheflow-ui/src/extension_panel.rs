//! Host-rendered extension panels shared by native and web clients.
use acheflow_extension_api::{ExtensionPanel, PanelWidget};
use acheflow_media_api::control::ControlAction;
use dioxus_native::prelude::*;

#[component]
pub fn SecretSetting(
    extension: String,
    setting: acheflow_extension_api::integration::Setting,
) -> Element {
    let client = use_context::<crate::control_client::ControlClient>();
    let mut value = use_signal(String::new);
    let state = client.state.read();
    let active = state
        .as_ref()
        .is_some_and(|s| s.workspace_id.as_deref() == Some(client.project.read().id.as_str()));
    let configured = active
        && state
            .as_ref()
            .and_then(|s| s.extensions.iter().find(|s| s.id == extension))
            .is_some_and(|s| s.configured_secrets.contains(&setting.id));
    let unavailable = !active || (client.busy)();
    let save_extension = extension.clone();
    let save_key = setting.id.clone();
    rsx! {
        div {class:"field extension-secret",
            label {span {"{setting.label}"}
                input {r#type:"password",value:value(),maxlength:setting.max_length as i64,autocomplete:"off",spellcheck:"false",disabled:unavailable,
                    oninput:move|event|value.set(event.value())}
            }
            div {class:"row",
                button {disabled:unavailable || value.read().is_empty(),onclick:move |_| {
                    let token=value();
                    value.set(String::new());
                    client.command(ControlAction::SetExtensionSecret{id:save_extension.clone(),key:save_key.clone(),value:acheflow_media_api::control::SecretValue(token),config:None});
                },"Save on server"}
                button {disabled:unavailable || !configured,onclick:move |_| {
                    value.set(String::new());
                    client.command(ControlAction::SetExtensionSecret{id:extension.clone(),key:setting.id.clone(),value:acheflow_media_api::control::SecretValue(String::new()),config:None});
                },"Remove"}
            }
            p {class:"form-help",role:"status",if !active {"Apply the workspace to its server first."} else if configured {"Saved on this server until restart. Enter a new value to replace it."} else {"Not set. Values are never returned by the server."}}
            if !setting.help.is_empty() {p {class:"form-help","{setting.help}"}}
        }
    }
}

pub fn render(panel: &ExtensionPanel) -> Element {
    rsx! { ExtensionPanelView { panel:panel.clone() } }
}
#[component]
fn ExtensionPanelView(panel: ExtensionPanel) -> Element {
    let client = use_context::<crate::control_client::ControlClient>();
    rsx! {
        div { class:"extension-panel", key:"{panel.extension_id}",
            h3 {"{panel.title}"}
            div {class:"extension-content",
                for widget in panel.widgets.clone() {{let extension=panel.extension_id.clone();match widget {
                    PanelWidget::Label{text}=>rsx!{p{"{text}"}},
                    PanelWidget::Link{label,url}=>rsx!{a{href:url,target:"_blank",rel:"noopener noreferrer","{label}"}},
                    PanelWidget::Metric{label,value}=>rsx!{div{class:"extension-metric",span{"{label}"}strong{"{value}"}}},
                    PanelWidget::Button{id,label}=>rsx!{button{disabled:(client.busy)(),onclick:move |_|client.command(ControlAction::ExtensionEvent{id:extension.clone(),action_id:id.clone(),value:String::new(),config:None}),"{label}"}},
                    PanelWidget::TextInput{id,label,value}=>rsx!{label{class:"field",span{"{label}"}input{value:"{value}",maxlength:8192,onchange:move |e|client.command(ControlAction::ExtensionEvent{id:extension.clone(),action_id:id.clone(),value:e.value(),config:None})}}},
                    PanelWidget::Select{id,label,value,options}=>rsx!{div{class:"field",span{"{label}"}div{class:"row",role:"group","aria-label":label.clone(),for option in options{button{class:if value==option{"selected"}else{""},onclick:{let extension=extension.clone();let id=id.clone();let option=option.clone();move |_|client.command(ControlAction::ExtensionEvent{id:extension.clone(),action_id:id.clone(),value:option.clone(),config:None})},"{option}"}}}}},
                }}}
            }
        }
    }
}
