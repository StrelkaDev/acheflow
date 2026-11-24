//! Credential-free adapter from authenticated executor MCP calls to domain services.
use crate::control::{ControlHost, now_ms};
use acheflow_extension_api::{client_execution::*, integration::Channel};
use acheflow_media_api::control::ControlRequest;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

/// Same trusted-operator boundary as /api/control/commands, not an MCP credential
/// bypass. Registration still intersects the saved workspace capability policy.
pub async fn studio(
    axum::extract::State(host): axum::extract::State<Arc<ControlHost>>,
    axum::Extension(principal): axum::Extension<crate::access::Principal>,
    axum::Json(request): axum::Json<acheflow_media_api::control::StudioExecutorRequest>,
) -> Result<axum::Json<Value>, (axum::http::StatusCode, axum::Json<Value>)> {
    let result = if !request.client_id.starts_with("studio-")
        || !acheflow_extension_api::integration::valid_id(&request.client_id, 64)
    {
        Err("invalid_argument: invalid Studio client identity".into())
    } else {
        let requested = request.client_id.clone();
        let actor = principal.executor_id(&requested);
        let result = call(
            &host,
            &ExecutorCredential {
                id: actor,
                workspace_id: request.workspace_id,
                token: String::new(),
            },
            request.operation,
        )
        .await;
        // The wire identity stays compatible with the local runtime's ledger;
        // registry ownership is always namespaced by the authenticated device.
        result.map(|mut value| {
            restore_client_id(&mut value, &requested);
            value
        })
    };
    result.map(axum::Json).map_err(|message: String| {
        let status = if message.starts_with("conflict") {
            axum::http::StatusCode::CONFLICT
        } else if message.starts_with("forbidden") {
            axum::http::StatusCode::FORBIDDEN
        } else {
            axum::http::StatusCode::BAD_REQUEST
        };
        (status, axum::Json(json!({"error":{"message":message}})))
    })
}

