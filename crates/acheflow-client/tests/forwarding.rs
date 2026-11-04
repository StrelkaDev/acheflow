//! Forwarding reaches real provider guests only within the pending local lifetime.
use acheflow_client::{catalog::ClientCatalog, local::LocalSession};
use acheflow_extension_api::forwarding::{ForwardAction, ForwardMode};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const WORKSPACE: &str = "forwarding-workspace";
const START: u64 = 1_000;
const CODE: &str = "forwarded-authorization-code-marker";
const TOKEN: &str = "forwarded-access-token-marker";

struct Attempt {
    session: LocalSession,
    environment: Value,
    instance: String,
    state: String,
}

impl Attempt {
    fn new(provider: &str, connect: bool) -> Self {
        let mut result = Self::unbound(provider);
        result.session.bind_workspace(WORKSPACE).unwrap();
        result
            .session
            .step(json!({"type":"local_start"}), &result.environment, START)
            .unwrap();
        result.instance = result.session.instance_id().to_owned();
        if connect {
            result.connect(START);
        }
        result
    }

    fn unbound(provider: &str) -> Self {
        let catalog = ClientCatalog::bundled();
        assert!(catalog.errors.is_empty(), "{:?}", catalog.errors);
        let installed = &catalog.installed[provider];
        let session = LocalSession::new(
            installed.manifest.clone(),
            &installed.module,
            BTreeMap::from([
                ("client_id".into(), "public-id".into()),
                ("read_chat".into(), "true".into()),
                ("read_audience".into(), "false".into()),
                ("send_chat".into(), "false".into()),
                ("manage_polls".into(), "false".into()),
            ]),
        )
        .unwrap();
        let environment = json!({
            "browser": false,
            "callback_uri": "http://127.0.0.1:21941/oauth/callback"
        });
        let instance = session.instance_id().to_owned();
        assert!(
            !instance.is_empty(),
            "a constructed guest needs an instance identity"
        );
        Self {
            session,
            environment,
            instance,
            state: String::new(),
        }
    }

    fn connect(&mut self, now: u64) {
        let effects = self
            .session
            .step(
                json!({"type":"local_action","action_id":"connect"}),
                &self.environment,
                now,
            )
            .unwrap();
        let browser = effects
            .iter()
            .find(|effect| effect["type"] == "browser")
            .expect("the real provider guest must request browser authorization");
        let url = url::Url::parse(browser["url"].as_str().unwrap()).unwrap();
        self.state = url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| value.into_owned())
            .expect("the real guest must bind authorization to its generated state");
        assert!(self.state.len() >= 32);
    }

    fn deliver(&mut self, address: &str, now: u64) -> Result<Vec<Value>, String> {
        self.session
            .forward(WORKSPACE, &self.instance, address, &self.environment, now)
    }
}

fn callback(provider: &str, state: &str) -> String {
    let mut fields = url::form_urlencoded::Serializer::new(String::new());
    fields.append_pair("state", state);
    let separator = if provider == "youtube" {
        fields.append_pair("code", CODE);
        '?'
    } else {
        fields
            .append_pair("access_token", TOKEN)
            .append_pair("expires_in", "3600")
            .append_pair("token_type", "bearer");
        '#'
    };
    format!(
        "https://client.acheflow.com/fwd/{provider}/oauth/callback{separator}{}",
        fields.finish()
    )
}

fn assert_provider_request(provider: &str, effects: &[Value]) {
    let id = if provider == "youtube" {
        "auth:token"
    } else {
        "auth:validate"
    };
    let effect = effects
        .iter()
        .find(|effect| effect["type"] == "http" && effect["id"] == id)
        .unwrap_or_else(|| panic!("forwarded callback did not reach {provider}'s real guest"));
    let serialized = serde_json::to_string(effects).unwrap();
    assert!(
        !serialized.contains(CODE),
        "authorization code escaped capture"
    );
    assert!(!serialized.contains(TOKEN), "access token escaped capture");
    let handle = if provider == "youtube" {
        "authorization_code"
    } else {
        "access_token"
    };
    assert!(
        serde_json::to_string(effect).unwrap().contains(handle),
        "the provider request must use an opaque credential handle"
    );
}

#[test]
fn two_real_providers_receive_only_their_own_pending_callback_with_redacted_credentials() {
    let mut twitch = Attempt::new("twitch", true);
    let mut youtube = Attempt::new("youtube", true);
    let twitch_address = callback("twitch", &twitch.state);
    let youtube_address = callback("youtube", &youtube.state);

    assert!(twitch.deliver(&youtube_address, START + 1).is_err());
    assert!(youtube.deliver(&twitch_address, START + 1).is_err());
    let twitch_effects = twitch.deliver(&twitch_address, START + 2).unwrap();
    let youtube_effects = youtube.deliver(&youtube_address, START + 2).unwrap();

    assert_provider_request("twitch", &twitch_effects);
    assert_provider_request("youtube", &youtube_effects);
    assert!(twitch.deliver(&twitch_address, START + 3).is_err());
    assert!(youtube.deliver(&youtube_address, START + 3).is_err());
}

