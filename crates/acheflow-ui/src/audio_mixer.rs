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
    let audio_sources = workspace
        .sources
        .iter()
        .filter(|s| s.kind.has_audio_input());
    rsx! {
        div {class:"page-heading mixer-heading",
            h1 {"Audio mixer"}
            span {class:"form-help",role:"status",if (client.scoped_pending)() {"Saving mix…"}}
        }
        if let Some(scene)=current {
            div {class:"mixer-scenes",role:"group","aria-label":"Scene mix",
                for option in &workspace.scenes {
                    {
                        let id=option.id;
                        let active=id==scene.id;
                        rsx! {button {key:"{id}",class:if active {"selected"} else {""},"aria-pressed":active.to_string(),onclick:move |_|selected.set(Some(id)),
                            strong {"{option.name}"}
                            span {"{option.orientation.label()}"}
                        }}
                    }
                }
            }
            div {class:"mixer-layout",
            section {class:"panel mixer-buses",div {class:"panel-heading",h2 {"Output buses"}
                button {disabled:scene.audio.buses.len()>=8,onclick:move |_|{
                    let id=project.peek().scenes.iter().find(|s|s.id==scene.id)
                        .filter(|s|s.audio.buses.len()<8)
                        .map(|s|(1..=9).find(|id|!s.audio.buses.iter().any(|b|b.id==*id)).unwrap());
                    if let Some(id)=id {edit_bus(client,scene.id,id,BusChange::Add);}
                },"+ Add bus"}
            }
            div {class:"audio-bus-grid",for bus in scene.audio.buses.clone() {
                div {key:"{scene.id}-{bus.id}",class:"audio-bus",
                    if bus.id==1 {span {class:"mixer-default-bus","Default destination bus"}}
                    crate::live::CommittedName {label:"Bus name",max_bytes:Some(128),value:bus.name.clone(),change:move |v|{edit_bus(client,scene.id,bus.id,BusChange::Name(v))}}
                    if visible {LiveMeters {target:MeterTarget::Bus {orientation:scene.orientation,scene_id:scene.id,bus_id:bus.id},fallback:status.as_ref().into_iter().flat_map(|s|&s.formats).flat_map(|f|&f.audio_buses).find(|b|b.scene_id==scene.id && b.bus_id==bus.id).map(|b|b.meter.clone()).unwrap_or_default()}}
                    GainFader {label:"Master gain",gain:bus.gain,change:move |v:u32|{edit_bus(client,scene.id,bus.id,BusChange::Gain(v))}}
                    div {class:"row",button {class:if bus.muted{"selected"}else{""},"aria-pressed":bus.muted.to_string(),onclick:move |_|{edit_bus(client,scene.id,bus.id,BusChange::Mute(!bus.muted))},"Mute"}
                        if bus.id!=1 {button {onclick:move |_|edit_bus(client,scene.id,bus.id,BusChange::Remove),"Remove bus"}}
                    }
                }
            }}
            div {class:"panel-note","Removing a bus moves its destinations to Program. Meters reflect the active server mix."}
            }
            section {class:"panel mixer-tracks",div {class:"panel-heading",h2 {"Source tracks"}span {class:"form-help","Stereo · 48 kHz"}}
                for source in audio_sources.clone() {
                    {
                        let input=status.as_ref().and_then(|s|s.sources.iter().find(|i|i.id==source.id));
                        let mut tracks=BTreeMap::from([(0,None)]);
                        for track in input.into_iter().flat_map(|i|&i.audio_tracks){tracks.insert(track.track_id,Some(track));}
                        for strip in scene.audio.strips.iter().filter(|s|s.source_id==source.id){tracks.entry(strip.track_id).or_insert(None);}
                        let effective=scene.audio.effective_strips(scene.layers.iter().map(|l|l.source_id));
                        rsx! {for (track_id,track) in tracks {
                            {
                                let key=TrackKey{source_id:source.id,track_id};
                                let strip=effective.get(&key).cloned().unwrap_or_else(||AudioStrip::new(key,false));
                                let receiving=track.is_some_and(|track|track.active);
                                rsx! {div {key:"{scene.id}-{source.id}-{track_id}",class:"audio-strip",
                                    div {class:"audio-strip-heading",
                                        div {h3 {"{source.name}"}
                                            p {class:"form-help","Track {track_id}"
                                                if let Some(track)=track.as_ref() {" · {track.codec} · {track.sample_rate} Hz · {track.channels} ch"}
                                            }
                                        }
                                        span {class:if receiving {"mixer-track-status receiving"} else {"mixer-track-status"},
                                            if receiving {"Receiving"} else {"Waiting"}
                                        }
                                    }
                                    if let Some(error)=track.as_ref().and_then(|track|track.error.as_ref()) {
                                        p {class:"field-error",role:"alert","{error}"}
                                    }
                                    if visible {
                                        div {class:"mixer-track-monitor",
                                            LiveMeters {target:MeterTarget::Source {source_id:source.id,track_id},fallback:track.map(|track|track.meter.clone()).unwrap_or_default()}
                                            crate::audio_monitor::MonitorButton {
                                                target:crate::audio_monitor::Target::Source {source:source.id,track:track_id},
                                                disabled:!receiving,
                                            }
                                        }
                                    }
                                    StripControls {scene_id:scene.id,strip:strip.clone()}
                                    div {class:"mixer-routing",role:"group","aria-label":"Bus sends",
                                        h4 {"Bus sends"}
                                        div {class:"audio-sends",for bus in &scene.audio.buses {
                                            div {key:"{bus.id}",class:"mixer-send",
                                                GainFader {label:bus.name.clone(),gain:strip.sends.get(&bus.id).copied().unwrap_or(0),reset_unity:false,change:{let id=bus.id;move |v:u32|edit_strip(client,scene.id,key,StripChange::Send {bus:id,gain:v})}}
                                            }
                                        }}
                                    }
                                }}
                            }
                        }}
                    }
                }
                if audio_sources.count()==0 {
                    div {class:"empty-state",p {"Add a microphone or a stream with audio in Live / Sources."}}
                }
            }
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
