//! Shared host-side renderer for the draft extension panel schema.
use acheflow_extension_api::{ExtensionPanel, PanelWidget};
use dioxus_native::prelude::*;

pub fn render(panel: &ExtensionPanel) -> Element {
    rsx! {
        section { class: "panel", key: "{panel.extension_id}",
            div { class: "panel-heading", h2 { "{panel.title}" } }
            div { class: "extension-content",
                for widget in &panel.widgets {
                    {match widget {
                        PanelWidget::Label { text } => rsx! { p { "{text}" } },
                        PanelWidget::Metric { label, value } => rsx! {
                            div { class: "extension-metric", span { "{label}" } strong { "{value}" } }
                        },
                    }}
                }
            }
        }
    }
}