#[test]
fn callbacks_need_a_pending_request_and_the_declared_action() {
    for provider in ["twitch", "youtube"] {
        let mut attempt = Attempt::new(provider, false);
        let unsolicited = callback(provider, "unsolicited-state");
        assert!(attempt.deliver(&unsolicited, START + 1).is_err());

        attempt.connect(START + 2);
        let valid = callback(provider, &attempt.state);
        let undeclared = valid.replacen("/oauth/", "/undeclared/", 1);
        assert!(attempt.deliver(&undeclared, START + 3).is_err());
        assert_provider_request(provider, &attempt.deliver(&valid, START + 4).unwrap());
    }
}

#[test]
fn recipient_workspace_and_instance_mismatches_leave_the_correct_request_pending() {
    for provider in ["twitch", "youtube"] {
        let mut attempt = Attempt::new(provider, true);
        let valid = callback(provider, &attempt.state);
        let server = valid.replacen("client.acheflow.com", "server.acheflow.com", 1);
        assert!(attempt.deliver(&server, START + 1).is_err());
        assert!(
            attempt
                .session
                .forward(
                    "another-workspace",
                    &attempt.instance,
                    &valid,
                    &attempt.environment,
                    START + 1,
                )
                .is_err()
        );
        assert!(
            attempt
                .session
                .forward(
                    WORKSPACE,
                    "another-instance",
                    &valid,
                    &attempt.environment,
                    START + 1,
                )
                .is_err()
        );
        assert_provider_request(provider, &attempt.deliver(&valid, START + 2).unwrap());
    }
}

#[test]
fn invalid_state_cannot_consume_the_legitimate_pending_callback() {
    for provider in ["twitch", "youtube"] {
        let mut attempt = Attempt::new(provider, true);
        let wrong = callback(provider, "attacker-selected-state");
        assert!(attempt.deliver(&wrong, START + 1).is_err());

        let valid = callback(provider, &attempt.state);
        assert_provider_request(provider, &attempt.deliver(&valid, START + 2).unwrap());
    }
}

#[test]
fn duplicate_parameters_cannot_choose_a_different_state_or_credential_value() {
    let mut attempt = Attempt::new("youtube", true);
    let prefix = "https://client.acheflow.com/fwd/youtube/oauth/callback";
    let state = &attempt.state;
    let ambiguous = [
        format!("{prefix}?state={state}&state={state}&code={CODE}"),
        format!("{prefix}?state={state}&code={CODE}#state={state}"),
        format!("{prefix}?state={state}&code=first&code=second"),
    ];
    for address in ambiguous {
        assert!(attempt.deliver(&address, START + 1).is_err());
    }

    let valid = callback("youtube", &attempt.state);
    assert_provider_request("youtube", &attempt.deliver(&valid, START + 2).unwrap());
}

#[test]
fn expired_and_stopped_instances_reject_late_callbacks() {
    for provider in ["twitch", "youtube"] {
        let mut expired = Attempt::new(provider, true);
        let address = callback(provider, &expired.state);
        assert!(expired.deliver(&address, START + 300_001).is_err());

        let mut stopped = Attempt::new(provider, true);
        let address = callback(provider, &stopped.state);
        stopped.session.stop();
        assert!(stopped.deliver(&address, START + 1).is_err());
    }
}

#[test]
fn restarting_rotates_instance_and_rejects_previous_authorization_state() {
    for provider in ["twitch", "youtube"] {
        let mut attempt = Attempt::new(provider, true);
        let old_instance = attempt.instance.clone();
        let old_address = callback(provider, &attempt.state);
        attempt
            .session
            .step(
                json!({"type":"local_start"}),
                &attempt.environment,
                START + 10,
            )
            .unwrap();
        attempt.instance = attempt.session.instance_id().to_owned();
        assert_ne!(attempt.instance, old_instance);
        attempt.connect(START + 11);

        assert!(
            attempt
                .session
                .forward(
                    WORKSPACE,
                    &old_instance,
                    &old_address,
                    &attempt.environment,
                    START + 12,
                )
                .is_err()
        );
        assert!(attempt.deliver(&old_address, START + 12).is_err());
        let valid = callback(provider, &attempt.state);
        assert_provider_request(provider, &attempt.deliver(&valid, START + 13).unwrap());
    }
}

