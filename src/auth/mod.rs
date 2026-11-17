//! Authentication: password hashing, server-side sessions, the
//! `CurrentUser` extractor and the same-origin check for form posts.

pub mod oauth;
pub mod verify;

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use axum::{
    extract::{FromRef, FromRequestParts, Request, State},
    http::{HeaderMap, Method, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use diesel::prelude::*;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::{
    AppState, db,
    error::AppError,
    models::{NewUser, Session, User},
    schema::{sessions, users},
};

pub const SESSION_COOKIE: &str = "gt_session";

// ------------------------------------------------------------ passwords

pub const MIN_PASSWORD_LEN: usize = 8;

pub fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::Internal(e.to_string()))
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

// ------------------------------------------------------------ tokens

/// A random URL-safe token with 256 bits of entropy.
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Sessions and email-verification tokens are stored by their SHA-256, so
/// a leaked database does not leak usable cookies or links.
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

// ------------------------------------------------------------ users

pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// Insert a user. The very first user of an installation becomes admin.
/// `email_verified` is true only when an OAuth provider vouched for the
/// address; everyone else confirms it via the emailed link.
pub fn create_user(
    conn: &mut SqliteConnection,
    email: &str,
    display_name: &str,
    password_hash: Option<&str>,
    email_verified: bool,
) -> Result<User, AppError> {
    let count: i64 = users::table.count().get_result(conn)?;
    let user = diesel::insert_into(users::table)
        .values(NewUser {
            email,
            display_name,
            password_hash,
            is_admin: count == 0,
            created_at: Utc::now().naive_utc(),
            email_verified_at: email_verified.then(|| Utc::now().naive_utc()),
        })
        .returning(User::as_returning())
        .get_result(conn)?;
    Ok(user)
}

// ------------------------------------------------------------ sessions

/// Create a session for `user_id` and return the cookie to set.
pub async fn start_session(
    state: &AppState,
    user_id: i32,
) -> Result<Cookie<'static>, AppError> {
    let token = random_token();
    let key = hash_token(&token);
    let hours = state.config.session_hours;
    db::run(&state.pool, move |conn| {
        let now = Utc::now().naive_utc();
        // Opportunistically purge expired sessions.
        diesel::delete(sessions::table.filter(sessions::expires_at.lt(now))).execute(conn)?;
        diesel::insert_into(sessions::table)
            .values(Session {
                id: key,
                user_id,
                created_at: now,
                expires_at: now + Duration::hours(hours),
            })
            .execute(conn)?;
        Ok(())
    })
    .await?;
    Ok(Cookie::build((SESSION_COOKIE, token))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(state.config.secure_cookies())
        .max_age(time::Duration::hours(hours))
        .build())
}

pub async fn end_session(state: &AppState, jar: &CookieJar) -> Result<(), AppError> {
    if let Some(c) = jar.get(SESSION_COOKIE) {
        let key = hash_token(c.value());
        db::run(&state.pool, move |conn| {
            diesel::delete(sessions::table.find(key)).execute(conn)?;
            Ok(())
        })
        .await?;
    }
    Ok(())
}

pub fn removal_cookie(name: &'static str) -> Cookie<'static> {
    Cookie::build((name, "")).path("/").removal().build()
}

async fn user_from_headers(state: &AppState, headers: &HeaderMap) -> Result<Option<User>, AppError> {
    let jar = CookieJar::from_headers(headers);
    let Some(cookie) = jar.get(SESSION_COOKIE) else {
        return Ok(None);
    };
    let key = hash_token(cookie.value());
    db::run(&state.pool, move |conn| {
        let now = Utc::now().naive_utc();
        let user = sessions::table
            .inner_join(users::table)
            .filter(sessions::id.eq(key))
            .filter(sessions::expires_at.gt(now))
            .select(User::as_select())
            .first(conn)
            .optional()?;
        Ok(user)
    })
    .await
}

/// Extractor: the logged-in user, or a redirect to `/login`.
pub struct CurrentUser(pub User);

impl<S> FromRequestParts<S> for CurrentUser
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        user_from_headers(&state, &parts.headers)
            .await?
            .map(CurrentUser)
            .ok_or(AppError::LoginRequired)
    }
}

/// Extractor: the logged-in user if there is one.
pub struct MaybeUser(pub Option<User>);

impl<S> FromRequestParts<S> for MaybeUser
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        Ok(MaybeUser(user_from_headers(&state, &parts.headers).await?))
    }
}

// ------------------------------------------------------------ CSRF

/// Cross-site request forgery protection for state-changing requests.
///
/// Session cookies are `SameSite=Lax`, and in addition every non-GET
/// request must carry an `Origin` (or, failing that, `Referer`) header whose
/// host matches the `Host` the request was sent to. Browsers always send
/// `Origin` on cross-origin form posts and fetches, so a forged post from
/// another site is rejected.
pub async fn same_origin_guard(
    State(_state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(req).await;
    }
    let headers = req.headers();
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get("host"))
        .and_then(|v| v.to_str().ok())
        .map(|h| h.split(',').next().unwrap_or("").trim().to_ascii_lowercase());
    let source = headers
        .get("origin")
        .or_else(|| headers.get("referer"))
        .and_then(|v| v.to_str().ok())
        .and_then(|v| url::Url::parse(v).ok())
        .map(|u| match u.port() {
            Some(p) => format!("{}:{p}", u.host_str().unwrap_or("")),
            None => u.host_str().unwrap_or("").to_string(),
        })
        .map(|s| s.to_ascii_lowercase());
    match (host, source) {
        (Some(h), Some(s)) if h == s => next.run(req).await,
        _ => {
            tracing::warn!("rejected cross-origin {} {}", req.method(), req.uri());
            AppError::Forbidden.into_response()
        }
    }
}
