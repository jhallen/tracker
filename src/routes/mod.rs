//! HTTP routes.

mod account;
mod auth;
mod issues;

use axum::{
    Router,
    http::header,
    response::IntoResponse,
    routing::{get, post},
};

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(issues::dashboard))
        .route("/healthz", get(|| async { "ok" }))
        .route("/manifest.webmanifest", get(manifest))
        // authentication
        .route("/register", get(auth::register_form).post(auth::register))
        .route("/login", get(auth::login_form).post(auth::login))
        .route("/logout", post(auth::logout))
        .route("/auth/{provider}/start", get(auth::oauth_start))
        .route("/auth/{provider}/callback", get(auth::oauth_callback))
        .route("/verify-email", get(auth::verify_email))
        // issues
        .route("/issues", get(issues::list))
        .route("/issues/new", get(issues::new_form).post(issues::create))
        .route("/issues/{id}", get(issues::show))
        .route("/issues/{id}/comments", post(issues::add_comment))
        .route("/issues/{id}/update", post(issues::update))
        .route("/issues/{id}/take", post(issues::take))
        .route("/attachments/{id}", get(issues::attachment))
        // account & admin
        .route("/profile", get(account::profile).post(account::update_profile))
        .route("/profile/password", post(account::change_password))
        .route("/profile/unlink/{id}", post(account::unlink))
        .route("/profile/resend-verification", post(account::resend_verification))
        .route("/admin/users", get(account::admin_users))
        .route("/admin/users/{id}/toggle-admin", post(account::toggle_admin))
}

async fn manifest() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/manifest+json")],
        r##"{
  "name": "Greenhouse Maintenance Tracker",
  "short_name": "Greenhouse",
  "start_url": "/",
  "display": "standalone",
  "background_color": "#f4f7f2",
  "theme_color": "#2f6b3a",
  "icons": [{"src": "/static/icon.svg", "sizes": "any", "type": "image/svg+xml"}]
}"##,
    )
}
