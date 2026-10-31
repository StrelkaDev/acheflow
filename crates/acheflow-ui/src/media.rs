use crate::project::{Orientation, Workspace};
use acheflow_media_api::{MediaCapabilities, MediaType, codec_label};
use dioxus_native::prelude::*;

type Capabilities = Resource<Result<MediaCapabilities, String>>;

pub fn use_capabilities() -> (Capabilities, Signal<u32>) {
    let revision = use_signal(|| 0u32);
    let capabilities = use_resource(move || {
        let refresh = revision() > 0;
        async move { fetch(refresh).await }
    });
    (capabilities, revision)
}

async fn fetch(refresh: bool) -> Result<MediaCapabilities, String> {
    let url = server_url("/api/media/capabilities")?;
    let method = if refresh {
        reqwest::Method::POST
    } else {
        reqwest::Method::GET
    };
    reqwest::Client::new()
        .request(method, url)
        .timeout(std::time::Duration::from_secs(25))
        .send()
        .await
        .map_err(|e| format!("Cannot reach media server: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Media API failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Invalid media API response: {e}"))
}

#[component]
pub fn FfmpegStatus(capabilities: Capabilities, mut refresh: Signal<u32>) -> Element {
    let mut inventory = use_signal(|| false);
    let loading = *capabilities.state().read() == UseResourceState::Pending;
    let snapshot = capabilities.read().clone();
    rsx! {section {class:"panel ffmpeg-status",
        div {class:"panel-heading",h2 {"FFmpeg"}div {class:"row",
            button {disabled:loading,onclick:move |_| {*refresh.write()+=1;},if loading {"Detecting…"}else{"Refresh encoders"}}
            button {onclick:move |_|inventory.set(!inventory()),if inventory(){"Hide inventory"}else{"Encoder inventory"}}
        }}
        div {class:"padded",
            if let Some(Ok(caps))=snapshot.as_ref() {
                if caps.available {
                    strong {{caps.version.as_deref().unwrap_or("FFmpeg")}}
                    p {"{caps.encoders.iter().filter(|e|e.media_type==MediaType::Video).count()} video encoders · {caps.encoders.iter().filter(|e|e.media_type==MediaType::Audio).count()} audio encoders"}
                    p {if caps.rtmp_output && caps.flv_muxer {"RTMP output / FLV muxer detected"}else{"This FFmpeg build cannot publish RTMP / FLV"}}
                    p {class:"form-help","Detected in the server FFmpeg build; hardware devices are not probed."}
                    if inventory() {div {class:"encoder-inventory",role:"table","aria-label":"FFmpeg encoders",
                        for encoder in caps.encoders.iter() {div {class:"encoder-inventory-row",role:"row",
                            span {role:"cell","{encoder.name}"}span {role:"cell","{encoder.codec}"}span {role:"cell","{encoder.description}"}
                        }}
                    }}
                }else {p {role:"alert",class:"field-error",{{caps.error.as_deref().unwrap_or("FFmpeg is unavailable on the server")}}}}
            }else if let Some(Err(error))=snapshot.as_ref() {p {role:"alert",class:"field-error","{error}"}}
            else {p {"Reading encoders from the media server…"}}
        }
    }}
}

#[component]
pub fn EncoderPicker(
    mut project: Signal<Workspace>,
    orientation: Orientation,
    index: usize,
    capabilities: Option<MediaCapabilities>,
) -> Element {
    let Some(output) = project
        .read()
        .stream(orientation)
        .outputs
        .get(index)
        .cloned()
    else {
        return rsx! {};
    };
    let offered = capabilities
        .as_ref()
        .map(|caps| caps.rtmp_codecs())
        .unwrap_or_default();
    let encoders = capabilities
        .as_ref()
        .map(|caps| {
            caps.video_encoders(&output.codec)
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let codec_available = offered.contains(&output.codec);
    let encoder_available = output
        .encoder
        .as_ref()
        .is_none_or(|name| encoders.iter().any(|e| &e.name == name));
    rsx! {div {class:"field codec-picker",
        span {"Video codec"}
        div {class:"row",role:"group","aria-label":"Video codec",
            for codec in offered {button {class:if output.codec==codec {"selected"}else{""},"aria-pressed":(output.codec==codec).to_string(),onclick:{let codec=codec.clone();move |_| {let mut p=project.write();let output=&mut p.stream_mut(orientation).outputs[index];output.codec=codec.clone();output.encoder=None;}},"{codec_label(&codec)}"}}
        }
        if codec_available {
            span {"FFmpeg encoder"}
            div {class:"row",role:"group","aria-label":"FFmpeg video encoder",
                button {class:if output.encoder.is_none(){"selected"}else{""},"aria-pressed":output.encoder.is_none().to_string(),onclick:move |_|project.write().stream_mut(orientation).outputs[index].encoder=None,"Automatic"}
                for encoder in encoders {button {class:if output.encoder.as_deref()==Some(encoder.name.as_str()) {"selected"}else{""},"aria-pressed":(output.encoder.as_deref()==Some(encoder.name.as_str())).to_string(),title:encoder.description,onclick:{let name=encoder.name.clone();move |_|project.write().stream_mut(orientation).outputs[index].encoder=Some(name.clone())},"{encoder.name}"}}
            }
            if output.codec!="h264" {span {class:"form-help","Enhanced RTMP · destination must support this codec"}}
        }else {span {class:"field-error",role:"status","Saved codec: {codec_label(&output.codec)}. No matching RTMP encoder detected on this server."}}
        if !encoder_available {span {class:"field-error",role:"status","Saved encoder is unavailable on this server. Choose another encoder."}}
    }}
}

pub fn server_url(path: &str) -> Result<String, String> {
    #[cfg(target_arch = "wasm32")]
    let base = web_sys::window()
        .ok_or("No browser window")?
        .location()
        .origin()
        .map_err(|e| format!("{e:?}"))?;
    #[cfg(not(target_arch = "wasm32"))]
    let base =
        std::env::var("ACHEFLOW_SERVER_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".into());
    Ok(format!("{}{path}", base.trim_end_matches('/')))
}
