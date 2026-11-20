//! Revalidate submitted credentials using only the installed guest's auth effects.
use crate::local::{LocalSession, LocalState};
use acheflow_control::extensions::GuestManifest;
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};

// Not serializable or printable. Only a successful guest validation creates this.
pub struct AuthorizedAccount {
    pub(super) state: LocalState,
    pub(super) account: String,
    provider: String,
}
impl AuthorizedAccount {
    pub fn account_id(&self) -> &str {
        &self.account
    }
    pub fn save(
        &self,
        server: &str,
        workspace: &str,
        provider: &str,
        slot: &str,
    ) -> Result<(), String> {
        if provider != self.provider {
            return Err("Account belongs to another provider.".into());
        }
        crate::vault::save(
            &super::storage_key(server, workspace, provider, slot, &self.account)?,
            &self.state,
        )
    }
}
pub async fn authorize(
    manifest: GuestManifest,
    module: &[u8],
    config: BTreeMap<String, String>,
    credentials: BTreeMap<String, String>,
) -> Result<AuthorizedAccount, String> {
    authorize_with(manifest, module, config, credentials, |session, effect| {
        Box::pin(async move { session.http(&effect).await })
    })
    .await
}
type HttpResult<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = Value> + Send + 'a>>;
async fn authorize_with(
    manifest: GuestManifest,
    module: &[u8],
    config: BTreeMap<String, String>,
    credentials: BTreeMap<String, String>,
    mut http: impl for<'a> FnMut(&'a mut LocalSession, Value) -> HttpResult<'a>,
) -> Result<AuthorizedAccount, String> {
    if credentials.is_empty() || credentials.len() > 32 {
        return Err("Invalid account credentials.".into());
    }
    let provider = manifest.id.clone();
    let mut session = LocalSession::new_for_host(
        manifest,
        module,
        config,
        acheflow_extension_api::parts::ExecutionHost::Server,
    )?;
    let environment = json!({"browser":false});
    session.step(json!({"type":"local_start"}), &environment, now())?;
    for (key, value) in credentials {
        session.set_secret(&key, value)?;
    }
    let effects = session.step(
        json!({"type":"local_server_authorize"}),
        &environment,
        now(),
    )?;
    let mut pending = VecDeque::from(effects);
    let mut requests = 0;
    for _ in 0..64 {
        let Some(effect) = pending.pop_front() else {
            let credentials = session.server_credentials_unbound()?;
            return Ok(AuthorizedAccount {
                provider,
                account: credentials.account_id,
                state: session.take_server_state(),
            });
        };
        match effect["type"].as_str() {
            Some("panel" | "status") => {}
            Some("http") if requests < 8 => {
                requests += 1;
                let mut input = http(&mut session, effect).await;
                input["type"] = json!("local_http");
                pending.extend(session.step(input, &environment, now())?);
            }
            _ => return Err("Server account validation failed.".into()),
        }
    }
    Err("Server account validation exceeded its limit.".into())
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    fn payload(id: &str) -> Value {
        if id == "youtube" {
            json!({"aud":"application","sub":"verified-user","scope":"https://www.googleapis.com/auth/youtube.readonly","expires_in":3600})
        } else {
            json!({"data":{"user":{"id":123,"nick":"Untrusted display name"}}})
        }
    }
    fn config() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("client_id".into(), "application".into()),
            ("send_chat".into(), "false".into()),
        ])
    }
    #[tokio::test]
    async fn two_real_wasm_providers_verify_identity_without_starting_collectors() {
        let catalog = crate::catalog::ClientCatalog::discover_server_at(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions"),
        );
        for id in ["youtube", "vklive"] {
            let installed = &catalog.installed[id];
            let response = payload(id);
            let mut requests = 0;
            let account = authorize_with(
                installed.manifest.clone(),
                &installed.module,
                config(),
                BTreeMap::from([("access_token".into(), "synthetic-token".into())]),
                |_session, effect| {
                    requests += 1;
                    assert_eq!(effect["id"], "auth:validate");
                    assert_eq!(effect["type"], "http");
                    let response = response.clone();
                    Box::pin(async move {
                        json!({"id":"auth:validate","status":200,"payload":response})
                    })
                },
            )
            .await
            .unwrap();
            assert_eq!(requests, 1);
            assert_eq!(
                account.account_id(),
                if id == "youtube" {
                    "verified-user"
                } else {
                    "123"
                }
            );
            assert!(account.state.secrets.contains_key("access_token"));
            assert!(
                account
                    .save("server", "workspace", "unrelated-provider", "slot")
                    .is_err()
            );
        }
    }
    #[tokio::test]
    async fn invalid_identity_scope_application_or_http_status_never_produces_a_saved_account() {
        let catalog = crate::catalog::ClientCatalog::discover_server_at(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions"),
        );
        for id in ["youtube", "vklive"] {
            let installed = &catalog.installed[id];
            let mut cases = vec![(401, payload(id)), (200, json!({})), (0, json!({}))];
            if id == "youtube" {
                let mut bad = payload(id);
                bad["scope"] = json!("");
                cases.push((200, bad));
                let mut bad = payload(id);
                bad["aud"] = json!("another-application");
                cases.push((200, bad));
            }
            for (status, response) in cases {
                assert!(
                    authorize_with(
                        installed.manifest.clone(),
                        &installed.module,
                        config(),
                        BTreeMap::from([("access_token".into(), "synthetic-token".into())]),
                        |_, _| {
                            let response = response.clone();
                            Box::pin(async move {
                                json!({"id":"auth:validate","status":status,"payload":response})
                            })
                        }
                    )
                    .await
                    .is_err()
                );
            }
        }
    }
    #[test]
    fn transfer_requires_matching_workspace_and_committed_validation_and_keeps_the_source() {
        let catalog = crate::catalog::ClientCatalog::bundled();
        let installed = &catalog.installed["vklive"];
        let mut source =
            LocalSession::new(installed.manifest.clone(), &installed.module, config()).unwrap();
        source.bind_workspace("workspace").unwrap();
        assert!(source.server_credentials("workspace").is_err());
        source
            .set_secret("access_token", "synthetic-token".into())
            .unwrap();
        source
            .step(
                json!({"type":"local_server_authorize"}),
                &json!({"browser":false}),
                1000,
            )
            .unwrap();
        source.step(json!({"type":"local_http","id":"auth:validate","status":200,"payload":payload("vklive")}),&json!({"browser":false}),1001).unwrap();
        assert!(source.server_credentials("other-workspace").is_err());
        assert_eq!(
            source.server_credentials("workspace").unwrap().account_id,
            "123"
        );
        assert!(source.has_secret("access_token"));
        source
            .set_secret("access_token", "draft-token".into())
            .unwrap();
        assert!(source.server_credentials("workspace").is_err());
    }
}
