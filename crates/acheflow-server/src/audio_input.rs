//! Independent, timestamped stereo PCM decoders for incoming audio tracks.
use crate::{
    audio_packet::split_audio,
    ingest::{Input, flv_tag},
};
use acheflow_media_api::audio::{AUDIO_RATE, AUDIO_SAMPLES, AudioMeter, AudioTrackStatus};
use std::{
    collections::BTreeMap,
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    sync::{broadcast, mpsc},
};
use tokio_util::sync::CancellationToken;

pub struct AudioFrame {
    pub pts: i64,
    pub samples: Arc<Vec<i16>>,
}
#[derive(Default)]
struct Telemetry {
    received: Option<Instant>,
    meter: AudioMeter,
    error: Option<String>,
}
pub struct AudioTrack {
    pub id: u8,
    pub codec: String,
    pub frames: broadcast::Sender<Arc<AudioFrame>>,
    telemetry: Mutex<Telemetry>,
}
impl AudioTrack {
    pub fn status(&self) -> AudioTrackStatus {
        let state = self.telemetry.lock().unwrap();
        let active = state
            .received
            .is_some_and(|t| t.elapsed() < Duration::from_millis(500));
        AudioTrackStatus {
            track_id: self.id,
            codec: self.codec.clone(),
            sample_rate: AUDIO_RATE,
            channels: 2,
            active,
            meter: if active {
                state.meter.clone()
            } else {
                AudioMeter::default()
            },
            error: state.error.clone(),
        }
    }
    fn error(&self, input: &Input, message: String) {
        self.telemetry.lock().unwrap().error = Some(message.clone());
        *input.error.lock().unwrap() = Some(format!("Audio track {}: {message}", self.id));
    }
}
struct Decoder {
    codec: &'static str,
    sender: mpsc::Sender<Vec<u8>>,
    cancel: CancellationToken,
}
pub struct AudioInputs {
    input: Arc<Input>,
    cancel: CancellationToken,
    decoders: Mutex<BTreeMap<u8, Decoder>>,
}
impl AudioInputs {
    pub fn new(input: Arc<Input>, cancel: CancellationToken) -> Self {
        Self {
            input,
            cancel,
            decoders: Mutex::new(BTreeMap::new()),
        }
    }
    pub async fn packet(&self, timestamp: u32, data: &[u8]) -> Result<(), String> {
        let packets = split_audio(data)?;
        {
            let slots = self.decoders.lock().unwrap();
            let new = packets
                .iter()
                .filter(|p| !p.ended && !slots.contains_key(&p.track_id))
                .count();
            if slots.len() + new > 8 {
                return Err("At most 8 audio tracks per RTMP source are supported".into());
            }
        }
        for packet in packets {
            let sender = {
                let mut slots = self.decoders.lock().unwrap();
                if packet.ended
                    || slots
                        .get(&packet.track_id)
                        .is_some_and(|s| s.codec != packet.codec)
                {
                    if let Some(old) = slots.remove(&packet.track_id) {
                        old.cancel.cancel();
                    }
                    self.input
                        .audio_tracks
                        .lock()
                        .unwrap()
                        .remove(&packet.track_id);
                }
                if packet.ended {
                    continue;
                }
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    slots.entry(packet.track_id)
                {
                    let cancel = self.cancel.child_token();
                    let (frames, _) = broadcast::channel(32);
                    let track = Arc::new(AudioTrack {
                        id: packet.track_id,
                        codec: packet.codec.into(),
                        frames,
                        telemetry: Mutex::new(Telemetry::default()),
                    });
                    let sender = start_decoder(self.input.clone(), track.clone(), cancel.clone())?;
                    self.input
                        .audio_tracks
                        .lock()
                        .unwrap()
                        .insert(packet.track_id, track);
                    entry.insert(Decoder {
                        codec: packet.codec,
                        sender,
                        cancel,
                    });
                }
                slots[&packet.track_id].sender.clone()
            };
            tokio::time::timeout(
                Duration::from_secs(2),
                sender.send(flv_tag(8, timestamp, &packet.data)),
            )
            .await
            .map_err(|_| "Audio decoder is overloaded")?
            .map_err(|_| "Audio decoder stopped")?;
        }
        Ok(())
    }
}

