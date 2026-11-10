use acheflow_client::{catalog::ClientCatalog, local::LocalSession};
use std::collections::BTreeMap;

#[test]
fn desktop_youtube_requires_its_local_application_secret_and_uses_pkce() {
    use serde_json::json;
    let catalog = ClientCatalog::bundled();
    let installed = &catalog.installed["youtube"];
    let mut session = LocalSession::new(
        installed.manifest.clone(),
        &installed.module,
        BTreeMap::from([
            ("client_id".into(), "desktop-public-id".into()),
            ("send_chat".into(), "false".into()),
        ]),
    )
    .unwrap();
    let environment =
        json!({"browser":false,"callback_uri":"http://127.0.0.1:51234/oauth/callback"});
    let missing_secret = session
        .step(
            json!({"type":"local_action","action_id":"connect"}),
            &environment,
            1000,
        )
        .unwrap();
    assert!(!missing_secret.iter().any(|e| e["type"] == "browser"));
    assert!(
        serde_json::to_string(&missing_secret)
            .unwrap()
            .contains("Application secret")
    );
    session
        .set_secret("client_secret", "desktop-local-secret".into())
        .unwrap();
    let effects = session
        .step(
            json!({"type":"local_action","action_id":"connect"}),
            &environment,
            1001,
        )
        .unwrap();
    let browser = effects
        .iter()
        .find(|e| e["type"] == "browser")
        .expect("Desktop PKCE starts after required application setup");
    let url = url::Url::parse(browser["url"].as_str().unwrap()).unwrap();
    let fields: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(fields["response_type"], "code");
    assert_eq!(fields["code_challenge_method"], "S256");
    assert_eq!(fields["code_challenge"].len(), 43);
    assert_eq!(
        fields["scope"],
        "https://www.googleapis.com/auth/youtube.readonly"
    );
    let effects = session
        .step(
            json!({"type":"local_browser","payload":{"state":fields["state"],"code":"test-code"}}),
            &environment,
            1001,
        )
        .unwrap();
    let exchange = effects.iter().find(|e| e["id"] == "auth:token").unwrap();
    assert_eq!(exchange["form"]["client_id"], "desktop-public-id");
    assert_eq!(
        exchange["form"]["redirect_uri"],
        environment["callback_uri"]
    );
    assert_eq!(exchange["form"]["code_verifier"]["secret"], "code_verifier");
    assert_eq!(exchange["form"]["client_secret"]["secret"], "client_secret");
    assert!(
        !serde_json::to_string(&exchange)
            .unwrap()
            .contains("desktop-local-secret")
    );
}

#[test]
fn provider_setup_uses_active_platform_addresses_and_only_required_local_secrets() {
    use acheflow_extension_api::ExtensionPanel;
    use serde_json::json;
    let catalog = ClientCatalog::bundled();
    for provider in ["twitch", "youtube", "vklive"] {
        let installed = &catalog.installed[provider];
        assert!(
            installed
                .manifest
                .settings
                .iter()
                .any(|s| s.id == "client_id" && !s.secret)
        );
        for browser in [false, true] {
            let callback = if browser {
                "https://app.example/base/oauth-callback.html".to_owned()
            } else if provider == "youtube" {
                "http://127.0.0.1:51234/oauth/callback".to_owned()
            } else {
                format!("https://app.acheflow.com/fwd/{provider}/oauth/callback")
            };
            let environment =
                json!({"browser":browser,"callback_uri":callback,"origin":"https://app.example"});
            let mut session = LocalSession::new(
                installed.manifest.clone(),
                &installed.module,
                BTreeMap::new(),
            )
            .unwrap();
            let effects = session
                .step(json!({"type":"local_start"}), &environment, 1000)
                .unwrap();
            let panel = &effects.iter().find(|e| e["type"] == "panel").unwrap()["panel"];
            let _: ExtensionPanel =
                serde_json::from_value(panel.clone()).expect("host can render guest declarations");
            let setup = panel["widgets"]
                .as_array()
                .unwrap()
                .iter()
                .find(|w| w["label"] == "Application setup instructions")
                .unwrap();
            let widgets = setup["widgets"].as_array().unwrap();
            let visible = panel["widgets"].as_array().unwrap();
            let credentials = visible
                .iter()
                .find(|w| w["label"] == "Application credentials")
                .expect("application credentials can be collapsed independently");
            assert_eq!(credentials["type"], "disclosure");
            let credential_fields = credentials["widgets"].as_array().unwrap();
            assert!(
                credential_fields
                    .iter()
                    .any(|w| w["type"] == "setting_input" && w["id"] == "client_id")
            );
            let connect = visible
                .iter()
                .filter_map(|w| w["widgets"].as_array())
                .flatten()
                .find(|w| w["id"] == "connect")
                .unwrap();
            assert_eq!(
                connect["requires"],
                if !browser && provider == "youtube" {
                    json!(["client_id", "client_secret"])
                } else {
                    json!(["client_id"])
                }
            );
            assert!(
                widgets
                    .iter()
                    .any(|w| w["type"] == "link"
                        && w["url"].as_str().unwrap().starts_with("https://")),
                "{provider}: developer-console links missing"
            );
            let address = widgets
                .iter()
                .find(|w| w["type"] == "copy_text")
                .expect("exact address with copy action");
            assert_eq!(
                address["value"],
                if browser && provider == "youtube" {
                    "https://app.example"
                } else {
                    callback.as_str()
                }
            );
            assert_eq!(
                credential_fields
                    .iter()
                    .any(|w| w["type"] == "secret_input" && w["id"] == "client_secret"),
                !browser && provider == "youtube"
            );
            let failure = session
                .step(
                    json!({"type":"local_action","action_id":"connect"}),
                    &environment,
                    1001,
                )
                .unwrap();
            assert!(!failure.iter().any(|e| matches!(
                e["type"].as_str(),
                Some("browser" | "browser_module" | "http")
            )));
            assert!(
                serde_json::to_string(&failure)
                    .unwrap()
                    .contains("public Client ID")
            );
            session
                .config
                .insert("client_id".into(), "public-id".into());
            if !browser && provider == "youtube" {
                session
                    .set_secret("client_secret", "local-only-marker".into())
                    .unwrap();
            }
            let effects = session
                .step(
                    json!({"type":"local_action","action_id":"connect"}),
                    &environment,
                    1003,
                )
                .unwrap();
            if browser && provider == "youtube" {
                assert!(effects.iter().any(|e| e["type"] == "browser_module"));
            } else {
                let request = effects.iter().find(|e| e["type"] == "browser").unwrap();
                let url = url::Url::parse(request["url"].as_str().unwrap()).unwrap();
                assert_eq!(
                    url.query_pairs()
                        .find(|(k, _)| k == "redirect_uri")
                        .unwrap()
                        .1,
                    callback
                );
            }
            assert!(
                !serde_json::to_string(&effects)
                    .unwrap()
                    .contains("local-only-marker")
            );
        }
    }
}

