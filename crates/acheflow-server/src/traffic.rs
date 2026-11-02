//! Rolling media byte rate; excludes TCP/IP and RTMP protocol overhead.
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub struct Traffic {
    total: u64,
    samples: VecDeque<(Instant, u64)>,
}
impl Default for Traffic {
    fn default() -> Self {
        Self {
            total: 0,
            samples: VecDeque::from([(Instant::now(), 0)]),
        }
    }
}
impl Traffic {
    pub fn record(&mut self, bytes: u64) {
        self.record_at(bytes, Instant::now());
    }
    fn record_at(&mut self, bytes: u64, now: Instant) {
        self.total = self.total.saturating_add(bytes);
        self.samples.push_back((now, self.total));
        self.trim(now);
    }
    fn trim(&mut self, now: Instant) {
        let cutoff = now - Duration::from_secs(2);
        while self.samples.len() > 1 && self.samples[1].0 <= cutoff {
            self.samples.pop_front();
        }
    }
    pub fn snapshot(&mut self) -> (u64, f64) {
        self.snapshot_at(Instant::now())
    }
    fn snapshot_at(&mut self, now: Instant) -> (u64, f64) {
        self.trim(now);
        let cutoff = now - Duration::from_secs(2);
        let &(at, bytes) = self.samples.front().unwrap();
        let elapsed = now.duration_since(at.max(cutoff)).as_secs_f64().max(0.1);
        (
            self.total,
            (self.total - bytes) as f64 * 8. / elapsed / 1000.,
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn traffic_tracks_bytes_and_decays_to_zero_when_idle() {
        let start = Instant::now();
        let mut traffic = Traffic {
            total: 0,
            samples: VecDeque::from([(start, 0)]),
        };
        traffic.record_at(125_000, start + Duration::from_secs(1));
        let (bytes, rate) = traffic.snapshot_at(start + Duration::from_secs(1));
        assert_eq!(bytes, 125_000);
        assert_eq!(rate, 1000.);
        assert_eq!(
            traffic.snapshot_at(start + Duration::from_secs(4)),
            (125_000, 0.)
        );
        traffic.record_at(125_000, start + Duration::from_secs(5));
        assert_eq!(
            traffic.snapshot_at(start + Duration::from_secs(5)),
            (250_000, 500.)
        );
    }
}