fn restore_client_id(value: &mut Value, requested: &str) {
    match value {
        Value::Object(fields) => {
            if fields.contains_key("client_id") {
                fields.insert("client_id".into(), json!(requested));
            }
            for value in fields.values_mut() {
                restore_client_id(value, requested);
            }
        }
        Value::Array(values) => {
            for value in values {
                restore_client_id(value, requested);
            }
        }
        _ => {}
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorCredential {
    pub id: String,
    pub workspace_id: String,
    // This is Acheflow transport authentication, never a platform credential.
    pub token: String,
}

pub async fn call(
    host: &Arc<ControlHost>,
    actor: &ExecutorCredential,
    request: ExecutorRequest,
) -> Result<Value, String> {
    call_inner(host, actor, request, None).await
}
pub(crate) async fn call_server(
    host: &Arc<ControlHost>,
    provider: &str,
    actor: &ExecutorCredential,
    request: ExecutorRequest,
) -> Result<Value, String> {
    call_inner(host, actor, request, Some(provider)).await
}
async fn call_inner(
    host: &Arc<ControlHost>,
    actor: &ExecutorCredential,
    request: ExecutorRequest,
    server_provider: Option<&str>,
) -> Result<Value, String> {
    let _command = host.command_lock.lock().await;
    if server_provider.is_none() && actor.token.is_empty() {
        match &request {
            ExecutorRequest::Register { registration }
                if registration.connections.iter().any(|c| {
                    c.capabilities
                        .iter()
                        .any(|cap| matches!(cap, Capability::ChatRead | Capability::AudienceRead))
                }) =>
            {
                return Err(
                    "forbidden: shared feeds are collected by the server, not device clients"
                        .into(),
                );
            }
            ExecutorRequest::Publish { .. } => {
                return Err("forbidden: device clients cannot publish the shared feed".into());
            }
            _ => {}
        }
    }
    if host.runtime.shutdown.is_cancelled() {
        return Err("unavailable: server stopped".into());
    }
    if let Some(provider) = server_provider
        && !host
            .providers
            .current(&actor.workspace_id, provider, &actor.id)
    {
        return Err("forbidden: server provider ownership changed".into());
    }

    if server_provider.is_none()
        && !host
            .runtime
            .activated
            .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(
            "unavailable: activate the saved workspace before running client integrations".into(),
        );
    }
    if !host.owns(&actor.workspace_id) {
        return Err("conflict: executor workspace is not active".into());
    }
    host.snapshot(0, 1);
    let now = now_ms();
    let workspace = &actor.workspace_id;
    match request {
        ExecutorRequest::Register { registration } => {
            let saved = host.runtime.workspace.read().unwrap();
            for connection in &registration.connections {
                let server_binding = saved
                    .as_ref()
                    .and_then(|w| w.server_integrations.get(&connection.provider))
                    .filter(|b| b.enabled && b.channel_id == connection.channel_id);
                if let Some(provider) = server_provider {
                    if provider != connection.provider
                        || !server_binding.is_some_and(|b| b.account_id == connection.account_id)
                    {
                        return Err(
                            "forbidden: connection does not match the server binding".into()
                        );
                    }
                } else if server_binding.is_some() {
                    return Err("forbidden: this channel is collected by the server".into());
                }
                if host
                    .installed
                    .read()
                    .unwrap()
                    .contains_key(&connection.provider)
                {
                    return Err("conflict: provider identity is reserved by server logic".into());
                }
            }
            let mut registry = host.client_execution.lock().unwrap();
            let status = if server_provider.is_some() {
                registry.register_server(workspace, &actor.id, registration, now)
            } else {
                registry.register(workspace, &actor.id, registration, now)
            }?;
            Ok(json!(status))
        }
        ExecutorRequest::Unregister { generation } => {
            host.client_execution
                .lock()
                .unwrap()
                .unregister(workspace, &actor.id, generation, now)?;
            Ok(json!({"ok":true}))
        }
        ExecutorRequest::Claim { generation, limit } => Ok(json!(
            host.client_execution
                .lock()
                .unwrap()
                .claim(workspace, &actor.id, generation, limit, now)?
        )),
        ExecutorRequest::Report {
            generation,
            receipt,
        } => {
            host.client_execution
                .lock()
                .unwrap()
                .report(workspace, &actor.id, generation, receipt, now)?;
            host.snapshot(0, 1);
            Ok(json!({"ok":true}))
        }
        ExecutorRequest::Publish {
            generation,
            connection_id,
            event,
        } => {
            let _control = host.runtime.control.lock().await;
            let connection = host.client_execution.lock().unwrap().connection(
                workspace,
                &actor.id,
                generation,
                &connection_id,
                now_ms(),
            )?;
            if !connection.capabilities.contains(&event.capability()) {
                return Err("forbidden: event capability is not granted".into());
            }
            let effect = match event {
                ClientEvent::Chat { message } => {
                    if message.platform != connection.provider
                        || message.channel_id != connection.channel_id
                    {
                        return Err("forbidden: chat source does not match connection".into());
                    }
                    json!({"type":"chat","message":message})
                }
                ClientEvent::Audience { sample } => {
                    if sample.platform != connection.provider
                        || sample.channel_id != connection.channel_id
                    {
                        return Err("forbidden: audience source does not match connection".into());
                    }
                    json!({"type":"audience","sample":sample})
                }
                ClientEvent::ClearChat {} => {
                    json!({"type":"delete","platform":connection.provider,"channel_id":connection.channel_id,"all":true})
                }
                ClientEvent::Delete { message_id } => {
                    if !acheflow_extension_api::integration::valid_id(&message_id, 256) {
                        return Err("invalid_argument: deletion identity".into());
                    }
                    json!({"type":"delete","platform":connection.provider,"channel_id":connection.channel_id,"message_id":message_id,"all":false})
                }
            };
            host.publish_inner(workspace, &connection.provider, "", effect, 0, true)?;
            Ok(json!({"ok":true}))
        }
    }
}

pub fn enqueue_chat(
    host: &ControlHost,
    request: &ControlRequest,
    channel: &Channel,
    text: &str,
) -> Result<(), String> {
    let now = now_ms();
    let mut registry = host.client_execution.lock().unwrap();
    let executors = registry.connections(&request.workspace_id, now);
    let (executor, connection) = executors
        .iter()
        .find_map(|e| {
            e.connections
                .iter()
                .find(|c| {
                    c.provider == channel.extension_id
                        && c.channel_id == channel.id
                        && c.capabilities.contains(&Capability::ChatSend)
                })
                .map(|c| (e, c))
        })
        .ok_or("unavailable: channel executor disconnected")?;
    let hash = openssl::sha::sha256(
        &serde_json::to_vec(&(&request.request_id, &channel.extension_id, &channel.id))
            .map_err(|_| "invalid instruction identity")?,
    );
    let suffix = hash.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let time = request
        .request_id
        .split(':')
        .next()
        .ok_or("invalid request identity")?;
    registry.enqueue(
        &request.workspace_id,
        InstructionRequest {
            id: format!("{time}:{suffix}"),
            request_id: Some(request.request_id.clone()),
            client_id: executor.client_id.clone(),
            generation: executor.generation,
            connection_id: connection.id.clone(),
            action: ClientAction::SendChat { text: text.into() },
            expires_at_ms: time
                .parse::<u64>()
                .map_err(|_| "invalid request time")?
                .saturating_add(30_000),
        },
        now,
    )?;
    Ok(())
}

/// Called with the host's workspace/services locks held; never reacquires them.
pub fn refresh(
    host: &ControlHost,
    workspace: Option<&acheflow_media_api::workspace::Workspace>,
    state: &mut crate::control::Services,
    now: u64,
) -> Vec<ExecutorStatus> {
    let owner = workspace.map(|w| w.id.clone());
    let executors = owner
        .as_deref()
        .map(|owner| {
            let mut registry = host.client_execution.lock().unwrap();
            let policy = workspace.unwrap().client_capabilities.clone();
            if registry.set_policy(owner, policy, now).is_err() {
                registry.withdraw_workspace(owner, now);
            }
            registry.connections(owner, now)
        })
        .unwrap_or_default();
    let live_providers: std::collections::BTreeSet<_> = executors
        .iter()
        .flat_map(|e| e.connections.iter().map(|c| c.provider.clone()))
        .collect();
    let providers: std::collections::BTreeSet<_> = state
        .client_providers
        .union(&live_providers)
        .cloned()
        .collect();
    state.client_providers = live_providers;
    for id in &providers {
        state.integrations.disconnect(id);
        let connections: Vec<_> = executors
            .iter()
            .flat_map(|e| &e.connections)
            .filter(|c| &c.provider == id)
            .collect();
        let unavailable: Vec<_> = state
            .analytics
            .snapshot(now)
            .channels
            .into_iter()
            .filter(|sample| {
                sample.sample.platform == *id
                    && !connections.iter().any(|c| {
                        c.channel_id == sample.sample.channel_id
                            && c.capabilities.contains(
                                &acheflow_extension_api::client_execution::Capability::AudienceRead,
                            )
                    })
            })
            .map(|sample| sample.sample.channel_id)
            .collect();
        for channel in unavailable {
            state.analytics.disconnect_channel(id, &channel);
        }
        for c in &connections {
            let mut caps = Vec::new();
            if c.capabilities.contains(&Capability::ChatRead) {
                caps.push("chat.read".into());
            }
            if c.capabilities.contains(&Capability::ChatSend) {
                caps.push("chat.send".into());
            }
            let _ = state.integrations.register_channel(
                id,
                acheflow_extension_api::integration::Channel {
                    stream_id: c.stream_id.clone(),
                    id: c.channel_id.clone(),
                    extension_id: id.clone(),
                    label: c
                        .channel_label
                        .clone()
                        .unwrap_or_else(|| format!("{} channel", c.provider)),
                    capabilities: caps,
                    constraints: Default::default(),
                },
            );
        }
    }
    if let Some(owner) = &owner {
        use acheflow_extension_api::client_execution::InstructionState as S;
        let instructions = host
            .client_execution
            .lock()
            .unwrap()
            .instructions(owner, now);
        for i in instructions {
            let Some(request_id) = i.request_id else {
                continue;
            };
            let sent = match i.state {
                S::Queued | S::InFlight => continue,
                S::Sent => Some(true),
                S::Unknown => None,
                _ => Some(false),
            };
            let error = if i.state == S::Sent {
                None
            } else {
                Some(format!("Client execution: {:?}", i.state))
            };
            let _ = state.interaction.delivery(
                &i.connection.provider,
                &i.connection.channel_id,
                &request_id,
                sent,
                i.external_id,
                error,
            );
        }
    }

    executors
}

#[cfg(test)]
mod tests {
    use super::*;
    use acheflow_client::{
        bridge::ExecutorConnection,
        runtime::{Identity, ProviderRuntime},
    };
    use acheflow_media_api::control::ControlAction;
    use std::collections::BTreeSet;

    fn setup() -> (Arc<ControlHost>, Vec<(ExecutorCredential, ProviderRuntime)>) {
        setup_with_prefix("client")
    }
    fn setup_with_prefix(
        prefix: &str,
    ) -> (Arc<ControlHost>, Vec<(ExecutorCredential, ProviderRuntime)>) {
        let (mut host, mut workspace) = crate::control::tests::host();
        let inner = Arc::get_mut(&mut host).unwrap();
        inner.installed.get_mut().unwrap().clear();
        let mut clients = vec![];
        for (n, id) in ["alpha", "beta"].into_iter().enumerate() {
            let manifest:acheflow_control::extensions::GuestManifest=serde_json::from_value(json!({"id":id,"name":id,"api_version":2,"module":"fixture.wasm","execution_side":"client","capabilities":["channel","chat","audience","delivery","poll","donation","interaction"],"services":[],"subscriptions":["chat.send"]})).unwrap();
            let connection = Connection {
                channel_label: None,
                stream_id: None,
                id: "local".into(),
                provider: id.into(),
                account_id: "bot".into(),
                channel_id: "channel".into(),
                capabilities: BTreeSet::from([
                    Capability::ChatSend,
                    Capability::ChatRead,
                    Capability::AudienceRead,
                ]),
            };
            workspace.client_capabilities.push(CapabilityGrant {
                provider: id.into(),
                account_id: "bot".into(),
                channel_id: "channel".into(),
                capabilities: connection.capabilities.clone(),
            });
            let actor = ExecutorCredential {
                id: format!("{prefix}-{n}"),
                workspace_id: workspace.id.clone(),
                token: format!("test-distinct-executor-credential-{n:04}"),
            };
            let runtime = ProviderRuntime::new(
                manifest.clone(),
                include_bytes!("../tests/fixtures/domain.wasm"),
                Identity {
                    workspace_id: workspace.id.clone(),
                    client_id: actor.id.clone(),
                    generation: 1,
                    connection,
                },
                Default::default(),
            )
            .unwrap();
            clients.push((actor, runtime));
        }
        *inner.runtime.workspace.write().unwrap() = Some(workspace);
        *inner.services.lock().unwrap() = Default::default();
        host.snapshot(0, 1);
        (host, clients)
    }
    #[tokio::test]
    async fn studio_http_bridge_rejects_device_collectors_from_two_real_guests() {
        let (host, mut clients) = setup_with_prefix("studio");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = axum::Router::new()
            .route("/api/control/executor", axum::routing::post(studio))
            .with_state(host.clone())
            .layer(axum::Extension(crate::access::Principal(
                acheflow_media_api::access::Session {
                    server_id: "fixture".into(),
                    user: acheflow_media_api::access::User {
                        id: "user".into(),
                        name: "User".into(),
                    },
                    device: acheflow_media_api::access::Device {
                        user_id: "user".into(),
                        id: "device".into(),
                        name: "Device".into(),
                        permissions: acheflow_media_api::access::Permission::ALL
                            .into_iter()
                            .collect(),
                        expires_at_ms: u64::MAX,
                    },
                },
            )));
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        for (actor, runtime) in &mut clients {
            let bridge = ExecutorConnection::studio(
                format!("http://{address}/api/control/executor"),
                actor.workspace_id.clone(),
                actor.id.clone(),
            )
            .unwrap();
            assert!(bridge.heartbeat(runtime).await.is_err());
            let output = runtime
                .turn(json!({"type":"start"}), now_ms())
                .await
                .unwrap();
            bridge.publish(runtime, &output).await.unwrap();
            let output=runtime.turn(json!({"type":"command","name":"chat.send","request_id":"fixture","text":"Never publish this device copy"}),now_ms()).await.unwrap();
            assert!(bridge.publish(runtime, &output).await.is_err());
        }
        let state = host.snapshot(0, 200);
        assert!(state.client_executors.is_empty());
        assert!(state.chat.entries.is_empty());
        server.abort();
    }
    #[tokio::test]
    async fn real_mcp_and_two_wasm_executors_complete_chat_without_server_provider_io() {
        let (host, mut clients) = setup();
        assert!(
            host.installed.read().unwrap().is_empty(),
            "client providers must not need server installations"
        );
        assert!(host.snapshot(0, 1).extensions.is_empty());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = crate::mcp::router_with_executors(
            host.clone(),
            "operator".into(),
            None,
            clients.iter().map(|(a, _)| a.clone()).collect(),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut sessions = vec![];
        for (actor, runtime) in &mut clients {
            let endpoint = format!("http://{address}/mcp/executor/{}", actor.id);
            let session = ExecutorConnection::connect(&endpoint, &actor.token)
                .await
                .unwrap();
            session.heartbeat(runtime).await.unwrap();
            runtime
                .turn(json!({"type":"start"}), now_ms())
                .await
                .unwrap();
            sessions.push(session);
        }
        // An executor cannot acquire another identity or publish arbitrary effects/secrets.
        assert!(
            ExecutorConnection::connect(
                &format!("http://{address}/mcp/executor/client-1"),
                &clients[0].0.token
            )
            .await
            .is_err()
        );
        assert!(serde_json::from_value::<ExecutorRequest>(json!({"type":"publish","generation":1,"connection_id":"local","event":{"type":"http","url":"https://example.test","access_token":"never"}})).is_err());
        let snapshot = host.snapshot(0, 200);
        assert_eq!(snapshot.channels.len(), 2);
        assert!(host.guests.lock().await.is_empty());
        let command = ControlRequest {
            workspace_id: snapshot.workspace_id.unwrap(),
            request_id: format!("{}:fanout", now_ms()),
            expected_revision: snapshot.revision,
            action: ControlAction::SendChat {
                targets: vec![],
                text: "hello from client".into(),
            },
        };
        host.execute(command.clone()).await.unwrap();
        for (session, (_, runtime)) in sessions.iter().zip(&mut clients) {
            assert_eq!(session.execute_pending(runtime, now_ms()).await.unwrap(), 1);
            assert_eq!(session.execute_pending(runtime, now_ms()).await.unwrap(), 0);
        }
        let result = host.snapshot(0, 200);
        let local = result
            .chat
            .entries
            .iter()
            .find(|e| e.message.platform == "acheflow")
            .unwrap();
        assert_eq!(local.delivery.len(), 2);
        assert!(
            local
                .delivery
                .iter()
                .all(|d| d.state == "sent" && d.external_id.is_some())
        );
        assert_eq!(
            result
                .chat
                .entries
                .iter()
                .filter(|e| e.merged_into.is_none() && !e.deleted)
                .count(),
            1
        );
        // Retrying UI command cannot issue a second external send.
        host.execute(command).await.unwrap();
        for (session, (_, runtime)) in sessions.iter().zip(&mut clients) {
            assert_eq!(session.execute_pending(runtime, now_ms()).await.unwrap(), 0);
        }
        let bad = ExecutorRequest::Publish {
            generation: 1,
            connection_id: "local".into(),
            event: ClientEvent::Chat {
                message: acheflow_extension_api::interaction::ChatMessage {
                    platform: "beta".into(),
                    channel_id: "channel".into(),
                    message_id: "spoof".into(),
                    user_id: "viewer".into(),
                    display_name: "Viewer".into(),
                    text: "bad".into(),
                    roles: vec![],
                    occurred_at_ms: now_ms(),
                },
            },
        };
        assert!(sessions[0].request(bad).await.is_err());
        for session in sessions {
            session.disconnect(1).await.unwrap();
        }
        assert!(host.snapshot(0, 1).channels.is_empty());
        assert!(
            call(
                &host,
                &clients[0].0,
                ExecutorRequest::Claim {
                    generation: 1,
                    limit: 1
                }
            )
            .await
            .is_err()
        );
        server.abort();
    }
    #[tokio::test]
    async fn policy_revocation_and_actor_binding_reject_late_events() {
        let (host, clients) = setup();
        let (actor, runtime) = &clients[0];
        call(
            &host,
            actor,
            ExecutorRequest::Register {
                registration: ClientRegistration {
                    version: 1,
                    generation: 1,
                    connections: vec![runtime.identity().connection.clone()],
                },
            },
        )
        .await
        .unwrap();
        let sample = acheflow_extension_api::audience::AudienceSample {
            platform: "alpha".into(),
            channel_id: "channel".into(),
            stream_id: Some("live".into()),
            observed_at_ms: now_ms(),
            concurrent: Some(42),
            offline: false,
            unique: None,
        };
        call(
            &host,
            actor,
            ExecutorRequest::Publish {
                generation: 1,
                connection_id: "local".into(),
                event: ClientEvent::Audience { sample },
            },
        )
        .await
        .unwrap();
        assert_eq!(host.snapshot(0, 1).audience.total, Some(42));
        host.runtime
            .workspace
            .write()
            .unwrap()
            .as_mut()
            .unwrap()
            .client_capabilities[0]
            .capabilities
            .remove(&Capability::AudienceRead);
        assert_eq!(host.snapshot(0, 1).audience.total, None);
        assert!(
            call(
                &host,
                actor,
                ExecutorRequest::Claim {
                    generation: 1,
                    limit: 1
                }
            )
            .await
            .is_ok()
        );
        let wrong = ExecutorCredential {
            workspace_id: "another-workspace".into(),
            id: actor.id.clone(),
            token: String::new(),
        };
        assert!(
            call(
                &host,
                &wrong,
                ExecutorRequest::Claim {
                    generation: 1,
                    limit: 1
                }
            )
            .await
            .is_err()
        );
        host.runtime
            .workspace
            .write()
            .unwrap()
            .as_mut()
            .unwrap()
            .client_capabilities
            .retain(|g| g.provider != "alpha");
        assert!(
            call(
                &host,
                actor,
                ExecutorRequest::Claim {
                    generation: 1,
                    limit: 1
                }
            )
            .await
            .is_err()
        );
        host.runtime
            .workspace
            .write()
            .unwrap()
            .as_mut()
            .unwrap()
            .client_capabilities
            .push(CapabilityGrant {
                provider: "alpha".into(),
                account_id: "bot".into(),
                channel_id: "channel".into(),
                capabilities: runtime.identity().connection.capabilities.clone(),
            });
        assert!(
            call(
                &host,
                actor,
                ExecutorRequest::Claim {
                    generation: 1,
                    limit: 1
                }
            )
            .await
            .is_err()
        );
    }
}
