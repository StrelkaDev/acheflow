//! Versioned provider-neutral contracts shared by guests, server and both UIs.
//! These types do not load or execute WASM modules. The plugin ABI is not stable yet.

use serde::{Deserialize, Serialize};

pub mod audience;
pub mod client_execution;
pub mod forwarding;
pub mod integration;
pub mod interaction;

pub const API_VERSION: u32 = 2;

/// Declarative content: the host owns rendering on both native and WASM targets.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(try_from = "PanelDocument")]
pub struct ExtensionPanel {
    pub extension_id: String,
    pub title: String,
    pub widgets: Vec<PanelWidget>,
}
#[derive(Deserialize)]
struct PanelDocument {
    extension_id: String,
    title: String,
    widgets: Vec<PanelWidget>,
}
impl TryFrom<PanelDocument> for ExtensionPanel {
    type Error = String;
    fn try_from(document: PanelDocument) -> Result<Self, String> {
        fn bounded(widgets: &[PanelWidget], depth: usize, remaining: &mut usize) -> bool {
            if depth > 8 {
                return false;
            }
            for widget in widgets {
                let Some(next) = remaining.checked_sub(1) else {
                    return false;
                };
                *remaining = next;
                if let PanelWidget::Row { widgets } | PanelWidget::Disclosure { widgets, .. } =
                    widget
                    && !bounded(widgets, depth + 1, remaining)
                {
                    return false;
                }
            }
            true
        }
        if !bounded(&document.widgets, 0, &mut 64) {
            return Err("extension panel exceeds widget or nesting limit".into());
        }
        Ok(Self {
            extension_id: document.extension_id,
            title: document.title,
            widgets: document.widgets,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PanelWidget {
    Row {
        widgets: Vec<PanelWidget>,
    },
    Disclosure {
        label: String,
        widgets: Vec<PanelWidget>,
    },
    SecretInput {
        id: String,
        label: String,
    },
    Label {
        text: String,
    },
    Metric {
        label: String,
        value: String,
    },
    Button {
        id: String,
        label: String,
        #[serde(default)]
        primary: bool,
    },
    TextInput {
        id: String,
        label: String,
        value: String,
    },
    Link {
        label: String,
        url: String,
    },
    Select {
        id: String,
        label: String,
        value: String,
        options: Vec<String>,
    },
}

#[cfg(test)]
mod panel_layout_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn nested_layouts_keep_the_existing_total_widget_limit() {
        let labels = vec![json!({"type":"label","text":"bounded"}); 64];
        assert!(
            serde_json::from_value::<ExtensionPanel>(
                json!({"extension_id":"fixture","title":"Panel","widgets":labels})
            )
            .is_ok()
        );
        assert!(serde_json::from_value::<ExtensionPanel>(json!({"extension_id":"fixture","title":"Panel","widgets":[{"type":"row","widgets":labels}]})).is_err());
        let mut nested = json!({"type":"label","text":"bounded"});
        for _ in 0..10 {
            nested = json!({"type":"disclosure","label":"Details","widgets":[nested]});
        }
        assert!(
            serde_json::from_value::<ExtensionPanel>(
                json!({"extension_id":"fixture","title":"Panel","widgets":[nested]})
            )
            .is_err()
        );
    }
    #[test]
    fn existing_buttons_remain_secondary_without_new_metadata() {
        let widget: PanelWidget =
            serde_json::from_value(json!({"type":"button","id":"run","label":"Run"})).unwrap();
        assert!(matches!(widget, PanelWidget::Button { primary: false, .. }));
    }
}
