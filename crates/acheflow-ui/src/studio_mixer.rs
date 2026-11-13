//! Compact controls for the scene actually being mixed in each Studio format.
use crate::{
    audio_mixer::{Meters, StripControls, edit_strip},
    live_client::{Action, LiveClient},
    project::{Orientation, Scene, Workspace},
    slider::Slider,
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
pub(crate) fn StudioMixer(mut project: Signal<Workspace>, orientation: Orientation) -> Element {
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
    let scene = current_scene(&workspace, orientation, running).cloned();
    rsx! {div {class:"studio-mixer",
        div {class:"panel-heading",div {h3 {"Audio mixer"}
            if let Some(scene)=scene.as_ref() {p {"{scene.name}"}}
        }div {class:"row studio-mixer-heading-actions",
            span {class:"badge",if running.is_some(){"Live levels"}else{"Offline"}}
            if scene.is_some(){button {disabled:(client.busy)(),title:"Apply mixer changes to live audio",onclick:move |_|client.command(Action::Apply),"Apply mixer"}}
        }}
        if let Some(scene)=scene {
            div {class:"studio-mixer-scroll",tabindex:0,role:"region","aria-label":"Scene audio mixer channels",
            div {class:"studio-mixer-strips",
                for (key,strip) in scene.audio.effective_strips(scene.layers.iter().filter(|l|workspace.sources.iter().any(|s|s.id==l.source_id&&s.kind.is_rtmp())).map(|l|l.source_id)) {
                    {
                        let source=workspace.sources.iter().find(|s|s.id==key.source_id);
                        let track=status.as_ref().into_iter().flat_map(|s|&s.sources).find(|s|s.id==key.source_id).and_then(|s|s.audio_tracks.iter().find(|t|t.track_id==key.track_id));
                        let sends=scene.audio.buses.iter().filter(|b|strip.sends.get(&b.id).is_some_and(|gain|*gain>0)).map(|b|b.name.as_str()).collect::<Vec<_>>().join(", ");
                        rsx! {div {key:"{scene.id}-{key.source_id}-{key.track_id}",class:"studio-mixer-channel",role:"group","aria-label":format!("{} · track {}",source.map(|s|s.name.as_str()).unwrap_or("Missing source"),key.track_id),
                            div {class:"studio-mixer-title",strong {title:source.map(|s|s.name.clone()).unwrap_or_default(),"{source.map(|s|s.name.as_str()).unwrap_or(\"Missing source\")}"}
                                span {class:"badge","Track {key.track_id} · ",if track.is_some_and(|t|t.active){"Receiving"}else{"Waiting"}}
                            }
                            p {class:"form-help",if sends.is_empty(){"Not routed"}else{"Sends → {sends}"}}
                            div {class:"studio-fader-bank",
                                Slider {label:"Gain",unit:"%",vertical:true,value:strip.gain as i32,min:0,max:200,reset:Some(100),change:move |v:i32|edit_strip(project,scene.id,key,|s|s.gain=v as u32)}
                                Meters {vertical:true,meter:track.map(|t|t.meter.clone()).unwrap_or_default()}
                            }
                            if let Some(error)=track.and_then(|t|t.error.as_ref()){p {class:"field-error",role:"alert","{error}"}}
                            StripControls {project,scene_id:scene.id,strip,show_gain:false}
                        }}
                    }
                }
                if scene.audio.effective_strips(scene.layers.iter().filter(|l|workspace.sources.iter().any(|s|s.id==l.source_id&&s.kind.is_rtmp())).map(|l|l.source_id)).is_empty(){p {class:"padded form-help","No tracks assigned. Add source layers or configure sends in Live / Mixer."}}
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
                            Slider {label:"Gain",unit:"%",vertical:true,value:bus.gain as i32,min:0,max:200,reset:Some(100),change:{let id=bus.id;move |gain:i32|{if let Some(b)=project.write().scenes.iter_mut().find(|s|s.id==scene.id).and_then(|s|s.audio.buses.iter_mut().find(|b|b.id==id)){b.gain=gain as u32;}}}}
                            Meters {vertical:true,meter:levels.map(|b|b.meter.clone()).unwrap_or_default()}
                        }
                        div {class:"studio-mixer-master",
                            button {class:if bus.muted{"selected"}else{""},"aria-label":format!("Mute {} bus",bus.name),"aria-pressed":bus.muted.to_string(),onclick:{let id=bus.id;move |_|{if let Some(b)=project.write().scenes.iter_mut().find(|s|s.id==scene.id).and_then(|s|s.audio.buses.iter_mut().find(|b|b.id==id)){b.muted= !b.muted;}}},"Mute"}
                        }
                    }}
                }
                }
            }}
        }else{p {class:"padded form-help",if running.is_some(){"No applied scene mixer is available. Select a scene and Apply."}else{"Select a scene to display its mixer."}}}
    }}
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
            orientation: Orientation::Horizontal,
            time_seconds: 0.,
            renderer: None,
            error: None,
            frames: 1,
            outputs: vec![],
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
