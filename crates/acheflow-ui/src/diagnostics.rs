//! Local diagnostic history and the Settings / Logs viewer.
use acheflow_media_api::diagnostics::{LogBuffer, LogEntry};
use dioxus_native::prelude::*;
use std::sync::OnceLock;

fn buffer() -> &'static LogBuffer {
    static LOGS: OnceLock<LogBuffer> = OnceLock::new();
    LOGS.get_or_init(LogBuffer::default)
}

pub(crate) fn record(level: &str, source: &str, message: &str) {
    #[cfg(target_arch = "wasm32")]
    let now = js_sys::Date::now() as u64;
    #[cfg(not(target_arch = "wasm32"))]
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    buffer().push(now, level, source, message);
}

pub(crate) fn observe(
    live: crate::live_client::LiveClient,
    control: crate::control_client::ControlClient,
) {
    use_hook(|| record("info", "client", "Client session started"));
    use_effect(move || {
        let connection = live.connection.read();
        record(
            if connection.is_some() { "warn" } else { "info" },
            "connection",
            connection
                .as_deref()
                .unwrap_or("Connected to workspace server"),
        );
    });
    use_effect(move || {
        if let Some(error) = live.error.read().as_deref() {
            record("error", "media", error);
        }
    });
    use_effect(move || {
        if let Some(error) = control.error.read().as_deref() {
            record("error", "control", error);
        }
    });
}

#[component]
pub(crate) fn TabButton(
    id: &'static str,
    selected: bool,
    label: String,
    onclick: EventHandler<()>,
) -> Element {
    rsx! {button {id,class:if selected {"selected"} else {""},"aria-pressed":selected.to_string(),
        onclick:move |_|onclick.call(()),
        onkeydown:move |event|{if !event.is_auto_repeating() && (event.key()==Key::Named(NamedKey::Enter)||event.key()==Key::Character(" ".into())) {event.prevent_default();onclick.call(());}},
        "{label}"
    }}
}

fn render_entries(entries: &[LogEntry], filter: &str) -> String {
    let filter = filter.to_lowercase();
    entries
        .iter()
        .filter_map(|entry| {
            let time = chrono::DateTime::from_timestamp_millis(entry.timestamp_ms as i64)
                .map(|time| time.format("%H:%M:%S").to_string())
                .unwrap_or_else(|| "—".into());
            let repeats = if entry.repetitions > 1 {
                format!(" (×{})", entry.repetitions)
            } else {
                String::new()
            };
            let line = format!(
                "[{time} UTC] {} {}: {}{repeats}",
                entry.level.to_uppercase(),
                entry.source,
                entry.message
            );
            line.to_lowercase().contains(&filter).then_some(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[component]
pub(crate) fn LogsPanel(project: Signal<crate::project::Workspace>) -> Element {
    let mut source = use_signal(|| "Client");
    let mut paused = use_signal(|| false);
    let mut filter = use_signal(String::new);
    let mut entries = use_signal(Vec::<LogEntry>::new);
    let mut error = use_signal(String::new);
    let mut loaded_source = use_signal(|| "Client");
    let target = use_memo(move || crate::media::server_url(&project.read(), "/api/server/logs"));
    use_resource(move || {
        let source = source();
        let paused = paused();
        let target = target();
        if source != *loaded_source.peek() || !paused {
            entries.set(vec![]);
            error.set(String::new());
            loaded_source.set(source);
        }
        async move {
            if paused {
                return;
            }
            loop {
                if source == "Client" {
                    entries.set(buffer().snapshot());
                } else {
                    let result = async {
                        let url = target.as_ref().map_err(|_| "Invalid server address")?;
                        reqwest::Client::new().get(url).timeout(std::time::Duration::from_secs(3))
                            .send().await.map_err(|_| "Cannot reach server log. Check the connection.")?
                            .error_for_status().map_err(|_| "Server log is unavailable. Update the server to a version with log support.")?
                            .json::<Vec<LogEntry>>().await.map_err(|_| "Invalid server log response")
                    }.await;
                    match result {
                        Ok(logs) => {
                            entries.set(logs);
                            error.set(String::new());
                        }
                        Err(message) => error.set(message.into()),
                    }
                }
                crate::scene_preview::pause(1000).await;
            }
        }
    });
    let text = render_entries(&entries.read(), &filter());
    rsx! {section {class:"panel logs-panel",
        div {class:"panel-heading",h2 {"Logs"}span {class:"badge",if paused(){"Paused"}else{"Updating every second"}}}
        div {class:"padded",
            nav {class:"settings-tabs","aria-label":"Log source",
                TabButton {id:"logs-client",selected:source()=="Client",label:"Client",onclick:move |_|source.set("Client")}
                TabButton {id:"logs-server",selected:source()=="Server",label:"Server",onclick:move |_|source.set("Server")}
            }
            div {class:"logs-toolbar",
                label {class:"field",span {"Search logs"}input {id:"logs-filter",r#type:"search",value:filter(),oninput:move|event|filter.set(event.value())}}
                TabButton {id:"logs-pause",selected:paused(),label:if paused(){"Resume"}else{"Pause"},onclick:move |_|paused.set(!paused())}
            }
            p {class:"form-help",if source()=="Client"{"This client session: connections, commands and extension runtime events."}else{"Selected server: startup, API operations and runtime events. History resets when the server restarts."}}
            if !error().is_empty(){p {class:"field-error",role:"alert","{error}"}}
            pre {id:"diagnostic-log",class:"diagnostic-log",tabindex:"0","aria-label":format!("{} log entries",source()),
                if text.is_empty(){if filter().is_empty(){"No log entries yet."}else{"No entries match this search."}}else{"{text}"}
            }
            if source()=="Server"{crate::server_settings::StartupLogs {}}
        }
    }}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn log_search_matches_levels_sources_and_messages() {
        let entries = vec![LogEntry {
            timestamp_ms: 1000,
            level: "error".into(),
            source: "extension".into(),
            message: "HTTP/2 negotiation failed".into(),
            repetitions: 2,
        }];
        assert!(
            render_entries(&entries, "http/2")
                .contains("ERROR extension: HTTP/2 negotiation failed (×2)")
        );
        assert!(!render_entries(&entries, "ERROR").is_empty());
        assert!(render_entries(&entries, "unrelated").is_empty());
    }
}
