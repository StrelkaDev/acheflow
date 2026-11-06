//! Bounded loopback browser/callback primitive; it does not interpret OAuth fields.
use std::{collections::BTreeMap, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
pub fn open(url: &str) -> Result<(), String> {
    webbrowser::open(url).map_err(|_| "Could not open the system browser.".into())
}
pub struct Callback {
    listener: std::net::TcpListener,
}
impl Callback {
    pub async fn bind() -> Result<Self, String> {
        Self::prepare()
    }
    pub fn prepare() -> Result<Self, String> {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| "Could not allocate a local browser callback.".to_owned())?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "Could not prepare the local browser callback.")?;
        Ok(Self { listener })
    }
    pub fn uri(&self) -> String {
        format!(
            "http://127.0.0.1:{}/oauth/callback",
            self.listener
                .local_addr()
                .expect("bound callback address")
                .port()
        )
    }
    pub fn open(&self, url: &str) -> Result<(), String> {
        open(url)
    }
    pub async fn receive(self) -> Result<serde_json::Value, String> {
        let listener =
            TcpListener::from_std(self.listener).map_err(|_| "Local callback unavailable.")?;
        let encoded = tokio::time::timeout(Duration::from_secs(300), Self::read(listener))
            .await
            .map_err(|_| "Browser callback timed out.")??;
        decode(&encoded)
    }
    async fn read(listener: TcpListener) -> Result<String, String> {
        loop {
            let (mut socket, peer) = listener
                .accept()
                .await
                .map_err(|_| "Callback interrupted.")?;
            if !peer.ip().is_loopback() {
                continue;
            }
            let request = tokio::time::timeout(Duration::from_secs(5), async {
                let mut bytes = vec![];
                let mut chunk = [0; 2048];
                loop {
                    let n = socket
                        .read(&mut chunk)
                        .await
                        .map_err(|_| "Callback interrupted.")?;
                    if n == 0 {
                        return Err("Incomplete callback.");
                    }
                    bytes.extend_from_slice(&chunk[..n]);
                    if bytes.len() > 32768 {
                        return Err("Callback too large.");
                    }
                    let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") else {
                        continue;
                    };
                    let header =
                        std::str::from_utf8(&bytes[..end]).map_err(|_| "Invalid callback.")?;
                    let mut length = 0usize;
                    let mut has_length = false;
                    for line in header.lines().skip(1) {
                        let Some((name, value)) = line.split_once(':') else {
                            return Err("Invalid callback header.");
                        };
                        if name.eq_ignore_ascii_case("transfer-encoding") {
                            return Err("Unsupported callback encoding.");
                        }
                        if name.eq_ignore_ascii_case("content-length") {
                            if has_length {
                                return Err("Duplicate callback length.");
                            }
                            has_length = true;
                            length = value
                                .trim()
                                .parse()
                                .map_err(|_| "Invalid callback length.")?;
                        }
                    }
                    if length > 16384 {
                        return Err("Callback too large.");
                    }
                    if bytes.len() >= end + 4 + length {
                        return Ok((
                            header.to_owned(),
                            String::from_utf8(bytes[end + 4..end + 4 + length].to_vec())
                                .map_err(|_| "Invalid callback body.")?,
                        ));
                    }
                }
            })
            .await;
            let Ok(Ok((header, body))) = request else {
                continue;
            };
            let line = header.lines().next().unwrap_or_default();
            let mut result = None;
            let mut page = "Return to Acheflow.";
            if let Some(target) = line
                .strip_prefix("GET ")
                .and_then(|s| s.strip_suffix(" HTTP/1.1"))
            {
                if target == "/oauth/callback" {
                    page = RELAY;
                } else if let Some(query) = target.strip_prefix("/oauth/callback?") {
                    result = Some(query.into());
                } else {
                    page = "Unknown callback.";
                }
            } else if line == "POST /oauth/complete HTTP/1.1" {
                let expected = format!(
                    "http://127.0.0.1:{}",
                    listener
                        .local_addr()
                        .map_err(|_| "Callback unavailable.")?
                        .port()
                );
                let origin = header
                    .lines()
                    .skip(1)
                    .filter_map(|l| l.split_once(':'))
                    .find(|(k, _)| k.eq_ignore_ascii_case("origin"))
                    .map(|(_, v)| v.trim());
                if origin == Some(expected.as_str()) {
                    result = Some(body);
                } else {
                    page = "Invalid callback origin.";
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; script-src 'unsafe-inline'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                page.len(),
                page
            );
            let _ = socket.write_all(response.as_bytes()).await;
            if let Some(result) = result {
                return Ok(result);
            }
        }
    }
}
pub fn decode(encoded: &str) -> Result<serde_json::Value, String> {
    if encoded.len() > 16384 {
        return Err("Callback too large.".into());
    }
    let mut fields = BTreeMap::new();
    for (k, v) in url::form_urlencoded::parse(encoded.as_bytes()) {
        if fields.insert(k.into_owned(), v.into_owned()).is_some() {
            return Err("Duplicate callback field.".into());
        }
    }
    serde_json::to_value(fields).map_err(|_| "Invalid callback.".into())
}
const RELAY: &str = r#"<!doctype html><meta charset="utf-8"><title>Acheflow callback</title><p id="result">Completing authorization…</p><script>
const data=location.hash.slice(1);history.replaceState(null,'','/oauth/callback');
if(data)fetch('/oauth/complete',{method:'POST',headers:{'Content-Type':'application/x-www-form-urlencoded'},body:data}).then(()=>document.getElementById('result').textContent='Return to Acheflow.').catch(()=>document.getElementById('result').textContent='Callback failed. Reconnect in Acheflow.');
else document.getElementById('result').textContent='No response. Reconnect in Acheflow.';
</script>"#;

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn callbacks_allocate_independent_ports_even_when_the_old_fixed_port_is_busy() {
        let _occupied = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 21941)).ok();
        let first = Callback::bind().await.unwrap();
        let second = Callback::bind().await.unwrap();
        assert_ne!(first.uri(), second.uri());
        assert_ne!(first.listener.local_addr().unwrap().port(), 21941);
        assert!(first.listener.local_addr().unwrap().ip().is_loopback());
    }
    #[tokio::test]
    async fn real_loopback_rejects_wrong_origins_and_returns_bounded_fields() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let callback = Callback {
            listener: listener.into_std().unwrap(),
        };
        let request = async {
            let client = crate::transport::client().unwrap();
            let relay = client
                .get(format!("http://127.0.0.1:{port}/oauth/callback"))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap();
            assert!(relay.contains("history.replaceState"));
            let invalid = client
                .post(format!("http://127.0.0.1:{port}/oauth/complete"))
                .header("origin", "https://other.example")
                .body("state=wrong")
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap();
            assert!(invalid.contains("Invalid callback origin"));
            client
                .post(format!("http://127.0.0.1:{port}/oauth/complete"))
                .header("origin", format!("http://127.0.0.1:{port}"))
                .body("state=nonce&code=private-test-code")
                .send()
                .await
                .unwrap();
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(callback.receive(), request)
        })
        .await
        .unwrap();
        assert_eq!(result.unwrap()["state"], "nonce");
        assert!(decode("code=a&code=b").is_err());
    }
}
