use acheflow_media_api::live::{RenderLoad, ServerLoadSample};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct ServerMonitor {
    started: OnceLock<()>,
    history: Arc<Mutex<VecDeque<ServerLoadSample>>>,
}

impl ServerMonitor {
    pub fn samples(&self) -> Vec<ServerLoadSample> {
        self.started.get_or_init(|| {
            let history = Arc::downgrade(&self.history);
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(Duration::from_secs(1));
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let mut system = sysinfo::System::new().ok();
                let mut primed = false;
                while let Some(history) = history.upgrade() {
                    ticker.tick().await;
                    let Some(mut current) = system.take() else {
                        break;
                    };
                    // System calls stay off the renderer and the async workers.
                    let Ok((updated, sample)) = tokio::task::spawn_blocking(move || {
                        current.refresh_cpu_usage();
                        current.refresh_memory_specifics(
                            sysinfo::MemoryRefreshKind::nothing().with_ram(),
                        );
                        let sample = ServerLoadSample {
                            time_ms: crate::control::now_ms(),
                            cpu_percent: current.global_cpu_usage().clamp(0., 100.),
                            ram_used_bytes: current.used_memory(),
                            ram_total_bytes: current.total_memory(),
                            busiest_gpu_percent: busiest_gpu_percent(),
                        };
                        (current, sample)
                    })
                    .await
                    else {
                        break;
                    };
                    system = Some(updated);
                    if primed && sample.ram_total_bytes > 0 && sample.cpu_percent.is_finite() {
                        let mut history = history.lock().unwrap();
                        history.push_back(sample);
                        if history.len() > 60 {
                            history.pop_front();
                        }
                    }
                    primed = true;
                    drop(history);
                }
            });
        });
        self.history.lock().unwrap().iter().cloned().collect()
    }
}

#[cfg(target_os = "linux")]
fn busiest_gpu_percent() -> Option<f32> {
    std::fs::read_dir("/sys/class/drm")
        .ok()?
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_prefix("card"))
                .is_some_and(|index| {
                    !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
                })
        })
        .filter_map(|entry| {
            std::fs::read_to_string(entry.path().join("device/gpu_busy_percent")).ok()
        })
        .filter_map(|value| {
            value
                .trim()
                .parse::<u8>()
                .ok()
                .filter(|&value| value <= 100)
        })
        .max()
        .map(f32::from)
}

#[cfg(not(target_os = "linux"))]
fn busiest_gpu_percent() -> Option<f32> {
    None
}

#[derive(Default)]
pub struct RenderMonitor {
    frames: VecDeque<(Instant, Duration, bool)>,
}

impl RenderMonitor {
    pub fn record(&mut self, now: Instant, elapsed: Duration, presented: bool) {
        self.frames.push_back((now, elapsed, presented));
        self.frames
            .retain(|(at, _, _)| now.duration_since(*at) <= Duration::from_secs(2));
    }

    pub fn status(&self, now: Instant, fps: u32) -> RenderLoad {
        let period = Duration::from_secs_f64(1. / f64::from(fps));
        let mut durations = Vec::new();
        let mut completed = 0;
        let mut missed = 0;
        let mut oldest = now;
        for (at, elapsed, presented) in &self.frames {
            if now.duration_since(*at) > Duration::from_secs(2) {
                continue;
            }
            oldest = oldest.min(at.checked_sub(*elapsed).unwrap_or(*at));
            durations.push(*elapsed);
            completed += usize::from(*presented);
            missed += usize::from(!presented || *elapsed > period);
        }
        if durations.is_empty() {
            return RenderLoad {
                target_fps: fps,
                ..Default::default()
            };
        }
        durations.sort_unstable();
        let p95 = durations[(durations.len() * 95).div_ceil(100) - 1];
        RenderLoad {
            target_fps: fps,
            observed_fps: completed as f64 / now.duration_since(oldest).max(period).as_secs_f64(),
            p95_ms: p95.as_secs_f64() * 1000.,
            deadline_miss_percent: 100. * missed as f64 / durations.len() as f64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_deadline_misses_separately_from_render_p95() {
        let start = Instant::now();
        let mut monitor = RenderMonitor::default();
        monitor.record(
            start + Duration::from_millis(20),
            Duration::from_millis(20),
            true,
        );
        monitor.record(
            start + Duration::from_millis(90),
            Duration::from_millis(50),
            true,
        );
        let status = monitor.status(start + Duration::from_millis(100), 30);
        assert_eq!(status.target_fps, 30);
        assert_eq!(status.p95_ms, 50.);
        assert_eq!(status.deadline_miss_percent, 50.);
        assert_eq!(status.observed_fps, 20.);
    }
}
