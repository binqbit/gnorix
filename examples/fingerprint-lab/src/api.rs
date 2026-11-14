//! Loopback HTTP adapter and asynchronous jobs around the synchronous lab engine.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
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
    gallery::{Gallery, ImageView},
    model::{
        ArtifactRequest, ImageRequest, LabConfig, LabData, PrepareRequest, PreparedId, SignRequest,
        TestRequest, VerifyRequest,
    },
    resources::{DEFAULT_MEMORY_GIB, MAX_MEMORY_GIB, MIN_MEMORY_GIB},
};

#[derive(Clone)]
struct App {
    token: Arc<String>,
    authority: Arc<String>,
    engine: Arc<Mutex<Engine>>,
    gallery: Arc<Gallery>,
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
    image: ImageView,
    gallery: Vec<ImageView>,
    defaults: LabConfig,
    enrollment_memory: EnrollmentMemory,
    #[serde(flatten)]
    data: LabData,
    job: Option<Job>,
    #[serde(skip)]
    preview: Arc<[u8]>,
    #[serde(skip)]
    preview_revision: u64,
}

#[derive(Clone, Serialize)]
struct EnrollmentMemory {
    default_gib: u8,
    min_gib: u8,
    max_gib: u8,
}

enum Command {
    Image(ImageRequest),
    Artifacts(ArtifactRequest),
    Prepare(PrepareRequest),
    Enroll(PreparedId),
    Sign(SignRequest),
    Verify(VerifyRequest),
    Test(TestRequest),
}

fn application(engine: Engine, gallery: Gallery, port: u16) -> Result<App> {
    let image = gallery
        .get(engine.source_id())
        .ok_or("engine source is absent from gallery")?;
    if engine.dimensions() != (image.view.width, image.view.height) {
        return Err("engine source dimensions differ from gallery".into());
    }
    let image = image.view.clone();
    let (preview_revision, preview) = engine.preview();
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes)?;
    let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let app = App {
        token: Arc::new(token.clone()),
        authority: Arc::new(format!("127.0.0.1:{port}")),
        snapshot: Arc::new(Mutex::new(Snapshot {
            token,
            image,
            gallery: gallery.views(),
            defaults: LabConfig::default(),
            enrollment_memory: EnrollmentMemory {
                default_gib: DEFAULT_MEMORY_GIB,
                min_gib: MIN_MEMORY_GIB,
                max_gib: MAX_MEMORY_GIB,
            },
            data: engine.view(),
            job: None,
            preview,
            preview_revision,
        })),
        engine: Arc::new(Mutex::new(engine)),
        gallery: Arc::new(gallery),
        sequence: Arc::new(AtomicU64::new(1)),
    };
    Ok(app)
}

