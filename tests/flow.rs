//! End-to-end tests driving the router in-process.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use greenhouse_tracker::{AppState, build_app, config::Config};
use http_body_util::BodyExt;
use tower::ServiceExt;

struct TestApp {
    app: Router,
    _dir: TempDir,
}

/// Minimal temp dir (avoids an extra dependency).
struct TempDir(std::path::PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn setup() -> TestApp {
    let dir = std::env::temp_dir().join(format!("gt-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = Config::for_tests(
        dir.join("test.db").to_string_lossy().into_owned(),
        dir.join("uploads"),
    );
    let state = AppState::new(cfg).unwrap();
    TestApp { app: build_app(state), _dir: TempDir(dir) }
}

struct Resp {
    status: StatusCode,
    location: Option<String>,
    cookie: Option<String>,
    body: String,
}

async fn send(app: &Router, req: Request<Body>) -> Resp {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let location = res.headers().get(header::LOCATION).map(|v| v.to_str().unwrap().to_string());
    let cookie = res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .find(|c| c.starts_with("gt_session=") && !c.starts_with("gt_session=;"))
        .map(|c| c.split(';').next().unwrap().to_string());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    Resp { status, location, cookie, body: String::from_utf8_lossy(&bytes).into_owned() }
}

fn get(path: &str, cookie: Option<&str>) -> Request<Body> {
    let mut b = Request::get(path).header(header::HOST, "localhost");
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    b.body(Body::empty()).unwrap()
}

fn post_form(path: &str, cookie: Option<&str>, body: &str) -> Request<Body> {
    let mut b = Request::post(path)
        .header(header::HOST, "localhost")
        .header(header::ORIGIN, "http://localhost")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    b.body(Body::from(body.to_string())).unwrap()
}

const BOUNDARY: &str = "XBOUNDARYX";

fn multipart(fields: &[(&str, &str)], photo: Option<&[u8]>) -> Vec<u8> {
    let mut out = Vec::new();
    for (k, v) in fields {
        out.extend(format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes());
    }
    if let Some(data) = photo {
        out.extend(
            format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"photos\"; filename=\"pump.png\"\r\nContent-Type: image/png\r\n\r\n")
                .as_bytes(),
        );
        out.extend(data);
        out.extend(b"\r\n");
    }
    out.extend(format!("--{BOUNDARY}--\r\n").as_bytes());
    out
}

fn post_multipart(path: &str, cookie: &str, body: Vec<u8>) -> Request<Body> {
    Request::post(path)
        .header(header::HOST, "localhost")
        .header(header::ORIGIN, "http://localhost")
        .header(header::COOKIE, cookie)
        .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={BOUNDARY}"))
        .body(Body::from(body))
        .unwrap()
}

async fn register(app: &Router, email: &str, name: &str) -> String {
    let r = send(
        app,
        post_form(
            "/register",
            None,
            &format!("email={email}&display_name={name}&password=hunter22hunter&password_confirm=hunter22hunter"),
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    r.cookie.expect("session cookie")
}

// A 1x1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 1, 0,
    0, 0, 1, 8, 6, 0, 0, 0, 0x1F, 0x15, 0xC4, 0x89, 0, 0, 0, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63,
    0, 1, 0, 0, 5, 0, 1, 0x0D, 0x0A, 0x2D, 0xB4, 0, 0, 0, 0, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

#[tokio::test]
async fn anonymous_users_are_sent_to_login() {
    let t = setup();
    let r = send(&t.app, get("/", None)).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(r.location.as_deref(), Some("/login"));
    let r = send(&t.app, get("/register", None)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("Create an account"));
}

#[tokio::test]
async fn cross_origin_posts_are_rejected() {
    let t = setup();
    let req = Request::post("/login")
        .header(header::HOST, "localhost")
        .header(header::ORIGIN, "https://evil.example")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from("email=a@b.c&password=x"))
        .unwrap();
    assert_eq!(send(&t.app, req).await.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn register_login_and_full_issue_conversation() {
    let t = setup();
    let alice = register(&t.app, "alice@example.com", "Alice").await;
    let bob = register(&t.app, "bob@example.com", "Bob").await;

    // Duplicate email is refused.
    let r = send(
        &t.app,
        post_form("/register", None, "email=ALICE@example.com&display_name=A&password=hunter22hunter&password_confirm=hunter22hunter"),
    )
    .await;
    assert!(r.body.contains("already exists"));

    // Password login works, wrong password does not.
    let r = send(&t.app, post_form("/login", None, "email=bob@example.com&password=nope")).await;
    assert!(r.body.contains("Incorrect email or password"));
    let r = send(&t.app, post_form("/login", None, "email=bob@example.com&password=hunter22hunter")).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);

    // Alice (first user → admin) submits an issue with a photo and barcode.
    let body = multipart(
        &[
            ("title", "Mister pump not running"),
            ("description", "<p>Pump <b>hums</b> but no water<script>alert(1)</script></p>"),
            ("location", "House 3"),
            ("equipment_code", "0123456789012"),
            ("equipment_code_format", "ean_13"),
            ("priority", "high"),
            ("assignee_id", ""),
        ],
        Some(PNG),
    );
    let r = send(&t.app, post_multipart("/issues/new", &alice, body)).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    let issue_url = r.location.unwrap();
    assert_eq!(issue_url, "/issues/1");

    let r = send(&t.app, get(&issue_url, Some(&alice))).await;
    assert!(r.body.contains("Mister pump not running"));
    assert!(r.body.contains("<b>hums</b>"));
    assert!(!r.body.contains("<script>alert"), "description must be sanitised");
    assert!(r.body.contains("badge state-unassigned"));
    assert!(r.body.contains("/attachments/1"));
    assert!(r.body.contains("0123456789012"));

    // The photo is served to logged-in users only.
    let r = send(&t.app, get("/attachments/1", Some(&bob))).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.as_bytes().starts_with(b"\x89PNG") || r.body.starts_with('\u{fffd}'));
    assert_eq!(send(&t.app, get("/attachments/1", None)).await.status, StatusCode::SEE_OTHER);

    // Bob cannot post before being involved...
    let r = send(&t.app, post_multipart("/issues/1/comments", &bob, multipart(&[("kind", "question"), ("body", "hi")], None))).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    // ...but can take the unassigned request.
    let r = send(&t.app, post_form("/issues/1/take", Some(&bob), "")).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    let r = send(&t.app, get("/", Some(&bob))).await;
    assert!(r.body.contains("Assigned to me <span class=\"count\">1</span>"), "{}", r.body);

    // Bob asks a question; Alice sees "awaiting your reply" on her dashboard.
    let r = send(
        &t.app,
        post_multipart("/issues/1/comments", &bob, multipart(&[("kind", "question"), ("body", "Is the breaker on?")], None)),
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    let r = send(&t.app, get("/", Some(&alice))).await;
    assert!(r.body.contains("awaiting your reply"));

    // Alice answers with a photo.
    let r = send(
        &t.app,
        post_multipart("/issues/1/comments", &alice, multipart(&[("kind", "response"), ("body", "Yes, see photo")], Some(PNG))),
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    let r = send(&t.app, get("/issues/1", Some(&bob))).await;
    assert!(r.body.contains("Is the breaker on?"));
    assert!(r.body.contains("Yes, see photo"));
    assert!(r.body.contains("took this request"));
    assert!(r.body.contains("badge state-assigned"));

    // Bob closes it; closed_at is shown.
    let r = send(&t.app, post_form("/issues/1/update", Some(&bob), "state=closed&assignee_id=2&priority=high")).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    let r = send(&t.app, get("/issues/1", Some(&alice))).await;
    assert!(r.body.contains("badge state-closed"));
    assert!(r.body.contains("changed state from assigned to closed"));
    assert!(r.body.contains("<dt>Closed</dt>"));

    // Searching by barcode finds it.
    let r = send(&t.app, get("/issues?state=all&q=0123456789012", Some(&bob))).await;
    assert!(r.body.contains("Mister pump not running"));
    let r = send(&t.app, get("/issues?state=all&q=nothing-like-this", Some(&bob))).await;
    assert!(r.body.contains("No requests match"));

    // Setting "assigned" with no assignee is refused with a message.
    let r = send(&t.app, post_form("/issues/1/update", Some(&alice), "state=assigned&assignee_id=&priority=high")).await;
    assert!(r.body.contains("Choose who the request is assigned to"));

    // Non-images are refused.
    let body = multipart(&[("title", "x"), ("description", "y")], Some(b"#!/bin/sh\necho hi"));
    let r = send(&t.app, post_multipart("/issues/new", &alice, body)).await;
    assert!(r.body.contains("not a supported image"));

    // Admin page: Alice yes, Bob no.
    assert_eq!(send(&t.app, get("/admin/users", Some(&alice))).await.status, StatusCode::OK);
    assert_eq!(send(&t.app, get("/admin/users", Some(&bob))).await.status, StatusCode::FORBIDDEN);

    // Logout ends the session.
    send(&t.app, post_form("/logout", Some(&bob), "")).await;
    assert_eq!(send(&t.app, get("/", Some(&bob))).await.status, StatusCode::SEE_OTHER);
}
