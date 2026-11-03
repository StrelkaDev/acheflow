use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UniqueViewers {
    pub value: u64,
    pub period_start_ms: u64,
    pub period_end_ms: u64,
    pub scope: String,
    pub provisional: bool,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudienceSample {
    pub platform: String,
    pub channel_id: String,
    pub stream_id: Option<String>,
    pub observed_at_ms: u64,
    pub concurrent: Option<u64>,
    pub offline: bool,
    pub unique: Option<UniqueViewers>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelAudience {
    pub sample: AudienceSample,
    pub current: Option<u64>,
    pub stale: bool,
    pub unique: Option<UniqueViewers>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AudienceSnapshot {
    pub total: Option<u64>,
    pub partial: bool,
    pub overflow: bool,
    pub peak: Option<u64>,
    pub channels: Vec<ChannelAudience>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudiencePoint {
    pub at_ms: u64,
    pub total: Option<u64>,
    pub partial: bool,
}
