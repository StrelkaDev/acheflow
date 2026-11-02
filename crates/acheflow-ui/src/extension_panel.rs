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
        section { class:"panel", key:"{panel.extension_id}",
            div {class:"panel-heading",h2 {"{panel.title}"}}
            div {class:"extension-content padded",
                for widget in panel.widgets.clone() {{let extension=panel.extension_id.clone();match widget {
                    PanelWidget::Label{text}=>rsx!{p{"{text}"}},
                    PanelWidget::Metric{label,value}=>rsx!{div{class:"extension-metric",span{"{label}"}strong{"{value}"}}},
                    PanelWidget::Button{id,label}=>rsx!{button{disabled:(client.busy)(),onclick:move |_|client.command(ControlAction::ExtensionEvent{id:extension.clone(),action_id:id.clone(),value:String::new()}),"{label}"}},
                    PanelWidget::TextInput{id,label,value}=>rsx!{label{class:"field",span{"{label}"}input{value:"{value}",maxlength:8192,onchange:move |e|client.command(ControlAction::ExtensionEvent{id:extension.clone(),action_id:id.clone(),value:e.value()})}}},
                    PanelWidget::Select{id,label,value,options}=>rsx!{div{class:"field",span{"{label}"}div{class:"row",role:"group","aria-label":label.clone(),for option in options{button{class:if value==option{"selected"}else{""},onclick:{let extension=extension.clone();let id=id.clone();let option=option.clone();move |_|client.command(ControlAction::ExtensionEvent{id:extension.clone(),action_id:id.clone(),value:option.clone()})},"{option}"}}}}},
                }}}
            }
        }
    }
}
