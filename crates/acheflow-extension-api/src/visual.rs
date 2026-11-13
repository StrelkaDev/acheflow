//! Inert, bounded visual documents. Extensions supply data; the host owns rendering.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceType {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub settings: Vec<crate::integration::Setting>,
}
impl SourceType {
    pub fn validate(&self) -> Result<(), String> {
        let mut ids = std::collections::BTreeSet::new();
        if self.settings.iter().any(|s| {
            s.secret
                || !crate::integration::valid_id(&s.id, 64)
                || !ids.insert(&s.id)
                || s.label.is_empty()
                || s.label.len() > 128
                || s.max_length > 8192
                || s.max_length == 0
        }) {
            return Err("invalid visual source settings".into());
        }
        if !crate::integration::valid_id(&self.id, 64)
            || self.id.contains([':', '.'])
            || self.name.trim().is_empty()
            || self.name.len() > 128
            || self.settings.len() > 32
        {
            return Err("invalid visual source type".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceTag {
    #[default]
    Div,
    Span,
    Strong,
    P,
}
impl SourceTag {
    pub fn html(self) -> &'static str {
        match self {
            Self::Div => "div",
            Self::Span => "span",
            Self::Strong => "strong",
            Self::P => "p",
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SourceNode {
    pub tag: SourceTag,
    pub classes: String,
    pub text: String,
    pub children: Vec<SourceNode>,
    /// Optional absolute message/phase start time for scene-clock CSS animation.
    pub animation_at_ms: Option<u64>,
    pub exit_at_ms: Option<u64>,
    pub remove_at_ms: Option<u64>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SourceDocument {
    pub css: String,
    pub nodes: Vec<SourceNode>,
}
impl SourceDocument {
    pub fn validate(&self) -> Result<(), String> {
        fn walk(
            nodes: &[SourceNode],
            depth: usize,
            count: &mut usize,
            bytes: &mut usize,
        ) -> Result<(), String> {
            if depth > 8 {
                return Err("visual source nesting limit reached".into());
            }
            for node in nodes {
                if node
                    .exit_at_ms
                    .zip(node.animation_at_ms)
                    .is_some_and(|(exit, start)| exit < start)
                    || node
                        .remove_at_ms
                        .zip(node.exit_at_ms)
                        .is_some_and(|(remove, exit)| remove < exit)
                {
                    return Err("invalid visual source lifecycle times".into());
                }
                *count += 1;
                *bytes = bytes.saturating_add(node.text.len());
                if *count > 512
                    || *bytes > 192 * 1024
                    || node.classes.len() > 256
                    || !node
                        .classes
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_- ".contains(&b))
                {
                    return Err("invalid or oversized visual source document".into());
                }
                walk(&node.children, depth + 1, count, bytes)?;
            }
            Ok(())
        }
        if self.css.len() > 32768 {
            return Err("visual source CSS limit reached".into());
        }
        walk(&self.nodes, 0, &mut 0, &mut 0)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisteredSourceType {
    pub extension_id: String,
    pub source_type: SourceType,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisualSourceFrame {
    #[serde(default)]
    pub extension_id: String,
    pub type_id: String,
    pub source_id: u32,
    pub document: SourceDocument,
}
