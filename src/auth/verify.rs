//! Email address verification: send a link, confirm it when clicked.
//!
//! A verification is a random token emailed as
//! `<BASE_URL>/verify-email?token=…`. Only its SHA-256 is stored, in
//! `email_verifications`. The link works once, for `TOKEN_HOURS`, and only
//! while the account still has the address it was sent to. Verification is
//! informational: an unverified user can still use the whole app.

use askama::Template;
use chrono::{Duration, NaiveDateTime, Utc};
use diesel::prelude::*;

use crate::{
    AppState, db,
    error::AppError,
    mailer::OutgoingEmail,
    models::NewEmailVerification,
    schema::{email_verifications, users},
};

pub const TOKEN_HOURS: i64 = 48;
/// Minimum time between two verification emails to the same user.
pub const RESEND_COOLDOWN_SECS: i64 = 60;

#[derive(Template)]
#[template(path = "email/verify.txt")]
struct VerifyText<'a> {
    name: &'a str,
    link: &'a str,
    hours: i64,
}

#[derive(Template)]
#[template(path = "email/verify.html")]
struct VerifyHtml<'a> {
    name: &'a str,
    link: &'a str,
    hours: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SendOutcome {
    Sent,
    AlreadyVerified,
    /// Asked again too soon; holds seconds left to wait.
    TooSoon(i64),
    Failed(String),
}

/// Create a token and email the verification link to the user's address.
pub async fn send_verification(state: &AppState, user_id: i32) -> Result<SendOutcome, AppError> {
    let token = super::random_token();
    let token_hash = super::hash_token(&token);
    let prepared = db::run(&state.pool, move |conn| {
        let (email, name, verified): (String, String, Option<NaiveDateTime>) = users::table
            .find(user_id)
            .select((users::email, users::display_name, users::email_verified_at))
            .first(conn)?;
        if verified.is_some() {
            return Ok(Err(SendOutcome::AlreadyVerified));
        }
        let now = Utc::now().naive_utc();
        let last: Option<NaiveDateTime> = email_verifications::table
            .filter(email_verifications::user_id.eq(user_id))
            .select(diesel::dsl::max(email_verifications::created_at))
            .first(conn)?;
        if let Some(last) = last {
            let wait = RESEND_COOLDOWN_SECS - (now - last).num_seconds();
            if wait > 0 {
                return Ok(Err(SendOutcome::TooSoon(wait)));
            }
        }
        diesel::insert_into(email_verifications::table)
            .values(NewEmailVerification {
                user_id,
                email: email.clone(),
                token_hash,
                created_at: now,
                expires_at: now + Duration::hours(TOKEN_HOURS),
            })
            .execute(conn)?;
        Ok(Ok((email, name)))
    })
    .await?;
    let (email, name) = match prepared {
        Ok(v) => v,
        Err(outcome) => return Ok(outcome),
    };

    let link = format!("{}/verify-email?token={token}", state.config.base_url);
    let text = VerifyText { name: &name, link: &link, hours: TOKEN_HOURS }.render()?;
    let html = VerifyHtml { name: &name, link: &link, hours: TOKEN_HOURS }.render()?;
    let result = state
        .mailer
        .send(OutgoingEmail {
            to: email,
            subject: "Confirm your email address for Greenhouse Maintenance".into(),
            text,
            html,
        })
        .await;
    Ok(match result {
        Ok(()) => SendOutcome::Sent,
        Err(e) => SendOutcome::Failed(e),
    })
}

/// Result of clicking a verification link.
#[derive(Debug, PartialEq, Eq)]
pub enum ConfirmOutcome {
    Verified { email: String },
    AlreadyVerified { email: String },
    Expired,
    Invalid,
}

pub async fn confirm(state: &AppState, token: String) -> Result<ConfirmOutcome, AppError> {
    let token_hash = super::hash_token(token.trim());
    db::run(&state.pool, move |conn| {
        conn.transaction::<_, AppError, _>(|conn| {
            let row: Option<(i32, i32, String, NaiveDateTime, Option<NaiveDateTime>)> =
                email_verifications::table
                    .filter(email_verifications::token_hash.eq(&token_hash))
                    .select((
                        email_verifications::id,
                        email_verifications::user_id,
                        email_verifications::email,
                        email_verifications::expires_at,
                        email_verifications::used_at,
                    ))
                    .first(conn)
                    .optional()?;
            let Some((id, user_id, email, expires_at, used_at)) = row else {
                return Ok(ConfirmOutcome::Invalid);
            };
            let (current_email, verified_at): (String, Option<NaiveDateTime>) = users::table
                .find(user_id)
                .select((users::email, users::email_verified_at))
                .first(conn)?;
            if current_email != email {
                return Ok(ConfirmOutcome::Invalid);
            }
            if verified_at.is_some() {
                return Ok(ConfirmOutcome::AlreadyVerified { email });
            }
            let now = Utc::now().naive_utc();
            if used_at.is_some() {
                return Ok(ConfirmOutcome::Invalid);
            }
            if expires_at < now {
                return Ok(ConfirmOutcome::Expired);
            }
            diesel::update(users::table.find(user_id))
                .set(users::email_verified_at.eq(Some(now)))
                .execute(conn)?;
            diesel::update(email_verifications::table.find(id))
                .set(email_verifications::used_at.eq(Some(now)))
                .execute(conn)?;
            Ok(ConfirmOutcome::Verified { email })
        })
    })
    .await
}

/// Mark a user's address verified without a link (an OAuth provider
/// confirmed the same address).
pub fn mark_verified(conn: &mut SqliteConnection, user_id: i32) -> QueryResult<usize> {
    diesel::update(users::table.find(user_id).filter(users::email_verified_at.is_null()))
        .set(users::email_verified_at.eq(Some(Utc::now().naive_utc())))
        .execute(conn)
}
