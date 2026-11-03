//! Authenticated MCP connection. Only typed domain DTOs can leave this adapter.
use crate::runtime::ProviderRuntime;
use acheflow_extension_api::client_execution::*;
use rmcp::{
    RoleClient, ServiceExt,
    model::CallToolRequestParams,
    service::RunningService,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::Value;

pub struct ExecutorConnection {
    service: RunningService<RoleClient, ()>,
}
impl ExecutorConnection {
    pub async fn connect(endpoint: &str, transport_token: &str) -> Result<Self, String> {
        let url = url::Url::parse(endpoint).map_err(|_| "invalid executor endpoint")?;
        let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        let secure_transport = url.scheme() == "https" || (url.scheme() == "http" && loopback);
        if !secure_transport {
            return Err("executor endpoint requires HTTPS or loopback HTTP".into());
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("executor endpoint must not contain credentials".into());
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err("executor endpoint must not contain query or fragment".into());
        }
        let http = crate::transport::client()?;
        let config = StreamableHttpClientTransportConfig::with_uri(endpoint.to_owned())
            .auth_header(transport_token.to_owned());
        let service =
            ().serve(StreamableHttpClientTransport::with_client(http, config))
                .await
                .map_err(|_| "executor MCP connection failed")?;
        Ok(Self { service })
    }
    pub async fn request(&self, request: ExecutorRequest) -> Result<Value, String> {
        let args = serde_json::to_value(request)
            .map_err(|_| "invalid executor operation")?
            .as_object()
            .cloned()
            .ok_or("invalid executor operation")?;
        let result = self
            .service
            .call_tool(CallToolRequestParams::new("acheflow_executor").with_arguments(args))
            .await
            .map_err(|_| "executor MCP request failed")?;
        if result.is_error == Some(true) {
            return Err("executor operation rejected by server".into());
        }
        result
            .structured_content
            .ok_or_else(|| "executor response has no structured result".into())
    }
    pub async fn heartbeat(&self, runtime: &ProviderRuntime) -> Result<ExecutorStatus, String> {
        let identity = runtime.identity();
        let value = self
            .request(ExecutorRequest::Register {
                registration: ClientRegistration {
                    version: CLIENT_EXECUTION_VERSION,
                    generation: identity.generation,
                    connections: vec![identity.connection.clone()],
                },
            })
            .await?;
        serde_json::from_value(value).map_err(|_| "invalid executor registration response".into())
    }
    /// Only these normalized, source-checked events are forwarded. OAuth panels,
    /// HTTP responses, status/errors, local storage and credentials stay local.
    pub async fn publish(&self, runtime: &ProviderRuntime, output: &[Value]) -> Result<(), String> {
        for effect in output {
            let event = match effect["type"].as_str() {
                Some("chat" | "audience") => serde_json::from_value::<ClientEvent>(effect.clone())
                    .map_err(|_| "invalid normalized client event")?,
                Some("delete") if effect["all"] == true => ClientEvent::ClearChat {},
                Some("delete") => ClientEvent::Delete {
                    message_id: effect["message_id"]
                        .as_str()
                        .ok_or("invalid deletion event")?
                        .into(),
                },
                _ => continue,
            };
            let connection = &runtime.identity().connection;
            let source_ok = match &event {
                ClientEvent::Chat { message } => {
                    message.platform == connection.provider
                        && message.channel_id == connection.channel_id
                }
                ClientEvent::Audience { sample } => {
                    sample.platform == connection.provider
                        && sample.channel_id == connection.channel_id
                }
                ClientEvent::Delete { .. } | ClientEvent::ClearChat { .. } => {
                    effect["platform"] == connection.provider
                        && effect["channel_id"] == connection.channel_id
                }
            };
            if !source_ok {
                return Err("client event source does not match local connection".into());
            }
            self.request(ExecutorRequest::Publish {
                generation: runtime.identity().generation,
                connection_id: connection.id.clone(),
                event,
            })
            .await?;
        }
        Ok(())
    }
    /// Call while registered; caller owns heartbeat, local UI and cancellation.
    pub async fn execute_pending(
        &self,
        runtime: &mut ProviderRuntime,
        now: u64,
    ) -> Result<usize, String> {
        let value = self
            .request(ExecutorRequest::Claim {
                generation: runtime.identity().generation,
                limit: 1,
            })
            .await?;
        let claims: Vec<ClaimedInstruction> =
            serde_json::from_value(value).map_err(|_| "invalid claimed instructions")?;
        let count = claims.len();
        for claim in claims {
            let current = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| "invalid client clock")?
                .as_millis() as u64;
            let (receipt, output) = runtime.execute(claim, now.max(current)).await?;
            self.request(ExecutorRequest::Report {
                generation: runtime.identity().generation,
                receipt,
            })
            .await?;
            self.publish(runtime, &output).await?;
        }
        Ok(count)
    }
    pub async fn disconnect(self, generation: u64) -> Result<(), String> {
        let result = self
            .request(ExecutorRequest::Unregister { generation })
            .await;
        let _ = self.service.cancel().await;
        result.map(|_| ())
    }
}
