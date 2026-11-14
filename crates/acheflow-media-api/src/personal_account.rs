//! User-owned account metadata. Credentials are accepted only by explicit connection.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalAccount {
    pub provider: String,
    pub account_id: String,
    pub version: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonalAccounts {
    #[serde(default)]
    pub send_epoch: String,
    #[serde(default)]
    pub server_time_ms: u64,
    pub accounts: Vec<PersonalAccount>,
}
// No Debug: contains credentials, never a workspace field or control command.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectPersonalAccount {
    pub trust_server: bool,
    pub expected_account: String,
    pub expected_version: Option<String>,
    pub config: BTreeMap<String, String>,
    pub credentials: BTreeMap<String, String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisconnectPersonalAccount {
    pub expected_version: String,
}

#[derive(Serialize, Deserialize)]
pub struct PersonalAccountDisconnected {
    pub credentials_removed: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendPersonalChat {
    pub request_id: String,
    pub epoch: String,
    pub issued_at_ms: u64,
    pub expected_version: String,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonalDelivery {
    pub outcome: acheflow_extension_api::client_execution::ExecutionOutcome,
}
