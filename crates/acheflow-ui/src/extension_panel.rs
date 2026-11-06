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
        PanelWidget::SettingInput { .. } => {
            rsx! {p{class:"form-help","Application settings belong to a local client extension."}}
        }
        PanelWidget::Label { text } => rsx! {p{"{text}"}},
        PanelWidget::Link { label, url } => {
            rsx! {a{href:url,target:"_blank",rel:"noopener noreferrer","{label}"}}
        }
        PanelWidget::Metric { label, value } => {
            rsx! {div{class:"extension-metric",span{"{label}"}strong{"{value}"}}}
        }
        PanelWidget::CopyText { label, value } => rsx! {CopyText{label,value}},
        PanelWidget::Button {
            id,
            label,
            primary,
            requires,
        } => {
            rsx! {button{class:if primary{"primary"}else{""},disabled:(client.busy)() || !requires.is_empty(),onclick:move |_|client.command(ControlAction::ExtensionEvent{id:extension.clone(),action_id:id.clone(),value:String::new(),config:None}),"{label}"}}
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
pub(crate) fn CopyText(label: String, value: String) -> Element {
    let mut copied = use_signal(|| None::<String>);
    let mut error = use_signal(String::new);
    let copy_value = value.clone();
    let copy = Callback::new(move |()| {
        let value = copy_value.clone();
        spawn(async move {
            match copy_text(&value).await {
                Ok(()) => {
                    copied.set(Some(value));
                    error.set(String::new());
                }
                Err(reason) => {
                    copied.set(None);
                    error.set(reason);
                }
            }
        });
    });
    rsx! {div{class:"extension-copy-field",
        label{class:"field",span{"{label}"}input{r#type:"text",readonly:true,value:value.clone()}},
        button{class:"local-extension-action","aria-label":"Copy {label}",onclick:move|_|copy.call(()),
            onkeydown:move|event|{if event.key()==Key::Named(NamedKey::Enter)||event.key()==Key::Character(" ".into()){event.prevent_default();copy.call(());}},
            if copied.read().as_ref()==Some(&value){"Copied"}else{"Copy"}},
        if !error().is_empty(){p{class:"field-error",role:"alert","{error}"}}
    }}
}

async fn copy_text(text: &str) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        let window = web_sys::window()
            .ok_or("Browser clipboard is unavailable. Select the address to copy it manually.")?;
        wasm_bindgen_futures::JsFuture::from(window.navigator().clipboard().write_text(text))
            .await
            .map(|_| ())
            .map_err(|_| {
                "Clipboard request was denied. Select the address to copy it manually.".into()
            })
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        try_consume_context::<std::sync::Arc<dyn blitz_traits::shell::ShellProvider>>()
            .ok_or_else(|| {
                "Clipboard is unavailable. Select the address to copy it manually.".to_owned()
            })?
            .set_clipboard_text(text.to_owned())
            .map_err(|_| "Clipboard request failed. Select the address to copy it manually.".into())
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
