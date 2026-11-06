//! Application setup references an installation's public settings and local secret slots.
use crate::integration::GuestManifest;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationSetup {
    #[serde(default)]
    pub links: Vec<SetupLink>,
    /// IDs from this manifest's settings. The host supplies labels, limits and persistence.
    #[serde(default)]
    pub settings: Vec<String>,
    #[serde(default)]
    pub native: PlatformSetup,
    #[serde(default)]
    pub browser: PlatformSetup,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupLink {
    pub label: String,
    pub url: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformSetup {
    #[serde(default)]
    pub instructions: String,
    /// Absent for device authorization and other flows without a redirect.
    pub address: Option<SetupAddress>,
    #[serde(default)]
    pub secrets: Vec<SetupSecret>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupAddress {
    pub source: SetupAddressSource,
    pub label: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupAddressSource {
    CallbackUri,
    Origin,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupSecret {
    /// ID from this manifest's secret_fields. Values never become public settings.
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub required: bool,
}

impl ApplicationSetup {
    pub fn platform(&self, browser: bool) -> &PlatformSetup {
        if browser { &self.browser } else { &self.native }
    }

    pub(crate) fn validate(&self, manifest: &GuestManifest) -> Result<(), String> {
        if self.links.len() > 8 || self.settings.len() > 8 || self.native.secrets.len() > 8 {
            return Err("application setup exceeds field limit".into());
        }
        self.validate_settings(manifest)?;
        for link in &self.links {
            link.validate()?;
        }
        if !self.browser.secrets.is_empty() {
            return Err("browser application setup cannot request application secrets".into());
        }
        self.native.validate(manifest)?;
        self.browser.validate(manifest)
    }

    fn validate_settings(&self, manifest: &GuestManifest) -> Result<(), String> {
        let mut ids = BTreeSet::new();
        for id in &self.settings {
            let public = manifest
                .settings
                .iter()
                .any(|s| &s.id == id && !s.secret && !s.toggle);
            if !ids.insert(id) || !public {
                return Err("application setup must reference unique public setting IDs".into());
            }
        }
        Ok(())
    }
}

impl SetupLink {
    fn validate(&self) -> Result<(), String> {
        if self.label.is_empty() || self.label.len() > 128 || self.url.len() > 2048 {
            return Err("invalid application setup link label or length".into());
        }
        let url = url::Url::parse(&self.url).map_err(|_| "invalid application setup link")?;
        if url.scheme() != "https" || url.host_str().is_none() {
            return Err("application setup links require public HTTPS addresses".into());
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("application setup links cannot embed credentials".into());
        }
        Ok(())
    }
}

impl PlatformSetup {
    fn validate(&self, manifest: &GuestManifest) -> Result<(), String> {
        if self.instructions.len() > 4096 {
            return Err("application setup instructions exceed limit".into());
        }
        if let Some(address) = &self.address
            && (address.label.is_empty() || address.label.len() > 128)
        {
            return Err("invalid application address label".into());
        }
        let mut ids = BTreeSet::new();
        for secret in &self.secrets {
            if !ids.insert(&secret.id) || !manifest.secret_fields.contains_key(&secret.id) {
                return Err("application setup must reference unique local secret IDs".into());
            }
            if secret.label.is_empty() || secret.label.len() > 128 {
                return Err("invalid application secret label".into());
            }
        }
        Ok(())
    }
}

impl GuestManifest {
    pub(crate) fn validate_application_setup(&self) -> Result<(), String> {
        let Some(setup) = &self.application_setup else {
            return Ok(());
        };
        if self.execution_side == crate::integration::ExecutionSide::Server {
            return Err("application setup belongs to client extensions".into());
        }
        setup.validate(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn manifest() -> serde_json::Value {
        json!({"id":"independent","name":"Independent","api_version":2,"module":"guest.wasm","execution_side":"client","capabilities":["panel"],"services":[],"settings":[{"id":"public_application_key","label":"Public application"}],"secret_fields":{"local_key":"/local_key"},"application_setup":{"links":[],"settings":["public_application_key"],"native":{"secrets":[{"id":"local_key","label":"Local key","required":true}]},"browser":{}}})
    }
    #[test]
    fn application_requisites_cannot_be_supplied_as_hidden_defaults() {
        let mut value = manifest();
        value["application_setup"]["native"]["defaults"] =
            json!({"public_application_key":"shared-app"});
        assert!(serde_json::from_value::<GuestManifest>(value).is_err());
    }
    #[test]
    fn setup_resolves_only_its_own_declared_ids_without_provider_names() {
        let original = manifest();
        let parsed: GuestManifest = serde_json::from_value(original.clone()).unwrap();
        assert!(parsed.validate().is_ok());
        for (path, value) in [
            ("/application_setup/settings", json!(["undeclared"])),
            (
                "/application_setup/settings",
                json!(["public_application_key", "public_application_key"]),
            ),
            (
                "/application_setup/native/secrets/0/id",
                json!("undeclared"),
            ),
            (
                "/application_setup/browser",
                json!({"secrets":[{"id":"local_key","label":"Forbidden"}]}),
            ),
            (
                "/application_setup/links",
                json!([{"label":"Unsafe","url":"javascript:alert(1)"}]),
            ),
            ("/execution_side", json!("server")),
        ] {
            let mut invalid = original.clone();
            *invalid.pointer_mut(path).unwrap() = value;
            assert!(
                serde_json::from_value::<GuestManifest>(invalid)
                    .unwrap()
                    .validate()
                    .is_err(),
                "{path}"
            );
        }
    }
}
