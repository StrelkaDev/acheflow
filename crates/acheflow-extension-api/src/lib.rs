//! Versioned provider-neutral contracts shared by guests, server and both UIs.
//! These types do not load or execute WASM modules. The plugin ABI is not stable yet.

use serde::{Deserialize, Serialize};

pub mod audience;
pub mod integration;
pub mod interaction;

pub const API_VERSION: u32 = 2;

/// Declarative content: the host owns rendering on both native and WASM targets.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ExtensionPanel {
    pub extension_id: String,
    pub title: String,
    pub widgets: Vec<PanelWidget>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PanelWidget {
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
