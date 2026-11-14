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
    if !app.config.trusted_request(host, origin) {
        return error(
            StatusCode::FORBIDDEN,
            "Only a configured demo origin is allowed",
        );
    }
    if request.method() != Method::GET
        && request.method() != Method::HEAD
        && request
            .headers()
            .get("X-Gnorix-Token")
            .and_then(|value| value.to_str().ok())
            != Some(app.snapshot().token.as_str())
    {
        return error(StatusCode::FORBIDDEN, "Missing demo session token");
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
    use std::{
        io::{Read, Write},
        net::{SocketAddr, TcpStream},
        time::Duration,
    };

    use super::*;
    use crate::config::Config;

    #[tokio::test]
    async fn public_proxy_requests_keep_origin_and_session_token_protection() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let config = Config::parse([
            "--port".into(),
            address.port().to_string(),
            "--public-origin".into(),
            "https://gnorix.online".into(),
        ])
        .unwrap();
        let app = App::new(config).unwrap();
        let token = app.snapshot().token;
        // The HTTP guard needs no sandbox, contracts, or biometric enrollment.
        let server = tokio::spawn(async move { axum::serve(listener, router(app)).await.unwrap() });
        let result = tokio::task::spawn_blocking(move || {
            let public = "Host: gnorix.online\r\nOrigin: https://gnorix.online\r\n";
            assert_eq!(request(address, "GET", "/api/state", public), 200);
            assert_eq!(request(address, "HEAD", "/", "Host: gnorix.online\r\n"), 200);
            assert_eq!(request(address, "HEAD", "/api/state", public), 200);
            assert_eq!(request(address, "HEAD", "/", "Host: gnorix.online\r\nOrigin: https://evil.example\r\n"), 403);
            assert_eq!(request(address, "GET", "/api/state", &format!("Host: {address}\r\n")), 200);
            assert_eq!(request(address, "GET", "/api/state", "Host: gnorix.online\r\nOrigin: https://evil.example\r\n"), 403);
            assert_eq!(request(address, "GET", "/api/state", "Host: evil.example\r\nX-Forwarded-Host: gnorix.online\r\nX-Forwarded-Proto: https\r\n"), 403);
            assert_eq!(request(address, "POST", "/api/refresh", public), 403);
            assert_eq!(request(address, "POST", "/api/refresh", &format!("{public}X-Gnorix-Token: wrong\r\n")), 403);
            assert_eq!(request(address, "POST", "/api/refresh", &format!("Host: gnorix.online\r\nOrigin: https://evil.example\r\nX-Gnorix-Token: {token}\r\n")), 403);
            // The valid request reaches the service, which is still starting.
            assert_eq!(request(address, "POST", "/api/refresh", &format!("{public}X-Gnorix-Token: {token}\r\n")), 409);
        }).await;
        server.abort();
        let _ = server.await;
        result.unwrap();
    }

    fn request(address: SocketAddr, method: &str, path: &str, headers: &str) -> u16 {
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(stream, "{method} {path} HTTP/1.1\r\n{headers}Content-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response.split_whitespace().nth(1).unwrap().parse().unwrap()
    }
}
