//! Registration, password login, logout and OAuth sign-in.

use axum::{
    Form,
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::Utc;
use diesel::prelude::*;
use serde::Deserialize;

use crate::{
    AppState,
    auth::{
        self, MIN_PASSWORD_LEN, MaybeUser, normalize_email,
        oauth::ExternalProfile,
        verify::{self, ConfirmOutcome, SendOutcome},
    },
    db,
    error::AppError,
    models::{NewOAuthIdentity, User},
    schema::{oauth_identities, users},
    views::{LoginPage, Nav, ProviderButton, RegisterPage, VerifyEmailPage, render},
};

const OAUTH_COOKIE: &str = "gt_oauth";

pub fn provider_buttons(state: &AppState) -> Vec<ProviderButton> {
    state
        .config
        .oauth_providers
        .iter()
        .map(|p| ProviderButton {
            slug: p.slug().to_string(),
            name: p.display_name.clone(),
        })
        .collect()
}

fn login_page(state: &AppState, error: Option<String>, email: String) -> LoginPage {
    LoginPage {
        nav: None,
        error,
        email,
        providers: provider_buttons(state),
        allow_registration: state.config.allow_registration,
    }
}

// ------------------------------------------------------------ register

#[derive(Deserialize)]
pub struct RegisterForm {
    email: String,
    display_name: String,
    password: String,
    password_confirm: String,
}

pub async fn register_form(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
) -> Result<Response, AppError> {
    if user.is_some() {
        return Ok(Redirect::to("/").into_response());
    }
    if !state.config.allow_registration {
        return Err(AppError::BadRequest(
            "Self-registration is disabled. Ask an administrator for an account.".into(),
        ));
    }
    render(&RegisterPage {
        nav: None,
        error: None,
        email: String::new(),
        display_name: String::new(),
        providers: provider_buttons(&state),
        min_password: MIN_PASSWORD_LEN,
    })
}

pub async fn register(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(form): Form<RegisterForm>,
) -> Result<Response, AppError> {
    if !state.config.allow_registration {
        return Err(AppError::Forbidden);
    }
    let email = normalize_email(&form.email);
    let name = form.display_name.trim().to_string();
    let fail = |msg: &str| {
        render(&RegisterPage {
            nav: None,
            error: Some(msg.to_string()),
            email: email.clone(),
            display_name: name.clone(),
            providers: provider_buttons(&state),
            min_password: MIN_PASSWORD_LEN,
        })
    };
    if !email.contains('@') || email.len() > 254 {
        return fail("Please enter a valid email address.");
    }
    if name.is_empty() || name.len() > 100 {
        return fail("Please enter your name (up to 100 characters).");
    }
    if form.password.len() < MIN_PASSWORD_LEN {
        return fail(&format!(
            "Passwords must be at least {MIN_PASSWORD_LEN} characters."
        ));
    }
    if form.password != form.password_confirm {
        return fail("The two passwords do not match.");
    }
    let hash = auth::hash_password(&form.password)?;
    let (e, n) = (email.clone(), name.clone());
    let created = db::run(&state.pool, move |conn| {
        let exists: bool = diesel::select(diesel::dsl::exists(
            users::table.filter(users::email.eq(&e)),
        ))
        .get_result(conn)?;
        if exists {
            return Ok(None);
        }
        auth::create_user(conn, &e, &n, Some(&hash), false).map(Some)
    })
    .await?;
    let Some(user) = created else {
        return fail("An account with that email already exists. Try logging in.");
    };
    send_verification_logged(&state, user.id).await;
    let cookie = auth::start_session(&state, user.id).await?;
    Ok((jar.add(cookie), Redirect::to("/")).into_response())
}

// ------------------------------------------------------------ login

#[derive(Deserialize)]
pub struct LoginForm {
    email: String,
    password: String,
}

#[derive(Deserialize)]
pub struct LoginQuery {
    error: Option<String>,
}

pub async fn login_form(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    Query(q): Query<LoginQuery>,
) -> Result<Response, AppError> {
    if user.is_some() {
        return Ok(Redirect::to("/").into_response());
    }
    render(&login_page(&state, q.error, String::new()))
}

pub async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(form): Form<LoginForm>,
) -> Result<Response, AppError> {
    let email = normalize_email(&form.email);
    let e = email.clone();
    let user: Option<User> = db::run(&state.pool, move |conn| {
        Ok(users::table
            .filter(users::email.eq(e))
            .select(User::as_select())
            .first(conn)
            .optional()?)
    })
    .await?;
    let password = form.password.clone();
    let user = tokio::task::spawn_blocking(move || {
        match user {
            Some(u) => match &u.password_hash {
                Some(h) if auth::verify_password(&password, h) => Some(u),
                _ => None,
            },
            None => {
                // Spend comparable time so response timing does not reveal
                // whether the email is registered.
                let _ = auth::hash_password(&password);
                None
            }
        }
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;
    match user {
        Some(u) => {
            let cookie = auth::start_session(&state, u.id).await?;
            Ok((jar.add(cookie), Redirect::to("/")).into_response())
        }
        None => render(&login_page(
            &state,
            Some("Incorrect email or password. If you signed up with Google or another provider, use that button instead.".into()),
            email,
        )),
    }
}

pub async fn logout(State(state): State<AppState>, jar: CookieJar) -> Result<Response, AppError> {
    auth::end_session(&state, &jar).await?;
    Ok((
        jar.remove(auth::removal_cookie(auth::SESSION_COOKIE)),
        Redirect::to("/login"),
    )
        .into_response())
}

// ------------------------------------------------------------ OAuth

#[derive(Deserialize)]
pub struct StartQuery {
    /// `link=1` attaches the identity to the logged-in account.
    link: Option<String>,
}

pub async fn oauth_start(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Query(q): Query<StartQuery>,
    jar: CookieJar,
) -> Result<Response, AppError> {
    let p = state.config.provider(&provider).ok_or(AppError::NotFound)?;
    let csrf_state = auth::random_token();
    let verifier = auth::random_token();
    let mode = if q.link.is_some() { "link" } else { "login" };
    let cookie = Cookie::build((
        OAUTH_COOKIE,
        format!("{}.{csrf_state}.{verifier}.{mode}", p.slug()),
    ))
    .path("/auth")
    .http_only(true)
    .same_site(SameSite::Lax)
    .secure(state.config.secure_cookies())
    .max_age(time::Duration::minutes(10))
    .build();
    let url = p.authorize_url(&state.config.base_url, &csrf_state, &verifier);
    Ok((jar.add(cookie), Redirect::to(&url)).into_response())
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

fn login_error(msg: &str) -> Response {
    let q = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("error", msg)
        .finish();
    Redirect::to(&format!("/login?{q}")).into_response()
}

pub async fn oauth_callback(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Query(q): Query<CallbackQuery>,
    MaybeUser(current): MaybeUser,
    jar: CookieJar,
) -> Result<Response, AppError> {
    let p = state.config.provider(&provider).ok_or(AppError::NotFound)?.clone();
    let cookie_val = jar.get(OAUTH_COOKIE).map(|c| c.value().to_string());
    let jar = jar.remove(Cookie::build(OAUTH_COOKIE).path("/auth").build());

    if let Some(err) = q.error {
        tracing::info!("{provider} sign-in cancelled: {err}");
        return Ok((jar, login_error("Sign-in was cancelled.")).into_response());
    }
    let parts: Vec<String> = cookie_val
        .unwrap_or_default()
        .split('.')
        .map(str::to_string)
        .collect();
    let (Some(code), Some(returned_state), [slug, expected_state, verifier, mode]) =
        (q.code, q.state, parts.as_slice())
    else {
        return Ok((jar, login_error("Sign-in expired. Please try again.")).into_response());
    };
    if slug != p.slug() || &returned_state != expected_state {
        return Ok((jar, login_error("Sign-in could not be verified. Please try again.")).into_response());
    }

    let profile = p
        .fetch_profile(&state.http, &state.config.base_url, &code, verifier)
        .await?;
    let link_to = if mode == "link" { current.map(|u| u.id) } else { None };
    let outcome = resolve_identity(
        &state,
        p.slug(),
        profile,
        link_to,
        state.config.allow_registration,
    )
    .await?;

    match outcome {
        Outcome::Created { user_id, verified } => {
            if !verified {
                send_verification_logged(&state, user_id).await;
            }
            let cookie = auth::start_session(&state, user_id).await?;
            Ok((jar.add(cookie), Redirect::to("/")).into_response())
        }
        Outcome::Login(user_id) => {
            let cookie = auth::start_session(&state, user_id).await?;
            Ok((jar.add(cookie), Redirect::to("/")).into_response())
        }
        Outcome::Linked => Ok((jar, Redirect::to("/profile?notice=linked")).into_response()),
        Outcome::Refused(msg) => Ok((jar, login_error(&msg)).into_response()),
    }
}

enum Outcome {
    Login(i32),
    Created { user_id: i32, verified: bool },
    Linked,
    Refused(String),
}

/// Map an external identity to a local user, creating one if needed.
async fn resolve_identity(
    state: &AppState,
    provider: &'static str,
    profile: ExternalProfile,
    link_to: Option<i32>,
    allow_registration: bool,
) -> Result<Outcome, AppError> {
    db::run(&state.pool, move |conn| {
        let now = Utc::now().naive_utc();
        let existing: Option<i32> = oauth_identities::table
            .filter(oauth_identities::provider.eq(provider))
            .filter(oauth_identities::subject.eq(&profile.subject))
            .select(oauth_identities::user_id)
            .first(conn)
            .optional()?;
        let email = profile.email.as_deref().map(normalize_email);

        if let Some(target) = link_to {
            return Ok(match existing {
                Some(uid) if uid == target => Outcome::Linked,
                Some(_) => Outcome::Refused(
                    "That account is already linked to a different user.".into(),
                ),
                None => {
                    diesel::insert_into(oauth_identities::table)
                        .values(NewOAuthIdentity {
                            user_id: target,
                            provider,
                            subject: &profile.subject,
                            email: email.as_deref(),
                            created_at: now,
                        })
                        .execute(conn)?;
                    // The provider vouches for the same address → verified.
                    let own_email: String =
                        users::table.find(target).select(users::email).first(conn)?;
                    if profile.email_verified && email.as_deref() == Some(own_email.as_str()) {
                        verify::mark_verified(conn, target)?;
                    }
                    Outcome::Linked
                }
            });
        }

        if let Some(uid) = existing {
            return Ok(Outcome::Login(uid));
        }

        let Some(email) = email else {
            return Ok(Outcome::Refused(
                "The provider did not share an email address, which is required to sign up.".into(),
            ));
        };
        let taken: bool = diesel::select(diesel::dsl::exists(
            users::table.filter(users::email.eq(&email)),
        ))
        .get_result(conn)?;
        if taken {
            // Never auto-link by email: the user proves ownership of the
            // local account first, then links from the profile page.
            return Ok(Outcome::Refused(format!(
                "An account for {email} already exists. Log in with your password, then link this provider from your profile page."
            )));
        }
        if !allow_registration {
            return Ok(Outcome::Refused(
                "Self-registration is disabled. Ask an administrator for an account.".into(),
            ));
        }
        let name = profile
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| email.split('@').next().unwrap_or("User").to_string());
        let user = conn.transaction::<_, AppError, _>(|conn| {
            let user = auth::create_user(conn, &email, &name, None, profile.email_verified)?;
            diesel::insert_into(oauth_identities::table)
                .values(NewOAuthIdentity {
                    user_id: user.id,
                    provider,
                    subject: &profile.subject,
                    email: Some(&email),
                    created_at: now,
                })
                .execute(conn)?;
            Ok(user)
        })?;
        Ok(Outcome::Created {
            user_id: user.id,
            verified: user.email_verified(),
        })
    })
    .await
}

/// Send the verification email after sign-up. Failures are logged, not
/// shown: the account works anyway and the user can resend from Profile.
async fn send_verification_logged(state: &AppState, user_id: i32) {
    match verify::send_verification(state, user_id).await {
        Ok(SendOutcome::Failed(e)) => tracing::warn!("verification email for user {user_id} failed: {e}"),
        Err(e) => tracing::warn!("verification email for user {user_id} failed: {e}"),
        Ok(_) => {}
    }
}

// ------------------------------------------------------------ verify email

#[derive(Deserialize)]
pub struct VerifyQuery {
    #[serde(default)]
    token: String,
}

/// The link in the verification email. Works whether or not the browser is
/// logged in (people often open mail on a different device).
pub async fn verify_email(
    State(state): State<AppState>,
    MaybeUser(user): MaybeUser,
    Query(q): Query<VerifyQuery>,
) -> Result<Response, AppError> {
    let outcome = verify::confirm(&state, q.token).await?;
    let (ok, heading, message) = match outcome {
        ConfirmOutcome::Verified { email } => (
            true,
            "Email verified".to_string(),
            format!("Thanks! {email} is confirmed."),
        ),
        ConfirmOutcome::AlreadyVerified { email } => (
            true,
            "Already verified".to_string(),
            format!("{email} was already confirmed. Nothing else to do."),
        ),
        ConfirmOutcome::Expired => (
            false,
            "Link expired".to_string(),
            "This verification link has expired. Log in and press \"Resend verification email\" on your profile page.".to_string(),
        ),
        ConfirmOutcome::Invalid => (
            false,
            "Link not valid".to_string(),
            "This verification link is invalid or has already been used. Log in and press \"Resend verification email\" on your profile page to get a new one.".to_string(),
        ),
    };
    // Re-read the viewer so the nav reflects the new state.
    let nav = match user {
        Some(u) => {
            let id = u.id;
            let fresh: User = db::run(&state.pool, move |conn| {
                Ok(users::table.find(id).select(User::as_select()).first(conn)?)
            })
            .await?;
            Nav::of(&fresh)
        }
        None => None,
    };
    render(&VerifyEmailPage { logged_in: nav.is_some(), nav, ok, heading, message })
}
