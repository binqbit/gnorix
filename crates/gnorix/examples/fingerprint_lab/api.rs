//! Loopback HTTP adapter and asynchronous jobs around the synchronous lab engine.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, State, rejection::JsonRejection},
    http::{HeaderValue, Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use serde_json::json;

use super::{
    Result,
    engine::Engine,
    model::{LabConfig, LabData, PrepareRequest, PreparedId, SignRequest, VerifyRequest},
};

const SOURCE: &[u8] = include_bytes!("../../tests/assets/test-fingerprint.png");

#[derive(Clone)]
struct App {
    token: Arc<String>,
    authority: Arc<String>,
    engine: Arc<Mutex<Engine>>,
    snapshot: Arc<Mutex<Snapshot>>,
    sequence: Arc<AtomicU64>,
}

#[derive(Clone, Serialize)]
struct Job {
    id: u64,
    state: &'static str,
    phase: String,
    error: Option<String>,
}

#[derive(Clone, Serialize)]
struct Snapshot {
    token: String,
    image: Image,
    defaults: LabConfig,
    #[serde(flatten)]
    data: LabData,
    job: Option<Job>,
}

#[derive(Clone, Serialize)]
struct Image {
    url: &'static str,
    width: u32,
    height: u32,
}

enum Command {
    Prepare(PrepareRequest),
    Enroll(PreparedId),
    Sign(SignRequest),
    Verify(VerifyRequest),
}

pub(super) fn router(engine: Engine, port: u16) -> Result<Router> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes)?;
    let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let app = App {
        token: Arc::new(token.clone()),
        authority: Arc::new(format!("127.0.0.1:{port}")),
        snapshot: Arc::new(Mutex::new(Snapshot {
            token,
            image: Image {
                url: "/fingerprint.png",
                width: 1086,
                height: 1448,
            },
            defaults: LabConfig::default(),
            data: engine.view(),
            job: None,
        })),
        engine: Arc::new(Mutex::new(engine)),
        sequence: Arc::new(AtomicU64::new(1)),
    };
    Ok(Router::new()
        .route("/", get(index))
        .route("/app.css", get(css))
        .route("/app.js", get(javascript))
        .route("/logo.svg", get(logo))
        .route("/fingerprint.png", get(source))
        .route("/api/state", get(state))
        .route("/api/prepare", post(prepare))
        .route("/api/enroll", post(enroll))
        .route("/api/sign", post(sign))
        .route("/api/verify", post(verify))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn_with_state(app.clone(), guard))
        .with_state(app))
}

