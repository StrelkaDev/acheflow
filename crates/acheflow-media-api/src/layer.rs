//! Additive scene-layer settings shared by native, WASM and the media server.
use crate::workspace::Resolution;
use serde::{Deserialize, Serialize};

/// Insets in pixels of the configured source decode canvas, before scaling.
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
    Wgsl,
}
impl EffectKind {
    pub const ALL: [Self; 7] = [
        Self::Brightness,
        Self::Contrast,
        Self::Saturation,
        Self::Grayscale,
        Self::Blur,
        Self::Opacity,
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
            Self::Grayscale => 100,
            Self::Blur => 3,
            Self::Opacity => 75,
            Self::Wgsl => 100,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerEffect {
    #[serde(default = "crate::workspace::new_entity_id")]
    pub id: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shader: String,
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
            amount: kind.default_amount(),
            enabled: true,
        }
    }
}
