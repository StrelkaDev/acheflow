//! Operator diagnostics: route templates and status codes, never request payloads.
use acheflow_media_api::diagnostics::LogBuffer;
use axum::{
    Json,
    extract::{MatchedPath, Request},
    middleware::Next,
    response::Response,
};
use std::sync::OnceLock;

fn buffer() -> &'static LogBuffer {
    static LOGS: OnceLock<LogBuffer> = OnceLock::new();
    LOGS.get_or_init(LogBuffer::default)
}

pub(crate) fn record(level: &str, source: &str, message: &str) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    buffer().push(now, level, source, message);
}

pub(crate) async fn get() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(buffer().snapshot()),
    )
}

pub(crate) async fn requests(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    // MatchedPath is a router template. URI paths/queries may contain OAuth data.
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_owned());
    let response = next.run(request).await;
    let status = response.status();
    if let Some(route) = route
        && route.starts_with("/api/")
        && route != "/api/server/logs"
        && (method != axum::http::Method::GET || !status.is_success())
    {
        record(
            if status.is_server_error() {
                "error"
            } else if status.is_client_error() {
                "warn"
            } else {
                "info"
            },
            "http",
            &format!("{method} {route} → {}", status.as_u16()),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::Request as HttpRequest,
        routing::{get, post},
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn api_logs_use_route_templates_and_never_capture_query_headers_or_body() {
        let app = Router::new()
            .route(
                "/api/example/{id}",
                post(|| async { axum::http::StatusCode::BAD_REQUEST }),
            )
            .route("/api/server/logs", get(super::get))
            .layer(axum::middleware::from_fn(requests));
        app.clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/example/private-marker?access_token=private-marker")
                    .header("authorization", "Bearer private-marker")
                    .body(Body::from("client_secret=private-marker"))
                    .unwrap(),
            )
            .await
            .unwrap();
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/server/logs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.headers()["cache-control"], "no-store");
        let text = String::from_utf8(
            to_bytes(response.into_body(), 256 * 1024)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(text.contains("POST /api/example/{id} → 400"));
        assert!(!text.contains("private-marker"));
        assert!(!text.contains("GET /api/server/logs"));
    }
}
