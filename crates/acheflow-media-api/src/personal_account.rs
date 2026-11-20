//! Read/delete-only metadata for retiring old server-side personal accounts.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalAccount {
    pub provider: String,
    pub account_id: String,
    pub version: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonalAccounts {
    pub accounts: Vec<PersonalAccount>,
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
