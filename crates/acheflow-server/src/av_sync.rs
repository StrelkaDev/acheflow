use crate::ingest::VideoFrame;
use acheflow_media_api::audio::{AUDIO_RATE, SyncMode};
use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::{Duration, Instant},
};

const VIDEO_HISTORY_BYTES: usize = 128 * 1024 * 1024;
const MAX_VIDEO_FRAMES_PER_SOURCE: usize = 32;
const DISCONTINUITY: i64 = AUDIO_RATE as i64 / 2;
const SAMPLES_PER_MS: i64 = AUDIO_RATE as i64 / 1000;
const CLOCK_WINDOW: usize = 7;
const CLOCK_WARMUP: usize = 12;
const MAX_NORMAL_SLEW: i64 = SAMPLES_PER_MS;

pub fn audio_target(
    mode: SyncMode,
    offset_ms: i32,
    video_delay_ms: u32,
    legacy_pts: i64,
    video_pts: Option<i64>,
) -> (i64, u32, bool) {
    let audio_delay = (video_delay_ms as i32 + offset_ms).max(0);
    if let Some(pts) = video_pts.filter(|_| mode != SyncMode::Manual) {
        // The presented video PTS already includes its own delay.
        (
            pts - i64::from(audio_delay - video_delay_ms as i32) * SAMPLES_PER_MS,
            audio_delay as u32,
            true,
        )
    } else {
        (
            legacy_pts - i64::from(audio_delay) * SAMPLES_PER_MS,
            audio_delay as u32,
            false,
        )
    }
}

#[derive(Clone, Copy)]
pub struct Presentation {
    pub pts: Option<i64>,
    pub generation: u64,
    pub received: Instant,
    pub shown: Instant,
    pub video_delay_ms: u32,
    requested_delay_ms: u32,
    pub limited: bool,
}

#[derive(Default)]
pub struct FormatSync {
    pub presented: Mutex<HashMap<u32, Presentation>>,
    pub audio: Mutex<Vec<acheflow_media_api::audio::TrackSyncStatus>>,
}

#[derive(Default)]
pub struct VideoHistory {
    frames: HashMap<u32, VecDeque<VideoFrame>>,
}

impl VideoHistory {
    pub fn retain(&mut self, keep: impl Fn(u32) -> bool) {
        self.frames.retain(|id, _| keep(*id));
    }

    pub fn select(
        &mut self,
        source: u32,
        latest: VideoFrame,
        delay_ms: u32,
        now: Instant,
    ) -> (VideoFrame, Presentation) {
        let wanted = Duration::from_millis(delay_ms.into());
        let mut selected = latest.clone();
        if delay_ms > 0 {
            let frames = self.frames.entry(source).or_default();
            if frames
                .back()
                .is_some_and(|frame| frame.generation != latest.generation)
            {
                frames.clear();
            }
            // Only past images consume the history budget. The live input
            // already owns the current frame.
            if frames
                .back()
                .is_none_or(|frame| frame.received != latest.received)
            {
                frames.push_back(latest.clone());
            }
            while frames.len() > MAX_VIDEO_FRAMES_PER_SOURCE
                || (frames.len() > 1 && now.duration_since(frames[1].received) >= wanted)
            {
                frames.pop_front();
            }
            self.trim_memory();
            if let Some(frames) = self.frames.get(&source) {
                selected = frames
                    .iter()
                    .rev()
                    .find(|frame| now.duration_since(frame.received) >= wanted)
                    .or_else(|| frames.front())
                    .cloned()
                    .unwrap_or(latest);
            }
        }
        let age = now.duration_since(selected.received);
        let limited = delay_ms > 0 && age < wanted;
        let video_delay_ms = if limited {
            age.as_millis() as u32
        } else {
            delay_ms
        };
        let presentation = Presentation {
            pts: selected.pts,
            generation: selected.generation,
            received: selected.received,
            shown: now,
            video_delay_ms,
            requested_delay_ms: delay_ms,
            limited,
        };
        (selected, presentation)
    }

    fn trim_memory(&mut self) {
        let mut bytes: usize = self
            .frames
            .values()
            .flatten()
            .map(|frame| frame.image.len())
            .sum();
        while bytes > VIDEO_HISTORY_BYTES {
            let oldest = self
                .frames
                .values_mut()
                .filter_map(|frames| frames.front().map(|frame| frame.received).zip(Some(frames)))
                .min_by_key(|(received, _)| *received);
            let Some((_, frames)) = oldest else {
                break;
            };
            bytes -= frames.pop_front().unwrap().image.len();
        }
    }
}

#[derive(Default)]
pub struct PresentationClock {
    offset: Option<i64>,
    last: Option<Presentation>,
    observations: VecDeque<i64>,
    acquired: usize,
}