async fn guard(State(app): State<App>, request: Request<Body>, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    let alternate = app.authority.replace("127.0.0.1", "localhost");
    if host != Some(app.authority.as_str()) && host != Some(alternate.as_str()) {
        return failure(StatusCode::FORBIDDEN, "Unexpected Host");
    }
    if let Some(origin) = request.headers().get(header::ORIGIN) {
        let expected = format!("http://{}", host.unwrap_or_default());
        if origin.to_str().ok() != Some(expected.as_str()) {
            return failure(StatusCode::FORBIDDEN, "Unexpected Origin");
        }
    }
    if request.method() != axum::http::Method::GET
        && request
            .headers()
            .get("X-Gnorix-Token")
            .and_then(|value| value.to_str().ok())
            != Some(app.token.as_str())
    {
        return failure(StatusCode::FORBIDDEN, "Missing session token");
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "X-Content-Type-Options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("Referrer-Policy", HeaderValue::from_static("no-referrer"));
    headers.insert("Content-Security-Policy", HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' blob:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'"));
    response
}

async fn state(State(app): State<App>) -> Response {
    match app.snapshot.lock() {
        Ok(snapshot) => Json(snapshot.clone()).into_response(),
        Err(_) => failure(StatusCode::INTERNAL_SERVER_ERROR, "State unavailable"),
    }
}

fn payload<T>(
    input: std::result::Result<Json<T>, JsonRejection>,
) -> std::result::Result<T, (StatusCode, String)> {
    input
        .map(|Json(value)| value)
        .map_err(|error| (error.status(), error.body_text()))
}

async fn prepare(
    State(app): State<App>,
    input: std::result::Result<Json<PrepareRequest>, JsonRejection>,
) -> Response {
    match payload(input) {
        Ok(value) => launch(app, Command::Prepare(value)),
        Err((status, message)) => failure(status, &message),
    }
}
async fn enroll(
    State(app): State<App>,
    input: std::result::Result<Json<PreparedId>, JsonRejection>,
) -> Response {
    match payload(input) {
        Ok(value) => launch(app, Command::Enroll(value)),
        Err((status, message)) => failure(status, &message),
    }
}
async fn sign(
    State(app): State<App>,
    input: std::result::Result<Json<SignRequest>, JsonRejection>,
) -> Response {
    match payload(input) {
        Ok(value) => launch(app, Command::Sign(value)),
        Err((status, message)) => failure(status, &message),
    }
}
async fn verify(
    State(app): State<App>,
    input: std::result::Result<Json<VerifyRequest>, JsonRejection>,
) -> Response {
    match payload(input) {
        Ok(value) => launch(app, Command::Verify(value)),
        Err((status, message)) => failure(status, &message),
    }
}

fn launch(app: App, command: Command) -> Response {
    let id = app.sequence.fetch_add(1, Ordering::Relaxed);
    {
        let Ok(mut snapshot) = app.snapshot.lock() else {
            return failure(StatusCode::INTERNAL_SERVER_ERROR, "State unavailable");
        };
        if snapshot
            .job
            .as_ref()
            .is_some_and(|job| job.state == "running")
        {
            return failure(StatusCode::CONFLICT, "Another operation is running");
        }
        snapshot.job = Some(Job {
            id,
            state: "running",
            phase: "Preparing".into(),
            error: None,
        });
    }
    tokio::spawn(async move {
        let worker = app.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let mut engine = worker.engine.lock().map_err(|_| "Engine unavailable")?;
            let phase = |text: &str| {
                if let Ok(mut state) = worker.snapshot.lock()
                    && let Some(job) = state.job.as_mut()
                {
                    job.phase = text.to_owned();
                }
            };
            let result = match command {
                Command::Prepare(request) => engine.prepare(request, phase),
                Command::Enroll(request) => engine.enroll(request, phase),
                Command::Sign(request) => engine.sign(request, phase),
                Command::Verify(request) => engine.verify(request),
            };
            Ok::<_, String>((engine.view(), result.map_err(|error| error.to_string())))
        })
        .await;
        if let Ok(mut snapshot) = app.snapshot.lock() {
            let error = match outcome {
                Ok(Ok((view, result))) => {
                    snapshot.data = view;
                    result.err()
                }
                Ok(Err(error)) => Some(error),
                Err(error) => Some(format!("Worker stopped: {error}")),
            };
            snapshot.job = Some(Job {
                id,
                state: if error.is_some() { "error" } else { "complete" },
                phase: "Complete".into(),
                error,
            });
        }
    });
    (StatusCode::ACCEPTED, Json(json!({"job_id": id}))).into_response()
}

fn failure(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}
async fn index() -> Response {
    asset("text/html; charset=utf-8", include_bytes!("web/index.html"))
}
async fn css() -> Response {
    asset("text/css; charset=utf-8", include_bytes!("web/app.css"))
}
async fn javascript() -> Response {
    asset(
        "text/javascript; charset=utf-8",
        include_bytes!("web/app.js"),
    )
}
async fn logo() -> Response {
    asset("image/svg+xml", include_bytes!("web/logo.svg"))
}
async fn source() -> Response {
    asset("image/png", SOURCE)
}
fn asset(content_type: &'static str, bytes: &'static [u8]) -> Response {
    ([(header::CONTENT_TYPE, content_type)], bytes).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use tower::ServiceExt;

    fn test_router() -> Router {
        let image = image::GrayImage::new(2, 2);
        router(
            Engine::new(image, std::env::temp_dir().join("gnorix-unused-lab-test")),
            8765,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn host_origin_and_session_are_checked_before_mutation() {
        let app = test_router();
        for (host, origin) in [
            ("attacker.invalid", None),
            ("127.0.0.1:8765", Some("https://attacker.invalid")),
            ("127.0.0.1:8765", None),
        ] {
            let mut request = Request::post("/api/enroll")
                .header("Host", host)
                .header("Content-Type", "application/json");
            if let Some(origin) = origin {
                request = request.header("Origin", origin);
            }
            let response = app
                .clone()
                .oneshot(request.body(Body::from(r#"{"prepared_id":1}"#)).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
    }

    #[tokio::test]
    async fn malformed_requests_return_json_and_static_assets_are_self_contained() {
        let app = test_router();
        let response = app
            .clone()
            .oneshot(
                Request::get("/api/state")
                    .header("Host", "127.0.0.1:8765")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
        let state: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let token = state["token"].as_str().unwrap();
        let response = app
            .clone()
            .oneshot(
                Request::post("/api/enroll")
                    .header("Host", "127.0.0.1:8765")
                    .header("X-Gnorix-Token", token)
                    .header("Content-Type", "application/json")
                    .body(Body::from("{"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(response.into_body(), 10_000).await.unwrap();
        assert!(serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["error"].is_string());
        for path in ["/", "/app.js", "/app.css", "/logo.svg", "/fingerprint.png"] {
            let response = app
                .clone()
                .oneshot(
                    Request::get(path)
                        .header("Host", "127.0.0.1:8765")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
    }
}
