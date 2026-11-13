//! Built-in and extension visual source configuration, shared by Studio and output.
use acheflow_extension_api::interaction::ChatEntry;
pub use acheflow_extension_api::visual::{SourceDocument, SourceNode, SourceTag};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceKind {
    #[default]
    Rtmp,
    Chat(ChatSource),
    Extension(ExtensionSource),
}
impl SourceKind {
    pub fn is_rtmp(&self) -> bool {
        matches!(self, Self::Rtmp)
    }
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Rtmp => Ok(()),
            Self::Chat(config) => config.validate(),
            Self::Extension(config) => {
                if !acheflow_extension_api::integration::valid_id(&config.extension_id, 64)
                    || !acheflow_extension_api::integration::valid_id(&config.type_id, 64)
                    || config.settings.len() > 32
                    || config
                        .settings
                        .iter()
                        .any(|(k, v)| k.len() > 64 || v.len() > 8192)
                {
                    return Err("invalid extension source identity or settings".into());
                }
                crate::appearance::validate_css(&config.css)
            }
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExtensionSource {
    pub extension_id: String,
    pub type_id: String,
    pub settings: BTreeMap<String, String>,
    pub css: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ChatSource {
    pub channels: Vec<String>,
    pub max_messages: u32,
    pub lifetime_ms: u64,
    pub enter_ms: u64,
    pub exit_ms: u64,
    pub css: String,
}
impl Default for ChatSource {
    fn default() -> Self {
        Self {
            channels: vec![],
            max_messages: 20,
            lifetime_ms: 15000,
            enter_ms: 250,
            exit_ms: 300,
            css: DEFAULT_CHAT_CSS.into(),
        }
    }
}
impl ChatSource {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=50).contains(&self.max_messages)
            || !(1000..=3600000).contains(&self.lifetime_ms)
            || self.enter_ms > 10000
            || self.exit_ms > 10000
            || self.channels.len() > 64
            || self.channels.iter().any(|s| s.len() > 256)
        {
            return Err("invalid chat source limits".into());
        }
        crate::appearance::validate_css(&self.css)
    }
    pub fn document(&self, entries: &[ChatEntry], now: u64) -> SourceDocument {
        let selected = entries
            .iter()
            .rev()
            .filter(|entry| {
                !entry.deleted
                    && entry.merged_into.is_none()
                    && now
                        < entry
                            .message
                            .occurred_at_ms
                            .saturating_add(self.lifetime_ms)
                            .saturating_add(self.exit_ms)
                    && entry.message.occurred_at_ms <= now
                    && (self.channels.is_empty()
                        || self.channels.contains(&format!(
                            "{}:{}",
                            entry.message.platform, entry.message.channel_id
                        ))
                        || entry
                            .delivery
                            .iter()
                            .any(|d| self.channels.contains(&d.channel)))
            })
            .take(self.max_messages as usize)
            .collect::<Vec<_>>();
        let mut children = vec![];
        for entry in selected.into_iter().rev() {
            let message = &entry.message;
            let mut classes = "chat-message".to_owned();
            for role in ["broadcaster", "moderator"] {
                if message.roles.iter().any(|r| r == role) {
                    classes.push(' ');
                    classes.push_str(role);
                }
            }
            let node = |tag, classes: &str, text: &str| SourceNode {
                tag,
                classes: classes.into(),
                text: bounded_text(text, 3072),
                ..Default::default()
            };
            children.push(SourceNode {
                classes,
                animation_at_ms: Some(message.occurred_at_ms),
                exit_at_ms: Some(message.occurred_at_ms.saturating_add(self.lifetime_ms)),
                remove_at_ms: Some(
                    message
                        .occurred_at_ms
                        .saturating_add(self.lifetime_ms)
                        .saturating_add(self.exit_ms),
                ),
                children: vec![
                    node(SourceTag::Span, "chat-platform", &message.platform),
                    node(SourceTag::Strong, "chat-author", &message.display_name),
                    node(SourceTag::Span, "chat-body", &message.text),
                ],
                ..Default::default()
            });
        }
        SourceDocument {
            css: format!(
                ":root{{--chat-enter-duration:{}ms;--chat-exit-duration:{}ms;}}\n{}",
                self.enter_ms, self.exit_ms, self.css
            ),
            nodes: vec![SourceNode {
                classes: "chat".into(),
                children,
                ..Default::default()
            }],
        }
    }
}
fn bounded_text(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.into();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}
pub const DEFAULT_CHAT_CSS: &str = r#"
.chat {display:flex;flex-direction:column;justify-content:flex-end;gap:8px;height:100%;padding:12px;box-sizing:border-box;font-family:sans-serif;font-size:22px;color:white;}
.chat-message {padding:10px 14px;border-radius:12px;background:#10151de6;overflow-wrap:anywhere;}
.chat-platform {font-size:13px;color:#ff9944;margin-right:8px;}
.chat-author {font-weight:700;margin-right:8px;}
.chat-body {white-space:pre-wrap;}
.entering {animation:chat-enter var(--chat-enter-duration) ease-out both;}
.leaving {animation:chat-exit var(--chat-exit-duration) ease-in both;}
@keyframes chat-enter {from {opacity:0;transform:translateX(24px);}to {opacity:1;transform:translateX(0px);}}
@keyframes chat-exit {from {opacity:1;transform:translateX(0px);}to {opacity:0;transform:translateX(-24px);}}
"#;

pub fn source_documents(
    sources: &[crate::workspace::Source],
    chat: &[ChatEntry],
    frames: &[acheflow_extension_api::visual::VisualSourceFrame],
    now: u64,
) -> std::collections::HashMap<u32, SourceDocument> {
    sources
        .iter()
        .filter_map(|source| {
            let document = match &source.kind {
                SourceKind::Rtmp => return None,
                SourceKind::Chat(settings) => settings.document(chat, now),
                SourceKind::Extension(settings) => {
                    let mut document = frames
                        .iter()
                        .find(|frame| {
                            frame.source_id == source.id
                                && frame.extension_id == settings.extension_id
                                && frame.type_id == settings.type_id
                        })?
                        .document
                        .clone();
                    document.css.push('\n');
                    document.css.push_str(&settings.css);
                    document
                }
            };
            Some((source.id, document))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use acheflow_extension_api::interaction::ChatMessage;
    fn entry(at: u64) -> ChatEntry {
        ChatEntry {
            cursor: 1,
            message: ChatMessage {
                platform: "fixture".into(),
                channel_id: "channel".into(),
                message_id: "message".into(),
                user_id: "user".into(),
                display_name: "Viewer".into(),
                text: "<script>message</script>".into(),
                roles: vec![],
                occurred_at_ms: at,
            },
            deleted: false,
            merged_into: None,
            delivery: vec![],
        }
    }
    #[test]
    fn chat_lifecycle_has_stable_animation_times_and_respects_deletion() {
        let settings = ChatSource::default();
        let entries = vec![entry(1000)];
        let entering = settings.document(&entries, 1100);
        assert_eq!(entering.nodes[0].children[0].animation_at_ms, Some(1000));
        assert_eq!(entering.nodes[0].children[0].exit_at_ms, Some(16000));
        assert_eq!(entering, settings.document(&entries, 1500));
        let leaving = settings.document(&entries, 16001);
        assert_eq!(entering, leaving);
        assert!(
            settings.document(&entries, 16300).nodes[0]
                .children
                .is_empty()
        );
        let mut deleted = entries;
        deleted[0].deleted = true;
        assert!(
            settings.document(&deleted, 1100).nodes[0]
                .children
                .is_empty()
        );
        entering.validate().unwrap();
        settings.validate().unwrap();
    }
    #[test]
    fn legacy_sources_default_to_rtmp_and_chat_needs_no_ingest_credentials() {
        let mut workspace = crate::workspace::Workspace::default();
        workspace.add_source();
        let mut json = serde_json::to_value(&workspace.sources[0]).unwrap();
        json.as_object_mut().unwrap().remove("kind");
        let legacy: crate::workspace::Source = serde_json::from_value(json).unwrap();
        assert!(legacy.kind.is_rtmp());
        workspace.sources[0].kind = SourceKind::Chat(ChatSource::default());
        workspace.sources[0].url.clear();
        workspace.sources[0].key.clear();
        workspace.validate().unwrap();
    }
    #[test]
    fn channel_filters_and_message_limits_are_applied_before_rendering() {
        let mut settings = ChatSource {
            max_messages: 1,
            ..Default::default()
        };
        let mut second = entry(1001);
        second.message.message_id = "second".into();
        assert_eq!(
            settings.document(&[entry(1000), second], 1500).nodes[0]
                .children
                .len(),
            1
        );
        settings.channels = vec!["other:channel".into()];
        assert!(
            settings.document(&[entry(1000)], 1500).nodes[0]
                .children
                .is_empty()
        );
    }
}