impl PresentationClock {
    /// Fit a source PTS clock to the actual presentation time. Small
    /// scheduling variations slew the clock instead of seeking the audio.
    pub fn source_pts(&mut self, video: Presentation, started: Instant, now: i64) -> Option<i64> {
        let pts = video.pts?;
        let same_policy = self.last.is_some_and(|last| {
            last.generation == video.generation
                && last.requested_delay_ms == video.requested_delay_ms
                && last.limited == video.limited
        });
        if !same_policy || self.last.is_none_or(|last| last.received != video.received) {
            let elapsed = video.shown.saturating_duration_since(started);
            let observed =
                pts - (elapsed.as_nanos() * u128::from(AUDIO_RATE) / 1_000_000_000) as i64;
            let discontinuity = self
                .observations
                .back()
                .is_some_and(|previous| (observed - previous).abs() > DISCONTINUITY);
            if !same_policy || discontinuity {
                self.observations.clear();
                self.offset = None;
                self.acquired = 0;
            }
            if self.observations.len() == CLOCK_WINDOW {
                self.observations.pop_front();
            }
            self.observations.push_back(observed);
            self.acquired = (self.acquired + 1).min(CLOCK_WARMUP);
            if self.observations.len() >= 3 {
                let mut offsets = [0; CLOCK_WINDOW];
                for (to, from) in offsets.iter_mut().zip(&self.observations) {
                    *to = *from;
                }
                let recent = &mut offsets[..self.observations.len()];
                recent.sort_unstable();
                let target = recent[recent.len() / 2];
                self.offset = Some(match self.offset {
                    Some(old) if self.acquired == CLOCK_WARMUP => {
                        old + (target - old).clamp(-MAX_NORMAL_SLEW, MAX_NORMAL_SLEW)
                    }
                    _ => target,
                });
            }
            self.last = Some(video);
        }
        self.offset.map(|offset| now + offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn signed_trim_preserves_a_common_video_delay_across_tracks() {
        let legacy = 48_000;
        let video = Some(43_200);
        assert_eq!(
            audio_target(SyncMode::AutoTrim, -100, 100, legacy, video),
            (48_000, 0, true)
        );
        assert_eq!(
            audio_target(SyncMode::AutoTrim, 50, 100, legacy, video),
            (40_800, 150, true)
        );
        assert_eq!(
            audio_target(SyncMode::Manual, 50, 100, legacy, video),
            (40_800, 150, false)
        );
        assert_eq!(
            audio_target(SyncMode::AutoTrim, 50, 0, legacy, None),
            (45_600, 50, false)
        );
    }

    #[test]
    fn negative_trim_selects_a_past_frame_and_clears_history_on_reconnect() {
        let mut history = VideoHistory::default();
        let start = Instant::now();
        let frame = |ms, generation| VideoFrame {
            pts: Some(ms as i64 * 48),
            generation,
            received: start + Duration::from_millis(ms),
            image: Arc::new(image::RgbImage::new(2, 2)),
        };
        let (_, warmup) = history.select(1, frame(0, 1), 100, start);
        assert!(warmup.limited);
        let (chosen, applied) =
            history.select(1, frame(120, 1), 100, start + Duration::from_millis(120));
        assert_eq!(chosen.pts, Some(0));
        assert!(!applied.limited);
        let (fresh, applied) =
            history.select(1, frame(140, 2), 100, start + Duration::from_millis(140));
        assert_eq!(fresh.generation, 2);
        assert!(applied.limited);
    }

    #[test]
    fn presentation_clock_waits_for_fresh_frames_and_discards_startup_outliers() {
        let mut clock = PresentationClock::default();
        let start = Instant::now();
        let presentation = |pts, ms| Presentation {
            pts: Some(pts),
            generation: 1,
            received: start + Duration::from_millis(ms),
            shown: start + Duration::from_millis(ms),
            video_delay_ms: 0,
            requested_delay_ms: 0,
            limited: false,
        };
        assert_eq!(clock.source_pts(presentation(0, 250), start, 12_000), None);
        assert_eq!(
            clock.source_pts(presentation(12_960, 270), start, 12_960),
            None
        );
        // The first frame was 250 ms old. Two newer frames establish the
        // correct session clock without an eight-second slew.
        assert_eq!(
            clock.source_pts(presentation(13_920, 290), start, 13_920),
            Some(13_920)
        );
        assert_eq!(
            clock.source_pts(presentation(50_000, 310), start, 14_880),
            None
        );
        assert_eq!(
            clock.source_pts(presentation(50_960, 330), start, 15_840),
            None
        );
        assert_eq!(
            clock.source_pts(presentation(51_920, 350), start, 16_800),
            Some(51_920)
        );
    }
}
