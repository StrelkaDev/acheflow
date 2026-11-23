//! Local playback of a single Studio monitoring target.
use crate::project::{Orientation, Workspace};
use acheflow_media_api::{
    access::Permission,
    audio::{AUDIO_RATE, AUDIO_SAMPLES},
};
use dioxus_native::prelude::*;
use futures_util::StreamExt;

mod output;
pub(crate) use output::Device as OutputDevice;
pub(crate) use output::OutputSettings;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Target {
    Source {
        source: u32,
        track: u8,
    },
    Bus {
        orientation: Orientation,
        scene: u32,
        bus: u32,
    },
}

impl Target {
    fn url(self, workspace: &Workspace) -> Result<String, String> {
        let path = match self {
            Self::Source { source, track } => format!("source/{source}/{track}"),
            Self::Bus {
                orientation,
                scene,
                bus,
            } => {
                let orientation = orientation.label().to_ascii_lowercase();
                format!("bus/{orientation}/{scene}/{bus}")
            }
        };
        crate::media::server_url(
            workspace,
            &format!("/api/live/monitor/{path}?workspace={}", workspace.id),
        )
    }
}

#[derive(Clone, PartialEq)]
enum PlaybackState {
    Connecting,
    Playing,
    Error(String),
}

#[derive(Clone, Copy)]
struct Monitor {
    owner: Signal<Option<(dioxus_core::ScopeId, Target)>>,
    task: Signal<Option<dioxus_core::Task>>,
    state: Signal<Option<PlaybackState>>,
    volume: Signal<u32>,
    catalog: Signal<crate::workspaces::Catalog>,
    project: Signal<Workspace>,
}

pub(crate) fn provide(project: Signal<Workspace>, catalog: Signal<crate::workspaces::Catalog>) {
    let monitor = Monitor {
        owner: use_signal(|| None),
        task: use_signal(|| None),
        state: use_signal(|| None),
        volume: use_signal(|| 65),
        catalog,
        project,
    };
    use_context_provider(|| monitor);
}

impl Monitor {
    fn stop(mut self) {
        if let Some(task) = self.task.take() {
            task.cancel();
        }
        self.owner.set(None);
        self.state.set(None);
    }

    fn start(mut self, scope: dioxus_core::ScopeId, target: Target) {
        self.stop();
        self.owner.set(Some((scope, target)));
        let playback = target.url(&self.project.peek()).and_then(|url| {
            Playback::open(self.catalog.peek().monitor_output()).map(|playback| (url, playback))
        });
        let (url, playback) = match playback {
            Ok(playback) => playback,
            Err(error) => {
                self.state.set(Some(PlaybackState::Error(error)));
                return;
            }
        };
        self.state.set(Some(PlaybackState::Connecting));
        let task = spawn(async move {
            let error = run(self, url, playback).await.err();
            self.state.set(error.map(PlaybackState::Error));
        });
        self.task.set(Some(task));
    }
}

async fn run(mut monitor: Monitor, url: String, mut playback: Playback) -> Result<(), String> {
    playback.ready().await?;
    let response = crate::server_access::request(reqwest::Method::GET, &url)?
        .send()
        .await
        .map_err(|_| "Cannot connect to the audio source.")?;
    match response.status().as_u16() {
        200 => {}
        401 | 403 => return Err("This device cannot monitor audio.".into()),
        404 => return Err("The selected source or bus has no audio stream yet.".into()),
        429 => return Err("The server audio monitoring slots are busy.".into()),
        _ => return Err("Cannot open the audio monitor.".into()),
    }
    if response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        != Some("application/octet-stream")
    {
        return Err("The server returned an unexpected audio format.".into());
    }
    let mut stream = response.bytes_stream();
    // 80 ms amortizes buffer scheduling while keeping the playback queue short.
    const BATCH_BYTES: usize = 4 * AUDIO_SAMPLES * 2;
    let mut waiting = Vec::with_capacity(BATCH_BYTES);
    let mut started = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "Audio monitor disconnected.")?;
        let mut bytes = chunk.as_ref();
        while !bytes.is_empty() {
            let take = bytes.len().min(BATCH_BYTES - waiting.len());
            waiting.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if waiting.len() == BATCH_BYTES {
                playback.push(&waiting, *monitor.volume.peek() as f32 / 100.)?;
                waiting.clear();
                if !started {
                    monitor.state.set(Some(PlaybackState::Playing));
                    started = true;
                }
            }
        }
    }
    Err("Audio monitor ended. Reopen it when the signal returns.".into())
}

fn allowed() -> bool {
    crate::server_access::allows(Permission::StudioView)
        && crate::server_access::allows(Permission::MediaManage)
}

