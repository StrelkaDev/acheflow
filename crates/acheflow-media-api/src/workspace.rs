pub use crate::layer::{EffectKind, LayerEffect, SourceCrop};
use std::fmt::Write as _;

pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("secure random source is unavailable");
    let mut token = String::with_capacity(64);
    for byte in bytes {
        write!(&mut token, "{byte:02x}").expect("write to string");
    }
    token
}
fn default_input_size() -> Resolution {
    Resolution {
        width: 1280,
        height: 720,
    }
}

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    Horizontal,
    Vertical,
}
impl Orientation {
    pub const ALL: [Self; 2] = [Self::Horizontal, Self::Vertical];
    pub fn label(self) -> &'static str {
        match self {
            Self::Horizontal => "Horizontal",
            Self::Vertical => "Vertical",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub name: String,
    pub url: String,
    pub key: String,
    pub resolution: Resolution,
    pub quality: String,
    pub codec: String,
    #[serde(default)]
    pub encoder: Option<String>,
    pub bitrate_kbps: u32,
    pub enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stream {
    pub base: Resolution,
    pub fps: u32,
    pub outputs: Vec<Output>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: u32,
    pub name: String,
    pub url: String,
    pub key: String,
    #[serde(default = "default_input_size")]
    pub resolution: Resolution,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[derive(Default)]
pub struct Layer {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub css: String,
    #[serde(default)]
    pub crop: SourceCrop,
    #[serde(default)]
    pub effects: Vec<LayerEffect>,
    pub source_id: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub id: u32,
    pub name: String,
    pub orientation: Orientation,
    pub layers: Vec<Layer>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    #[serde(default = "new_token")]
    pub id: String,
    pub version: u32,
    pub name: String,
    pub horizontal: Stream,
    pub vertical: Stream,
    pub sources: Vec<Source>,
    pub scenes: Vec<Scene>,
    pub horizontal_scene: Option<u32>,
    pub vertical_scene: Option<u32>,
}
impl Default for Workspace {
    fn default() -> Self {
        Self {
            id: new_token(),
            version: 1,
            name: "My workspace".into(),
            horizontal: Stream {
                base: Resolution {
                    width: 1920,
                    height: 1080,
                },
                fps: 30,
                outputs: vec![],
            },
            vertical: Stream {
                base: Resolution {
                    width: 1080,
                    height: 1920,
                },
                fps: 30,
                outputs: vec![],
            },
            sources: vec![],
            scenes: vec![],
            horizontal_scene: None,
            vertical_scene: None,
        }
    }
}
impl Workspace {
    pub fn stream(&self, o: Orientation) -> &Stream {
        match o {
            Orientation::Horizontal => &self.horizontal,
            Orientation::Vertical => &self.vertical,
        }
    }
    pub fn stream_mut(&mut self, o: Orientation) -> &mut Stream {
        match o {
            Orientation::Horizontal => &mut self.horizontal,
            Orientation::Vertical => &mut self.vertical,
        }
    }
    pub fn active(&self, o: Orientation) -> Option<u32> {
        match o {
            Orientation::Horizontal => self.horizontal_scene,
            Orientation::Vertical => self.vertical_scene,
        }
    }
    pub fn set_active(&mut self, o: Orientation, id: Option<u32>) {
        match o {
            Orientation::Horizontal => self.horizontal_scene = id,
            Orientation::Vertical => self.vertical_scene = id,
        }
    }
    pub fn add_source(&mut self) {
        let id = (1..=u32::MAX)
            .find(|id| !self.sources.iter().any(|s| s.id == *id))
            .expect("source IDs exhausted");
        self.sources.push(Source {
            id,
            name: format!("Source {id}"),
            url: "rtmp://localhost:1935/live".into(),
            key: new_token(),
            resolution: default_input_size(),
        });
    }
    pub fn remove_source(&mut self, id: u32) {
        self.sources.retain(|s| s.id != id);
        for scene in &mut self.scenes {
            scene.layers.retain(|l| l.source_id != id);
        }
    }
    pub fn add_scene(&mut self, o: Orientation) -> u32 {
        let id = (1..=u32::MAX)
            .find(|id| !self.scenes.iter().any(|s| s.id == *id))
            .expect("scene IDs exhausted");
        self.scenes.push(Scene {
            id,
            name: format!("Scene {id}"),
            orientation: o,
            layers: vec![],
        });
        if self.active(o).is_none() {
            self.set_active(o, Some(id));
        }
        id
    }
    pub fn remove_scene(&mut self, id: u32) {
        self.scenes.retain(|s| s.id != id);
        for o in Orientation::ALL {
            if self.active(o) == Some(id) {
                let next = self
                    .scenes
                    .iter()
                    .find(|s| s.orientation == o)
                    .map(|s| s.id);
                self.set_active(o, next);
            }
        }
    }
    pub fn add_layer(&mut self, scene_id: u32, source_id: u32) {
        if !self.sources.iter().any(|s| s.id == source_id) {
            return;
        }
        if let Some(o) = self
            .scenes
            .iter()
            .find(|s| s.id == scene_id)
            .map(|s| s.orientation)
        {
            let size = self.stream(o).base.clone();
            self.scenes
                .iter_mut()
                .find(|s| s.id == scene_id)
                .unwrap()
                .layers
                .push(Layer {
                    source_id,
                    x: 0,
                    y: 0,
                    width: size.width,
                    height: size.height,
                    ..Layer::default()
                });
        }
    }
    /// Index zero is the back; the last layer is the front.
    pub fn move_layer(&mut self, scene_id: u32, from: usize, to: usize) -> bool {
        let Some(scene) = self.scenes.iter_mut().find(|s| s.id == scene_id) else {
            return false;
        };
        if from >= scene.layers.len() || to >= scene.layers.len() {
            return false;
        }
        let layer = scene.layers.remove(from);
        scene.layers.insert(to, layer);
        true
    }
    pub fn resize_base(&mut self, o: Orientation, size: Resolution) {
        let old = self.stream(o).base.clone();
        for scene in self.scenes.iter_mut().filter(|s| s.orientation == o) {
            for layer in &mut scene.layers {
                layer.x =
                    (u64::from(layer.x) * u64::from(size.width) / u64::from(old.width)) as u32;
                layer.y =
                    (u64::from(layer.y) * u64::from(size.height) / u64::from(old.height)) as u32;
                layer.width = ((u64::from(layer.width) * u64::from(size.width)
                    / u64::from(old.width)) as u32)
                    .max(1);
                layer.height = ((u64::from(layer.height) * u64::from(size.height)
                    / u64::from(old.height)) as u32)
                    .max(1);
            }
        }
        self.stream_mut(o).base = size;
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || self.id.len() > 128
            || !self
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err("Invalid workspace identity".into());
        }
        if self.version != 1 {
            return Err("Unsupported workspace version".into());
        }
        if self.name.trim().is_empty() {
            return Err("Workspace name is required".into());
        }
        let valid_size = |r: &Resolution| {
            r.width >= 16
                && r.height >= 16
                && r.width <= 7680
                && r.height <= 7680
                && r.width.is_multiple_of(2)
                && r.height.is_multiple_of(2)
        };
        for o in Orientation::ALL {
            let s = self.stream(o);
            if !valid_size(&s.base) || !(1..=120).contains(&s.fps) {
                return Err("Invalid base resolution or FPS".into());
            }
            for out in &s.outputs {
                if out.name.trim().is_empty()
                    || !out.url.starts_with("rtmp://")
                    || out.url.len() <= 7
                    || out.key.trim().is_empty()
                    || !valid_size(&out.resolution)
                    || !crate::RTMP_VIDEO_CODECS.contains(&out.codec.as_str())
                    || out
                        .encoder
                        .as_ref()
                        .is_some_and(|name| !crate::valid_encoder_name(name))
                    || !["Low", "Standard", "High", "Custom"].contains(&out.quality.as_str())
                    || !(100..=100000).contains(&out.bitrate_kbps)
                {
                    return Err("Each output needs a name, rtmp:// URL, key, valid resolution, RTMP video codec/encoder and bitrate (100–100000 kbps)".into());
                }
            }
        }
        let mut ids = std::collections::HashSet::new();
        for source in &self.sources {
            if source.id == 0
                || !ids.insert(source.id)
                || source.name.trim().is_empty()
                || !source.url.starts_with("rtmp://")
                || source.url.len() <= 7
                || source.key.trim().is_empty()
            {
                return Err("Invalid or duplicate RTMP source".into());
            }
        }
        let mut tokens = std::collections::HashSet::new();
        for source in &self.sources {
            if !tokens.insert((&source.url, &source.key)) {
                return Err("RTMP source tokens must be unique per ingest URL".into());
            }
            if !valid_size(&source.resolution)
                || source.resolution.width as u64 * source.resolution.height as u64 > 1920 * 1080
            {
                return Err("Source decode size must be at most 1920 × 1080 pixels (portrait also supported)".into());
            }
            if source.key.len() > 256
                || source.key.len() < 8
                || !source
                    .key
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
            {
                return Err("Source token must have 8–256 URL-safe characters".into());
            }
        }
        ids.clear();
        for scene in &self.scenes {
            if scene.id == 0 || !ids.insert(scene.id) || scene.name.trim().is_empty() {
                return Err("Invalid or duplicate scene".into());
            }
            let base = &self.stream(scene.orientation).base;
            for l in &scene.layers {
                crate::appearance::validate_css(&l.css)?;
                for effect in &l.effects {
                    if effect.kind == EffectKind::Wgsl {
                        crate::appearance::validate_shader(&effect.shader)?;
                    } else if !effect.shader.is_empty() {
                        return Err("Shader code requires a WGSL effect".into());
                    }
                }
                if let Some(source) = self.sources.iter().find(|s| s.id == l.source_id)
                    && !l.crop.valid(&source.resolution)
                {
                    return Err(
                        "Source crop must leave at least one source pixel in each dimension".into(),
                    );
                }
                if l.effects.len() > 8 || l.effects.iter().any(|e| e.amount > e.kind.max()) {
                    return Err(
                        "Layer effects: maximum 8, with values in the supported range".into(),
                    );
                }
                if !self.sources.iter().any(|s| s.id == l.source_id)
                    || l.width == 0
                    || l.height == 0
                    || (l.css.is_empty()
                        && (l.x.saturating_add(l.width) > base.width
                            || l.y.saturating_add(l.height) > base.height))
                {
                    return Err(
                        "Scene layer references a missing source or is outside its base canvas"
                            .into(),
                    );
                }
            }
        }
        for o in Orientation::ALL {
            if let Some(id) = self.active(o)
                && !self.scenes.iter().any(|s| s.id == id && s.orientation == o)
            {
                return Err("Invalid studio scene selection".into());
            }
        }
        Ok(())
    }
    pub fn json(&self) -> Result<String, String> {
        self.validate()?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        if json.len() > 2 * 1024 * 1024 {
            return Err("Workspace exceeds 2 MiB".into());
        }
        Ok(json)
    }
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 2 * 1024 * 1024 {
            return Err("Workspace exceeds 2 MiB".into());
        }
        let project: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        project.validate()?;
        Ok(project)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_layers_crop_effects_and_reordering_roundtrip() {
        let mut p = Workspace::default();
        p.add_source();
        p.add_source();
        let id = p.add_scene(Orientation::Horizontal);
        p.add_layer(id, 1);
        p.add_layer(id, 2);
        let mut legacy = serde_json::to_value(&p).unwrap();
        for l in legacy["scenes"][0]["layers"].as_array_mut().unwrap() {
            let obj = l.as_object_mut().unwrap();
            obj.remove("crop");
            obj.remove("effects");
        }
        let restored = Workspace::from_json(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert_eq!(restored, p);
        p.scenes[0].layers[0].crop.left = 100;
        p.scenes[0].layers[0]
            .effects
            .push(LayerEffect::new(EffectKind::Opacity));
        let first = p.scenes[0].layers[0].clone();
        assert!(p.move_layer(id, 0, 1));
        assert_eq!(p.scenes[0].layers[1], first);
        assert!(!p.move_layer(id, 7, 0));
        assert_eq!(
            Workspace::from_json(p.json().unwrap().as_bytes()).unwrap(),
            p
        );
        p.scenes[0].layers[1].crop.left = p.sources[0].resolution.width;
        assert!(p.validate().is_err());
        p.scenes[0].layers[1].crop = SourceCrop::default();
        p.scenes[0].layers[1].effects[0].amount = 101;
        assert!(p.validate().is_err());
        p.scenes[0].layers[1].effects = vec![LayerEffect::new(EffectKind::Blur); 9];
        assert!(p.validate().is_err());
    }
    #[test]
    fn css_and_custom_shader_survive_workspace_export() {
        let mut p = Workspace::default();
        p.add_source();
        let scene = p.add_scene(Orientation::Horizontal);
        p.add_layer(scene, 1);
        p.scenes[0].layers[0].css="left:10%;top:16px;width:50%;height:300px;--crop-left:10%;object-fit:cover;border:3px solid red;".into();
        p.scenes[0].layers[0]
            .effects
            .push(LayerEffect::new(EffectKind::Wgsl));
        assert_eq!(
            Workspace::from_json(p.json().unwrap().as_bytes()).unwrap(),
            p
        );
    }
    #[test]
    fn roundtrip_both_formats_and_scene_layers() {
        let mut p = Workspace::default();
        p.add_source();
        let id = p.add_scene(Orientation::Vertical);
        p.add_layer(id, 1);
        p.vertical.outputs.push(Output {
            name: "Vertical channel".into(),
            url: "rtmp://example.com/live".into(),
            key: "test-key".into(),
            resolution: p.vertical.base.clone(),
            quality: "High".into(),
            codec: "h264".into(),
            encoder: None,
            bitrate_kbps: 6000,
            enabled: true,
        });
        assert_eq!(
            Workspace::from_json(p.json().unwrap().as_bytes()).unwrap(),
            p
        );
    }
    #[test]
    fn resize_scales_layers_and_removing_source_cleans_scenes() {
        let mut p = Workspace::default();
        p.add_source();
        let id = p.add_scene(Orientation::Horizontal);
        p.add_layer(id, 1);
        p.resize_base(
            Orientation::Horizontal,
            Resolution {
                width: 1280,
                height: 720,
            },
        );
        assert_eq!(p.scenes[0].layers[0].width, 1280);
        p.remove_source(1);
        assert!(p.scenes[0].layers.is_empty());
        assert!(p.validate().is_ok());
    }
    #[test]
    fn rejects_broken_references_and_versions() {
        let mut p = Workspace {
            horizontal_scene: Some(50),
            ..Workspace::default()
        };
        assert!(p.json().is_err());
        p.horizontal_scene = None;
        p.version = 2;
        assert!(Workspace::from_json(serde_json::to_string(&p).unwrap().as_bytes()).is_err());
    }
    #[test]
    fn imported_maximum_ids_do_not_break_creation() {
        let mut p = Workspace::default();
        p.add_source();
        p.sources[0].id = u32::MAX;
        p.add_source();
        assert_eq!(p.sources[1].id, 1);
        p.add_scene(Orientation::Horizontal);
        p.scenes[0].id = u32::MAX;
        p.horizontal_scene = Some(u32::MAX);
        assert_eq!(p.add_scene(Orientation::Horizontal), 1);
        assert!(p.validate().is_ok());
    }

    #[test]
    fn preserves_encoder_and_independent_fps_with_legacy_workspace_import() {
        let mut p = Workspace::default();
        p.horizontal.fps = 60;
        p.vertical.fps = 24;
        p.horizontal.outputs.push(Output {
            name: "Encoder test".into(),
            url: "rtmp://example.com/live".into(),
            key: "test-key".into(),
            resolution: p.horizontal.base.clone(),
            quality: "Standard".into(),
            codec: "hevc".into(),
            encoder: Some("hevc_videotoolbox".into()),
            bitrate_kbps: 4500,
            enabled: true,
        });
        assert_eq!(
            Workspace::from_json(p.json().unwrap().as_bytes()).unwrap(),
            p
        );
        let mut legacy = serde_json::to_value(&p).unwrap();
        legacy["horizontal"]["outputs"][0]
            .as_object_mut()
            .unwrap()
            .remove("encoder");
        let restored = Workspace::from_json(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert_eq!(restored.horizontal.outputs[0].encoder, None);
        assert_eq!(restored.horizontal.fps, 60);
        assert_eq!(restored.vertical.fps, 24);
    }
}