fn start_decoder(
    input: Arc<Input>,
    track: Arc<AudioTrack>,
    cancel: CancellationToken,
) -> Result<mpsc::Sender<Vec<u8>>, String> {
    let mut command = tokio::process::Command::new(crate::ffmpeg::executable());
    command.env("AV_LOG_FORCE_NOCOLOR","1").args([
        "-hide_banner","-loglevel","level+info","-nostdin","-nostats","-threads","1","-max_alloc","67108864",
        "-probesize","32768","-analyzeduration","100000","-copyts","-f","flv","-i","pipe:0",
        "-map","0:a:0","-vn","-af","aresample=48000,aformat=sample_fmts=s16:channel_layouts=stereo,asetnsamples=n=960:p=1,ashowinfo@acheflow",
        "-ar","48000","-ac","2","-c:a","pcm_s16le","-f","s16le","pipe:1",
    ]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|e| format!("Cannot start audio decoder: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("Missing audio stdin")?;
    let mut stdout = child.stdout.take().ok_or("Missing audio stdout")?;
    let stderr = child.stderr.take().ok_or("Missing audio diagnostics")?;
    let (sender, mut receiver) = mpsc::channel::<Vec<u8>>(8);
    let (times_tx, mut times_rx) = mpsc::channel::<Result<i64, String>>(8);
    tokio::spawn(async move {
        let writer_cancel = cancel.clone();
        let writer = tokio::spawn(async move {
            let result = async {
                stdin
                    .write_all(&[b'F', b'L', b'V', 1, 4, 0, 0, 0, 9, 0, 0, 0, 0])
                    .await?;
                while let Some(tag) = receiver.recv().await {
                    stdin.write_all(&tag).await?;
                }
                Ok::<_, std::io::Error>(())
            }
            .await;
            if result.is_err() {
                writer_cancel.cancel();
            }
        });
        let read_input = input.clone();
        let read_track = track.clone();
        let read_cancel = cancel.clone();
        let reader = tokio::spawn(async move {
            let result = async {
                while let Some(pts) = times_rx.recv().await {
                    let pts = pts.map_err(std::io::Error::other)?;
                    let mut bytes = vec![0; AUDIO_SAMPLES * 2];
                    stdout.read_exact(&mut bytes).await?;
                    let samples = Arc::new(
                        bytes
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|p| i16::from_le_bytes([p[0], p[1]]))
                            .collect::<Vec<_>>(),
                    );
                    let meter = pcm_meter(&samples);
                    {
                        let mut state = read_track.telemetry.lock().unwrap();
                        state.received = Some(Instant::now());
                        state.meter = meter;
                    }
                    read_input
                        .has_audio
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    if read_track.id == 0 {
                        let _ = read_input.audio.send(samples.clone());
                    }
                    let _ = read_track
                        .frames
                        .send(Arc::new(AudioFrame { pts, samples }));
                }
                Ok::<_, std::io::Error>(())
            }
            .await;
            if let Err(error) = result
                && error.kind() != std::io::ErrorKind::UnexpectedEof
            {
                read_track.error(&read_input, error.to_string());
            }
            read_cancel.cancel();
        });
        let error_input = input.clone();
        let error_track = track.clone();
        let errors = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(pts) = audio_pts(&line) {
                    if times_tx.send(pts).await.is_err() {
                        break;
                    }
                } else if line.contains("[error]") || line.contains("[fatal]") {
                    error_track.error(&error_input, line.chars().take(320).collect());
                }
            }
        });
        let exited = tokio::select! {_=cancel.cancelled()=>{let _=child.kill().await;child.wait().await},status=child.wait()=>status};
        if let Ok(status) = exited
            && !status.success()
            && !cancel.is_cancelled()
        {
            track.error(&input, "FFmpeg audio decoder exited".into());
        }
        cancel.cancel();
        writer.abort();
        reader.abort();
        errors.abort();
        let _ = writer.await;
        let _ = reader.await;
        let _ = errors.await;
    });
    Ok(sender)
}

fn audio_pts(line: &str) -> Option<Result<i64, String>> {
    if !line.starts_with("[ashowinfo@acheflow @ ") || !line.contains(" n:") {
        return None;
    }
    Some(
        (|| {
            if !line.contains("nb_samples:960")
                || !line.contains("channels:2")
                || !line.contains("rate:48000")
            {
                return None;
            }
            line.split_once(" pts:")?
                .1
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        })()
        .ok_or_else(|| "Invalid FFmpeg PCM timestamp/layout".into()),
    )
}

