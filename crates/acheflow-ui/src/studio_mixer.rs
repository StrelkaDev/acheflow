//! Compact controls for the scene actually being mixed in each Studio format.
use crate::{
    audio_mixer::{GainFader, Meters, StripControls, edit_bus, edit_strip},
    live_client::LiveClient,
    project::{Orientation, Scene, Workspace},
    slider::Slider,
};
use acheflow_media_api::audio::{
    AudioStrip, BusChange, MAX_SYNC_OFFSET_MS, StripChange, SyncMode, TrackKey, TrackSyncStatus,
};
use acheflow_media_api::live::FormatStatus;
use dioxus_native::prelude::*;

fn current_scene<'a>(
    workspace: &'a Workspace,
    orientation: Orientation,
    running: Option<&FormatStatus>,
) -> Option<&'a Scene> {
    // A selected scene is a local draft until Apply. On air, follow the server.
    let id = match running {
        Some(format) => format.audio_buses.first().map(|bus| bus.scene_id),
        None => workspace.active(orientation),
    };
    workspace
        .scenes
        .iter()
        .find(|s| Some(s.id) == id && s.orientation == orientation)
}

#[component]
pub(crate) fn StudioMixer(
    project: Signal<Workspace>,
    orientation: Orientation,
    #[props(default)] preview_scene: Option<Option<u32>>,
) -> Element {
    let client = use_context::<LiveClient>();
    let workspace = project.read().clone();
    let status = if client.owns() {
        client.status.read().clone()
    } else {
        None
    };
    let running = status
        .as_ref()
        .and_then(|s| s.formats.iter().find(|f| f.orientation == orientation));
    let scene = if running.is_none()
        && let Some(preview) = preview_scene
    {
        workspace
            .scenes
            .iter()
            .find(|scene| Some(scene.id) == preview && scene.orientation == orientation)
    } else {
        current_scene(&workspace, orientation, running)
    }
    .cloned();
    let strips = scene
        .as_ref()
        .map(|scene| {
            let mut strips = scene.audio.effective_strips(
                scene
                    .layers
                    .iter()
                    .filter(|layer| {
                        workspace.sources.iter().any(|source| {
                            source.id == layer.source_id && source.kind.has_audio_input()
                        })
                    })
                    .map(|layer| layer.source_id),
            );
            for source in workspace
                .sources
                .iter()
                .filter(|source| source.kind.is_audio_only())
            {
                let key = TrackKey {
                    source_id: source.id,
                    track_id: 0,
                };
                strips
                    .entry(key)
                    .or_insert_with(|| AudioStrip::new(key, false));
            }
            strips
        })
        .unwrap_or_default();
    let input_levels = status.as_ref().is_some_and(|status| {
        status.sources.iter().any(|source| {
            source.audio_tracks.iter().any(|track| {
                track.active
                    && strips.contains_key(&TrackKey {
                        source_id: source.id,
                        track_id: track.track_id,
                    })
            })
        })
    });
    rsx! {div {class:"studio-mixer",
        div {class:"panel-heading",div {h3 {"Audio mixer"}
            if let Some(scene)=scene.as_ref() {p {"{scene.name}"}}
        }div {class:"row studio-mixer-heading-actions",
            span {class:"badge",if running.is_some(){"Live levels"}else if input_levels {"Input levels"}else{"Offline"}}
            if (client.scoped_pending)() {span {class:"form-help",role:"status","Saving mix…"}}
        }}
        if let Some(scene)=scene {
            div {class:"studio-mixer-scroll",tabindex:0,role:"region","aria-label":"Scene audio mixer channels",
            div {class:"studio-mixer-strips",
                for (key,strip) in strips.clone() {
                    {
                        let source=workspace.sources.iter().find(|s|s.id==key.source_id);
                        let track=status.as_ref().into_iter().flat_map(|s|&s.sources).find(|s|s.id==key.source_id).and_then(|s|s.audio_tracks.iter().find(|t|t.track_id==key.track_id));
                        let sync=running.and_then(|f|f.audio_sync.iter().find(|t|t.key==key)).cloned();
                        let sends=scene.audio.buses.iter().filter(|b|strip.sends.get(&b.id).is_some_and(|gain|*gain>0)).map(|b|b.name.as_str()).collect::<Vec<_>>().join(", ");
                        rsx! {div {key:"{scene.id}-{key.source_id}-{key.track_id}",class:"studio-mixer-channel",role:"group","aria-label":format!("{} · track {}",source.map(|s|s.name.as_str()).unwrap_or("Missing source"),key.track_id),
                            div {class:"studio-mixer-title",strong {title:source.map(|s|s.name.clone()).unwrap_or_default(),"{source.map(|s|s.name.as_str()).unwrap_or(\"Missing source\")}"}
                                span {class:"badge","Track {key.track_id} · ",if track.is_some_and(|t|t.active){"Receiving"}else{"Waiting"}}
                            }
                            p {class:"form-help",if sends.is_empty(){"Not routed"}else{"Sends → {sends}"}}
                            div {class:"studio-fader-bank",
                                GainFader {label:"Gain",vertical:true,gain:strip.gain,change:move |v:u32|edit_strip(client,scene.id,key,StripChange::Gain(v))}
                                Meters {vertical:true,meter:track.map(|t|t.meter.clone()).unwrap_or_default()}
                            }
                            if let Some(error)=track.and_then(|t|t.error.as_ref()){p {class:"field-error",role:"alert","{error}"}}
                            if source.is_some_and(|source| source.kind.is_audio_only()) {
                                Slider {label:"Audio delay",unit:" ms",value:strip.active_sync_offset().max(0),min:0,max:MAX_SYNC_OFFSET_MS,reset:Some(0),
                                    change:move |ms:i32|edit_strip(client,scene.id,key,StripChange::ManualDelay(ms))}
                            } else {
                                TrackSyncControls {scene_id:scene.id,strip:strip.clone(),sync}
                            }
                            StripControls {scene_id:scene.id,strip,show_gain:false}
                        }}
                    }
                }
                if strips.is_empty(){p {class:"padded form-help","No tracks assigned. Add source layers or configure sends in Live / Mixer."}}
                for bus in &scene.audio.buses {
                {
                    let levels=running.into_iter().flat_map(|f|&f.audio_buses).find(|b|b.scene_id==scene.id && b.bus_id==bus.id);
                    let destinations=if let Some(format)=running {
                        format.outputs.iter().filter(|o|o.audio_scene==Some(scene.id) && o.audio_bus==Some(bus.id)).map(|o|o.name.as_str()).collect::<Vec<_>>()
                    }else{
                        workspace.stream(orientation).outputs.iter().filter(|o|o.enabled && o.audio.bus(scene.id)==bus.id).map(|o|o.name.as_str()).collect::<Vec<_>>()
                    }.join(", ");
                    rsx! {div {key:"{scene.id}-{bus.id}",class:"studio-mixer-channel studio-mixer-bus",role:"group","aria-label":format!("{} bus",bus.name),
                        div {class:"studio-mixer-title",strong {"{bus.name}"}span {class:"badge","Bus"}}
                        p {class:"form-help",if destinations.is_empty(){"No destinations"}else{"→ {destinations}"}}
                        div {class:"studio-fader-bank",
                            GainFader {label:"Gain",vertical:true,gain:bus.gain,change:{let id=bus.id;move |gain:u32|{edit_bus(client,scene.id,id,BusChange::Gain(gain))}}}
                            Meters {vertical:true,meter:levels.map(|b|b.meter.clone()).unwrap_or_default()}
                        }
                        div {class:"studio-mixer-master",
                            button {class:if bus.muted{"selected"}else{""},"aria-label":format!("Mute {} bus",bus.name),"aria-pressed":bus.muted.to_string(),onclick:{let id=bus.id;let muted=bus.muted;move |_|edit_bus(client,scene.id,id,BusChange::Mute(!muted))},"Mute"}
                        }
                    }}
                }
                }
            }}
        }else{p {class:"padded form-help",if running.is_some(){"No applied scene mixer is available. Select a scene and Apply."}else{"Select a scene to display its mixer."}}}
    }}
}

