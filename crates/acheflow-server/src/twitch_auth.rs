//! Host-owned public-client device authorization. Secrets never enter a guest.
use crate::{
    control::{ControlHost, now_ms},
    extension_host,
};
use acheflow_media_api::control::AuthorizationStatus;
use serde::Deserialize;
use serde_json::Value;
use std::{sync::Arc, time::Duration};

const SCOPES: &str = "user:read:chat user:write:chat";
#[derive(Clone)]
pub struct Credentials {
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: String,
    pub user_id: String,
    pub login: String,
    pub expires_at_ms: u64,
}
#[derive(Deserialize)]
struct Device {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}
#[derive(Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
}

pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "HTTP client initialization failed".into())
}
pub async fn bounded_json(mut response: reqwest::Response) -> Result<Value, String> {
    let mut bytes = vec![];
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "provider response interrupted")?
    {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err("provider response exceeds 1 MiB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "invalid provider response".into())
}
pub async fn begin(host: Arc<ControlHost>, owner: String, client_id: String) -> Result<(), String> {
    if !(8..=128).contains(&client_id.len())
        || !client_id.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err("invalid_argument: Twitch public client id".into());
    }
    let http = client()?;
    let response = http
        .post("https://id.twitch.tv/oauth2/device")
        .form(&[("client_id", client_id.as_str()), ("scopes", SCOPES)])
        .send()
        .await
        .map_err(|_| "unavailable: Twitch authorization service")?;
    if !response.status().is_success() {
        return Err("unavailable: Twitch rejected the public client id".into());
    }
    let device: Device = serde_json::from_value(bounded_json(response).await?)
        .map_err(|_| "invalid Twitch device response")?;
    let uri = url::Url::parse(&device.verification_uri).map_err(|_| "invalid authorization URL")?;
    if uri.scheme() != "https"
        || !matches!(uri.host_str(), Some("www.twitch.tv" | "twitch.tv"))
        || device.user_code.len() > 128
        || device.device_code.len() > 1024
    {
        return Err("invalid Twitch device response".into());
    }
    let expires_at_ms = now_ms().saturating_add(device.expires_in.min(3600) * 1000);
    let cancel = host.runtime.shutdown.child_token();
    if let Some(old) = host.auth_cancel.lock().unwrap().replace(cancel.clone()) {
        old.cancel();
    }
    host.services.lock().unwrap().authorization = Some(AuthorizationStatus {
        verification_uri: device.verification_uri.clone(),
        user_code: device.user_code.clone(),
        expires_at_ms,
    });
    tokio::spawn(async move {
        let result = tokio::select! {
            _=cancel.cancelled()=>return,
            result=finish(&http,&client_id,&device,expires_at_ms)=>result,
        };
        if !host.owns(&owner) {
            return;
        }
        host.services.lock().unwrap().authorization = None;
        match result {
            Ok(credentials) => {
                *host.credentials.write().await = Some(credentials);
                if let Err(error) =
                    extension_host::set_enabled(host.clone(), &owner, "twitch", true).await
                {
                    extension_host::failed(&host, &owner, "twitch", error);
                }
            }
            Err(error) => extension_host::failed(&host, &owner, "twitch", error),
        }
    });
    Ok(())
}
async fn finish(
    http: &reqwest::Client,
    client_id: &str,
    device: &Device,
    expires: u64,
) -> Result<Credentials, String> {
    let mut interval = device.interval.clamp(1, 60);
    while now_ms() < expires {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        let response = http
            .post("https://id.twitch.tv/oauth2/token")
            .form(&[
                ("client_id", client_id),
                ("scopes", SCOPES),
                ("device_code", device.device_code.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await
            .map_err(|_| "Twitch authorization connection failed")?;
        let success = response.status().is_success();
        let value = bounded_json(response).await?;
        if success {
            return identify(
                http,
                client_id,
                serde_json::from_value(value).map_err(|_| "invalid Twitch token response")?,
            )
            .await;
        }
        let message = value["message"]
            .as_str()
            .or(value["error"].as_str())
            .unwrap_or_default();
        match message {
            "authorization_pending" => {}
            "slow_down" => interval = (interval + 5).min(120),
            _ => return Err("Twitch authorization was denied or expired".into()),
        }
    }
    Err("Twitch authorization code expired".into())
}
async fn identify(
    http: &reqwest::Client,
    client_id: &str,
    tokens: Tokens,
) -> Result<Credentials, String> {
    if tokens.access_token.len() > 4096 || tokens.refresh_token.len() > 4096 {
        return Err("invalid token response".into());
    }
    let response = http
        .get("https://id.twitch.tv/oauth2/validate")
        .header("Authorization", format!("OAuth {}", tokens.access_token))
        .send()
        .await
        .map_err(|_| "Twitch token validation failed")?;
    if !response.status().is_success() {
        return Err("Twitch authorization is no longer valid".into());
    }
    let identity = bounded_json(response).await?;
    let scopes = identity["scopes"]
        .as_array()
        .ok_or("Twitch authorization scopes are missing")?;
    if identity["client_id"].as_str() != Some(client_id)
        || !SCOPES
            .split_whitespace()
            .all(|s| scopes.iter().any(|v| v.as_str() == Some(s)))
    {
        return Err("Twitch authorization does not grant chat access".into());
    }
    let user_id = identity["user_id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 128 && s.bytes().all(|c| c.is_ascii_digit()))
        .ok_or("Twitch user identity missing")?
        .to_owned();
    let login = identity["login"]
        .as_str()
        .unwrap_or("Twitch account")
        .chars()
        .take(128)
        .collect();
    Ok(Credentials {
        client_id: client_id.into(),
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        user_id,
        login,
        expires_at_ms: now_ms().saturating_add(tokens.expires_in.saturating_mul(1000)),
    })
}
pub async fn refresh(
    http: &reqwest::Client,
    credentials: &Credentials,
) -> Result<Credentials, String> {
    let response = http
        .post("https://id.twitch.tv/oauth2/token")
        .form(&[
            ("client_id", credentials.client_id.as_str()),
            ("refresh_token", credentials.refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|_| "Twitch token refresh failed")?;
    if !response.status().is_success() {
        return Err("Reconnect your Twitch account".into());
    }
    identify(
        http,
        &credentials.client_id,
        serde_json::from_value(bounded_json(response).await?)
            .map_err(|_| "invalid token refresh response")?,
    )
    .await
}
