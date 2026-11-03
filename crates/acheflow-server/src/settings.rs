use acheflow_media_api::{
    live::ApiError,
    server::{ServerSettings, ServerSettingsStatus},
};
use axum::{Json, extract::State, http::StatusCode};
use clap::Parser;
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;

#[derive(Debug, Default, Parser)]
#[command(version, about = "Acheflow stream server")]
pub struct Args {
    /// Server settings JSON file.
    #[arg(long, env = "ACHEFLOW_CONFIG", default_value = "acheflow-server.json")]
    pub config: PathBuf,
    /// HTTP(S) bind host/IP, optionally including a port (IPv6: [::1]:21295).
    #[arg(long)]
    pub listen: Option<String>,
    /// HTTP(S) port; overrides the port in --listen.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
    pub port: Option<u16>,
    /// RTMP(S) bind host/IP, optionally including a port.
    #[arg(long)]
    pub rtmp_listen: Option<String>,
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
    pub rtmp_port: Option<u16>,
    /// HTTPS PEM certificate chain path on the server (requires --http-key).
    #[arg(long, env = "ACHEFLOW_HTTP_CERT")]
    pub http_cert: Option<String>,
    /// HTTPS PEM private key path on the server.
    #[arg(long, env = "ACHEFLOW_HTTP_KEY")]
    pub http_key: Option<String>,
    /// RTMPS PEM certificate chain path on the server (requires --rtmp-key).
    #[arg(long, env = "ACHEFLOW_RTMP_CERT")]
    pub rtmp_cert: Option<String>,
    /// RTMPS PEM private key path on the server.
    #[arg(long, env = "ACHEFLOW_RTMP_KEY")]
    pub rtmp_key: Option<String>,
}

pub struct SettingsState {
    pub active: ServerSettings,
    saved: Mutex<ServerSettings>,
    startup_saved: ServerSettings,
    path: PathBuf,
    overrides: Vec<String>,
}

pub fn load(path: &Path) -> Result<ServerSettings, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("Invalid settings file {}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ServerSettings::default()),
        Err(e) => Err(format!("Cannot read settings file {}: {e}", path.display())),
    }
}

fn listen(value: &str, host: &mut String, port: &mut u16) {
    if let Ok(address) = value.parse::<SocketAddr>() {
        *host = address.ip().to_string();
        *port = address.port();
    } else {
        if let Some((name, number)) = value.rsplit_once(':')
            && !name.contains(':')
            && let Ok(number) = number.parse::<u16>()
        {
            *host = name.to_owned();
            *port = number;
            return;
        }
        *host = value.to_owned();
    }
}

impl Args {
    fn apply(&self, settings: &mut ServerSettings) -> Vec<String> {
        let mut overrides = Vec::new();
        for (name, value, host, port) in [
            (
                "--listen",
                &self.listen,
                &mut settings.http_host,
                &mut settings.http_port,
            ),
            (
                "--rtmp-listen",
                &self.rtmp_listen,
                &mut settings.rtmp_host,
                &mut settings.rtmp_port,
            ),
        ] {
            if let Some(value) = value {
                listen(value, host, port);
                overrides.push(name.into());
            }
        }
        for (name, value, target) in [
            ("--port", self.port, &mut settings.http_port),
            ("--rtmp-port", self.rtmp_port, &mut settings.rtmp_port),
        ] {
            if let Some(value) = value {
                *target = value;
                overrides.push(name.into());
            }
        }
        for (name, value, target) in [
            (
                "--http-cert / ACHEFLOW_HTTP_CERT",
                &self.http_cert,
                &mut settings.http_tls.cert,
            ),
            (
                "--http-key / ACHEFLOW_HTTP_KEY",
                &self.http_key,
                &mut settings.http_tls.key,
            ),
            (
                "--rtmp-cert / ACHEFLOW_RTMP_CERT",
                &self.rtmp_cert,
                &mut settings.rtmp_tls.cert,
            ),
            (
                "--rtmp-key / ACHEFLOW_RTMP_KEY",
                &self.rtmp_key,
                &mut settings.rtmp_tls.key,
            ),
        ] {
            if let Some(value) = value {
                target.clone_from(value);
                overrides.push(name.into());
            }
        }
        overrides
    }
}

impl SettingsState {
    pub fn from_args(args: Args) -> Result<Arc<Self>, String> {
        let saved = load(&args.config)?;
        let mut active = saved.clone();
        let mut overrides = Vec::new();
        for (name, host, port) in [
            (
                "ACHEFLOW_HTTP_ADDR",
                &mut active.http_host,
                &mut active.http_port,
            ),
            (
                "ACHEFLOW_RTMP_ADDR",
                &mut active.rtmp_host,
                &mut active.rtmp_port,
            ),
        ] {
            if let Ok(value) = std::env::var(name) {
                let address: SocketAddr = value.parse().map_err(|e| format!("{name}: {e}"))?;
                *host = address.ip().to_string();
                *port = address.port();
                overrides.push(name.into());
            }
        }
        overrides.extend(args.apply(&mut active));
        active.validate()?;
        Ok(Arc::new(Self {
            active,
            startup_saved: saved.clone(),
            saved: Mutex::new(saved),
            path: args.config,
            overrides,
        }))
    }

