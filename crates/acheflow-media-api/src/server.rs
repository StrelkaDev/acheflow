//! Server-wide listener settings. Certificate values are server-local PEM paths.
use serde::{Deserialize, Serialize};

/// Desktop readiness contract. The launch identifier correlates a child, not an
/// authorization credential; it grants no access to control operations.
pub const SERVER_PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub protocol_version: u32,
    pub version: String,
    pub process_id: u32,
    pub managed_instance: Option<String>,
    pub http_address: std::net::SocketAddr,
    pub rtmp_address: std::net::SocketAddr,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TlsFiles {
    pub cert: String,
    pub key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerSettings {
    pub http_host: String,
    pub http_port: u16,
    pub rtmp_host: String,
    pub rtmp_port: u16,
    pub http_tls: TlsFiles,
    pub rtmp_tls: TlsFiles,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServerSettingsStatus {
    pub active: ServerSettings,
    pub saved: ServerSettings,
    pub restart_required: bool,
    pub overrides: Vec<String>,
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            http_host: "127.0.0.1".into(),
            http_port: 21295,
            rtmp_host: "127.0.0.1".into(),
            rtmp_port: 1935,
            http_tls: TlsFiles::default(),
            rtmp_tls: TlsFiles::default(),
        }
    }
}

impl ServerSettings {
    pub fn validate(&self) -> Result<(), String> {
        for (name, host, port, tls) in [
            ("HTTP", &self.http_host, self.http_port, &self.http_tls),
            ("RTMP", &self.rtmp_host, self.rtmp_port, &self.rtmp_tls),
        ] {
            let valid_host = host.parse::<std::net::IpAddr>().is_ok()
                || (host.len() <= 253
                    && host.split('.').all(|label| {
                        !label.is_empty()
                            && label.len() <= 63
                            && !label.starts_with('-')
                            && !label.ends_with('-')
                            && label
                                .bytes()
                                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
                    }));
            if !valid_host || port == 0 {
                return Err(format!(
                    "{name}: enter a host/IP without a scheme or port, and a port from 1 to 65535"
                ));
            }
            if tls.cert.is_empty() != tls.key.is_empty() {
                return Err(format!(
                    "{name}: certificate and private key must be supplied together"
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_partial_settings_use_new_port() {
        let settings: ServerSettings = serde_json::from_str(r#"{"rtmp_port":1940}"#).unwrap();
        assert_eq!(settings.http_host, "127.0.0.1");
        assert_eq!(settings.http_port, 21295);
        assert_eq!(settings.rtmp_port, 1940);
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn reject_invalid_hosts_ports_and_incomplete_tls_pairs() {
        for host in [
            "",
            "https://localhost",
            "localhost/path",
            "host name",
            "[::1]",
            "localhost:80",
        ] {
            let settings = ServerSettings {
                rtmp_host: host.into(),
                ..Default::default()
            };
            assert!(settings.validate().is_err(), "{host}");
        }
        for host in ["localhost", "0.0.0.0", "::1", "media.example.com"] {
            let settings = ServerSettings {
                rtmp_host: host.into(),
                ..Default::default()
            };
            assert!(settings.validate().is_ok(), "{host}");
        }
        let mut settings = ServerSettings {
            http_port: 0,
            ..Default::default()
        };
        assert!(settings.validate().is_err());
        settings.http_port = 21295;
        settings.http_tls.cert = "/cert.pem".into();
        assert!(settings.validate().is_err());
        settings.http_tls.key = "/key.pem".into();
        assert!(settings.validate().is_ok());
    }
}
