use std::{path::PathBuf, process::Stdio, sync::Arc, time::Duration};

use acheflow_media_api::{Encoder, MediaCapabilities, MediaType, valid_encoder_name};
use axum::{Json, extract::State};
use tokio::{process::Command, sync::Mutex};

pub type CapabilityState = Arc<Mutex<MediaCapabilities>>;

pub async fn get(State(state): State<CapabilityState>) -> Json<MediaCapabilities> {
    Json(state.lock().await.clone())
}

pub async fn refresh(State(state): State<CapabilityState>) -> Json<MediaCapabilities> {
    // Serialize scans: repeated refreshes must not launch a process storm.
    let mut cached = state.lock().await;
    *cached = discover().await;
    Json(cached.clone())
}

pub async fn discover() -> MediaCapabilities {
    match scan(&executable()).await {
        Ok(capabilities) => capabilities,
        Err(error) => MediaCapabilities {
            error: Some(error),
            ..Default::default()
        },
    }
}

pub(crate) fn executable() -> PathBuf {
    if let Some(path) = std::env::var_os("ACHEFLOW_FFMPEG") {
        return path.into();
    }
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    if let Some(path) = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join(name))
            .find(|p| p.is_file())
    }) {
        return path;
    }
    #[cfg(target_os = "macos")]
    for path in ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"] {
        if std::path::Path::new(path).is_file() {
            return path.into();
        }
    }
    name.into()
}

async fn output(executable: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new(executable);
    command
        .args(["-hide_banner", "-nostdin"])
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(5), command.output())
        .await
        .map_err(|_| "FFmpeg discovery timed out".to_owned())?
        .map_err(|e| {
            format!("Cannot run FFmpeg: {e}. Install FFmpeg or set ACHEFLOW_FFMPEG on the server.")
        })?;
    if !output.status.success() {
        return Err(format!(
            "FFmpeg discovery failed: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(400)
                .collect::<String>()
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| format!("Invalid FFmpeg response: {e}"))
}

async fn scan(executable: &std::path::Path) -> Result<MediaCapabilities, String> {
    let version = output(executable, &["-version"]).await?;
    let version = version
        .lines()
        .next()
        .filter(|l| l.starts_with("ffmpeg version "))
        .ok_or("Unexpected FFmpeg version response")?
        .to_owned();
    let encoders = parse_encoders(&output(executable, &["-encoders"]).await?);
    if encoders.is_empty() {
        return Err("FFmpeg returned no encoders".into());
    }
    let protocols = output(executable, &["-protocols"]).await?;
    let muxers = output(executable, &["-muxers"]).await?;
    let enhanced_rtmp = version
        .strip_prefix("ffmpeg version ")
        .and_then(|s| s.trim_start_matches('n').split('.').next())
        .and_then(|s| s.parse::<u32>().ok())
        .is_some_and(|major| major >= 7);
    Ok(MediaCapabilities {
        available: true,
        version: Some(version),
        encoders,
        rtmp_output: protocols
            .split_once("Output:")
            .is_some_and(|(_, s)| s.lines().any(|l| l.trim() == "rtmp")),
        flv_muxer: muxers.lines().any(|line| {
            let mut parts = line.split_whitespace();
            let flags = parts.next().unwrap_or("");
            let formats = parts.next().unwrap_or("");
            flags.contains('E') && formats.split(',').any(|f| f == "flv")
        }),
        enhanced_rtmp,
        error: None,
    })
}

fn parse_encoders(output: &str) -> Vec<Encoder> {
    let mut encoders = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        let Some((flags, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if flags.len() != 6 || !flags.bytes().all(|c| b"VAS.FSXBD".contains(&c)) {
            continue;
        }
        let media_type = match flags.as_bytes()[0] {
            b'V' => MediaType::Video,
            b'A' => MediaType::Audio,
            b'S' => MediaType::Subtitle,
            _ => continue,
        };
        let rest = rest.trim_start();
        let Some((name, description)) = rest.split_once(char::is_whitespace) else {
            continue;
        };
        if !valid_encoder_name(name) {
            continue;
        }
        let description = description.trim();
        let codec = description
            .rsplit_once("(codec ")
            .and_then(|(_, s)| s.strip_suffix(')'))
            .unwrap_or(name);
        if !valid_encoder_name(codec) {
            continue;
        }
        encoders.push(Encoder {
            name: name.into(),
            codec: codec.into(),
            media_type,
            description: description.into(),
            experimental: flags.as_bytes()[3] == b'X',
        });
    }
    encoders.sort_by(|a, b| a.name.cmp(&b.name));
    encoders.dedup_by(|a, b| a.name == b.name);
    encoders
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_encoder_families_and_ignores_legends_and_malformed_rows() {
        let encoders = parse_encoders(
            "Encoders:\n V..... = Video\n V....D libx264 libx264 AVC (codec h264)\n V....D h264_videotoolbox VideoToolbox H.264 Encoder (codec h264)\n A....D aac AAC (Advanced Audio Coding)\n A..X.. opus Opus\n S..... srt SubRip subtitle (codec subrip)\n bad-data\n V..... -bad invalid\n",
        );
        assert_eq!(encoders.len(), 5);
        assert_eq!(
            encoders.iter().find(|e| e.name == "libx264").unwrap().codec,
            "h264"
        );
        assert_eq!(
            encoders
                .iter()
                .find(|e| e.name == "aac")
                .unwrap()
                .media_type,
            MediaType::Audio
        );
        assert!(
            encoders
                .iter()
                .find(|e| e.name == "opus")
                .unwrap()
                .experimental
        );
        assert_eq!(
            encoders.iter().find(|e| e.name == "srt").unwrap().codec,
            "subrip"
        );
    }
    #[tokio::test]
    async fn missing_binary_is_an_explicit_error_not_a_fake_codec_list() {
        assert!(
            scan(std::path::Path::new("/nonexistent/acheflow-ffmpeg-test"))
                .await
                .unwrap_err()
                .contains("Cannot run FFmpeg")
        );
    }
    #[tokio::test]
    #[ignore = "requires an installed FFmpeg; run explicitly on the integration host"]
    async fn installed_ffmpeg_reports_video_audio_and_rtmp() {
        let caps = scan(&executable()).await.unwrap();
        assert!(caps.rtmp_output && caps.flv_muxer);
        assert!(caps.video_encoders("h264").next().is_some());
        assert!(
            caps.encoders
                .iter()
                .any(|e| e.media_type == MediaType::Audio && e.codec == "aac")
        );
        println!(
            "{}: {} encoders, RTMP profiles {:?}",
            caps.version.as_deref().unwrap(),
            caps.encoders.len(),
            caps.rtmp_codecs()
        );
    }
}
