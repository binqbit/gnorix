use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;

use super::{
    fingerprint,
    model::{RefreshRequest, SignRequest, SubmitRequest},
    service::Session,
    sessions::{Demo, SESSION_LIFETIME, SessionError},
};

const SESSION_COOKIE: &str = "gnorix_demo_session";

enum SessionAccessError {
    InvalidCookie,
    MissingCookie,
    Registry(SessionError),
}

impl IntoResponse for SessionAccessError {
    fn into_response(self) -> Response {
        match self {
            Self::InvalidCookie => error(StatusCode::FORBIDDEN, "Invalid session cookie"),
            Self::MissingCookie => error(
                StatusCode::FORBIDDEN,
                "Open the demo to create a session first",
            ),
            Self::Registry(reason) => session_error(reason),
        }
    }
}

pub fn router(app: Demo) -> Router {
    let session_routes = Router::new()
        .route("/api/capture/{id}", get(capture_image))
        .route("/api/sign", post(sign))
        .route("/api/submit", post(submit))
        .route("/api/refresh", post(refresh))
        .route_layer(middleware::from_fn_with_state(app.clone(), require_session));

    Router::new()
        .route("/", get(index))
        .route("/app.css", get(css))
        .route("/app.js", get(javascript))
        .route("/proof.js", get(proof_javascript))
        .route("/logo.svg", get(logo))
        .route("/api/state", get(state))
        .route("/api/fingerprints/{id}", get(image))
        .merge(session_routes)
        .layer(DefaultBodyLimit::max(8 * 1024))
        .layer(middleware::from_fn_with_state(app.clone(), guard))
        .with_state(app)
}

async fn guard(State(app): State<Demo>, request: Request<Body>, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .map(|value| value.to_str().unwrap_or("invalid"));
    let response = if app.config.trusted_request(host, origin) {
        next.run(request).await
    } else {
        error(
            StatusCode::FORBIDDEN,
            "Only a configured demo origin is allowed",
        )
    };
    secure_response(response)
}

async fn require_session(
    State(app): State<Demo>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let session = match existing_session(&app, request.headers()) {
        Ok(session) => session,
        Err(reason) => return reason.into_response(),
    };

    if request.method() != Method::GET
        && request.method() != Method::HEAD
        && request
            .headers()
            .get("X-Gnorix-Token")
            .and_then(|value| value.to_str().ok())
            != Some(session.snapshot().token.as_str())
    {
        return error(StatusCode::FORBIDDEN, "Missing demo session token");
    }
    request.extensions_mut().insert(session);
    next.run(request).await
}

fn secure_response(mut response: Response) -> Response {
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

async fn state(State(app): State<Demo>, method: Method, headers: HeaderMap) -> Response {
    // Axum also routes HEAD to GET handlers. Only an actual GET may allocate a
    // session; health checks and other methods must leave the registry alone.
    if method != Method::GET {
        return match existing_session(&app, &headers) {
            Ok(session) => Json(session.snapshot()).into_response(),
            Err(reason) => reason.into_response(),
        };
    }

    let cookie = match session_cookie(&headers) {
        Ok(cookie) => cookie,
        Err(reason) => return reason.into_response(),
    };
    let (session, created) = match app.session(cookie) {
        Ok(session) => session,
        Err(reason) => return session_error(reason),
    };
    let mut response = Json(session.snapshot()).into_response();
    if created {
        let secure = app
            .config
            .public_origin()
            .and_then(|origin| origin.strip_prefix("https://"))
            .is_some_and(|public_host| {
                headers
                    .get(header::HOST)
                    .and_then(|host| host.to_str().ok())
                    == Some(public_host)
            });
        response
            .headers_mut()
            .insert(header::SET_COOKIE, new_session_cookie(session.id(), secure));
    }
    response
}

fn session_cookie(headers: &HeaderMap) -> Result<Option<&str>, SessionAccessError> {
    let mut session_id = None;
    for header in headers.get_all(header::COOKIE) {
        let value = header
            .to_str()
            .map_err(|_| SessionAccessError::InvalidCookie)?;
        for cookie in value.split(';') {
            let cookie = cookie.trim();
            let Some((name, value)) = cookie.split_once('=') else {
                if cookie == SESSION_COOKIE {
                    return Err(SessionAccessError::InvalidCookie);
                }
                continue;
            };
            if name.trim() != SESSION_COOKIE {
                continue;
            }
            if session_id.is_some()
                || value.len() != 64
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(SessionAccessError::InvalidCookie);
            }
            session_id = Some(value);
        }
    }
    Ok(session_id)
}

fn existing_session(app: &Demo, headers: &HeaderMap) -> Result<Session, SessionAccessError> {
    let id = session_cookie(headers)?.ok_or(SessionAccessError::MissingCookie)?;
    app.existing_session(id)
        .map_err(SessionAccessError::Registry)
}

fn new_session_cookie(id: &str, secure: bool) -> HeaderValue {
    let secure = if secure { "; Secure" } else { "" };
    let max_age = SESSION_LIFETIME.as_secs();
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}={id}; Path=/api; HttpOnly; SameSite=Strict; Max-Age={max_age}{secure}"
    ))
    .expect("registry session IDs are lowercase hex")
}

fn session_error(reason: SessionError) -> Response {
    match reason {
        SessionError::Expired => error(
            StatusCode::GONE,
            "This demo session has expired. Reload the page to start a new session",
        ),
        SessionError::Capacity => {
            let mut response = error(
                StatusCode::SERVICE_UNAVAILABLE,
                "The demo is at capacity. Please try again later",
            );
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
            response
        }
        SessionError::Internal(reason) => {
            eprintln!("Demo session error: {reason:#}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to create a demo session",
            )
        }
    }
}

async fn sign(
    Extension(session): Extension<Session>,
    input: Result<Json<SignRequest>, JsonRejection>,
) -> Response {
    launch(input, |request| session.sign(request))
}

async fn submit(
    Extension(session): Extension<Session>,
    input: Result<Json<SubmitRequest>, JsonRejection>,
) -> Response {
    launch(input, |request| session.submit(request))
}

async fn refresh(
    Extension(session): Extension<Session>,
    input: Result<Json<RefreshRequest>, JsonRejection>,
) -> Response {
    launch(input, |_| session.refresh())
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

async fn capture_image(Extension(session): Extension<Session>, Path(id): Path<String>) -> Response {
    match session.capture_image(&id) {
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
mod tests;
