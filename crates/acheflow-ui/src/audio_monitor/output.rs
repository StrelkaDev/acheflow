use super::Monitor;
use dioxus_native::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct Device {
    pub(super) id: String,
    label: String,
}

#[derive(Clone, Deserialize)]
struct Inventory {
    devices: Vec<Device>,
    action: Option<BrowserAction>,
    message: Option<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BrowserAction {
    Choose,
    Discover,
}

impl BrowserAction {
    fn label(self) -> &'static str {
        match self {
            Self::Choose => "Choose another device…",
            Self::Discover => "Allow browser device discovery…",
        }
    }
}

#[component]
pub(crate) fn OutputSettings() -> Element {
    let Some(monitor) = try_consume_context::<Monitor>() else {
        return rsx! {};
    };
    let mut inventory = use_resource(list);
    let mut selection_error = use_signal(|| None::<String>);
    let mut choosing = use_signal(|| false);
    let output = monitor.catalog.read().monitor_output().cloned();
    let result = inventory.read().clone();
    let devices = result.as_ref().and_then(|r| r.as_ref().ok());
    let mut choices = vec!["System default".to_string()];
    let mut ids = vec![String::new()];
    if let Some(devices) = devices {
        choices.extend(devices.devices.iter().map(|device| device.label.clone()));
        ids.extend(devices.devices.iter().map(|device| device.id.clone()));
    }
    let selected = output
        .as_ref()
        .map(|device| device.id.as_str())
        .unwrap_or("");
    let unavailable = result.is_some() && !ids.iter().any(|id| id == selected);
    rsx! {
        section {class:"panel",
            div {class:"panel-heading",h2 {"Audio monitoring"}}
            div {class:"padded",
                crate::live::Choice {label:"Monitor output",value:selected.to_owned(),choices,values:Some(ids),
                    disabled:choosing(),
                    change:move |id: String| {
                        let device = if id.is_empty() {
                            None
                        } else {
                            let Some(device) = inventory.peek().as_ref()
                                .and_then(|r| r.as_ref().ok())
                                .and_then(|list| list.devices.iter().find(|device| device.id == id))
                                .cloned() else {return;};
                            Some(device)
                        };
                        selection_error.set(None);
                        set_output(monitor, device);
                    }
                }
                p {class:"form-help","Changing the output stops local listening. Use Listen to resume."}
                if let Some(device) = output.filter(|_| unavailable) {
                    p {class:"form-help",role:"status","Saved output: {device.label}. Reconnect or grant access to this device, then refresh the list."}
                }
                if let Some(message) = devices.and_then(|list| list.message.as_deref()) {
                    p {class:"form-help",role:"status","{message}"}
                }
                if let Some(Err(error)) = result.as_ref() {
                    p {class:"field-error",role:"alert","{error}"}
                }
                if let Some(error) = selection_error() {
                    p {class:"field-error",role:"alert","{error}"}
                }
                div {class:"row",
                    button {r#type:"button",disabled:choosing(),onclick:move |_|inventory.restart(),"Refresh audio outputs"}
                    if let Some(action) = devices.and_then(|list| list.action) {
                        button {r#type:"button",disabled:choosing(),onclick:move |_| {
                            let request = request_access(action);
                            choosing.set(true);
                            selection_error.set(None);
                            spawn(async move {
                                match request.await {
                                    Ok(device) => {
                                        if let Some(device) = device {
                                            set_output(monitor, Some(device));
                                        }
                                        inventory.restart();
                                    }
                                    Err(error) => selection_error.set(Some(error)),
                                }
                                choosing.set(false);
                            });
                        },"{action.label()}"}
                    }
                }
            }
        }
    }
}

fn set_output(mut monitor: Monitor, output: Option<Device>) {
    if monitor.catalog.peek().monitor_output() != output.as_ref() {
        monitor.stop();
        monitor.catalog.write().set_monitor_output(output);
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn list() -> Result<Inventory, String> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    let devices = rodio::cpal::default_host()
        .output_devices()
        .map_err(|_| "Cannot list audio output devices.")?;
    let mut devices = devices
        .filter_map(|device| {
            Some(Device {
                id: device.id().ok()?.to_string(),
                label: device.description().ok()?.name().to_owned(),
            })
        })
        .collect::<Vec<_>>();
    devices.sort_by(|a, b| a.label.cmp(&b.label).then(a.id.cmp(&b.id)));
    Ok(Inventory {
        devices,
        action: None,
        message: None,
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn native_device(output: Option<&Device>) -> Result<rodio::Device, String> {
    use rodio::cpal::traits::HostTrait;
    let host = rodio::cpal::default_host();
    match output {
        Some(device) => device
            .id
            .parse()
            .ok()
            .and_then(|id| host.device_by_id(&id))
            .ok_or_else(|| {
                format!(
                    "Monitor output \"{}\" is unavailable. Check Settings.",
                    device.label
                )
            }),
        None => host
            .default_output_device()
            .ok_or_else(|| "The system has no default audio output.".into()),
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn request_access(_: BrowserAction) -> Result<Option<Device>, String> {
    Ok(None)
}

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(module = "/src/audio_monitor.js")]
extern "C" {
    #[wasm_bindgen(js_name = listMonitorOutputs)]
    fn list_monitor_outputs() -> js_sys::Promise;
    #[wasm_bindgen(js_name = requestMonitorOutputAccess)]
    fn request_monitor_output_access(choose: bool) -> js_sys::Promise;
}

#[cfg(target_arch = "wasm32")]
async fn list() -> Result<Inventory, String> {
    decode(list_monitor_outputs()).await
}

#[cfg(target_arch = "wasm32")]
fn request_access(
    action: BrowserAction,
) -> impl std::future::Future<Output = Result<Option<Device>, String>> {
    decode(request_monitor_output_access(matches!(
        action,
        BrowserAction::Choose
    )))
}

#[cfg(target_arch = "wasm32")]
async fn decode<T: serde::de::DeserializeOwned>(request: js_sys::Promise) -> Result<T, String> {
    let value = wasm_bindgen_futures::JsFuture::from(request)
        .await
        .map_err(|error| {
            error
                .as_string()
                .unwrap_or_else(|| "Cannot access browser audio outputs.".into())
        })?;
    serde_json::from_str(&value.as_string().unwrap_or_default())
        .map_err(|_| "The browser returned an invalid audio device list.".into())
}
