//! Email verification: link sent on sign-up, resend, confirm, expiry.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use diesel::prelude::*;
use greenhouse_tracker::{AppState, build_app, config::Config, schema::email_verifications};
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn call(app: &Router, req: Request<Body>) -> (StatusCode, Option<String>, Option<String>, String) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let loc = res.headers().get(header::LOCATION).map(|v| v.to_str().unwrap().to_string());
    let cookie = res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap())
        .find(|c| c.starts_with("gt_session=") && !c.starts_with("gt_session=;"))
        .map(|c| c.split(';').next().unwrap().to_string());
    let body = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    (status, loc, cookie, body)
}

fn get(path: &str, cookie: Option<&str>) -> Request<Body> {
    let mut b = Request::get(path).header(header::HOST, "localhost");
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    b.body(Body::empty()).unwrap()
}

fn post(path: &str, cookie: Option<&str>, body: &str) -> Request<Body> {
    let mut b = Request::post(path)
        .header(header::HOST, "localhost")
        .header(header::ORIGIN, "http://localhost")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    b.body(Body::from(body.to_string())).unwrap()
}

fn link_path(text: &str) -> String {
    let start = text.find("http://localhost/verify-email?token=").expect("link in email");
    let link: String = text[start..].chars().take_while(|c| !c.is_whitespace()).collect();
    link.trim_start_matches("http://localhost").to_string()
}

/// Let the next resend through without waiting a minute.
fn skip_cooldown(state: &AppState) {
    let mut conn = state.pool.get().unwrap();
    diesel::update(email_verifications::table)
        .set(email_verifications::created_at.eq(chrono::Utc::now().naive_utc() - chrono::Duration::hours(1)))
        .execute(&mut conn)
        .unwrap();
}

#[tokio::test]
async fn verification_link_flow() {
    let dir = std::env::temp_dir().join(format!("gt-verify-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let state = AppState::new(Config::for_tests(dir.join("t.db").to_string_lossy().into(), dir.join("up"))).unwrap();
    let app = build_app(state.clone());

    // Sign-up labels make email the username and ask for a display name.
    let (_, _, _, body) = call(&app, get("/register", None)).await;
    assert!(body.contains("this is your username"));
    assert!(body.contains("Display name"));

    let (status, _, cookie, _) = call(
        &app,
        post("/register", None, "email=Pat%40Example.com&display_name=Pat&password=hunter22hunter&password_confirm=hunter22hunter"),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let cookie = cookie.unwrap();

    // One email, to the normalised address, with a link.
    let sent = state.mailer.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].to, "pat@example.com");
    assert!(sent[0].html.contains("Verify my email"));
    let link = link_path(&sent[0].text);

    // Unverified users can use the app; status is shown.
    let (status, _, _, body) = call(&app, get("/", Some(&cookie))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Please confirm your email address"));
    let (_, _, _, body) = call(&app, get("/profile", Some(&cookie))).await;
    assert!(body.contains("Not verified") && body.contains("Resend verification email"));
    let (status, _, _, _) = call(&app, get("/issues/new", Some(&cookie))).await;
    assert_eq!(status, StatusCode::OK);

    // Resending immediately is throttled.
    let (_, _, _, body) = call(&app, post("/profile/resend-verification", Some(&cookie), "")).await;
    assert!(body.contains("Please wait"), "{body}");
    assert_eq!(state.mailer.sent().len(), 1);

    // After the cooldown a second email goes out; both links stay valid.
    skip_cooldown(&state);
    let (status, loc, _, _) = call(&app, post("/profile/resend-verification", Some(&cookie), "")).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(loc.as_deref(), Some("/profile?notice=sent"));
    assert_eq!(state.mailer.sent().len(), 2);

    // Bad token.
    let (_, _, _, body) = call(&app, get("/verify-email?token=nope", None)).await;
    assert!(body.contains("Link not valid"));

    // Click the first link while logged out (e.g. on a phone).
    let (status, _, _, body) = call(&app, get(&link, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Email verified") && body.contains("pat@example.com"));

    let (_, _, _, body) = call(&app, get("/profile", Some(&cookie))).await;
    assert!(body.contains("✓ Verified"));
    assert!(!body.contains("Resend verification email"));
    let (_, _, _, body) = call(&app, get("/", Some(&cookie))).await;
    assert!(!body.contains("Please confirm your email address"));

    // Clicking again is harmless; resend is a no-op once verified.
    let (_, _, _, body) = call(&app, get(&link, None)).await;
    assert!(body.contains("Already verified"));
    let (_, loc, _, _) = call(&app, post("/profile/resend-verification", Some(&cookie), "")).await;
    assert_eq!(loc.as_deref(), Some("/profile?notice=verified"));
    assert_eq!(state.mailer.sent().len(), 2);

    // Admin list shows the status.
    let (_, _, _, body) = call(&app, get("/admin/users", Some(&cookie))).await;
    assert!(body.contains("✓ yes"));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn expired_link_is_refused() {
    let dir = std::env::temp_dir().join(format!("gt-verify-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let state = AppState::new(Config::for_tests(dir.join("t.db").to_string_lossy().into(), dir.join("up"))).unwrap();
    let app = build_app(state.clone());
    let (_, _, cookie, _) = call(
        &app,
        post("/register", None, "email=a%40b.com&display_name=A&password=hunter22hunter&password_confirm=hunter22hunter"),
    )
    .await;
    let link = link_path(&state.mailer.sent()[0].text);
    {
        let mut conn = state.pool.get().unwrap();
        diesel::update(email_verifications::table)
            .set(email_verifications::expires_at.eq(chrono::Utc::now().naive_utc() - chrono::Duration::minutes(1)))
            .execute(&mut conn)
            .unwrap();
    }
    let (_, _, _, body) = call(&app, get(&link, None)).await;
    assert!(body.contains("Link expired"));
    let (_, _, _, body) = call(&app, get("/profile", Some(&cookie.unwrap()))).await;
    assert!(body.contains("Not verified"));
    let _ = std::fs::remove_dir_all(dir);
}