pub fn pcm_meter(samples: &[i16]) -> AudioMeter {
    let mut meter = AudioMeter::default();
    let mut power = [0f64; 2];
    for frame in samples.as_chunks::<2>().0.iter() {
        for channel in 0..2 {
            let value = f32::from(frame[channel]) / 32768.;
            meter.peak[channel] = meter.peak[channel].max(value.abs());
            power[channel] += f64::from(value).powi(2);
        }
    }
    for (channel, power) in power.into_iter().enumerate() {
        meter.rms[channel] = (power / (samples.len() / 2).max(1) as f64).sqrt() as f32;
    }
    meter
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use acheflow_media_api::audio::AUDIO_FRAMES;
    #[test]
    fn named_audio_descriptors_are_checked_before_pairing_pcm() {
        assert_eq!(audio_pts("[ashowinfo@acheflow @ 0x1] [info] n:0 pts:48000 pts_time:1 channels:2 rate:48000 nb_samples:960").unwrap().unwrap(),48000);
        assert!(
            audio_pts("[ashowinfo@acheflow @ 0x1] n:0 pts:0 channels:1 rate:48000 nb_samples:960")
                .unwrap()
                .is_err()
        );
        assert!(audio_pts("unrelated pts:0 nb_samples:960").is_none());
    }

    #[tokio::test]
    #[ignore = "requires installed FFmpeg with Enhanced RTMP audio; run explicitly"]
    async fn real_multitrack_aac_preserves_each_track_and_stereo_channels() {
        let result = tokio::process::Command::new(crate::ffmpeg::executable())
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=0.15*sin(2*PI*440*t)|0.15*sin(2*PI*660*t):s=48000",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=0.15*sin(2*PI*880*t)|0.15*sin(2*PI*1100*t):s=48000",
                "-map",
                "0:a",
                "-map",
                "1:a",
                "-c:a",
                "aac",
                "-t",
                "1",
                "-f",
                "flv",
                "pipe:1",
            ])
            .output()
            .await
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let mut workspace = acheflow_media_api::workspace::Workspace::default();
        workspace.add_source();
        let input = Arc::new(Input::new(workspace.sources[0].clone()));
        let cancel = CancellationToken::new();
        let decoders = AudioInputs::new(input.clone(), cancel.clone());
        let mut receivers = BTreeMap::new();
        let bytes = &result.stdout;
        let mut cursor = 13;
        while cursor + 11 <= bytes.len() {
            let size = (usize::from(bytes[cursor + 1]) << 16)
                | (usize::from(bytes[cursor + 2]) << 8)
                | usize::from(bytes[cursor + 3]);
            let timestamp = (u32::from(bytes[cursor + 7]) << 24)
                | (u32::from(bytes[cursor + 4]) << 16)
                | (u32::from(bytes[cursor + 5]) << 8)
                | u32::from(bytes[cursor + 6]);
            if bytes[cursor] == 8 {
                decoders
                    .packet(timestamp, &bytes[cursor + 11..cursor + 11 + size])
                    .await
                    .unwrap();
                for (id, track) in input.audio_tracks.lock().unwrap().iter() {
                    receivers
                        .entry(*id)
                        .or_insert_with(|| track.frames.subscribe());
                }
            }
            cursor += 15 + size;
        }
        drop(decoders);
        assert_eq!(receivers.len(), 2);
        for (id, mut receiver) in receivers {
            let samples = tokio::time::timeout(Duration::from_secs(10), async {
                let mut samples = vec![];
                let mut previous = None;
                while samples.len() < AUDIO_SAMPLES * 10 {
                    match receiver.recv().await {
                        Ok(frame) => {
                            if previous.is_some_and(|pts| frame.pts != pts + AUDIO_FRAMES as i64) {
                                samples.clear();
                            }
                            previous = Some(frame.pts);
                            samples.extend_from_slice(&frame.samples);
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            samples.clear();
                            previous = None;
                        }
                        Err(error) => panic!("track {id}: {error}"),
                    }
                }
                samples
            })
            .await
            .unwrap();
            let frequencies = if id == 0 { [440., 660.] } else { [880., 1100.] };
            for (channel, frequency) in frequencies.into_iter().enumerate() {
                let wanted = tone_amplitude(&samples, channel, frequency);
                let crossed = tone_amplitude(&samples, channel, frequencies[1 - channel]);
                assert!(
                    wanted > 0.08 && crossed < 0.02,
                    "track {id}, channel {channel}: wanted {wanted}, cross {crossed}"
                );
            }
            assert!(
                input.audio_tracks.lock().unwrap()[&id]
                    .status()
                    .error
                    .is_none()
            );
        }
        cancel.cancel();
    }

    pub fn tone_amplitude(samples: &[i16], channel: usize, hz: f64) -> f64 {
        let mut real = 0.;
        let mut imag = 0.;
        let frames = samples.len() / 2;
        for (i, frame) in samples.as_chunks::<2>().0.iter().enumerate() {
            let angle = 2. * std::f64::consts::PI * hz * i as f64 / 48000.;
            let value = f64::from(frame[channel]) / 32768.;
            real += value * angle.cos();
            imag += value * angle.sin();
        }
        2. * real.hypot(imag) / frames.max(1) as f64
    }
}