#[component]
fn TrackSyncControls(scene_id: u32, strip: AudioStrip, sync: Option<TrackSyncStatus>) -> Element {
    let key = strip.key();
    let client = use_context::<LiveClient>();
    let requested = strip.active_sync_offset();
    let video_delay = requested.unsigned_abs();
    rsx! {
        div {class:"studio-track-sync",
            span {"Sync mode"}
            div {class:"row studio-sync-modes",role:"group","aria-label":"Sync mode",
                for (mode,label) in [(SyncMode::Auto,"Auto"),(SyncMode::AutoTrim,"Auto + trim"),(SyncMode::Manual,"Manual")] {
                    button {key:"{label}",class:if strip.sync_mode==mode {"selected"}else{""},
                        "aria-pressed":(strip.sync_mode==mode).to_string(),
                        onclick:move |_|edit_strip(client,scene_id,key,StripChange::SyncMode(mode)),"{label}"}
                }
            }
            if strip.sync_mode != SyncMode::Auto {
                Slider {label:"Sync offset",unit:" ms",value:strip.sync_offset_ms,min:-MAX_SYNC_OFFSET_MS,max:MAX_SYNC_OFFSET_MS,reset:Some(0),
                    change:move |ms:i32|edit_strip(client,scene_id,key,StripChange::SyncOffset(ms))}
                small {class:"form-help",
                    if requested > 0 {"Delay audio by {requested} ms"}
                    else if requested < 0 {"Delay source video by {video_delay} ms"}
                    else {"Positive delays audio. Negative delays source video."}
                }
            }
            if let Some(sync)=sync {
                span {class:"badge",
                    if sync.buffering || sync.video_delay_limited {"Sync buffering"}
                    else if sync.mode==SyncMode::Manual {"Manual sync"}
                    else if sync.tracking_pts {"Tracking source PTS"}
                    else {"No common A/V timestamp"}
                }
                small {class:"form-help","Applied · A {sync.audio_delay_ms} ms · V {sync.video_delay_ms} ms"}
                if let Some(offset)=sync.estimated_clock_offset_ms {
                    small {class:"form-help","Clock correction: {offset:+.0} ms"}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acheflow_media_api::audio::{AudioBusStatus, AudioMeter};
    #[test]
    fn mixer_follows_applied_scene_until_selection_is_applied() {
        let mut workspace = Workspace::default();
        let first = workspace.add_scene(Orientation::Horizontal);
        let second = workspace.add_scene(Orientation::Horizontal);
        workspace.set_active(Orientation::Horizontal, Some(second));
        let mut running = FormatStatus {
            applied_scene: None,
            orientation: Orientation::Horizontal,
            time_seconds: 0.,
            renderer: None,
            error: None,
            frames: 1,
            outputs: vec![],
            preview_profiles: vec![],
            audio_sync: vec![],
            render_load: Default::default(),
            audio_buses: vec![AudioBusStatus {
                scene_id: first,
                bus_id: 1,
                name: "Program".into(),
                meter: AudioMeter::default(),
            }],
        };
        assert_eq!(
            current_scene(&workspace, Orientation::Horizontal, None)
                .unwrap()
                .id,
            second
        );
        assert_eq!(
            current_scene(&workspace, Orientation::Horizontal, Some(&running))
                .unwrap()
                .id,
            first
        );
        running.audio_buses[0].scene_id = second;
        assert_eq!(
            current_scene(&workspace, Orientation::Horizontal, Some(&running))
                .unwrap()
                .id,
            second
        );
        assert!(current_scene(&workspace, Orientation::Vertical, Some(&running)).is_none());
        running.audio_buses.clear();
        assert!(current_scene(&workspace, Orientation::Horizontal, Some(&running)).is_none());
        workspace.set_active(Orientation::Horizontal, None);
        assert!(current_scene(&workspace, Orientation::Horizontal, None).is_none());
    }
}
