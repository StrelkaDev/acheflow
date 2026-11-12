pub mod appearance;
pub mod audio;
pub mod control;
pub mod diagnostics;
pub mod layer;
pub mod live;
pub mod server;
pub mod spatial;
pub mod workspace;

use serde::{Deserialize, Serialize};

pub const RTMP_VIDEO_CODECS: [&str; 4] = ["h264", "hevc", "av1", "vp9"];

pub fn codec_label(codec: &str) -> &str {
    match codec {
        "h264" => "H.264 / AVC",
        "hevc" => "H.265 / HEVC",
        "av1" => "AV1",
        "vp9" => "VP9",
        _ => codec,
    }
}

/// Encoder identifiers are data, never shell fragments or free-form arguments.
pub fn valid_encoder_name(name: &str) -> bool {
    name.as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphanumeric)
        && name.len() <= 128
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    Video,
    Audio,
    Subtitle,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Encoder {
    pub name: String,
    pub codec: String,
    pub media_type: MediaType,
    pub description: String,
    pub experimental: bool,
    /// Hardware-backed video encoder (including hybrid), as reported by FFmpeg help.
    #[serde(default)]
    pub hardware: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MediaCapabilities {
    pub available: bool,
    pub version: Option<String>,
    pub encoders: Vec<Encoder>,
    pub rtmp_output: bool,
    pub flv_muxer: bool,
    /// Conservatively enabled on FFmpeg 7+; older builds retain H.264 only.
    pub enhanced_rtmp: bool,
    pub error: Option<String>,
}

impl MediaCapabilities {
    /// Select within one encoder type; callers store the concrete encoder name.
    pub fn preferred_video_encoder<'a>(
        &'a self,
        codec: &'a str,
        hardware: bool,
    ) -> Option<&'a Encoder> {
        let preferred = match codec {
            "h264" => "libx264",
            "hevc" => "libx265",
            "av1" => "libsvtav1",
            "vp9" => "libvpx-vp9",
            _ => "",
        };
        self.video_encoders(codec)
            .filter(|e| e.hardware == hardware)
            .min_by_key(|e| (e.name != preferred, &e.name))
    }

    /// An explicit selection must exist for the codec; never fall back from it.
    pub fn resolve_video_encoder<'a>(
        &'a self,
        codec: &'a str,
        requested: Option<&str>,
    ) -> Result<&'a Encoder, String> {
        if let Some(name) = requested {
            self.video_encoders(codec)
                .find(|e| e.name == name)
                .ok_or_else(|| {
                    format!("Selected encoder {name} is unavailable for {codec} on this server")
                })
        } else {
            self.preferred_video_encoder(codec, false)
                .or_else(|| self.preferred_video_encoder(codec, true))
                .ok_or_else(|| format!("No FFmpeg encoder for {codec}"))
        }
    }

    pub fn video_encoders<'a>(&'a self, codec: &'a str) -> impl Iterator<Item = &'a Encoder> {
        self.encoders.iter().filter(move |e| {
            e.media_type == MediaType::Video && e.codec == codec && !e.experimental
        })
    }

    pub fn rtmp_codecs(&self) -> Vec<String> {
        if !self.available || !self.rtmp_output || !self.flv_muxer {
            return vec![];
        }
        RTMP_VIDEO_CODECS
            .into_iter()
            .filter(|codec| {
                (*codec == "h264" || self.enhanced_rtmp)
                    && self.video_encoders(codec).next().is_some()
            })
            .map(str::to_owned)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_detected_video_encoders_and_transport_compatible_codecs_are_offered() {
        let mut caps = MediaCapabilities {
            available: true,
            rtmp_output: true,
            flv_muxer: true,
            enhanced_rtmp: true,
            ..Default::default()
        };
        for (name, codec, media_type) in [
            ("libx264", "h264", MediaType::Video),
            ("libsvtav1", "av1", MediaType::Video),
            ("aac", "aac", MediaType::Audio),
            ("prores_ks", "prores", MediaType::Video),
        ] {
            caps.encoders.push(Encoder {
                name: name.into(),
                codec: codec.into(),
                media_type,
                description: String::new(),
                experimental: false,
                hardware: false,
            });
        }
        assert_eq!(caps.rtmp_codecs(), ["h264", "av1"]);
        caps.enhanced_rtmp = false;
        assert_eq!(caps.rtmp_codecs(), ["h264"]);
        caps.rtmp_output = false;
        assert!(caps.rtmp_codecs().is_empty());
    }

    #[test]
    fn explicit_hardware_and_software_selection_never_substitute_each_other() {
        let mut caps = MediaCapabilities::default();
        for (name, hardware) in [("h264_videotoolbox", true), ("libx264", false)] {
            caps.encoders.push(Encoder {
                name: name.into(),
                codec: "h264".into(),
                media_type: MediaType::Video,
                description: String::new(),
                experimental: false,
                hardware,
            });
        }
        assert_eq!(
            caps.resolve_video_encoder("h264", None).unwrap().name,
            "libx264"
        );
        assert_eq!(
            caps.preferred_video_encoder("h264", true).unwrap().name,
            "h264_videotoolbox"
        );
        assert!(
            caps.resolve_video_encoder("h264", Some("h264_videotoolbox"))
                .unwrap()
                .hardware
        );
        assert!(
            caps.resolve_video_encoder("hevc", Some("h264_videotoolbox"))
                .is_err()
        );
        caps.encoders.retain(|e| !e.hardware);
        assert!(caps.preferred_video_encoder("h264", true).is_none());
        assert!(
            caps.resolve_video_encoder("h264", Some("h264_videotoolbox"))
                .is_err()
        );
        assert_eq!(
            caps.resolve_video_encoder("h264", None).unwrap().name,
            "libx264"
        );
        caps.encoders[0].name = "libopenh264".into();
        assert_eq!(
            caps.resolve_video_encoder("h264", None).unwrap().name,
            "libopenh264"
        );
        caps.encoders[0].experimental = true;
        assert!(caps.resolve_video_encoder("h264", None).is_err());
    }

    #[test]
    fn legacy_encoder_capabilities_remain_readable() {
        let encoder: Encoder = serde_json::from_str(r#"{"name":"libx264","codec":"h264","media_type":"video","description":"AVC","experimental":false}"#).unwrap();
        assert!(!encoder.hardware);
    }
}
