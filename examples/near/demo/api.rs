use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::{HeaderValue, Method, Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;

use super::{
    fingerprint,
    model::{RefreshRequest, SignRequest, SubmitRequest},
    service::App,
};

pub fn router(app: App) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.css", get(css))
        .route("/app.js", get(javascript))
        .route("/proof.js", get(proof_javascript))
        .route("/logo.svg", get(logo))
        .route("/api/state", get(state))
        .route("/api/fingerprints/{id}", get(image))
        .route("/api/capture/{id}", get(capture_image))
        .route("/api/sign", post(sign))
        .route("/api/submit", post(submit))
        .route("/api/refresh", post(refresh))
        .layer(DefaultBodyLimit::max(8 * 1024))
        .layer(middleware::from_fn_with_state(app.clone(), guard))
        .with_state(app)
}

async fn guard(State(app): State<App>, request: Request<Body>, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .map(|value| value.to_str().unwrap_or("invalid"));
    if !trusted_request(app.port, host, origin) {
        return error(
            StatusCode::FORBIDDEN,
            "Only this local demo origin is allowed",
        );
    }
    if request.method() != Method::GET
        && request
            .headers()
            .get("X-Gnorix-Token")
            .and_then(|value| value.to_str().ok())
            != Some(app.snapshot().token.as_str())
    {
        return error(StatusCode::FORBIDDEN, "Missing local session token");
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "X-Content-Type-Options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("Referrer-Policy", HeaderValue::from_static("no-referrer"));
    headers.insert("Content-Security-Policy", HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"));
    response
}

fn trusted_request(port: u16, host: Option<&str>, origin: Option<&str>) -> bool {
    let allowed = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    host.is_some_and(|host| allowed.iter().any(|value| value == host))
        && origin.is_none_or(|origin| {
            allowed
                .iter()
                .any(|host| origin == format!("http://{host}"))
        })
}

async fn state(State(app): State<App>) -> Response {
    Json(app.snapshot()).into_response()
}

async fn sign(State(app): State<App>, input: Result<Json<SignRequest>, JsonRejection>) -> Response {
    launch(input, |request| app.sign(request))
}

async fn submit(
    State(app): State<App>,
    input: Result<Json<SubmitRequest>, JsonRejection>,
) -> Response {
    launch(input, |request| app.submit(request))
}

async fn refresh(
    State(app): State<App>,
    input: Result<Json<RefreshRequest>, JsonRejection>,
) -> Response {
    launch(input, |_| app.refresh())
}

fn launch<T>(
    input: Result<Json<T>, JsonRejection>,
    operation: impl FnOnce(T) -> anyhow::Result<()>,
) -> Response {
    match input {
        Ok(Json(request)) => match operation(request) {
            Ok(()) => (StatusCode::ACCEPTED, Json(json!({"accepted": true}))).into_response(),
            Err(reason) => error(StatusCode::CONFLICT, &reason.to_string()),
        },
        Err(reason) => error(reason.status(), &reason.body_text()),
    }
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

async fn image(Path(id): Path<usize>) -> Response {
    match fingerprint::image(id) {
        Some(bytes) => ([(header::CONTENT_TYPE, "image/png")], bytes).into_response(),
        None => error(StatusCode::NOT_FOUND, "Unknown test fingerprint"),
    }
}

async fn capture_image(State(app): State<App>, Path(id): Path<String>) -> Response {
    match app.capture_image(&id) {
        Some(bytes) => ([(header::CONTENT_TYPE, "image/png")], bytes.to_vec()).into_response(),
        None => error(StatusCode::NOT_FOUND, "This capture is no longer available"),
    }
}

async fn index() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        include_str!("../web/index.html"),
    )
}
async fn css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../web/app.css"),
    )
}
async fn javascript() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../web/app.js"),
    )
}
async fn proof_javascript() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../web/proof.js"),
    )
}
async fn logo() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml")],
        include_str!("../web/logo.svg"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_origin_guard_rejects_cross_site_and_rebinding_requests() {
        assert!(trusted_request(8780, Some("127.0.0.1:8780"), None));
        assert!(trusted_request(
            8780,
            Some("localhost:8780"),
            Some("http://localhost:8780")
        ));
        for (host, origin) in [
            (Some("example.com:8780"), None),
            (Some("localhost:8780"), Some("https://evil.example")),
            (Some("localhost:8780"), Some("null")),
            (Some("localhost:8781"), None),
            (None, None),
        ] {
            assert!(!trusted_request(8780, host, origin));
        }
    }
}
