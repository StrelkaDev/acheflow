use crate::{live_client::LiveClient, project::Workspace, slider::Slider};
use acheflow_media_api::audio::{
    AudioMeter, AudioStrip, BusChange, MixerEdit, StripChange, TrackKey,
};
use acheflow_media_api::live::MeterTarget;
use dioxus_native::prelude::*;
use std::collections::BTreeMap;

pub(crate) fn edit_strip(client: LiveClient, scene: u32, key: TrackKey, change: StripChange) {
    client.mixer_edit(MixerEdit::Strip { scene, key, change });
}

pub(crate) fn edit_bus(client: LiveClient, scene: u32, bus: u32, change: BusChange) {
    client.mixer_edit(MixerEdit::Bus { scene, bus, change });
}

#[component]
pub(crate) fn LiveMeters(
    target: MeterTarget,
    fallback: AudioMeter,
    #[props(default)] vertical: bool,
) -> Element {
    let client = use_context::<LiveClient>();
    let project = client.project.read();
    let status = client.status.read();
    let telemetry = client.telemetry.read();
    let meter = telemetry
        .as_ref()
        .filter(|snapshot| {
            snapshot.workspace_id.as_deref() == Some(project.id.as_str())
                && (!matches!(target, MeterTarget::Output { .. })
                    || status
                        .as_ref()
                        .is_some_and(|status| status.edit_revision == snapshot.edit_revision))
        })
        .map_or(fallback, |snapshot| {
            snapshot
                .meters
                .iter()
                .find(|reading| reading.target == target)
                .map(|reading| reading.meter.clone())
                .unwrap_or_default()
        });
    rsx! {Meters {meter,vertical,source:matches!(target, MeterTarget::Source { .. })}}
}

#[component]
pub fn Meters(
    meter: AudioMeter,
    #[props(default)] vertical: bool,
    #[props(default)] source: bool,
) -> Element {
    rsx! {div {class:if vertical {"audio-meters vertical-meters"}else{"audio-meters"},
        for channel in 0..2 {
            {
                let peak=f64::from(meter.peak[channel]);
                let db=decibels(peak);
                let percentage=db_position(db, -60.);
                let held=f64::from(meter.held_peak[channel]);
                let readout=db_text(peak, "dBFS");
                rsx! {div {class:"audio-meter-row",span {if channel==0 {"L"}else{"R"}}
                    div {class:"audio-meter",role:"meter","aria-label":if channel==0 {"Left peak"}else{"Right peak"},"aria-valuemin":"-60","aria-valuemax":"6.02","aria-valuenow":format!("{:.1}",db.clamp(-60.,max_gain_db())),"aria-valuetext":readout.clone(),
                        div {class:"audio-level"}
                        div {class:"audio-meter-mask",style:format!("{}:{:.2}%;",if vertical {"height"}else{"width"},100.-percentage)}
                        div {class:"audio-meter-zero",title:"0 dBFS"}
                        if held > 0. {
                            div {class:"audio-meter-peak-hold","aria-hidden":"true",style:format!("{}:{:.2}%;",if vertical {"bottom"}else{"left"},db_position(decibels(held), -60.))}
                        }
                    }
                    span {class:"audio-db","{readout}"}
                }}
            }
        }
        span {class:"field-error audio-meter-clip",role:"status",
            if meter.recently_clipped || meter.clipped_samples > 0 {
                if source {"Full scale"} else {"Clipping"}
            }
        }
    }}
}

const UNITY_POSITION: i32 = 800;
const FADER_END: i32 = 1000;
const FADER_MIN_DB: f64 = -40.;

fn decibels(linear: f64) -> f64 {
    20. * linear.max(0.).log10()
}

fn max_gain_db() -> f64 {
    decibels(2.)
}

fn db_text(linear: f64, unit: &str) -> String {
    if linear <= 0. {
        format!("−∞ {unit}")
    } else {
        format!("{:+.1} {unit}", decibels(linear))
    }
}

fn db_position(db: f64, floor: f64) -> f64 {
    if db <= 0. {
        80. * (1. - db.clamp(floor, 0.) / floor)
    } else {
        80. + 20. * (db / max_gain_db()).min(1.)
    }
}

fn gain_position(gain: u32) -> i32 {
    let db = decibels(f64::from(gain) / 100.);
    if gain == 0 {
        0
    } else if gain <= 100 {
        1 + ((db_position(db, FADER_MIN_DB) / 80.) * f64::from(UNITY_POSITION - 1)).round() as i32
    } else {
        (db_position(db, FADER_MIN_DB) * 10.).round() as i32
    }
}

