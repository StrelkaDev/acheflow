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
    ColorCorrection,
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
    pub const ALL: [Self; 9] = [
        Self::ColorCorrection,
        Self::Brightness,
        Self::Contrast,
        Self::Saturation,
        Self::Grayscale,
        Self::Blur,
        Self::Opacity,
        Self::ChromaKey,
        Self::Wgsl,
    ];
    pub const ADDABLE: [Self; 4] = [
        Self::ColorCorrection,
        Self::ChromaKey,
        Self::Blur,
        Self::Wgsl,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::ColorCorrection => "Color correction",
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
            Self::ColorCorrection | Self::Grayscale | Self::ChromaKey | Self::Wgsl => 100,
            Self::Blur => 3,
            Self::Opacity => 75,
        }
    }
}

/// Percentages, with 100 representing a neutral multiplier or a gamma of 1.0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ColorCorrection {
    pub brightness: u32,
    pub contrast: u32,
    pub saturation: u32,
    pub grayscale: u32,
    pub gamma: u32,
    pub opacity: u32,
}

impl Default for ColorCorrection {
    fn default() -> Self {
        Self {
            brightness: 100,
            contrast: 100,
            saturation: 100,
            grayscale: 0,
            gamma: 100,
            opacity: 100,
        }
    }
}

impl ColorCorrection {
    pub const MIN_GAMMA: u32 = 10;
    pub const MAX_GAMMA: u32 = 300;

    fn is_valid(self) -> bool {
        self.brightness <= EffectKind::Brightness.max()
            && self.contrast <= EffectKind::Contrast.max()
            && self.saturation <= EffectKind::Saturation.max()
            && self.grayscale <= EffectKind::Grayscale.max()
            && self.opacity <= EffectKind::Opacity.max()
            && (Self::MIN_GAMMA..=Self::MAX_GAMMA).contains(&self.gamma)
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_correction: Option<ColorCorrection>,
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
            color_correction: (kind == EffectKind::ColorCorrection).then(ColorCorrection::default),
            amount: kind.default_amount(),
            enabled: true,
        }
    }

    pub(crate) fn validate_settings(&self) -> Result<(), String> {
        if self.color_correction.is_some_and(|correction| {
            self.kind != EffectKind::ColorCorrection || !correction.is_valid()
        }) {
            return Err("Invalid Color correction settings".into());
        }
        if self
            .chroma_key
            .is_some_and(|key| self.kind != EffectKind::ChromaKey || !key.is_valid())
        {
            return Err("Invalid Chroma key settings".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_correction_can_be_saved_with_a_gamma_value() {
        let saved = serde_json::json!({
            "id": 1, "kind": "color_correction", "amount": 100, "enabled": true,
            "color_correction": {"gamma": 150}
        });
        let effect = serde_json::from_value::<LayerEffect>(saved).unwrap();
        assert!(effect.validate_settings().is_ok());
        let encoded = serde_json::to_value(&effect).unwrap();
        assert_eq!(encoded["color_correction"]["gamma"], 150);
        assert_eq!(encoded["color_correction"]["brightness"], 100);
        let mut invalid = effect;
        invalid.color_correction.as_mut().unwrap().gamma = 0;
        assert!(invalid.validate_settings().is_err());
    }
}
