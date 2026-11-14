//! Public server execution configuration. Credentials never belong in these DTOs.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerIntegration {
    pub enabled: bool,
    pub application: String,
    pub account_id: String,
    pub channel_id: String,
    #[serde(default)]
    pub config: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ServerProviderStatus {
    pub provider: String,
    pub name: String,
    pub state: String,
    pub account_id: Option<String>,
    pub channel_id: Option<String>,
    pub generation: u64,
    #[serde(default)]
    pub server_owned: bool,
}

// Intentionally no Debug: credentials are submitted only to an explicitly trusted server.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectServerAccount {
    pub workspace_id: String,
    pub expected_revision: u64,
    pub trust_server: bool,
    pub expected_account: String,
    pub channel_id: String,
    pub config: BTreeMap<String, String>,
    pub credentials: BTreeMap<String, String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisconnectServerAccount {
    pub workspace_id: String,
    pub expected_revision: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerAccountConnected {
    pub revision: u64,
    pub binding: ServerIntegration,
}
