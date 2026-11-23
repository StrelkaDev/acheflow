use acheflow_media_api::audio::AudioMeter;
use std::time::{Duration, Instant};

const HOLD: Duration = Duration::from_secs(1);

#[derive(Default)]
pub(crate) struct PeakHold {
    peaks: [Option<(f32, Instant)>; 2],
    clipped_until: Option<Instant>,
}

impl PeakHold {
    pub(crate) fn record(&mut self, now: Instant, meter: &mut AudioMeter) {
        for (peak, &value) in self.peaks.iter_mut().zip(&meter.peak) {
            if peak.is_none_or(|(held, until)| now >= until || value >= held) {
                *peak = Some((value, now + HOLD));
            }
        }
        if meter.clipped_samples > 0 {
            self.clipped_until = Some(now + HOLD);
        }
        self.read(now, meter);
    }

    pub(crate) fn read(&self, now: Instant, meter: &mut AudioMeter) {
        for (held, peak) in meter.held_peak.iter_mut().zip(self.peaks) {
            *held = peak
                .filter(|&(_, until)| now < until)
                .map_or(0., |(value, _)| value);
        }
        meter.recently_clipped = self.clipped_until.is_some_and(|until| now < until);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_clip_and_independent_stereo_peaks_survive_quieter_blocks() {
        let start = Instant::now();
        let mut hold = PeakHold::default();
        let mut loud = AudioMeter {
            peak: [1.1, 0.2],
            clipped_samples: 1,
            ..Default::default()
        };
        hold.record(start, &mut loud);

        let mut quiet = AudioMeter {
            peak: [0.1, 0.4],
            ..Default::default()
        };
        hold.record(start + Duration::from_millis(400), &mut quiet);
        assert_eq!(quiet.peak, [0.1, 0.4]);
        assert_eq!(quiet.held_peak, [1.1, 0.4]);
        assert!(quiet.recently_clipped);

        let mut silence = AudioMeter::default();
        hold.record(start + HOLD, &mut silence);
        assert_eq!(silence.held_peak, [0., 0.4]);
        assert!(!silence.recently_clipped);
    }
}