pub(super) fn router(engine: Engine, gallery: Gallery, port: u16) -> Result<Router> {
    let app = application(engine, gallery, port)?;
    Ok(Router::new()
        .route("/", get(index))
        .route("/app.css", get(css))
        .route("/app.js", get(javascript))
        .route("/logo.svg", get(logo))
        .route("/api/images/{id}", get(source))
        .route("/api/image", post(select_image))
        .route("/api/scan/{revision}", get(scan))
        .route("/api/state", get(state))
        .route("/api/prepare", post(prepare))
        .route("/api/enroll", post(enroll))
        .route("/api/sign", post(sign))
        .route("/api/verify", post(verify))
        .route("/api/artifacts", post(artifact_preview))
        .route("/api/test", post(test_scan))
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

async fn select_image(
    State(app): State<App>,
    input: std::result::Result<Json<ImageRequest>, JsonRejection>,
) -> Response {
    match payload(input) {
        Ok(value) if app.gallery.get(&value.id).is_some() => launch(app, Command::Image(value)),
        Ok(_) => failure(StatusCode::NOT_FOUND, "Unknown fingerprint image"),
        Err((status, message)) => failure(status, &message),
    }
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

async fn artifact_preview(
    State(app): State<App>,
    input: std::result::Result<Json<ArtifactRequest>, JsonRejection>,
) -> Response {
    match payload(input) {
        Ok(value) => launch(app, Command::Artifacts(value)),
        Err((status, message)) => failure(status, &message),
    }
}

async fn test_scan(
    State(app): State<App>,
    input: std::result::Result<Json<TestRequest>, JsonRejection>,
) -> Response {
    match payload(input) {
        Ok(value) => launch(app, Command::Test(value)),
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
                Command::Image(request) => {
                    phase("Loading fingerprint image");
                    let image = worker.gallery.get(&request.id).ok_or("Image unavailable")?;
                    engine.replace_source(&request.id, image.image.clone())
                }
                Command::Artifacts(request) => engine.change_artifacts(request, phase),
                Command::Prepare(request) => engine.prepare(request, phase),
                Command::Enroll(request) => engine.enroll(request, phase),
                Command::Sign(request) => engine.sign(request, phase),
                Command::Verify(request) => engine.verify(request),
                Command::Test(request) => engine.test(request, phase),
            };
            Ok::<_, String>((
                engine.view(),
                engine.preview(),
                worker
                    .gallery
                    .get(engine.source_id())
                    .ok_or("Image unavailable")?
                    .view
                    .clone(),
                result.map_err(|error| error.to_string()),
            ))
        })
        .await;
        if let Ok(mut snapshot) = app.snapshot.lock() {
            let error = match outcome {
                Ok(Ok((view, (revision, preview), image, result))) => {
                    snapshot.image = image;
                    snapshot.data = view;
                    snapshot.preview_revision = revision;
                    snapshot.preview = preview;
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
    asset(
        "text/html; charset=utf-8",
        include_bytes!("../web/index.html"),
    )
}
async fn css() -> Response {
    asset("text/css; charset=utf-8", include_bytes!("../web/app.css"))
}
async fn javascript() -> Response {
    asset(
        "text/javascript; charset=utf-8",
        include_bytes!("../web/app.js"),
    )
}
async fn logo() -> Response {
    asset("image/svg+xml", include_bytes!("../web/logo.svg"))
}
async fn source(State(app): State<App>, Path(id): Path<String>) -> Response {
    match app.gallery.get(&id) {
        Some(image) => (
            [(header::CONTENT_TYPE, "image/png")],
            image.png.as_ref().to_vec(),
        )
            .into_response(),
        None => failure(StatusCode::NOT_FOUND, "Unknown fingerprint image"),
    }
}

async fn scan(State(app): State<App>, Path(revision): Path<String>) -> Response {
    let Some(revision) = revision
        .strip_suffix(".png")
        .and_then(|value| value.parse::<u64>().ok())
    else {
        return failure(StatusCode::NOT_FOUND, "Unknown scan revision");
    };
    let preview = match app.snapshot.lock() {
        Ok(snapshot) if snapshot.preview_revision == revision => Arc::clone(&snapshot.preview),
        Ok(_) => return failure(StatusCode::NOT_FOUND, "Scan revision is no longer current"),
        Err(_) => return failure(StatusCode::INTERNAL_SERVER_ERROR, "State unavailable"),
    };
    (
        [(header::CONTENT_TYPE, "image/png")],
        preview.as_ref().to_vec(),
    )
        .into_response()
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
            Engine::new(image, std::env::temp_dir().join("gnorix-unused-lab-test")).unwrap(),
            Gallery::test_catalog(),
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
        assert_eq!(
            state["enrollment_memory"],
            json!({"default_gib": 2, "min_gib": 1, "max_gib": 10})
        );
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
        for path in ["/", "/app.js", "/app.css", "/logo.svg", "/api/images/test"] {
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

    #[tokio::test]
    async fn preview_png_is_revision_bound_and_matches_original_pixels_at_zero() {
        let app = test_router();
        let request = |path: &str| {
            Request::get(path)
                .header("Host", "127.0.0.1:8765")
                .body(Body::empty())
                .unwrap()
        };
        let response = app
            .clone()
            .oneshot(request("/api/scan/0.png"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
        let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
        assert_eq!(
            image::load_from_memory(&bytes).unwrap().to_luma8(),
            image::GrayImage::new(2, 2)
        );
        for path in ["/api/scan/1.png", "/api/scan/no.png", "/api/scan/0"] {
            assert_eq!(
                app.clone().oneshot(request(path)).await.unwrap().status(),
                StatusCode::NOT_FOUND
            );
        }
        let mut bad_host = request("/api/scan/0.png");
        bad_host
            .headers_mut()
            .insert(header::HOST, HeaderValue::from_static("attacker.invalid"));
        assert_eq!(
            app.oneshot(bad_host).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }

    async fn state_json(app: &Router) -> serde_json::Value {
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
        serde_json::from_slice(&to_bytes(response.into_body(), 1_000_000).await.unwrap()).unwrap()
    }

    fn image_request(id: &str, token: &str) -> Request<Body> {
        Request::post("/api/image")
            .header("Host", "127.0.0.1:8765")
            .header("X-Gnorix-Token", token)
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&json!({"id": id})).unwrap()))
            .unwrap()
    }

    async fn completed_state(app: &Router) -> serde_json::Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let state = state_json(app).await;
            if state["job"]["state"] != "running" {
                assert_eq!(state["job"]["state"], "complete", "{state}");
                return state;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "image job did not complete"
            );
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn catalog_selection_updates_image_and_preview_atomically_without_reusing_revisions() {
        let app = test_router();
        let initial = state_json(&app).await;
        assert_eq!(initial["image"]["id"], "test");
        assert_eq!(initial["gallery"].as_array().unwrap().len(), 2);
        let token = initial["token"].as_str().unwrap();
        for (id, revision, value) in [("other", 1, 200), ("test", 2, 0)] {
            let response = app.clone().oneshot(image_request(id, token)).await.unwrap();
            assert_eq!(response.status(), StatusCode::ACCEPTED);
            let state = completed_state(&app).await;
            assert_eq!(state["image"]["id"], id);
            assert_eq!(state["image"]["url"], format!("/api/images/{id}"));
            assert_eq!(state["artifact"]["revision"], revision);
            assert_eq!(state["artifact"]["level"], 0);
            for field in ["enrollment", "prepared", "result"] {
                assert!(state[field].is_null());
            }
            for path in [
                format!("/api/images/{id}"),
                format!("/api/scan/{revision}.png"),
            ] {
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
                assert_eq!(response.status(), StatusCode::OK);
                let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
                assert_eq!(
                    image::load_from_memory(&bytes)
                        .unwrap()
                        .into_luma8()
                        .as_raw(),
                    &[value; 4]
                );
            }
            let stale = format!("/api/scan/{}.png", revision - 1);
            assert_eq!(
                app.clone()
                    .oneshot(
                        Request::get(stale)
                            .header("Host", "127.0.0.1:8765")
                            .body(Body::empty())
                            .unwrap()
                    )
                    .await
                    .unwrap()
                    .status(),
                StatusCode::NOT_FOUND
            );
        }
        let before = state_json(&app).await;
        assert_eq!(
            app.clone()
                .oneshot(image_request("missing", token))
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(before, state_json(&app).await);
        assert_eq!(
            app.clone()
                .oneshot(image_request("other", "wrong-token"))
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(before, state_json(&app).await);
    }

    #[tokio::test]
    async fn source_selection_cannot_overlap_an_existing_job() {
        let gallery = Gallery::test_catalog();
        let engine = Engine::new(
            gallery.default_image().image.clone(),
            std::path::PathBuf::new(),
        )
        .unwrap();
        let app = application(engine, gallery, 8765).unwrap();
        app.snapshot.lock().unwrap().job = Some(Job {
            id: 7,
            state: "running",
            phase: "Analyzing fingerprint".into(),
            error: None,
        });
        let response = select_image(
            State(app.clone()),
            Ok(Json(ImageRequest { id: "other".into() })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(app.snapshot.lock().unwrap().image.id, "test");
        assert_eq!(app.snapshot.lock().unwrap().job.as_ref().unwrap().id, 7);
        assert_eq!(app.engine.lock().unwrap().source_id(), "test");
    }
}
