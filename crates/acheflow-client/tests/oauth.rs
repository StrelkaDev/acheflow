use acheflow_client::{catalog::ClientCatalog, local::LocalSession};
use std::collections::BTreeMap;

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