#[component]
pub(crate) fn MonitorButton(
    target: Target,
    #[props(default)] disabled: bool,
    #[props(default)] source_preview: bool,
) -> Element {
    let Some(mut monitor) = try_consume_context::<Monitor>() else {
        return rsx! {};
    };
    let scope = dioxus_core::current_scope_id();
    let permitted = allowed();
    use_drop(move || {
        if monitor
            .owner
            .peek()
            .is_some_and(|(owner, _)| owner == scope)
        {
            monitor.stop();
        }
    });
    use_effect(use_reactive(
        (&target, &permitted, &disabled),
        move |(target, permitted, disabled)| {
            if monitor.owner.peek().is_some_and(|(owner, playing)| {
                owner == scope && (playing != target || !permitted || disabled)
            }) {
                monitor.stop();
            }
        },
    ));
    if !permitted {
        return rsx! {};
    }
    let active = *monitor.owner.read() == Some((scope, target));
    let state = active.then(|| monitor.state.read().clone()).flatten();
    let failed = matches!(state, Some(PlaybackState::Error(_)));
    rsx! {
        div {class:"audio-monitor-control",
            crate::server_settings::ActionButton {class:if active {"selected"} else {""},pressed:Some(active),disabled,
                onclick:move |_| {
                    if active && !failed {monitor.stop();} else {monitor.start(scope,target);}
                },
                if failed {"Retry audio"} else if active {"Stop listening"} else if source_preview {"Listen to source"} else {"Listen"}
            }
            if active && !failed {
                crate::slider::Slider {label:"Monitor volume",unit:"%",min:0,max:100,
                    value:(monitor.volume)() as i32,change:move |value:i32|monitor.volume.set(value as u32)
                }
            }
            match state {
                Some(PlaybackState::Connecting) => rsx! {p {class:"form-help",role:"status","Connecting to audio…"}},
                Some(PlaybackState::Error(error)) => rsx! {p {class:"field-error",role:"alert","{error}"}},
                _ => rsx! {},
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct Playback {
    player: rodio::Player,
    _device: rodio::MixerDeviceSink,
}

#[cfg(not(target_arch = "wasm32"))]
impl Playback {
    fn open(output: Option<&output::Device>) -> Result<Self, String> {
        let device = output::native_device(output)?;
        let device = rodio::DeviceSinkBuilder::from_device(device)
            .and_then(|builder| builder.open_stream())
            .map_err(|_| "Cannot open the monitor output. Check the device in Settings.")?;
        let player = rodio::Player::connect_new(device.mixer());
        Ok(Self {
            player,
            _device: device,
        })
    }

    async fn ready(&self) -> Result<(), String> {
        Ok(())
    }

    fn push(&mut self, bytes: &[u8], volume: f32) -> Result<(), String> {
        self.player.set_volume(volume);
        if self.player.len() < 3 {
            let samples = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|sample| f32::from(i16::from_le_bytes(*sample)) / 32768.)
                .collect::<Vec<_>>();
            self.player.append(rodio::buffer::SamplesBuffer::new(
                std::num::NonZero::new(2).unwrap(),
                std::num::NonZero::new(AUDIO_RATE).unwrap(),
                samples,
            ));
        }
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(module = "/src/audio_monitor.js")]
extern "C" {
    type WebPlayback;
    #[wasm_bindgen(catch, js_name = openMonitor)]
    fn open_monitor(rate: u32, sink_id: &str) -> Result<WebPlayback, JsValue>;
    #[wasm_bindgen(method, getter)]
    fn ready(this: &WebPlayback) -> js_sys::Promise;
    #[wasm_bindgen(method, catch)]
    fn play(this: &WebPlayback, bytes: &[u8], volume: f32) -> Result<(), JsValue>;
    #[wasm_bindgen(method)]
    fn close(this: &WebPlayback);
}

#[cfg(target_arch = "wasm32")]
struct Playback(WebPlayback);

#[cfg(target_arch = "wasm32")]
impl Playback {
    fn open(output: Option<&output::Device>) -> Result<Self, String> {
        open_monitor(AUDIO_RATE, output.map_or("", |device| device.id.as_str()))
            .map(Self)
            .map_err(playback_error)
    }

    async fn ready(&self) -> Result<(), String> {
        wasm_bindgen_futures::JsFuture::from(self.0.ready())
            .await
            .map(|_| ())
            .map_err(playback_error)
    }

    fn push(&mut self, bytes: &[u8], volume: f32) -> Result<(), String> {
        self.0
            .play(bytes, volume)
            .map_err(|_| "The browser stopped audio playback.".into())
    }
}

#[cfg(target_arch = "wasm32")]
fn playback_error(error: JsValue) -> String {
    error
        .as_string()
        .unwrap_or_else(|| "Audio playback is unavailable in this browser.".into())
}

#[cfg(target_arch = "wasm32")]
impl Drop for Playback {
    fn drop(&mut self) {
        self.0.close();
    }
}
