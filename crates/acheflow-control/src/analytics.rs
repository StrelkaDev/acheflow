use acheflow_extension_api::audience::*;
use std::collections::{BTreeMap, VecDeque};

pub const FRESHNESS_MS: u64 = 90_000;
const HISTORY_LIMIT: usize = 1000;

#[derive(Default)]
pub struct Analytics {
    samples: BTreeMap<(String, String), AudienceSample>,
    points: VecDeque<AudiencePoint>,
    peak: Option<u64>,
}

impl Analytics {
    pub fn record(&mut self, mut sample: AudienceSample) -> Result<(), String> {
        if sample.platform.is_empty()
            || sample.platform.len() > 64
            || sample.channel_id.is_empty()
            || sample.channel_id.len() > 128
            || sample.stream_id.as_ref().is_some_and(|s| s.len() > 128)
            || sample.unique.as_ref().is_some_and(|u| {
                u.period_start_ms > u.period_end_ms || u.scope.is_empty() || u.scope.len() > 128
            })
        {
            return Err("invalid audience sample".into());
        }
        let key = (sample.platform.clone(), sample.channel_id.clone());
        if !self.samples.contains_key(&key) && self.samples.len() >= 64 {
            return Err("audience channel limit reached".into());
        }
        if let Some(old) = self.samples.get(&key) {
            if old.observed_at_ms > sample.observed_at_ms {
                return Ok(());
            }
            if old.stream_id == sample.stream_id && sample.unique.is_none() {
                sample.unique = old.unique.clone();
            }
        }
        self.samples.insert(key, sample);
        Ok(())
    }

    pub fn disconnect(&mut self, origin: &str) {
        for ((platform, _), sample) in &mut self.samples {
            if platform == origin {
                sample.concurrent = None;
                sample.observed_at_ms = 0;
                sample.offline = false;
            }
        }
    }
    pub fn disconnect_channel(&mut self, origin: &str, channel: &str) {
        if let Some(sample) = self.samples.get_mut(&(origin.into(), channel.into())) {
            sample.concurrent = None;
            sample.observed_at_ms = 0;
            sample.offline = false;
        }
    }
    pub fn snapshot(&self, now: u64) -> AudienceSnapshot {
        let mut snapshot = AudienceSnapshot {
            peak: self.peak,
            ..Default::default()
        };
        let mut total = Some(0u64);
        let mut known = false;
        for sample in self.samples.values() {
            let stale = sample.observed_at_ms > now || now - sample.observed_at_ms > FRESHNESS_MS;
            let current = if stale {
                None
            } else if sample.offline {
                Some(0)
            } else {
                sample.concurrent
            };
            if let Some(count) = current {
                known = true;
                total = total.and_then(|n| n.checked_add(count));
            } else {
                snapshot.partial = true;
            }
            snapshot.channels.push(ChannelAudience {
                sample: sample.clone(),
                current,
                stale,
                unique: sample.unique.clone(),
            });
        }
        snapshot.overflow = known && total.is_none();
        snapshot.total = known.then_some(total).flatten();
        snapshot
    }

    pub fn capture(&mut self, now: u64) {
        let snapshot = self.snapshot(now);
        if !snapshot.partial
            && let Some(total) = snapshot.total
        {
            self.peak = Some(self.peak.unwrap_or(0).max(total));
        }
        self.points.push_back(AudiencePoint {
            at_ms: now,
            total: snapshot.total,
            partial: snapshot.partial,
        });
        if self.points.len() > HISTORY_LIMIT {
            self.points.pop_front();
        }
    }

    pub fn history(&self) -> &VecDeque<AudiencePoint> {
        &self.points
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(platform: &str, channel: &str, viewers: Option<u64>, at: u64) -> AudienceSample {
        AudienceSample {
            platform: platform.into(),
            channel_id: channel.into(),
            stream_id: Some("live".into()),
            observed_at_ms: at,
            concurrent: viewers,
            offline: false,
            unique: None,
        }
    }

    #[test]
    fn sums_current_counts_but_keeps_platform_uniques_separate() {
        let mut metrics = Analytics::default();
        let mut twitch = sample("twitch", "a", Some(100), 1000);
        twitch.unique = Some(UniqueViewers {
            value: 120,
            period_start_ms: 0,
            period_end_ms: 1000,
            scope: "stream".into(),
            provisional: true,
        });
        metrics.record(twitch).unwrap();
        metrics
            .record(sample("youtube", "b", Some(50), 1000))
            .unwrap();
        let snapshot = metrics.snapshot(1000);
        assert_eq!(snapshot.total, Some(150));
        assert!(!snapshot.partial);
        assert_eq!(snapshot.channels[0].unique.as_ref().unwrap().value, 120);
        assert!(snapshot.channels[1].unique.is_none());
    }

    #[test]
    fn stale_failed_offline_and_duplicate_connections_have_distinct_meanings() {
        let mut metrics = Analytics::default();
        metrics
            .record(sample("twitch", "a", Some(100), 1000))
            .unwrap();
        metrics
            .record(sample("twitch", "a", Some(100), 1000))
            .unwrap();
        metrics.record(sample("youtube", "b", None, 1000)).unwrap();
        assert_eq!(metrics.snapshot(1000).total, Some(100));
        assert!(metrics.snapshot(1000).partial);
        assert_eq!(metrics.snapshot(92000).total, None);
        let mut offline = sample("twitch", "a", None, 93000);
        offline.offline = true;
        metrics.record(offline).unwrap();
        assert_eq!(metrics.snapshot(93000).total, Some(0));
        metrics
            .record(sample("twitch", "a", Some(300), 900))
            .unwrap();
        assert_eq!(metrics.snapshot(93000).total, Some(0));
    }

    #[test]
    fn peak_is_simultaneous_and_history_is_bounded() {
        let mut metrics = Analytics::default();
        metrics.record(sample("twitch", "a", Some(100), 0)).unwrap();
        metrics.record(sample("youtube", "b", Some(10), 0)).unwrap();
        metrics.capture(0);
        metrics.record(sample("twitch", "a", Some(10), 1)).unwrap();
        metrics
            .record(sample("youtube", "b", Some(100), 1))
            .unwrap();
        for now in 1..1100 {
            metrics.capture(now);
        }
        assert_eq!(metrics.snapshot(1099).peak, Some(110));
        assert_eq!(metrics.history().len(), 1000);
    }

    #[test]
    fn invalid_unique_windows_and_overflow_are_visible() {
        let mut metrics = Analytics::default();
        let mut bad = sample("twitch", "a", Some(1), 0);
        bad.unique = Some(UniqueViewers {
            value: 0,
            period_start_ms: 10,
            period_end_ms: 0,
            scope: "stream".into(),
            provisional: false,
        });
        assert!(metrics.record(bad).is_err());
        metrics
            .record(sample("twitch", "a", Some(u64::MAX), 0))
            .unwrap();
        metrics.record(sample("youtube", "b", Some(1), 0)).unwrap();
        assert!(metrics.snapshot(0).overflow);
        assert_eq!(metrics.snapshot(0).total, None);
    }
}
