//! Public package review: declarations and hashes, never account credentials.
use crate::parts::{ExecutionHost, PartKind};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageReview {
    pub id: String,
    pub name: String,
    pub digest: String,
    pub host: ExecutionHost,
    pub kind: PartKind,
    pub capabilities: Vec<String>,
    pub supported_actions: Vec<String>,
    pub services: Vec<String>,
    pub settings: Vec<crate::integration::Setting>,
    pub requires_authorization: bool,
    pub replaces: Option<String>,
    pub execution_allowed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallPackage {
    pub digest: String,
    pub expected_current: Option<String>,
}