#[test]
fn real_wasm_guests_respect_disabled_write_functions_and_read_only_tokens() {
    use acheflow_control::extensions::Guest;
    use serde_json::json;
    let catalog = ClientCatalog::bundled();
    for provider in ["twitch", "youtube"] {
        let installed = &catalog.installed[provider];
        let mut guest =
            Guest::load(&installed.module, installed.manifest.capabilities.clone()).unwrap();
        let config = json!({"client_id":"public-id","send_chat":"false","manage_polls":"false","read_audience":"false","read_chat":"true","video_id":"video"});
        guest
            .call(json!({"type":"start","config":config,"secrets":["access_token"],"now_ms":1000}))
            .unwrap();
        if provider == "twitch" {
            let effects=guest.call(json!({"type":"http_result","id":"validate","status":200,"payload":{"client_id":"public-id","user_id":"user","login":"login","scopes":["user:read:chat"],"expires_in":3600},"config":config,"now_ms":1000})).unwrap();
            assert!(
                effects.iter().any(|e| e["type"] == "channel"),
                "read-only token must authorize"
            );
            assert!(!effects.iter().any(|e| e["state"] == "needs_authorization"));
        }
        let effects=guest.call(json!({"type":"command","name":"chat.send","request_id":"test","channel_id":"video","text":"hello","config":config,"now_ms":1000})).unwrap();
        assert!(
            !effects.iter().any(|e| e["type"] == "http"),
            "disabled chat sends must not reach a provider"
        );
        let tick = guest
            .call(json!({"type":"tick","config":config,"secrets":["access_token"],"now_ms":2000}))
            .unwrap();
        assert!(
            !tick
                .iter()
                .any(|e| e["id"] == "viewers" || e["id"] == "refresh"),
            "no audience request or refresh attempt without a refresh token"
        );
    }
}

#[test]
fn local_panels_and_scopes_are_emitted_by_two_independent_wasm_guests() {
    use serde_json::json;
    let catalog = ClientCatalog::bundled();
    assert!(catalog.errors.is_empty(), "{:?}", catalog.errors);
    for provider in ["twitch", "youtube", "vklive"] {
        let installed = &catalog.installed[provider];
        let mut session = LocalSession::new(
            installed.manifest.clone(),
            &installed.module,
            BTreeMap::from([
                ("client_id".into(), "public-id".into()),
                ("send_chat".into(), "false".into()),
                ("manage_polls".into(), "false".into()),
            ]),
        )
        .unwrap();
        let environment =
            json!({"browser":false,"callback_uri":"http://127.0.0.1:21941/oauth/callback"});
        let panels = session
            .step(json!({"type":"local_start"}), &environment, 1000)
            .unwrap();
        assert!(panels.iter().any(|e| e["type"] == "panel"));
        if provider == "youtube" {
            session
                .set_secret("client_secret", "fixture-local-secret".into())
                .unwrap();
        }
        let effects = session
            .step(
                json!({"type":"local_action","action_id":"connect"}),
                &environment,
                1000,
            )
            .unwrap();
        let browser = effects
            .iter()
            .find(|e| e["type"] == "browser")
            .expect("guest owns browser request");
        let url = url::Url::parse(browser["url"].as_str().unwrap()).unwrap();
        let scopes = url
            .query_pairs()
            .find(|(k, _)| k == "scope")
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default();
        match provider {
            "twitch" => assert_eq!(scopes, "user:read:chat"),
            "youtube" => assert_eq!(scopes, "https://www.googleapis.com/auth/youtube.readonly"),
            _ => assert!(scopes.is_empty()),
        }
        let effects = session
            .step(
                json!({"type":"local_browser","payload":{"state":"wrong","code":null}}),
                &environment,
                1000,
            )
            .unwrap();
        assert!(
            !effects.iter().any(|e| e["type"] == "http"),
            "state mismatch cannot exchange credentials"
        );
    }
}
