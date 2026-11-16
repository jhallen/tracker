//! OAuth sign-in against a mock OpenID Connect provider.

use axum::{
    Json, Router,
    body::Body,
    http::{Request, StatusCode, header},
    routing::{get, post},
};
use greenhouse_tracker::{
    AppState,
    auth::oauth::{OAuthProvider, ProviderKind},
    build_app,
    config::Config,
};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

async fn mock_provider() -> String {
    let app = Router::new()
        .route("/token", post(|| async { Json(json!({"access_token": "tok", "token_type": "Bearer"})) }))
        .route(
            "/userinfo",
            get(|| async {
                Json(json!({"sub": "abc-123", "email": "Carol@Example.com", "email_verified": true, "name": "Carol"}))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn cookie_named(res: &axum::response::Response, name: &str) -> Option<String> {
    res.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap())
        .find(|c| c.starts_with(&format!("{name}=")) && !c.starts_with(&format!("{name}=;")))
        .map(|c| c.split(';').next().unwrap().to_string())
}

#[tokio::test]
async fn oidc_sign_up_then_sign_in() {
    let base = mock_provider().await;
    let dir = std::env::temp_dir().join(format!("gt-oauth-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = Config::for_tests(dir.join("t.db").to_string_lossy().into(), dir.join("up"));
    cfg.oauth_providers.push(OAuthProvider {
        kind: ProviderKind::Oidc,
        display_name: "Test SSO".into(),
        client_id: "cid".into(),
        client_secret: "secret".into(),
        auth_url: format!("{base}/authorize"),
        token_url: format!("{base}/token"),
        userinfo_url: format!("{base}/userinfo"),
        scopes: "openid email profile".into(),
    });
    let app = build_app(AppState::new(cfg).unwrap());

    // Login page shows the provider button.
    let res = app.clone().oneshot(Request::get("/login").body(Body::empty()).unwrap()).await.unwrap();
    let body = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    assert!(body.contains("/auth/oidc/start") && body.contains("Test SSO"));

    for round in 0..2 {
        let res = app
            .clone()
            .oneshot(Request::get("/auth/oidc/start").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let oauth_cookie = cookie_named(&res, "gt_oauth").unwrap();
        let loc = url::Url::parse(res.headers()[header::LOCATION].to_str().unwrap()).unwrap();
        assert!(loc.as_str().starts_with(&format!("{base}/authorize")));
        let q: std::collections::HashMap<_, _> = loc.query_pairs().into_owned().collect();
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["redirect_uri"], "http://localhost/auth/oidc/callback");

        // Wrong state is rejected.
        let bad = app
            .clone()
            .oneshot(
                Request::get("/auth/oidc/callback?code=x&state=wrong")
                    .header(header::COOKIE, &oauth_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(bad.headers()[header::LOCATION].to_str().unwrap().starts_with("/login?error="));

        let res = app
            .clone()
            .oneshot(
                Request::get(format!("/auth/oidc/callback?code=x&state={}", q["state"]))
                    .header(header::COOKIE, &oauth_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.headers()[header::LOCATION], "/", "round {round}");
        let session = cookie_named(&res, "gt_session").expect("logged in");

        let res = app
            .clone()
            .oneshot(Request::get("/profile").header(header::COOKIE, &session).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
        assert!(body.contains("carol@example.com"), "email is normalised");
        assert!(body.contains("Test SSO"));
    }
    let _ = std::fs::remove_dir_all(dir);
}
