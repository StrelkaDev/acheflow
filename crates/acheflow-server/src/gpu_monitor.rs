use acheflow_media_api::live::{GpuLoadSample, ServerLoadSample};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

type Pending = Arc<Mutex<Vec<GpuLoadSample>>>;

pub(super) fn start(history: Weak<Mutex<VecDeque<ServerLoadSample>>>) -> Pending {
    let pending = Pending::default();
    let samples = pending.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if history.strong_count() == 0 {
                break;
            }
            let result = probe().await;
            *samples.lock().unwrap() = result;
        }
    });
    pending
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
async fn command(executable: &str, args: &[&str]) -> Option<String> {
    use std::process::Stdio;
    let mut command = tokio::process::Command::new(executable);
    command
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(target_os = "windows")]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let output = tokio::time::timeout(Duration::from_millis(900), command.output())
        .await
        .ok()?
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "macos")]
async fn probe() -> Vec<GpuLoadSample> {
    command(
        "/usr/sbin/ioreg",
        &["-r", "-l", "-w0", "-c", "IOAccelerator"],
    )
    .await
    .map(|output| apple_gpus(&output))
    .unwrap_or_default()
}

#[cfg(target_os = "macos")]
fn apple_gpus(output: &str) -> Vec<GpuLoadSample> {
    output
        .lines()
        .filter_map(|line| {
            line.split_once("\"PerformanceStatistics\"")
                .map(|(_, fields)| fields)
        })
        .enumerate()
        .filter_map(|(index, fields)| {
            let read = |key: &str| -> Option<u64> {
                let (_, value) = fields.split_once(&format!("\"{key}\""))?;
                value
                    .trim_start()
                    .strip_prefix('=')?
                    .trim_start()
                    .split(|c: char| !c.is_ascii_digit())
                    .next()?
                    .parse()
                    .ok()
            };
            let utilization_percent = read("Device Utilization %").and_then(percent);
            let memory_used_bytes = read("In use system memory");
            (utilization_percent.is_some() || memory_used_bytes.is_some()).then(|| GpuLoadSample {
                id: format!("apple-{index}"),
                name: if index == 0 {
                    "GPU".into()
                } else {
                    format!("GPU {}", index + 1)
                },
                utilization_percent,
                memory_used_bytes,
                unified_memory: true,
                ..Default::default()
            })
        })
        .collect()
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
async fn probe() -> Vec<GpuLoadSample> {
    let nvidia = command("nvidia-smi", &["--query", "--display=UTILIZATION,MEMORY"])
        .await
        .map(|output| nvidia_gpus(&output))
        .unwrap_or_default();
    #[cfg(target_os = "linux")]
    {
        let include_nvidia_drm = nvidia.is_empty();
        let drm = tokio::task::spawn_blocking(move || drm_gpus(include_nvidia_drm))
            .await
            .unwrap_or_default();
        return nvidia.into_iter().chain(drm).collect();
    }
    #[cfg(target_os = "windows")]
    nvidia
}

#[cfg(any(test, target_os = "windows", target_os = "linux"))]
fn nvidia_gpus(output: &str) -> Vec<GpuLoadSample> {
    output
        .split("\nGPU ")
        .skip(1)
        .filter_map(|device| {
            let (address, properties) = device.split_once('\n')?;
            // PCI address is used only to link this device's samples during the process.
            let address = address.trim();
            if address.is_empty()
                || !address
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() || matches!(byte, b':' | b'.'))
            {
                return None;
            }
            let mut gpu = GpuLoadSample {
                id: format!("nvidia-{address}"),
                ..Default::default()
            };
            enum Section {
                Utilization,
                FrameBuffer,
                Other,
            }
            let mut section = Section::Other;
            for line in properties
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
            {
                let Some((key, value)) = line.split_once(':') else {
                    section = match line {
                        "Utilization" => Section::Utilization,
                        "FB Memory Usage" => Section::FrameBuffer,
                        _ => Section::Other,
                    };
                    continue;
                };
                let key = key.trim_end();
                let value = value.trim();
                match (&section, key) {
                    (Section::Utilization, "Gpu") => {
                        gpu.utilization_percent = units(value, "%").and_then(percent)
                    }
                    (Section::Utilization, "Encoder") => {
                        gpu.encoder_percent = units(value, "%").and_then(percent)
                    }
                    (Section::Utilization, "Decoder") => {
                        gpu.decoder_percent = units(value, "%").and_then(percent)
                    }
                    (Section::FrameBuffer, "Used") => {
                        gpu.memory_used_bytes = units(value, "MiB").and_then(mib)
                    }
                    (Section::FrameBuffer, "Total") => {
                        gpu.memory_total_bytes =
                            units(value, "MiB").filter(|total| *total > 0).and_then(mib)
                    }
                    _ => {}
                }
            }
            let has_metrics = gpu.utilization_percent.is_some()
                || gpu.encoder_percent.is_some()
                || gpu.decoder_percent.is_some()
                || gpu.memory_used_bytes.is_some();
            has_metrics.then_some(gpu)
        })
        .enumerate()
        .map(|(index, mut gpu)| {
            gpu.name = format!("NVIDIA GPU {}", index + 1);
            gpu
        })
        .collect()
}