#[test]
fn binding_cannot_attach_a_preexisting_authorization_request_to_a_workspace() {
    for provider in ["twitch", "youtube"] {
        let mut attempt = Attempt::unbound(provider);
        attempt.connect(START);
        let old_state = attempt.state.clone();
        let old_address = callback(provider, &old_state);
        assert!(attempt.deliver(&old_address, START + 1).is_err());

        attempt.session.bind_workspace(WORKSPACE).unwrap();
        assert!(
            attempt.deliver(&old_address, START + 2).is_err(),
            "binding must revoke the grant created without workspace ownership"
        );
        attempt.connect(START + 3);
        assert_ne!(attempt.state, old_state);
        assert!(attempt.deliver(&old_address, START + 4).is_err());
        let valid = callback(provider, &attempt.state);
        assert_provider_request(provider, &attempt.deliver(&valid, START + 5).unwrap());
    }
}

#[test]
fn disconnect_revokes_pending_callbacks_and_late_results_cannot_restore_the_account() {
    for provider in ["twitch", "youtube"] {
        let mut attempt = Attempt::new(provider, false);
        attempt
            .session
            .set_secret("access_token", "existing-account-token".into())
            .unwrap();
        attempt
            .session
            .step(
                json!({"type":"local_action","action_id":"use_tokens"}),
                &attempt.environment,
                START,
            )
            .unwrap();
        let payload = if provider == "twitch" {
            json!({"client_id":"public-id","user_id":"existing-user","login":"existing-account","scopes":["user:read:chat"],"expires_in":3600})
        } else {
            json!({"aud":"public-id","sub":"existing-account","scope":"https://www.googleapis.com/auth/youtube.readonly","expires_in":3600})
        };
        let authorized = attempt
            .session
            .step(
                json!({"type":"local_http","id":"auth:validate","status":200,"payload":payload}),
                &attempt.environment,
                START + 1,
            )
            .unwrap();
        let authorized_panel = serde_json::to_string(&authorized).unwrap();
        assert!(authorized_panel.contains("Authorized locally"));
        assert!(authorized_panel.contains("existing-account"));

        attempt.connect(START + 2);
        let old_state = attempt.state.clone();
        let pending_address = callback(provider, &old_state);
        let disconnected = attempt
            .session
            .step(
                json!({"type":"local_action","action_id":"disconnect"}),
                &attempt.environment,
                START + 3,
            )
            .unwrap();
        assert!(
            !serde_json::to_string(&disconnected)
                .unwrap()
                .contains("Authorized locally")
        );
        assert!(attempt.deliver(&pending_address, START + 4).is_err());
        let after_late_callback = attempt
            .session
            .step(
                json!({"type":"local_tick"}),
                &attempt.environment,
                START + 5,
            )
            .unwrap();
        assert!(
            !after_late_callback
                .iter()
                .any(|effect| effect["type"] == "http")
        );
        let panel = serde_json::to_string(&after_late_callback).unwrap();
        assert!(!panel.contains("Authorized locally") && !panel.contains("existing-account"));

        attempt.connect(START + 6);
        assert_ne!(attempt.state, old_state);
        assert!(attempt.deliver(&pending_address, START + 7).is_err());
        let valid = callback(provider, &attempt.state);
        assert_provider_request(provider, &attempt.deliver(&valid, START + 8).unwrap());
    }
}

#[test]
fn ordinary_events_cannot_stage_credentials_or_disrupt_a_pending_authorization() {
    for provider in ["twitch", "youtube"] {
        let mut attempt = Attempt::new(provider, false);
        attempt.session.manifest.forwarding.push(ForwardAction {
            id: "open".into(),
            mode: ForwardMode::Event,
        });
        attempt.session.manifest.validate().unwrap();
        attempt.connect(START);

        for (parameter, marker) in [("access_token", TOKEN), ("code", CODE)] {
            let event =
                format!("https://client.acheflow.com/fwd/{provider}/open?{parameter}={marker}");
            let error = attempt
                .deliver(&event, START + 1)
                .expect_err("an unsolicited Event must not stage a credential field");
            assert!(
                !error.contains(marker),
                "a rejected secret must not enter diagnostics"
            );
        }

        let ordinary = format!(
            "https://client.acheflow.com/fwd/{provider}/open?section=account&mode=callback"
        );
        let effects = attempt.deliver(&ordinary, START + 2).unwrap();
        assert!(
            !effects.iter().any(|effect| effect["type"] == "http"),
            "ordinary navigation must not perform an OAuth exchange"
        );
        let legitimate = callback(provider, &attempt.state);
        assert_provider_request(provider, &attempt.deliver(&legitimate, START + 3).unwrap());
    }
}