fn position_gain(position: i32) -> u32 {
    if position == 0 {
        return 0;
    }
    let db = if position <= UNITY_POSITION {
        FADER_MIN_DB * (1. - f64::from(position - 1) / f64::from(UNITY_POSITION - 1))
    } else {
        max_gain_db() * f64::from(position - UNITY_POSITION) / f64::from(FADER_END - UNITY_POSITION)
    };
    (100. * 10_f64.powf(db / 20.)).round() as u32
}

#[component]
pub(crate) fn GainFader(
    label: String,
    gain: u32,
    change: EventHandler<u32>,
    #[props(default = false)] vertical: bool,
    #[props(default = true)] reset_unity: bool,
) -> Element {
    rsx! {Slider {
        label, vertical, value:gain_position(gain),min:0,max:FADER_END,
        marker:Some(UNITY_POSITION),reset:reset_unity.then_some(UNITY_POSITION),
        format_value:Callback::new(|position:i32| db_text(f64::from(position_gain(position))/100., "dB")),
        change:move |position|change.call(position_gain(position)),
    }}
}

#[cfg(test)]
mod scale_tests {
    use super::*;

    #[test]
    fn audio_scales_place_unity_at_eighty_percent_and_keep_saved_gain_exact() {
        assert_eq!(gain_position(100), 800);
        assert_eq!(db_position(decibels(1.), -60.), 80.);
        assert_eq!(db_position(decibels(0.), -60.), 0.);
        assert!(db_position(decibels(0.1), -60.) > 50.);
        for gain in 0..=200 {
            assert_eq!(position_gain(gain_position(gain)), gain);
        }
    }
}
#[component]
pub fn AudioMixer(project: Signal<Workspace>, visible: bool) -> Element {
    let client = use_context::<LiveClient>();
    let mut selected = use_signal(|| None::<u32>);
    let workspace = project.read().clone();
    let current = workspace
        .scenes
        .iter()
        .find(|s| Some(s.id) == selected())
        .or(workspace.scenes.first())
        .cloned();
    let status = if client.owns() {
        client.status.read().clone()
    } else {
        None
    };
    rsx! {
        div {class:"page-heading",div {h1 {"Audio mixer"}p {"Each scene owns its mix. Destinations select a stereo bus."}}
            if (client.scoped_pending)() {span {class:"form-help",role:"status","Saving mix…"}}
        }
        p {class:"form-help","Mixer edits are saved as you work. Edits to the server’s scenes update the mix immediately. Local scenes keep their mix with the draft."}
        div {class:"scene-list",for scene in &workspace.scenes {
            button {class:if current.as_ref().is_some_and(|s|s.id==scene.id){"selected"}else{""},onclick:{let id=scene.id;move |_|selected.set(Some(id))},"{scene.name} / {scene.orientation.label()}"}
        }}
        if let Some(scene)=current {
            section {class:"panel",div {class:"panel-heading",h2 {"{scene.name} · buses"}
                button {disabled:scene.audio.buses.len()>=8,onclick:move |_|{
                    let id=project.peek().scenes.iter().find(|s|s.id==scene.id)
                        .filter(|s|s.audio.buses.len()<8)
                        .map(|s|(1..=9).find(|id|!s.audio.buses.iter().any(|b|b.id==*id)).unwrap());
                    if let Some(id)=id {edit_bus(client,scene.id,id,BusChange::Add);}
                },"+ Bus"}
            }
            div {class:"audio-bus-grid",for bus in scene.audio.buses.clone() {
                div {key:"{scene.id}-{bus.id}",class:"audio-bus",
                    crate::live::CommittedName {label:"Bus name",max_bytes:Some(128),value:bus.name.clone(),change:move |v|{edit_bus(client,scene.id,bus.id,BusChange::Name(v))}}
                    GainFader {label:"Master gain",gain:bus.gain,change:move |v:u32|{edit_bus(client,scene.id,bus.id,BusChange::Gain(v))}}
                    div {class:"row",button {class:if bus.muted{"selected"}else{""},"aria-pressed":bus.muted.to_string(),onclick:move |_|{edit_bus(client,scene.id,bus.id,BusChange::Mute(!bus.muted))},"Mute"}
                        if bus.id!=1 {button {onclick:move |_|edit_bus(client,scene.id,bus.id,BusChange::Remove),"Remove bus"}}
                    }
                    if visible {LiveMeters {target:MeterTarget::Bus {orientation:scene.orientation,scene_id:scene.id,bus_id:bus.id},fallback:status.as_ref().into_iter().flat_map(|s|&s.formats).flat_map(|f|&f.audio_buses).find(|b|b.scene_id==scene.id && b.bus_id==bus.id).map(|b|b.meter.clone()).unwrap_or_default()}}
                }
            }}
            div {class:"panel-note","Program is the default bus. Removing another bus resets its destination routes to Program. Bus meters show the applied active scene."}
            }
            section {class:"panel",div {class:"panel-heading",h2 {"Source tracks"}span {class:"badge","Stereo · 48 kHz"}}
                div {class:"panel-note","Track 0 of sources in this scene starts on Program. Additional tracks start unrouted. Sends are independent; gain does not change when other sources connect. Solo applies across this scene."}
                for source in workspace.sources.iter().filter(|s|s.kind.has_audio_input()) {
                    {
                        let input=status.as_ref().and_then(|s|s.sources.iter().find(|i|i.id==source.id));
                        let mut tracks=BTreeMap::from([(0,None)]);
                        for track in input.into_iter().flat_map(|i|&i.audio_tracks){tracks.insert(track.track_id,Some(track.clone()));}
                        for strip in scene.audio.strips.iter().filter(|s|s.source_id==source.id){tracks.entry(strip.track_id).or_insert(None);}
                        let effective=scene.audio.effective_strips(scene.layers.iter().map(|l|l.source_id));
                        rsx! {for (track_id,track) in tracks {
                            {
                                let key=TrackKey{source_id:source.id,track_id};
                                let strip=effective.get(&key).cloned().unwrap_or_else(||AudioStrip::new(key,false));
                                rsx! {div {key:"{scene.id}-{source.id}-{track_id}",class:"audio-strip",
                                    div {class:"audio-strip-heading",h3 {"{source.name} · track {track_id}"}
                                        span {class:"badge",if track.as_ref().is_some_and(|t|t.active){"Receiving"}else{"Waiting"}}
                                    }
                                    if let Some(track)=track.as_ref() {p {class:"form-help","{track.codec} → {track.sample_rate} Hz / {track.channels} channels"}
                                        if let Some(error)=track.error.as_ref() {p {class:"field-error",role:"alert","{error}"}}
                                    }
                                    if visible {LiveMeters {target:MeterTarget::Source {source_id:source.id,track_id},fallback:track.map(|track|track.meter).unwrap_or_default()}}
                                    StripControls {scene_id:scene.id,strip:strip.clone()}
                                    div {class:"audio-sends",for bus in &scene.audio.buses {
                                        div {class:"field",span {"Send → {bus.name}"}GainFader {label:"Send gain",gain:strip.sends.get(&bus.id).copied().unwrap_or(0),reset_unity:false,change:{let id=bus.id;move |v:u32|edit_strip(client,scene.id,key,StripChange::Send {bus:id,gain:v})}}}
                                    }}
                                }}
                            }
                        }}
                    }
                }
                if workspace.sources.is_empty(){div {class:"empty-state",p {"Register sources in Live / Sources to configure their tracks."}}}
            }
        }else{div {class:"panel empty-state",h3 {"Create a scene first"}p {"Each scene has an independent audio mixer."}}}
    }
}
#[component]
pub(crate) fn StripControls(
    scene_id: u32,
    strip: AudioStrip,
    #[props(default = true)] show_gain: bool,
) -> Element {
    let key = strip.key();
    let client = use_context::<LiveClient>();
    rsx! {div {class:"form-grid",
        if show_gain {GainFader {label:"Gain",gain:strip.gain,change:move |v:u32|edit_strip(client,scene_id,key,StripChange::Gain(v))}}
        Slider {label:"Balance L / R",value:strip.pan,min:-100,max:100,reset:Some(0),change:move |v|edit_strip(client,scene_id,key,StripChange::Pan(v))}
        div {class:"field",span {"Controls"}div {class:"row",
            button {class:if strip.muted{"selected"}else{""},"aria-pressed":strip.muted.to_string(),onclick:move |_|edit_strip(client,scene_id,key,StripChange::Mute(!strip.muted)),"Mute"}
            button {class:if strip.solo{"selected"}else{""},"aria-pressed":strip.solo.to_string(),onclick:move |_|edit_strip(client,scene_id,key,StripChange::Solo(!strip.solo)),"Solo"}
        }}
    }}
}