#[cfg(any(test, target_os = "linux", target_os = "windows"))]
fn units(value: &str, unit: &str) -> Option<u64> {
    value.strip_suffix(unit)?.trim().parse().ok()
}

#[cfg(any(test, target_os = "linux", target_os = "windows"))]
fn mib(value: u64) -> Option<u64> {
    value.checked_mul(1 << 20)
}

#[cfg(target_os = "linux")]
fn drm_gpus(include_nvidia: bool) -> Vec<GpuLoadSample> {
    let Ok(cards) = std::fs::read_dir("/sys/class/drm") else {
        return Vec::new();
    };
    let mut gpus = cards
        .flatten()
        .filter_map(|card| {
            let filename = card.file_name();
            let index = filename
                .to_str()?
                .strip_prefix("card")?
                .parse::<u32>()
                .ok()?;
            let device = card.path().join("device");
            let read = |key: &str| std::fs::read_to_string(device.join(key)).ok();
            let vendor = read("vendor");
            let name = match vendor.as_deref().map(str::trim) {
                Some("0x1002") => "AMD GPU",
                Some("0x8086") => "Intel GPU",
                Some("0x10de") if include_nvidia => "NVIDIA GPU",
                Some("0x10de") => return None,
                _ => "GPU",
            };
            let number = |key| read(key)?.trim().parse::<u64>().ok();
            let utilization_percent = number("gpu_busy_percent").and_then(percent);
            let memory_used_bytes = number("mem_info_vram_used");
            let memory_total_bytes = number("mem_info_vram_total").filter(|total| *total != 0);
            (utilization_percent.is_some() || memory_used_bytes.is_some()).then(|| GpuLoadSample {
                id: format!("drm-{index}"),
                name: format!("{name} · card{index}"),
                utilization_percent,
                memory_used_bytes,
                memory_total_bytes,
                ..Default::default()
            })
        })
        .collect::<Vec<_>>();
    gpus.sort_unstable_by(|left, right| left.id.cmp(&right.id));
    gpus
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
async fn probe() -> Vec<GpuLoadSample> {
    Vec::new()
}

#[cfg(any(test, target_os = "macos", target_os = "linux", target_os = "windows"))]
fn percent(value: u64) -> Option<f32> {
    (value <= 100).then_some(value as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvidia_reports_each_device_and_keeps_missing_counters_distinct_from_zero() {
        let gpus = nvidia_gpus(
            r#"
GPU 00000000:01:00.0
    FB Memory Usage
        Total                             : 16384 MiB
        Reserved                          : 256 MiB
        Used                              : 4096 MiB
    BAR1 Memory Usage
        Total                             : 256 MiB
        Used                              : 16 MiB
    Utilization
        Gpu                               : 73 %
        Encoder                           : 48 %
        Decoder                           : 0 %

GPU 00000000:02:00.0
    FB Memory Usage
        Total                             : 8192 MiB
        Used                              : 512 MiB
    Utilization
        Gpu                               : N/A
        Encoder                           : [Not Supported]
        Decoder                           : 101 %
"#,
        );
        assert_eq!(gpus.len(), 2);
        assert_ne!(gpus[0].id, gpus[1].id);
        assert_eq!(gpus[0].utilization_percent, Some(73.));
        assert_eq!(gpus[0].encoder_percent, Some(48.));
        assert_eq!(gpus[0].decoder_percent, Some(0.));
        assert_eq!(gpus[0].memory_used_bytes, mib(4096));
        assert_eq!(gpus[0].memory_total_bytes, mib(16384));
        assert_eq!(gpus[1].utilization_percent, None);
        assert_eq!(gpus[1].encoder_percent, None);
        assert_eq!(gpus[1].decoder_percent, None);
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "requires a Mac with IOAccelerator performance counters"]
    async fn real_apple_gpu_counters_are_available_to_the_server() {
        let gpus = probe().await;
        assert!(gpus.iter().any(|gpu| gpu.utilization_percent.is_some()
            && gpu.memory_used_bytes.is_some()
            && gpu.unified_memory));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn apple_uses_device_activity_and_gpu_owned_unified_memory() {
        let gpus = apple_gpus(
            r#" |   "PerformanceStatistics" = {"In use system memory (driver)"=0,"Alloc system memory"=8192,"Renderer Utilization %"=45,"Device Utilization %"=55,"In use system memory"=4096}"#,
        );
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].utilization_percent, Some(55.));
        assert_eq!(gpus[0].memory_used_bytes, Some(4096));
        assert_eq!(gpus[0].memory_total_bytes, None);
        assert!(gpus[0].unified_memory);
    }
}
