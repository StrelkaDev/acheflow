//! Additive scene-layer settings shared by native, WASM and the media server.
use crate::workspace::Resolution;
use serde::{Deserialize, Serialize};

/// Insets in native source pixels, before fitting into a scene layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCrop {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}
impl SourceCrop {
    pub fn valid(self, size: &Resolution) -> bool {
        u64::from(self.left) + u64::from(self.right) < u64::from(size.width)
            && u64::from(self.top) + u64::from(self.bottom) < u64::from(size.height)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    Brightness,
    Contrast,
    Saturation,
    Grayscale,
    Blur,
    Opacity,
    ChromaKey,
    Wgsl,
}
impl EffectKind {
    pub const ALL: [Self; 8] = [
        Self::Brightness,
        Self::Contrast,
        Self::Saturation,
        Self::Grayscale,
        Self::Blur,
        Self::Opacity,
        Self::ChromaKey,
        Self::Wgsl,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Brightness => "Brightness",
            Self::Contrast => "Contrast",
            Self::Saturation => "Saturation",
            Self::Grayscale => "Grayscale",
            Self::Blur => "Blur",
            Self::Opacity => "Opacity",
            Self::ChromaKey => "Chroma key",
            Self::Wgsl => "WGSL shader",
        }
    }
    pub fn amount_label(self) -> &'static str {
        if self == Self::Blur {
            "Radius (px)"
        } else {
            "Amount (%)"
        }
    }
    pub fn max(self) -> u32 {
        match self {
            Self::Brightness | Self::Contrast | Self::Saturation | Self::Wgsl => 200,
            Self::Blur => 32,
            _ => 100,
        }
    }
    pub fn default_amount(self) -> u32 {
        match self {
            Self::Brightness | Self::Contrast => 120,
            Self::Saturation => 50,
            Self::Grayscale | Self::ChromaKey => 100,
            Self::Blur => 3,
            Self::Opacity => 75,
            Self::Wgsl => 100,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ChromaKey {
    pub color: [u8; 3],
    pub tolerance: u8,
    pub softness: u8,
    pub edge: u8,
    pub despill: u8,
}

impl Default for ChromaKey {
    fn default() -> Self {
        Self {
            color: [13, 191, 20],
            tolerance: 12,
            softness: 12,
            edge: 1,
            despill: 85,
        }
    }
}

impl ChromaKey {
    pub const MAX_EDGE: u8 = 4;
    pub const MAX_PERCENT: u8 = 100;

    pub fn is_valid(self) -> bool {
        self.tolerance <= Self::MAX_PERCENT
            && (1..=Self::MAX_PERCENT).contains(&self.softness)
            && self.edge <= Self::MAX_EDGE
            && self.despill <= Self::MAX_PERCENT
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerEffect {
    #[serde(default = "crate::workspace::new_entity_id")]
    pub id: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shader: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chroma_key: Option<ChromaKey>,
    pub kind: EffectKind,
    pub amount: u32,
    pub enabled: bool,
}
impl LayerEffect {
    pub fn new(kind: EffectKind) -> Self {
        Self {
            id: crate::workspace::new_entity_id(),
            shader: if kind == EffectKind::Wgsl {
                crate::appearance::DEFAULT_SHADER.into()
            } else {
                String::new()
            },
            kind,
            chroma_key: (kind == EffectKind::ChromaKey).then(ChromaKey::default),
            amount: kind.default_amount(),
            enabled: true,
        }
    }

    pub(crate) fn validate_chroma_key(&self) -> Result<(), String> {
        if self
            .chroma_key
            .is_some_and(|key| self.kind != EffectKind::ChromaKey || !key.is_valid())
        {
            return Err("Invalid Chroma key settings".into());
        }
        Ok(())
    }
}
