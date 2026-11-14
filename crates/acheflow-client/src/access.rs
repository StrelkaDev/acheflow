//! Native device sessions are separate from provider authorization.
#[cfg(target_os = "macos")]
mod vault;
use std::{
    collections::HashMap,
    sync::{OnceLock, RwLock},
};

fn sessions() -> &'static RwLock<HashMap<String, String>> {
    static SESSIONS: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();
    SESSIONS.get_or_init(Default::default)
}
pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Cannot create server connection.".into())
}
pub fn base(endpoint: &str) -> Result<String, String> {
    let url = url::Url::parse(endpoint).map_err(|_| "Invalid server address.")?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Device authorization requires HTTPS or loopback HTTP.".into());
    }
    let path = url
        .path()
        .rsplit_once("/api/")
        .map(|(base, _)| base)
        .unwrap_or(url.path())
        .trim_end_matches('/');
    Ok(format!("{}{path}", url.origin().ascii_serialization()))
}
pub fn restore(endpoint: &str) -> Result<(), String> {
    let server = base(endpoint)?;
    // An owned local server's authority must not be replaced by an older session.
    if sessions()
        .read()
        .map_err(|_| "Device session unavailable.")?
        .contains_key(&server)
    {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    if let Some(token) = vault::load(&server)? {
        sessions()
            .write()
            .map_err(|_| "Device session unavailable.")?
            .insert(server, token);
    }
    Ok(())
}
pub fn remember(endpoint: &str, token: &str) -> Result<(), String> {
    let server = base(endpoint)?;
    #[cfg(target_os = "macos")]
    {
        vault::save(&server, token)?;
    }
    sessions()
        .write()
        .map_err(|_| "Device session unavailable.")?
        .insert(server, token.into());
    Ok(())
}
pub fn forget(endpoint: &str) -> Result<(), String> {
    sessions()
        .write()
        .map_err(|_| "Device session unavailable.")?
        .remove(&base(endpoint)?);
    #[cfg(target_os = "macos")]
    vault::remove(&base(endpoint)?)?;
    Ok(())
}
pub fn authorize(
    request: reqwest::RequestBuilder,
    endpoint: &str,
) -> Result<reqwest::RequestBuilder, String> {
    // Public discovery can still probe a not-yet-paired HTTP server.
    let Ok(server) = base(endpoint) else {
        return Ok(request);
    };
    let sessions = sessions()
        .read()
        .map_err(|_| "Device session unavailable.")?;
    Ok(match sessions.get(&server) {
        Some(token) => request.bearer_auth(token),
        None => request,
    })
}
/// Only the manager that owns a live child supplies this protected local path.
/// The endpoint's loopback address alone never grants server-side authority.
pub fn attach_owned(endpoint: &str, path: &std::path::Path) -> Result<(), String> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| "Cannot read local server authority.")?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return Err("Invalid local server authority.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("Local server authority must be private.".into());
        }
    }
    let bytes = std::fs::read(path).map_err(|_| "Cannot read local server authority.")?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid local server authority.")?;
    let token = value["owner_key"]
        .as_str()
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("Invalid local server authority.")?;
    remember(endpoint, token)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tokens_are_attached_only_to_the_matching_server_and_proxy_base_path() {
        let server = "https://access-fixture.example/proxy/api/studio";
        let endpoint = format!("{server}/api/control/state");
        sessions()
            .write()
            .unwrap()
            .insert(base(&endpoint).unwrap(), "synthetic-device-token".into());
        let own = authorize(client().unwrap().get(&endpoint), &endpoint)
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(
            own.headers()[reqwest::header::AUTHORIZATION],
            "Bearer synthetic-device-token"
        );
        for other in [
            "https://access-fixture.example/other/api/control/state",
            "https://other.example/proxy/api/studio/api/control/state",
            "http://access-fixture.example/proxy/api/studio/api/control/state",
        ] {
            let request = authorize(client().unwrap().get(other), other)
                .unwrap()
                .build()
                .unwrap();
            assert!(
                request
                    .headers()
                    .get(reqwest::header::AUTHORIZATION)
                    .is_none()
            );
        }
        sessions()
            .write()
            .unwrap()
            .remove(&base(&endpoint).unwrap());
    }
    #[test]
    fn authorization_rejects_remote_plaintext_and_credential_bearing_addresses() {
        for endpoint in [
            "http://studio.example/api/access/pair",
            "https://user:pass@studio.example/api/access/pair",
            "file:///api/access/pair",
        ] {
            assert!(base(endpoint).is_err());
        }
        assert!(base("http://localhost:21295/api/access/pair").is_ok());
        assert!(base("https://studio.example/api/access/pair").is_ok());
    }
}
