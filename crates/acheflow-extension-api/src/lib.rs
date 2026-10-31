//! Draft host-rendered UI contract shared by server, native GUI and browser GUI.
//! These types do not load or execute WASM modules. The plugin ABI is not stable yet.

use serde::{Deserialize, Serialize};

pub const API_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExtensionManifest {
    pub id: String,
    pub name: String,
    pub api_version: u32,
    pub capabilities: Vec<Capability>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    ServerLogic,
    GuiPanel,
}

/// Declarative content: the host owns rendering on both native and WASM targets.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExtensionPanel {
    pub extension_id: String,
    pub title: String,
    pub widgets: Vec<PanelWidget>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PanelWidget {
    Label { text: String },
    Metric { label: String, value: String },
}
