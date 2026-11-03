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
/// Stable editor identity, independent of source identity and stacking order.
pub fn new_entity_id() -> u64 {
    let mut bytes = [0; 8];
    getrandom::fill(&mut bytes).expect("secure random source is unavailable");
    u64::from_le_bytes(bytes).max(1)
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BitrateMode {
    Cbr,
    #[default]
    Vbr,
}
impl BitrateMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cbr => "CBR",
            Self::Vbr => "VBR",
        }
    }
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
    #[serde(default)]
    pub bitrate_mode: BitrateMode,
    pub enabled: bool,
    #[serde(default)]
    pub audio: crate::audio::OutputAudio,
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
    /// Offline editor pattern size. Live frames retain their decoded dimensions.
    #[serde(default = "default_input_size")]
    pub resolution: Resolution,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    #[serde(default = "new_entity_id")]
    pub id: u64,
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
impl Default for Layer {
    fn default() -> Self {
        Self {
            id: new_entity_id(),
            css: String::new(),
            crop: SourceCrop::default(),
            effects: vec![],
            source_id: 0,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub id: u32,
    pub name: String,
    pub orientation: Orientation,
    pub layers: Vec<Layer>,
    #[serde(default)]
    pub audio: crate::audio::SceneAudio,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spatial: Option<crate::spatial::SpatialScene>,
}
impl Scene {
    pub fn visual_layers(&self) -> impl Iterator<Item = &Layer> {
        self.layers.iter().chain(
            self.spatial
                .iter()
                .flat_map(|s| s.objects.iter())
                .flat_map(|o| match &o.kind {
                    crate::spatial::ObjectKind::Viewport { layers, .. } => layers.as_slice(),
                    _ => &[],
                }),
        )
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionSettings {
    #[serde(default)]
    pub enabled: bool,
    /// Public extension-defined settings. Integration credentials stay on clients.
    pub config: std::collections::BTreeMap<String, String>,
}
impl<'de> Deserialize<'de> for ExtensionSettings {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize, Default)]
        #[serde(default, deny_unknown_fields)]
        struct Saved {
            enabled: bool,
            config: std::collections::BTreeMap<String, String>,
            client_id: String,
        }
        let mut saved = Saved::deserialize(deserializer)?;
        if !saved.client_id.is_empty() {
            saved
                .config
                .entry("client_id".into())
                .or_insert(saved.client_id);
        }
        Ok(Self {
            enabled: saved.enabled,
            config: saved.config,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    #[serde(default)]
    pub edit_revision: u64,
    #[serde(default = "new_token")]
    pub id: String,
    pub version: u32,
    pub name: String,
    /// HTTP(S) API base. Empty in legacy files: use the UI's startup default.
    #[serde(default)]
    pub server_url: String,
    #[serde(default)]
    pub extensions: std::collections::BTreeMap<String, ExtensionSettings>,
    /// Local client integrations. Excluded from Apply/Start by server_configuration().
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub client_extensions: std::collections::BTreeMap<String, ExtensionSettings>,
    #[serde(default)]
    pub client_capabilities: Vec<acheflow_extension_api::client_execution::CapabilityGrant>,
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
            edit_revision: 0,
            id: new_token(),
            version: 1,
            name: "My workspace".into(),
            server_url: String::new(),
            extensions: Default::default(),
            client_extensions: Default::default(),
            client_capabilities: Default::default(),
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
    /// Upgrade legacy composition containers without changing layer identities,
    /// image geometry, orientation selection or scene-scoped audio routing.
    pub fn migrate_spatial(&mut self) {
        for scene in &mut self.scenes {
            if scene.spatial.is_none() {
                let size = match scene.orientation {
                    Orientation::Horizontal => &self.horizontal.base,
                    Orientation::Vertical => &self.vertical.base,
                };
                scene.spatial = Some(crate::spatial::SpatialScene::from_canvas(size));
            }
        }
    }
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
            if let Some(spatial) = &mut scene.spatial {
                for object in &mut spatial.objects {
                    if let crate::spatial::ObjectKind::Viewport { layers, .. } = &mut object.kind {
                        layers.retain(|l| l.source_id != id);
                    }
                }
            }
            scene.audio.strips.retain(|strip| strip.source_id != id);
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
            audio: crate::audio::SceneAudio::default(),
            spatial: None,
        });
        if self.active(o).is_none() {
            self.set_active(o, Some(id));
        }
        id
    }
    pub fn remove_scene(&mut self, id: u32) {
        self.scenes.retain(|s| s.id != id);
        for o in Orientation::ALL {
            for output in &mut self.stream_mut(o).outputs {
                output.audio.scene_buses.remove(&id);
            }
        }
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
    /// Encoding/transport changes require a restart; audio controls are live.
    pub fn same_output_profile(&self, other: &Self, orientation: Orientation) -> bool {
        let strip_audio = |stream: &Stream| {
            let mut stream = stream.clone();
            for output in &mut stream.outputs {
                output.audio = Default::default();
            }
            stream
        };
        strip_audio(self.stream(orientation)) == strip_audio(other.stream(orientation))
    }
    pub fn validate_audio(&self) -> Result<(), String> {
        let sources = self.sources.iter().map(|s| s.id).collect::<Vec<_>>();
        for scene in &self.scenes {
            if let Some(spatial) = &scene.spatial {
                spatial.validate(&self.sources.iter().map(|s| s.id).collect::<Vec<_>>())?;
            }
            scene.audio.validate(&sources)?;
        }
        for orientation in Orientation::ALL {
            for output in &self.stream(orientation).outputs {
                if output.audio.gain > 200 {
                    return Err("Destination audio gain must be 0–200 percent".into());
                }
                for (scene, bus) in &output.audio.scene_buses {
                    if !self.scenes.iter().any(|s| {
                        s.id == *scene
                            && s.orientation == orientation
                            && s.audio.buses.iter().any(|b| b.id == *bus)
                    }) {
                        return Err(
                            "Destination audio route refers to a missing scene or bus".into()
                        );
                    }
                }
            }
        }
        Ok(())
    }
    pub fn remove_audio_bus(&mut self, scene_id: u32, bus_id: u32) {
        if bus_id == crate::audio::PROGRAM_BUS {
            return;
        }
        if let Some(scene) = self.scenes.iter_mut().find(|s| s.id == scene_id) {
            scene.audio.buses.retain(|b| b.id != bus_id);
            for strip in &mut scene.audio.strips {
                strip.sends.remove(&bus_id);
            }
        }
        for orientation in Orientation::ALL {
            for output in &mut self.stream_mut(orientation).outputs {
                if output.audio.scene_buses.get(&scene_id) == Some(&bus_id) {
                    output.audio.scene_buses.remove(&scene_id);
                }
            }
        }
    }
    fn validate_extensions(&self) -> Result<(), String> {
        if self.extensions.len() > 8
            || self.client_extensions.len() > 8
            || self
                .extensions
                .iter()
                .chain(&self.client_extensions)
                .any(|(id, settings)| {
                    id.is_empty()
                        || id.len() > 64
                        || !id
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
                        || settings.config.len() > 32
                        || settings.config.iter().any(|(key, value)| {
                            !acheflow_extension_api::integration::valid_id(key, 64)
                                || value.len() > 8192
                        })
                })
        {
            return Err("Invalid workspace extension settings".into());
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        self.validate_audio()?;
        self.validate_extensions()?;
        if !self.server_url.is_empty() {
            normalize_server_url(&self.server_url)?;
        }
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
                || !["rtmp://", "rtmps://"]
                    .iter()
                    .any(|scheme| source.url.starts_with(scheme))
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
                return Err("Offline source preview must be at most 1920 × 1080 pixels (portrait also supported)".into());
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
            let mut layer_ids = std::collections::HashSet::new();
            for l in &scene.layers {
                if l.id == 0 || !layer_ids.insert(l.id) {
                    return Err("Invalid or duplicate layer identity".into());
                }
                crate::appearance::validate_css(&l.css)?;
                let mut effect_ids = std::collections::HashSet::new();
                for effect in &l.effects {
                    if effect.id == 0 || !effect_ids.insert(effect.id) {
                        return Err("Invalid or duplicate effect identity".into());
                    }
                    if effect.kind == EffectKind::Wgsl {
                        crate::appearance::validate_shader(&effect.shader)?;
                    } else if !effect.shader.is_empty() {
                        return Err("Shader code requires a WGSL effect".into());
                    }
                }
                // The incoming size is known only at runtime. Reject impossible
                // insets here; the renderer checks the actual decoded image.
                if !l.crop.valid(&Resolution {
                    width: 7680,
                    height: 7680,
                }) {
                    return Err("Source crop exceeds the maximum input dimensions".into());
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
    pub fn server_configuration(&self) -> Self {
        let mut server = self.clone();
        server.client_extensions.clear();
        server
    }
    pub fn migrate_client_extensions<'a>(&mut self, ids: impl IntoIterator<Item = &'a str>) {
        for id in ids {
            if let Some(saved) = self.extensions.remove(id) {
                self.client_extensions.entry(id.into()).or_insert(saved);
            }
        }
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

/// Keep an optional reverse-proxy path while rejecting ambiguous API bases.
pub fn normalize_server_url(address: &str) -> Result<String, String> {
    let invalid =
        || "Enter an HTTP(S) server address without credentials, query or fragment.".to_owned();
    let url = url::Url::parse(address.trim()).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port() == Some(0)
    {
        return Err(invalid());
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_preferences_roundtrip_and_legacy_defaults_are_disabled() {
        let mut legacy = serde_json::to_value(Workspace::default()).unwrap();
        legacy.as_object_mut().unwrap().remove("extensions");
        let mut configured = legacy.clone();
        configured["extensions"] = serde_json::json!({
            "twitch": {"enabled": true, "client_id": "publicapplication123"},
            "sample": {"enabled": false}
        });
        let workspace = Workspace::from_json(&serde_json::to_vec(&configured).unwrap()).unwrap();
        let saved: serde_json::Value = serde_json::from_str(&workspace.json().unwrap()).unwrap();
        assert_eq!(saved["extensions"]["twitch"]["enabled"], true);
        assert_eq!(
            saved["extensions"]["twitch"]["config"]["client_id"],
            "publicapplication123"
        );
        assert_eq!(saved["extensions"]["sample"]["enabled"], false);
        let restored = Workspace::from_json(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert!(
            serde_json::to_value(restored).unwrap()["extensions"]
                .as_object()
                .is_none_or(|e| e.is_empty())
        );
    }

    #[test]
    fn server_address_survives_export_and_legacy_workspaces_still_load() {
        let workspace = Workspace {
            server_url: "https://studio.example:8443/acheflow".into(),
            ..Workspace::default()
        };
        assert_eq!(
            Workspace::from_json(workspace.json().unwrap().as_bytes()).unwrap(),
            workspace
        );
        let mut legacy = serde_json::to_value(&workspace).unwrap();
        legacy.as_object_mut().unwrap().remove("server_url");
        assert!(
            Workspace::from_json(&serde_json::to_vec(&legacy).unwrap())
                .unwrap()
                .server_url
                .is_empty()
        );
    }

    #[test]
    fn server_addresses_require_http_without_credentials_query_or_fragment() {
        for address in [
            "",
            "localhost:21295",
            "rtmp://host/live",
            "https://user:pass@host",
            "https://host/?token=secret",
            "https://host/#fragment",
            "http://host:0",
        ] {
            assert!(normalize_server_url(address).is_err(), "{address}");
        }
        assert_eq!(
            normalize_server_url(" https://host:8443/prefix/ ").unwrap(),
            "https://host:8443/prefix"
        );
        assert_eq!(
            normalize_server_url("http://[::1]:21295/").unwrap(),
            "http://[::1]:21295"
        );
    }
    #[test]
    fn native_crop_is_independent_of_offline_pattern_size() {
        let mut workspace = Workspace::default();
        workspace.add_source();
        workspace.sources[0].resolution = Resolution {
            width: 64,
            height: 64,
        };
        let scene = workspace.add_scene(Orientation::Horizontal);
        workspace.add_layer(scene, 1);
        workspace.scenes[0].layers[0].crop.left = 100;
        let json = workspace.json().unwrap();
        assert_eq!(Workspace::from_json(json.as_bytes()).unwrap(), workspace);
    }
    #[test]
    fn layer_and_effect_identity_survives_reorder_and_legacy_import() {
        let mut w = Workspace::default();
        w.add_source();
        let scene = w.add_scene(Orientation::Horizontal);
        w.add_layer(scene, 1);
        w.add_layer(scene, 1);
        w.scenes[0].layers[0]
            .effects
            .push(LayerEffect::new(EffectKind::Wgsl));
        let before = serde_json::to_value(&w).unwrap();
        let first = &before["scenes"][0]["layers"][0];
        let second = &before["scenes"][0]["layers"][1];
        assert!(first["id"].as_u64().is_some_and(|id| id != 0));
        assert_ne!(first["id"], second["id"]);
        assert!(w.move_layer(scene, 0, 1));
        let after = serde_json::to_value(&w).unwrap();
        assert_eq!(after["scenes"][0]["layers"][1], *first);
        let mut legacy = before;
        for layer in legacy["scenes"][0]["layers"].as_array_mut().unwrap() {
            layer.as_object_mut().unwrap().remove("id");
            for effect in layer["effects"].as_array_mut().unwrap() {
                effect.as_object_mut().unwrap().remove("id");
            }
        }
        let migrated = Workspace::from_json(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        let saved = migrated.json().unwrap();
        assert_eq!(Workspace::from_json(saved.as_bytes()).unwrap(), migrated);
    }
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
        p.scenes[0].layers[1].crop.left = u32::MAX;
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
    fn scene_audio_routes_validate_roundtrip_and_cleanup() {
        use crate::audio::*;
        let mut p = Workspace::default();
        p.add_source();
        let scene = p.add_scene(Orientation::Horizontal);
        p.add_layer(scene, 1);
        p.horizontal.outputs.push(Output {
            name: "Audio test".into(),
            url: "rtmp://example.com/live".into(),
            key: "test-key".into(),
            resolution: p.horizontal.base.clone(),
            quality: "Standard".into(),
            codec: "h264".into(),
            encoder: None,
            bitrate_kbps: 4500,
            bitrate_mode: Default::default(),
            enabled: true,
            audio: Default::default(),
        });
        let legacy = {
            let mut value = serde_json::to_value(&p).unwrap();
            value["scenes"][0].as_object_mut().unwrap().remove("audio");
            value["horizontal"]["outputs"][0]
                .as_object_mut()
                .unwrap()
                .remove("audio");
            value
        };
        assert_eq!(
            Workspace::from_json(&serde_json::to_vec(&legacy).unwrap()).unwrap(),
            p
        );
        let second = p.add_scene(Orientation::Horizontal);
        p.scenes[0].audio.buses.push(AudioBus {
            id: 2,
            name: "Clean".into(),
            ..AudioBus::default()
        });
        p.scenes[0].audio.strips.push(AudioStrip {
            sends: std::collections::BTreeMap::from([(2, 100)]),
            ..AudioStrip::new(
                TrackKey {
                    source_id: 1,
                    track_id: 1,
                },
                false,
            )
        });
        p.horizontal.outputs[0].audio.scene_buses.insert(scene, 2);
        assert_eq!(
            Workspace::from_json(p.json().unwrap().as_bytes()).unwrap(),
            p
        );
        assert_eq!(p.scenes[1].audio, SceneAudio::default());
        let mut changed = p.clone();
        changed.horizontal.outputs[0].audio.muted = true;
        assert!(p.same_output_profile(&changed, Orientation::Horizontal));
        changed.horizontal.outputs[0].bitrate_kbps += 1;
        assert!(!p.same_output_profile(&changed, Orientation::Horizontal));
        p.horizontal.outputs[0].audio.scene_buses.insert(second, 2);
        assert!(p.validate_audio().is_err());
        p.horizontal.outputs[0].audio.scene_buses.remove(&second);
        p.remove_audio_bus(scene, 1);
        assert_eq!(p.scenes[0].audio.buses.len(), 2);
        p.remove_audio_bus(scene, 2);
        assert!(p.horizontal.outputs[0].audio.scene_buses.is_empty());
        assert!(p.scenes[0].audio.strips[0].sends.is_empty());
        p.remove_source(1);
        assert!(p.scenes[0].audio.strips.is_empty());
        p.horizontal.outputs[0].audio.scene_buses.insert(second, 1);
        p.remove_scene(second);
        assert!(p.horizontal.outputs[0].audio.scene_buses.is_empty());
        assert!(p.validate().is_ok());
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
            bitrate_mode: Default::default(),
            enabled: true,
            audio: Default::default(),
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
            bitrate_mode: Default::default(),
            enabled: true,
            audio: Default::default(),
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
        legacy["horizontal"]["outputs"][0]
            .as_object_mut()
            .unwrap()
            .remove("bitrate_mode");
        let restored = Workspace::from_json(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert_eq!(
            restored.horizontal.outputs[0].bitrate_mode,
            BitrateMode::Vbr
        );
        let mut cbr = restored.clone();
        cbr.horizontal.outputs[0].bitrate_mode = BitrateMode::Cbr;
        assert!(!cbr.same_output_profile(&restored, Orientation::Horizontal));
        assert_eq!(
            Workspace::from_json(cbr.json().unwrap().as_bytes()).unwrap(),
            cbr
        );
        assert_eq!(restored.horizontal.outputs[0].encoder, None);
        assert_eq!(restored.horizontal.fps, 60);
        assert_eq!(restored.vertical.fps, 24);
    }
}

#[cfg(test)]
mod client_extension_tests {
    use super::*;
    #[test]
    fn local_provider_settings_migrate_and_persist_but_never_enter_server_configuration() {
        let mut workspace = Workspace::default();
        workspace.extensions.insert(
            "sample".into(),
            ExtensionSettings {
                enabled: true,
                ..Default::default()
            },
        );
        workspace.extensions.insert(
            "alpha".into(),
            ExtensionSettings {
                enabled: true,
                config: std::collections::BTreeMap::from([(
                    "application".into(),
                    "local-app".into(),
                )]),
            },
        );
        workspace.migrate_client_extensions(["alpha"]);
        assert!(!workspace.extensions.contains_key("alpha"));
        assert!(workspace.client_extensions["alpha"].enabled);
        let local = Workspace::from_json(workspace.json().unwrap().as_bytes()).unwrap();
        assert_eq!(local.client_extensions, workspace.client_extensions);
        let server = local.server_configuration();
        let payload = serde_json::to_value(&server).unwrap();
        assert!(server.client_extensions.is_empty());
        assert!(payload.get("client_extensions").is_none());
        assert!(!payload.to_string().contains("local-app"));
        assert!(server.extensions["sample"].enabled);
        workspace.extensions.insert(
            "alpha".into(),
            ExtensionSettings {
                enabled: false,
                ..Default::default()
            },
        );
        workspace.migrate_client_extensions(["alpha"]);
        assert!(
            workspace.client_extensions["alpha"].enabled,
            "new local settings take precedence over old-format entries"
        );
        assert!(!workspace.extensions.contains_key("alpha"));
    }
}
