use crate::project::{Orientation, Workspace};
use acheflow_media_api::live::{ApiError, ControlRequest, LiveStatus, StartRequest};
use dioxus_native::prelude::*;

#[derive(Clone, Copy)]
pub struct LiveClient {
    pub project: Signal<Workspace>,
    pub status: Signal<Option<LiveStatus>>,
    pub busy: Signal<bool>,
    pub error: Signal<Option<String>>,
    pub connection: Signal<Option<String>>,
    generation: Signal<u64>,
}
#[derive(Clone, Copy)]
pub enum Action {
    Apply,
    Start,
    Stop,
    Deactivate,
    DeactivateActive,
}

pub fn use_live_client(project: Signal<Workspace>) -> LiveClient {
    let mut client = LiveClient {
        project,
        status: use_signal(|| None),
        busy: use_signal(|| false),
        error: use_signal(|| None),
        connection: use_signal(|| None),
        generation: use_signal(|| 0),
    };
    use_context_provider(|| client);
    use_future(move || async move {
        loop {
            if !*client.busy.peek() {
                let generation = *client.generation.peek();
                let response = request(reqwest::Method::GET, "/api/live/status", None).await;
                if generation == *client.generation.peek() && !*client.busy.peek() {
                    match response {
                        Ok(status) => {
                            client.status.set(Some(status));
                            client.connection.set(None);
                        }
                        Err(error) => client.connection.set(Some(error)),
                    }
                }
            }
            pause().await;
        }
    });
    client
}

impl LiveClient {
    pub fn owns(&self) -> bool {
        self.status
            .read()
            .as_ref()
            .is_some_and(|s| s.workspace_id.as_deref() == Some(self.project.read().id.as_str()))
    }
    pub fn running(&self) -> bool {
        self.owns()
            && self
                .status
                .read()
                .as_ref()
                .is_some_and(|s| !s.formats.is_empty())
    }
    pub fn command(mut self, action: Action) {
        if (self.busy)() {
            return;
        }
        let workspace = self.project.peek().clone();
        self.busy.set(true);
        self.error.set(None);
        *self.generation.write() += 1;
        spawn(async move {
            let (method, path, body) = match action {
                Action::Apply => (
                    reqwest::Method::PUT,
                    "/api/live/config",
                    serde_json::to_value(&workspace),
                ),
                Action::Start => {
                    let formats = Orientation::ALL
                        .into_iter()
                        .filter(|o| workspace.active(*o).is_some())
                        .collect();
                    (
                        reqwest::Method::POST,
                        "/api/live/start",
                        serde_json::to_value(StartRequest { workspace, formats }),
                    )
                }
                Action::Stop => (
                    reqwest::Method::POST,
                    "/api/live/stop",
                    serde_json::to_value(ControlRequest {
                        workspace_id: workspace.id,
                    }),
                ),
                Action::Deactivate => (
                    reqwest::Method::DELETE,
                    "/api/live/config",
                    serde_json::to_value(ControlRequest {
                        workspace_id: workspace.id,
                    }),
                ),
                Action::DeactivateActive => (
                    reqwest::Method::DELETE,
                    "/api/live/config",
                    serde_json::to_value(ControlRequest {
                        workspace_id: self
                            .status
                            .peek()
                            .as_ref()
                            .and_then(|s| s.workspace_id.clone())
                            .unwrap_or_default(),
                    }),
                ),
            };
            let result = match body {
                Ok(body) => request(method, path, Some(body)).await,
                Err(e) => Err(e.to_string()),
            };
            match result {
                Ok(status) => {
                    self.status.set(Some(status));
                    self.connection.set(None);
                }
                Err(error) => self.error.set(Some(error)),
            }
            self.busy.set(false);
        });
    }
    pub fn add_source(mut self) {
        let port = self
            .status
            .peek()
            .as_ref()
            .and_then(|s| s.rtmp_address.parse::<std::net::SocketAddr>().ok())
            .map(|a| a.port())
            .unwrap_or(1935);
        let base = crate::media::server_url("")
            .ok()
            .and_then(|s| reqwest::Url::parse(&s).ok());
        let host = base
            .as_ref()
            .and_then(|u| u.host_str())
            .unwrap_or("localhost");
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.to_owned()
        };
        let mut project = self.project.write();
        project.add_source();
        if let Some(source) = project.sources.last_mut() {
            source.url = format!("rtmp://{host}:{port}/live");
        }
    }
    pub async fn copy_text(mut self, text: String) -> bool {
        #[cfg(target_arch = "wasm32")]
        let result = match web_sys::window() {
            Some(window) => wasm_bindgen_futures::JsFuture::from(
                window.navigator().clipboard().write_text(&text),
            )
            .await
            .map(|_| ())
            .map_err(|_| {
                "Clipboard request was denied; reveal the token to copy it manually".to_owned()
            }),
            None => Err("Browser clipboard is unavailable".into()),
        };
        #[cfg(not(target_arch = "wasm32"))]
        let result =
            try_consume_context::<std::sync::Arc<dyn blitz_traits::shell::ShellProvider>>()
                .ok_or_else(|| "Clipboard is unavailable".to_owned())
                .and_then(|shell| {
                    shell
                        .set_clipboard_text(text)
                        .map_err(|_| "Clipboard request failed".to_owned())
                });
        match result {
            Ok(()) => true,
            Err(error) => {
                self.error.set(Some(error));
                false
            }
        }
    }
    pub fn preview(&self, path: &str) -> String {
        crate::media::server_url(&format!("{path}?workspace={}", self.project.read().id))
            .unwrap_or_default()
    }
}

async fn request(
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<LiveStatus, String> {
    let mut request = reqwest::Client::new()
        .request(method, crate::media::server_url(path)?)
        .timeout(std::time::Duration::from_secs(15));
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("Media server is unreachable: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let bytes = response.bytes().await.map_err(|e| e.to_string())?;
        return Err(serde_json::from_slice::<ApiError>(&bytes)
            .map(|e| e.error)
            .unwrap_or_else(|_| format!("Media API rejected the request ({status})")));
    }
    response
        .json()
        .await
        .map_err(|e| format!("Invalid live status: {e}"))
}
async fn pause() {
    #[cfg(target_arch = "wasm32")]
    gloo_timers::future::TimeoutFuture::new(500).await;
    #[cfg(not(target_arch = "wasm32"))]
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
}

#[component]
pub fn LiveNotice() -> Element {
    let client = use_context::<LiveClient>();
    rsx! {
        if let Some(error)=client.connection.read().as_ref() {p {class:"live-notice",role:"status","{error}"}}
        if let Some(error)=client.error.read().as_ref() {p {class:"live-notice field-error",role:"alert","{error}"}}
        if !client.owns() && client.status.read().as_ref().is_some_and(|s|s.workspace_id.is_some()) {
            div {class:"live-notice row",span {"The server is configured for another workspace."}button {disabled:(client.busy)(),onclick:move |_|client.command(Action::DeactivateActive),"Stop & deactivate active workspace"}}
        }
    }
}
