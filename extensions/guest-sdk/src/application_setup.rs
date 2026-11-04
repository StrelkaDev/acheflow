//! Shared setup types bind controls to each installation's declared IDs.
use acheflow_extension_api::application_setup::{ApplicationSetup, SetupAddressSource};
use serde_json::{Value, json};

fn declaration(manifest: &Value) -> Option<ApplicationSetup> {
    serde_json::from_value(manifest["application_setup"].clone()).ok()
}

pub(super) fn widgets(manifest: &Value, input: &Value) -> Vec<Value> {
    let mut widgets = vec![
        json!({"type":"label","text":manifest["authentication"]["help"].as_str().unwrap_or_default()}),
    ];
    let Some(setup) = declaration(manifest) else {
        return widgets;
    };
    let browser = input["environment"]["browser"] == true;
    let platform = setup.platform(browser);
    widgets.extend(
        setup
            .links
            .iter()
            .map(|link| json!({"type":"link","label":link.label,"url":link.url})),
    );
    widgets.push(json!({"type":"label","text":platform.instructions}));
    if let Some(address) = &platform.address {
        let source = match address.source {
            SetupAddressSource::CallbackUri => "callback_uri",
            SetupAddressSource::Origin => "origin",
        };
        let value = input["environment"][source].as_str().unwrap_or_default();
        if value.is_empty() {
            widgets.push(json!({"type":"label","text":"Application address is unavailable in this environment. Open the client from its installed or hosted location."}));
        } else {
            widgets.push(json!({"type":"copy_text","label":address.label,"value":value}));
        }
    }
    widgets.extend(
        setup
            .settings
            .iter()
            .map(|id| json!({"type":"setting_input","id":id})),
    );
    if !browser {
        widgets.extend(
            platform
                .secrets
                .iter()
                .map(|field| json!({"type":"secret_input","id":field.id,"label":field.label})),
        );
    }
    widgets
}

pub(super) fn missing_secret(manifest: &Value, input: &Value) -> Option<String> {
    if input["environment"]["browser"] == true {
        return None;
    }
    declaration(manifest)?
        .native
        .secrets
        .iter()
        .find_map(|field| {
            let present = input["secrets"]
                .as_array()
                .is_some_and(|keys| keys.iter().any(|key| key.as_str() == Some(&field.id)));
            (field.required && !present).then(|| {
                format!(
                    "Enter {} in Application setup before connecting.",
                    field.label
                )
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn device_setup_binds_arbitrary_setting_ids_without_inventing_a_callback() {
        let manifest = json!({"application_setup":{"settings":["custom_public_key"],"native":{"instructions":"Use the displayed device code."},"browser":{}}});
        let native = widgets(&manifest, &json!({"environment":{"callback_uri":"unused"}}));
        assert!(native.iter().all(|w| w["type"] != "copy_text"));
        assert!(
            native
                .iter()
                .any(|w| w["type"] == "setting_input" && w["id"] == "custom_public_key")
        );
        let browser = json!({"environment":{"browser":true}});
        assert!(
            widgets(&manifest, &browser)
                .iter()
                .all(|w| w["type"] != "secret_input")
        );
        assert!(missing_secret(&manifest, &browser).is_none());
    }
}
