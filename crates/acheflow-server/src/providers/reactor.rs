use super::*;
use acheflow_client::bridge::{Executor, ExecutorTransport};
use serde_json::{Value, json};
pub(super) struct Bridge {
    pub host: Weak<ControlHost>,
    pub provider: String,
    pub actor: ExecutorCredential,
}
impl ExecutorTransport for Bridge {
    async fn request(&self, request: ExecutorRequest) -> Result<Value, String> {
        let host = self.host.upgrade().ok_or("Server stopped.")?;
        crate::client_bridge::call_server(&host, &self.provider, &self.actor, request).await
    }
}
struct Registration<'a> {
    bridge: &'a Bridge,
    active: bool,
}
impl Registration<'_> {
    fn withdraw(&mut self) {
        if let Some(host) = self.bridge.host.upgrade() {
            host.providers
                .set_lease(&self.bridge.provider, &self.bridge.actor.id, None);
            withdraw(
                &host,
                &self.bridge.actor.workspace_id,
                &self.bridge.actor.id,
            );
        }
        self.active = false;
    }
    fn status(&mut self, state: &str, generation: u64) -> Result<(), String> {
        match state {
            "needs_authorization" => Err("provider_needs_authorization".into()),
            "failed" => Err("provider_unavailable".into()),
            "reconnecting" => {
                self.withdraw();
                if let Some(host) = self.bridge.host.upgrade() {
                    host.providers.mark(
                        &self.bridge.provider,
                        &self.bridge.actor.id,
                        "reconnecting",
                        generation,
                    );
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    async fn channel(
        &mut self,
        runtime: &mut ProviderRuntime,
        spec: &Spec,
        channel: &Value,
    ) -> Result<(), String> {
        if channel["id"].as_str() != Some(spec.binding.channel_id.as_str()) {
            return Err("Provider channel does not match its server binding.".into());
        }
        let capabilities: BTreeSet<Capability> = channel["capabilities"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| serde_json::from_value(v.clone()).ok())
            .filter(|c| spec.capabilities.contains(c))
            .collect();
        if !capabilities.contains(&Capability::ChatRead) {
            return Err("Provider cannot collect this channel.".into());
        }
        if self.active && runtime.identity().connection.capabilities == capabilities {
            return Ok(());
        }
        self.withdraw();
        let mut connection = runtime.identity().connection.clone();
        connection.capabilities = capabilities;
        connection.id = acheflow_media_api::workspace::new_token();
        runtime.select_connection(connection)?;
        Executor(self.bridge).heartbeat(runtime).await?;
        self.active = true;
        if let Some(host) = self.bridge.host.upgrade() {
            host.providers.set_lease(
                &self.bridge.provider,
                &self.bridge.actor.id,
                Some(ClientRegistration {
                    version: CLIENT_EXECUTION_VERSION,
                    generation: runtime.identity().generation,
                    connections: vec![runtime.identity().connection.clone()],
                }),
            );
            host.providers.mark(
                &self.bridge.provider,
                &self.bridge.actor.id,
                "running",
                runtime.identity().generation,
            );
        }
        Ok(())
    }
    async fn output(
        &mut self,
        runtime: &mut ProviderRuntime,
        spec: &Spec,
        output: &[Value],
    ) -> Result<(), String> {
        for effect in output {
            match effect["type"].as_str() {
                Some("status") => self.status(
                    effect["state"].as_str().unwrap_or_default(),
                    runtime.identity().generation,
                )?,
                Some("channel") if effect["reset"] == true => self.withdraw(),
                Some("channel") => self.channel(runtime, spec, &effect["channel"]).await?,
                _ => {}
            }
        }
        if self.active {
            Executor(self.bridge).publish(runtime, output).await?;
        }
        Ok(())
    }
}
pub(super) async fn run(
    bridge: &Bridge,
    runtime: &mut ProviderRuntime,
    spec: &Spec,
    account: &mut Option<ServerAccount>,
) -> Result<(), String> {
    let mut registration = Registration {
        bridge,
        active: false,
    };
    let mut output = runtime
        .turn(json!({"type":"start"}), crate::control::now_ms())
        .await?;
    let mut tick = 0;
    loop {
        if let Some(account) = account {
            account
                .checkpoint(runtime)
                .map_err(|_| "account_storage_unavailable")?;
        }
        registration.output(runtime, spec, &output).await?;
        output.clear();
        let now = crate::control::now_ms();
        if now >= tick {
            output = runtime.turn(json!({"type":"tick"}), now).await?;
            tick = crate::control::now_ms() + 1000;
            registration.output(runtime, spec, &output).await?;
            output.clear();
            if registration.active {
                Executor(bridge).execute_pending(runtime, now).await?;
            }
            continue;
        }
        let event = tokio::select! {event=runtime.receive_event()=>event?,_=tokio::time::sleep(std::time::Duration::from_millis(50))=>None};
        if let Some(event) = event {
            output = runtime.turn(event, crate::control::now_ms()).await?;
        }
    }
}
/// Polled alongside the reactor, including while a turn waits for HTTP/gRPC or a
/// send. Dropping the owning worker drops both futures and their pending I/O.
pub(super) async fn heartbeat(bridge: &Bridge, period: std::time::Duration) -> Result<(), String> {
    loop {
        tokio::time::sleep(period).await;
        let registration = bridge
            .host
            .upgrade()
            .ok_or("Server stopped.")?
            .providers
            .lease(&bridge.provider, &bridge.actor.id);
        if let Some(registration) = registration {
            bridge
                .request(ExecutorRequest::Register { registration })
                .await?;
        }
    }
}
