//! Host-rendered extension panels shared by native and web clients.
use acheflow_extension_api::{ExtensionPanel, PanelWidget};
use acheflow_media_api::control::ControlAction;
use dioxus_native::prelude::*;

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
                for widget in panel.widgets.clone() {{render_widget(widget,panel.extension_id.clone(),client)}}
            }
        }
    }
}

fn render_widget(
    widget: PanelWidget,
    extension: String,
    client: crate::control_client::ControlClient,
) -> Element {
    match widget {
        PanelWidget::Row { widgets } => {
            rsx! {div{class:"extension-action-row",for widget in widgets{{render_widget(widget,extension.clone(),client)}}}}
        }
        PanelWidget::Disclosure { label, widgets } => {
            rsx! {PanelDisclosure{label,widgets,render:Callback::new(move|widget|render_widget(widget,extension.clone(),client))}}
        }

        PanelWidget::SecretInput { .. } => {
            rsx! {p{class:"form-help","Credentials can only be entered in a local client extension."}}
        }
        PanelWidget::Label { text } => rsx! {p{"{text}"}},
        PanelWidget::Link { label, url } => {
            rsx! {a{href:url,target:"_blank",rel:"noopener noreferrer","{label}"}}
        }
        PanelWidget::Metric { label, value } => {
            rsx! {div{class:"extension-metric",span{"{label}"}strong{"{value}"}}}
        }
        PanelWidget::Button { id, label, primary } => {
            rsx! {button{class:if primary{"primary"}else{""},disabled:(client.busy)(),onclick:move |_|client.command(ControlAction::ExtensionEvent{id:extension.clone(),action_id:id.clone(),value:String::new(),config:None}),"{label}"}}
        }
        PanelWidget::TextInput { id, label, value } => {
            rsx! {label{class:"field",span{"{label}"}input{value:"{value}",maxlength:8192,onchange:move |e|client.command(ControlAction::ExtensionEvent{id:extension.clone(),action_id:id.clone(),value:e.value(),config:None})}}}
        }
        PanelWidget::Select {
            id,
            label,
            value,
            options,
        } => {
            rsx! {div{class:"field",span{"{label}"}div{class:"row",role:"group","aria-label":label.clone(),for option in options{button{class:if value==option{"selected"}else{""},onclick:{let extension=extension.clone();let id=id.clone();let option=option.clone();move |_|client.command(ControlAction::ExtensionEvent{id:extension.clone(),action_id:id.clone(),value:option.clone(),config:None})},"{option}"}}}}}
        }
    }
}
#[component]
pub(crate) fn PanelDisclosure(
    label: String,
    widgets: Vec<PanelWidget>,
    render: Callback<PanelWidget, Element>,
) -> Element {
    let mut open = use_signal(|| false);
    rsx! {div{class:"extension-disclosure",
        button{class:"extension-disclosure-toggle","aria-expanded":open().to_string(),
            onclick:move|_|{let current=open();open.set(!current);},
            onkeydown:move|event|{if event.key()==Key::Named(NamedKey::Enter)||event.key()==Key::Character(" ".into()){event.prevent_default();let current=open();open.set(!current);}},
            span{"{label}"}span{class:"extension-disclosure-caret","aria-hidden":"true",if open(){"−"}else{"+"}}
        }
        if open(){div{class:"extension-disclosure-content",for widget in widgets{{render.call(widget)}}}}
    }}
}