    async fn status(&self) -> ServerSettingsStatus {
        let saved = self.saved.lock().await.clone();
        ServerSettingsStatus {
            restart_required: saved != self.active && saved != self.startup_saved,
            active: self.active.clone(),
            saved,
            overrides: self.overrides.clone(),
        }
    }

    async fn save(&self, settings: ServerSettings) -> Result<(), String> {
        settings.validate()?;
        let mut saved = self.saved.lock().await;
        let path = self.path.clone();
        let pending = settings.clone();
        tokio::task::spawn_blocking(move || persist(&path, &pending))
            .await
            .map_err(|e| format!("Settings worker failed: {e}"))??;
        *saved = settings;
        Ok(())
    }
}

fn persist(path: &Path, settings: &ServerSettings) -> Result<(), String> {
    crate::tls::load(&settings.http_tls)?;
    crate::tls::load(&settings.rtmp_tls)?;
    let bytes = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create settings directory: {e}"))?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes).map_err(|e| format!("Cannot save settings: {e}"))?;
    std::fs::rename(&temporary, path).map_err(|e| format!("Cannot replace settings: {e}"))
}

pub async fn get(State(state): State<Arc<SettingsState>>) -> Json<ServerSettingsStatus> {
    Json(state.status().await)
}

pub async fn save(
    State(state): State<Arc<SettingsState>>,
    Json(settings): Json<ServerSettings>,
) -> Result<Json<ServerSettingsStatus>, (StatusCode, Json<ApiError>)> {
    state
        .save(settings)
        .await
        .map_err(|error| (StatusCode::BAD_REQUEST, Json(ApiError { error })))?;
    Ok(Json(state.status().await))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_port_wins_over_listen_port_and_preserves_rtmp() {
        let args = Args::try_parse_from([
            "acheflow-server",
            "--listen",
            "[::1]:9000",
            "--port",
            "21296",
            "--rtmp-listen",
            "0.0.0.0",
            "--http-cert",
            "/cert.pem",
            "--http-key",
            "/key.pem",
        ])
        .unwrap();
        let mut settings = ServerSettings::default();
        args.apply(&mut settings);
        assert_eq!(
            (settings.http_host.as_str(), settings.http_port),
            ("::1", 21296)
        );
        assert_eq!(
            (settings.rtmp_host.as_str(), settings.rtmp_port),
            ("0.0.0.0", 1935)
        );
        assert_eq!(settings.http_tls.cert, "/cert.pem");
        assert!(settings.validate().is_ok());
        assert!(Args::try_parse_from(["server", "--port", "0"]).is_err());
        assert!(Args::try_parse_from(["server", "--port", "65536"]).is_err());
    }

    #[tokio::test]
    async fn settings_persist_without_changing_active_listener() {
        let path =
            std::env::temp_dir().join(format!("acheflow-settings-{}.json", std::process::id()));
        let state = SettingsState {
            active: ServerSettings::default(),
            startup_saved: ServerSettings::default(),
            saved: Mutex::new(ServerSettings::default()),
            path: path.clone(),
            overrides: vec![],
        };
        let settings = ServerSettings {
            rtmp_host: "0.0.0.0".into(),
            rtmp_port: 1940,
            ..Default::default()
        };
        state.save(settings.clone()).await.unwrap();
        assert_eq!(load(&path).unwrap(), settings);
        assert_eq!(state.active.rtmp_port, 1935);
        assert!(state.status().await.restart_required);
        let restarted = SettingsState {
            active: settings.clone(),
            startup_saved: settings.clone(),
            saved: Mutex::new(settings.clone()),
            path: path.clone(),
            overrides: vec![],
        };
        assert!(!restarted.status().await.restart_required);
        let mut invalid = settings.clone();
        invalid.http_tls.cert = "/missing.pem".into();
        invalid.http_tls.key = "/missing.key".into();
        assert!(state.save(invalid).await.is_err());
        assert_eq!(load(&path).unwrap(), settings);
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod storage_tests {
    use super::*;
    #[tokio::test]
    async fn custom_settings_path_is_created_and_restored_by_a_new_server_state() {
        let root = std::env::temp_dir().join(format!(
            "acheflow-server-settings-{}",
            acheflow_media_api::workspace::new_token()
        ));
        let path = root.join("nested/server.json");
        let state = SettingsState::from_args(Args {
            config: path.clone(),
            port: Some(22295),
            rtmp_port: Some(2935),
            ..Default::default()
        })
        .unwrap();
        let settings = ServerSettings {
            http_port: 22296,
            rtmp_port: 2936,
            ..Default::default()
        };
        state.save(settings.clone()).await.unwrap();
        assert_eq!(state.active.http_port, 22295);
        let restarted = SettingsState::from_args(Args {
            config: path.clone(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(restarted.status().await.saved, settings);
        assert!(!restarted.status().await.restart_required);
        let before = std::fs::read(&path).unwrap();
        let invalid = ServerSettings {
            http_port: 0,
            ..settings
        };
        assert!(state.save(invalid).await.is_err());
        assert_eq!(std::fs::read(path).unwrap(), before);
        std::fs::remove_dir_all(root).unwrap();
    }
}
