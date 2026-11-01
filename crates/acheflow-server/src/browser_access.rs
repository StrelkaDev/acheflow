//! Browser access from explicitly trusted UI origins; same-origin/native need no opt-in.
use axum::http::{HeaderValue, Method, header::CONTENT_TYPE};
use tower_http::cors::CorsLayer;

pub fn from_env() -> Result<CorsLayer, String> {
    origins(&std::env::var("ACHEFLOW_UI_ORIGINS").unwrap_or_default())
}

fn origins(value: &str) -> Result<CorsLayer, String> {
    let origins = value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|origin| {
            let normalized = acheflow_media_api::workspace::normalize_server_url(origin)?;
            let url = url::Url::parse(&normalized).map_err(|e| e.to_string())?;
            if url.path() != "/" {
                return Err(
                    "ACHEFLOW_UI_ORIGINS must contain HTTP(S) origins without paths.".into(),
                );
            }
            HeaderValue::from_str(&url.origin().ascii_serialization()).map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([CONTENT_TYPE]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn response(allowed: &str, origin: &str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = axum::Router::new()
            .route("/api/live/config", axum::routing::put(|| async { "ok" }))
            .layer(origins(allowed).unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
        socket.write_all(format!("OPTIONS /api/live/config HTTP/1.1\r\nHost: {address}\r\nOrigin: {origin}\r\nAccess-Control-Request-Method: PUT\r\nAccess-Control-Request-Headers: content-type\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        let mut bytes = Vec::new();
        socket.read_to_end(&mut bytes).await.unwrap();
        task.abort();
        String::from_utf8(bytes).unwrap().to_lowercase()
    }

    #[tokio::test]
    async fn cross_origin_api_requests_require_an_explicitly_trusted_ui() {
        let allowed = response(
            "https://studio.example, http://localhost:8081",
            "https://studio.example",
        )
        .await;
        assert!(allowed.contains("access-control-allow-origin: https://studio.example"));
        assert!(allowed.contains("access-control-allow-methods: get,post,put,delete"));
        assert!(allowed.contains("access-control-allow-headers: content-type"));
        for trusted in ["", "https://studio.example"] {
            assert!(
                !response(trusted, "https://untrusted.example")
                    .await
                    .contains("access-control-allow-origin:")
            );
        }
    }

    #[test]
    fn invalid_origins_fail_at_startup() {
        for value in [
            "*",
            "null",
            "https://host/path",
            "http://user:pass@host",
            "https://host?query=1",
        ] {
            assert!(origins(value).is_err(), "{value}");
        }
    }
}
