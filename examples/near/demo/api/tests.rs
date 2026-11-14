use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

use tokio::task::JoinHandle;

use super::*;
use crate::config::Config;

struct Server {
    address: SocketAddr,
    app: Demo,
    task: JoinHandle<()>,
}

impl Server {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let config = Config::parse([
            "--port".into(),
            address.port().to_string(),
            "--public-origin".into(),
            "https://gnorix.online".into(),
        ])
        .unwrap();
        // No sandbox, biometric enrollment, or contract deployment is needed
        // to exercise the actual HTTP session boundary.
        let app = Demo::new(config).unwrap();
        let routes = router(app.clone());
        let task = tokio::spawn(async move { axum::serve(listener, routes).await.unwrap() });
        Self { address, app, task }
    }

    async fn request(&self, method: &str, path: &str, headers: &str, body: &str) -> HttpResponse {
        let address = self.address;
        let request = format!(
            "{method} {path} HTTP/1.1\r\n{headers}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        tokio::task::spawn_blocking(move || exchange(address, &request))
            .await
            .unwrap()
    }

    async fn get(&self, path: &str, cookie: Option<&str>) -> HttpResponse {
        let mut headers = format!("Host: {}\r\n", self.address);
        if let Some(cookie) = cookie {
            headers.push_str(&format!("Cookie: {cookie}\r\n"));
        }
        self.request("GET", path, &headers, "").await
    }

    async fn open_session(&self) -> (String, String) {
        let response = self.get("/api/state", None).await;
        assert_eq!(response.status, 200);
        let cookie = response.cookie();
        let token = response.json()["token"].as_str().unwrap().to_owned();
        (cookie, token)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn cookie(&self) -> String {
        self.header("set-cookie")
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned()
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

fn exchange(address: SocketAddr, request: &str) -> HttpResponse {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
    let boundary = bytes
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .unwrap();
    let headers = std::str::from_utf8(&bytes[..boundary]).unwrap();
    let mut lines = headers.lines();
    let status = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .map(|line| {
            let (name, value) = line.split_once(':').unwrap();
            (name.to_owned(), value.trim().to_owned())
        })
        .collect();
    HttpResponse {
        status,
        headers,
        body: bytes[boundary + 4..].to_vec(),
    }
}

fn session_id(cookie: &str) -> &str {
    cookie.strip_prefix(&format!("{SESSION_COOKIE}=")).unwrap()
}

#[tokio::test]
async fn only_get_state_creates_sessions() {
    let server = Server::new().await;
    for path in [
        "/",
        "/app.css",
        "/app.js",
        "/proof.js",
        "/logo.svg",
        "/api/fingerprints/0",
    ] {
        let response = server.get(path, None).await;
        assert_eq!(response.status, 200, "{path}");
        assert!(response.header("set-cookie").is_none());
    }
    let host = format!("Host: {}\r\n", server.address);
    assert_eq!(server.request("HEAD", "/", &host, "").await.status, 200);
    assert_eq!(
        server.request("HEAD", "/api/state", &host, "").await.status,
        403
    );
    assert_eq!(
        server
            .request("POST", "/api/state", &host, "{}")
            .await
            .status,
        405
    );
    assert_eq!(server.get("/api/capture/1", None).await.status, 403);
    assert_eq!(server.app.session_count(), 0);
}

#[tokio::test]
async fn cookie_and_csrf_are_distinct_and_polling_resumes_only_its_session() {
    let server = Server::new().await;
    let (first, first_token) = server.open_session().await;
    let (second, second_token) = server.open_session().await;
    assert_ne!(first, second);
    assert_ne!(first_token, second_token);
    assert_ne!(session_id(&first), first_token);
    assert_eq!(session_id(&first).len(), 64);
    assert!(
        session_id(&first)
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
    );

    let resumed = server.get("/api/state", Some(&first)).await;
    assert_eq!(resumed.status, 200);
    assert_eq!(resumed.json()["token"], first_token);
    assert_eq!(resumed.json()["phase"], "starting");
    assert!(
        resumed.header("set-cookie").is_none(),
        "polling must not renew the hard TTL"
    );
    assert!(!String::from_utf8_lossy(&resumed.body).contains(session_id(&first)));
    assert_eq!(server.app.session_count(), 2);

    let headers = format!("Host: {}\r\nCookie: {first}\r\n", server.address);
    assert_eq!(
        server
            .request("HEAD", "/api/state", &headers, "")
            .await
            .status,
        200
    );
    assert_eq!(server.app.session_count(), 2);
}

#[tokio::test]
async fn public_cookie_is_secure_and_local_proxy_headers_cannot_change_policy() {
    let server = Server::new().await;
    let public = "Host: gnorix.online\r\nOrigin: https://gnorix.online\r\n";
    let response = server.request("GET", "/api/state", public, "").await;
    assert_eq!(response.status, 200);
    let cookie = response.header("set-cookie").unwrap();
    for attribute in [
        "Path=/api",
        "HttpOnly",
        "SameSite=Strict",
        "Max-Age=3600",
        "Secure",
    ] {
        assert!(
            cookie.split(';').any(|part| part.trim() == attribute),
            "{attribute}"
        );
    }
    assert!(!cookie.contains("Domain="));

    let local = format!(
        "Host: {}\r\nX-Forwarded-Host: gnorix.online\r\nX-Forwarded-Proto: https\r\n",
        server.address
    );
    let response = server.request("GET", "/api/state", &local, "").await;
    assert_eq!(response.status, 200);
    assert!(!response.header("set-cookie").unwrap().contains("Secure"));

    for headers in [
        "Host: gnorix.online\r\nOrigin: https://evil.example\r\n",
        "Host: evil.example\r\nX-Forwarded-Host: gnorix.online\r\nX-Forwarded-Proto: https\r\n",
        "Host: gnorix.online\r\nOrigin: null\r\n",
    ] {
        let response = server.request("GET", "/api/state", headers, "").await;
        assert_eq!(response.status, 403);
        assert_eq!(response.header("cache-control"), Some("no-store"));
        assert_eq!(response.header("x-content-type-options"), Some("nosniff"));
    }
    assert_eq!(server.app.session_count(), 2);
}

#[tokio::test]
async fn every_write_requires_its_own_cookie_and_csrf_token() {
    let server = Server::new().await;
    let (first, first_token) = server.open_session().await;
    let (_, second_token) = server.open_session().await;
    let public = "Host: gnorix.online\r\nOrigin: https://gnorix.online\r\n";
    for (path, body) in [
        ("/api/sign", r#"{"wallet":0,"recipient":1,"amount":"1"}"#),
        ("/api/submit", r#"{"prepared_id":"unprepared"}"#),
        ("/api/refresh", "{}"),
    ] {
        for credentials in [
            format!("X-Gnorix-Token: {first_token}\r\n"),
            format!("Cookie: {first}\r\n"),
            format!("Cookie: {first}\r\nX-Gnorix-Token: {second_token}\r\n"),
        ] {
            assert_eq!(
                server
                    .request("POST", path, &format!("{public}{credentials}"), body)
                    .await
                    .status,
                403
            );
        }
        let valid = format!("{public}Cookie: {first}\r\nX-Gnorix-Token: {first_token}\r\n");
        // Correct credentials reach the still-starting service.
        assert_eq!(server.request("POST", path, &valid, body).await.status, 409);
        let foreign_origin = format!(
            "Host: gnorix.online\r\nOrigin: https://evil.example\r\nCookie: {first}\r\nX-Gnorix-Token: {first_token}\r\n"
        );
        assert_eq!(
            server
                .request("POST", path, &foreign_origin, body)
                .await
                .status,
            403
        );
    }
    assert_eq!(server.app.session_count(), 2);
}

#[tokio::test]
async fn capture_ids_are_scoped_to_the_cookie_session() {
    let server = Server::new().await;
    let (first, _) = server.open_session().await;
    let (second, _) = server.open_session().await;
    let a = server.app.existing_session(session_id(&first)).unwrap();
    let b = server.app.existing_session(session_id(&second)).unwrap();
    a.set_capture_for_test("same-id", vec![1, 2, 3]);
    b.set_capture_for_test("same-id", vec![4, 5, 6]);
    assert_eq!(
        server.get("/api/capture/same-id", Some(&first)).await.body,
        [1, 2, 3]
    );
    assert_eq!(
        server.get("/api/capture/same-id", Some(&second)).await.body,
        [4, 5, 6]
    );
    a.set_capture_for_test("only-a", vec![7, 8, 9]);
    assert_eq!(
        server.get("/api/capture/only-a", Some(&first)).await.status,
        200
    );
    assert_eq!(
        server
            .get("/api/capture/only-a", Some(&second))
            .await
            .status,
        404
    );
    assert_eq!(server.get("/api/capture/only-a", None).await.status, 403);
    assert_eq!(server.app.session_count(), 2);
}

#[tokio::test]
async fn expired_sessions_are_not_recreated_by_posts_capture_or_head() {
    let server = Server::new().await;
    let (cookie, token) = server.open_session().await;
    server.app.expire_for_test(session_id(&cookie));
    let headers = format!(
        "Host: {}\r\nCookie: {cookie}\r\nX-Gnorix-Token: {token}\r\n",
        server.address
    );
    for (method, path, body) in [
        ("POST", "/api/refresh", "{}"),
        ("GET", "/api/capture/1", ""),
        ("HEAD", "/api/state", ""),
    ] {
        let response = server.request(method, path, &headers, body).await;
        assert_eq!(response.status, 410);
        assert!(response.header("set-cookie").is_none());
    }
    assert_eq!(server.app.session_count(), 1);

    let renewed = server.get("/api/state", Some(&cookie)).await;
    assert_eq!(renewed.status, 200);
    assert_ne!(renewed.cookie(), cookie);
    assert_ne!(renewed.json()["token"], token);
    assert_eq!(server.app.session_count(), 2);
}

#[tokio::test]
async fn malformed_or_ambiguous_cookie_ids_never_allocate_a_session() {
    let server = Server::new().await;
    let id = "a".repeat(64);
    for cookie_headers in [
        format!("Cookie: {SESSION_COOKIE}=\r\n"),
        format!("Cookie: {SESSION_COOKIE}\r\n"),
        format!("Cookie: {SESSION_COOKIE}={}\r\n", "A".repeat(64)),
        format!("Cookie: {SESSION_COOKIE}=short\r\n"),
        format!("Cookie: {SESSION_COOKIE}={id}; {SESSION_COOKIE}={id}\r\n"),
        format!("Cookie: {SESSION_COOKIE}={id}\r\nCookie: {SESSION_COOKIE}={id}\r\n"),
    ] {
        let headers = format!("Host: {}\r\n{cookie_headers}", server.address);
        assert_eq!(
            server
                .request("GET", "/api/state", &headers, "")
                .await
                .status,
            403
        );
    }
    assert_eq!(server.app.session_count(), 0);

    let headers = format!(
        "Host: {}\r\nCookie: {SESSION_COOKIE}={id}\r\nX-Gnorix-Token: unknown\r\n",
        server.address
    );
    for (method, path, body) in [
        ("POST", "/api/refresh", "{}"),
        ("GET", "/api/capture/1", ""),
        ("HEAD", "/api/state", ""),
    ] {
        assert_eq!(
            server.request(method, path, &headers, body).await.status,
            410
        );
    }
    assert_eq!(server.app.session_count(), 0);

    let cookie = format!("unrelated=value; {SESSION_COOKIE}={id}");
    let response = server.get("/api/state", Some(&cookie)).await;
    assert_eq!(response.status, 200);
    assert_ne!(session_id(&response.cookie()), id);
    assert_eq!(server.app.session_count(), 1);
}

#[tokio::test]
async fn capacity_has_retry_after_and_does_not_block_existing_sessions() {
    let server = Server::new().await;
    let mut cookies = Vec::new();
    for _ in 0..16 {
        let (cookie, _) = server.open_session().await;
        cookies.push(cookie);
    }
    let full = server.get("/api/state", None).await;
    assert_eq!(full.status, 503);
    assert_eq!(full.header("retry-after"), Some("60"));
    assert!(full.header("set-cookie").is_none());
    assert_eq!(
        server.get("/api/state", Some(&cookies[1])).await.status,
        200
    );
    assert_eq!(server.get("/", None).await.status, 200);
    assert_eq!(server.app.session_count(), 16);
}
