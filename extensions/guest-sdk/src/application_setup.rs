//! Shared setup types bind controls to each installation's declared IDs.
use acheflow_extension_api::application_setup::{ApplicationSetup, SetupAddressSource};
use serde_json::{Value, json};

fn declaration(manifest: &Value) -> Option<ApplicationSetup> {
    serde_json::from_value(manifest["application_setup"].clone()).ok()
}

pub(super) fn callback_uri(manifest: &Value, input: &Value) -> Result<String, String> {
    if input["environment"]["browser"] != true
        && let Some(suffix) = manifest["authentication"]["native_callback"].as_str()
    {
        use acheflow_extension_api::forwarding::{ForwardLink, ForwardRecipient};
        let base = input["environment"]["link"]
            .as_str()
            .ok_or("Application link unavailable.")?;
        let address = format!("{base}/{suffix}");
        let parsed = ForwardLink::parse_for(&address, ForwardRecipient::Client)
            .map_err(|_| "Invalid application callback suffix.")?;
        if parsed.extension_id != manifest["id"].as_str().unwrap_or_default()
            || parsed.query.is_some()
            || parsed.fragment.is_some()
            || !manifest["forwarding"].as_array().is_some_and(|actions| {
                actions
                    .iter()
                    .any(|a| a["id"] == parsed.action && a["mode"] == "callback")
            })
        {
            return Err(
                "Application callback must belong to this extension's declared action.".into(),
            );
        }
        return Ok(address);
    }
    input["environment"]["callback_uri"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "Local callback unavailable.".into())
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
        let resolved = if source == "callback_uri" {
            callback_uri(manifest, input).unwrap_or_default()
        } else {
            input["environment"][source]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        };
        let value = resolved.as_str();
        if value.is_empty() {
            let message = if !browser && source == "callback_uri" {
                "The active callback address is allocated when sign-in starts."
            } else {
                "Application address is unavailable in this environment. Open the client from its installed or hosted location."
            };
            widgets.push(json!({"type":"label","text":message}));
        } else {
            widgets.push(json!({"type":"copy_text","label":address.label,"value":value}));
        }
    }
    widgets
}

pub(super) fn fields(manifest: &Value, input: &Value) -> Vec<Value> {
    let Some(setup) = declaration(manifest) else {
        return vec![];
    };
    let browser = input["environment"]["browser"] == true;
    let mut widgets: Vec<Value> = setup
        .settings
        .iter()
        .map(|id| json!({"type":"setting_input","id":id}))
        .collect();
    if !browser {
        widgets.extend(
            setup
                .native
                .secrets
                .iter()
                .map(|field| json!({"type":"secret_input","id":field.id,"label":field.label})),
        );
    }
    widgets
}

pub(super) fn required_fields(manifest: &Value, input: &Value) -> Vec<String> {
    let Some(setup) = declaration(manifest) else {
        return vec!["client_id".into()];
    };
    let mut ids = setup.settings.clone();
    if input["environment"]["browser"] != true {
        ids.extend(
            setup
                .native
                .secrets
                .iter()
                .filter(|s| s.required)
                .map(|s| s.id.clone()),
        );
    }
    ids
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
    fn callback_suffix_cannot_change_extension_action_or_embed_response_parameters() {
        let input = json!({"environment":{"link":"https://app.acheflow.com/fwd/independent"}});
        for suffix in [
            "../other/oauth/callback",
            "oauth/callback?state=injected",
            "oauth/callback#token=injected",
            "undeclared/callback",
            "https://evil.example/callback",
        ] {
            let manifest = json!({"id":"independent","authentication":{"native_callback":suffix},"forwarding":[{"id":"oauth","mode":"callback"}]});
            assert!(callback_uri(&manifest, &input).is_err(), "{suffix}");
        }
    }
    #[test]
    fn device_setup_binds_arbitrary_setting_ids_without_inventing_a_callback() {
        let manifest = json!({"application_setup":{"settings":["custom_public_key"],"native":{"instructions":"Use the displayed device code."},"browser":{}}});
        let input = json!({"environment":{"callback_uri":"unused"}});
        let native = fields(&manifest, &input);
        assert!(
            widgets(&manifest, &input)
                .iter()
                .all(|w| w["type"] != "copy_text")
        );
        assert!(
            native
                .iter()
                .any(|w| w["type"] == "setting_input" && w["id"] == "custom_public_key")
        );
        let browser = json!({"environment":{"browser":true}});
        assert!(
            fields(&manifest, &browser)
                .iter()
                .all(|w| w["type"] != "secret_input")
        );
        assert!(missing_secret(&manifest, &browser).is_none());
        assert_eq!(required_fields(&manifest, &browser), ["custom_public_key"]);
    }
}
