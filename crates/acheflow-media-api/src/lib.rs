pub mod appearance;
pub mod layer;
pub mod live;
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
            });
        }
        assert_eq!(caps.rtmp_codecs(), ["h264", "av1"]);
        caps.enhanced_rtmp = false;
        assert_eq!(caps.rtmp_codecs(), ["h264"]);
        caps.rtmp_output = false;
        assert!(caps.rtmp_codecs().is_empty());
    }
}
