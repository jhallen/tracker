//! Profile page (name, password, linked sign-in providers) and the small
//! admin user list.

use axum::{
    Form,
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect, Response},
};
use diesel::prelude::*;
use serde::Deserialize;

use crate::{
    AppState,
    auth::{self, CurrentUser, MIN_PASSWORD_LEN},
    db,
    error::AppError,
    models::{OAuthIdentity, User},
    schema::{oauth_identities, sessions, users},
    views::*,
};

#[derive(Deserialize)]
pub struct NoticeQuery {
    notice: Option<String>,
}

async fn profile_page(
    state: &AppState,
    user_id: i32,
    error: Option<String>,
    notice: Option<String>,
) -> Result<Response, AppError> {
    let (user, ids) = db::run(&state.pool, move |conn| {
        let user: User = users::table.find(user_id).select(User::as_select()).first(conn)?;
        let ids: Vec<OAuthIdentity> = OAuthIdentity::belonging_to(&user)
            .select(OAuthIdentity::as_select())
            .load(conn)?;
        Ok((user, ids))
    })
    .await?;
    let name_of = |slug: &str| {
        state
            .config
            .provider(slug)
            .map(|p| p.display_name.clone())
            .unwrap_or_else(|| slug.to_string())
    };
    let identities = ids
        .iter()
        .map(|i| IdentityView {
            id: i.id,
            provider_name: name_of(&i.provider),
            email: i.email.clone(),
        })
        .collect();
    let linkable = state
        .config
        .oauth_providers
        .iter()
        .filter(|p| !ids.iter().any(|i| i.provider == p.slug()))
        .map(|p| ProviderButton {
            slug: p.slug().to_string(),
            name: p.display_name.clone(),
        })
        .collect();
    render(&ProfilePage {
        nav: Nav::of(&user),
        error,
        notice,
        email: user.email.clone(),
        display_name: user.display_name.clone(),
        has_password: user.password_hash.is_some(),
        identities,
        linkable,
        min_password: MIN_PASSWORD_LEN,
    })
}

pub async fn profile(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<NoticeQuery>,
) -> Result<Response, AppError> {
    let notice = q.notice.map(|n| match n.as_str() {
        "linked" => "Sign-in provider linked.".to_string(),
        "unlinked" => "Sign-in provider removed.".to_string(),
        "password" => "Password updated.".to_string(),
        "saved" => "Profile saved.".to_string(),
        _ => String::new(),
    }).filter(|n| !n.is_empty());
    profile_page(&state, user.id, None, notice).await
}

#[derive(Deserialize)]
pub struct ProfileForm {
    display_name: String,
}

pub async fn update_profile(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<ProfileForm>,
) -> Result<Response, AppError> {
    let name = form.display_name.trim().to_string();
    if name.is_empty() || name.len() > 100 {
        return profile_page(&state, user.id, Some("Please enter a name (up to 100 characters).".into()), None).await;
    }
    let uid = user.id;
    db::run(&state.pool, move |conn| {
        diesel::update(users::table.find(uid))
            .set(users::display_name.eq(name))
            .execute(conn)?;
        Ok(())
    })
    .await?;
    Ok(Redirect::to("/profile?notice=saved").into_response())
}

#[derive(Deserialize)]
pub struct PasswordForm {
    #[serde(default)]
    current_password: String,
    new_password: String,
    new_password_confirm: String,
}

pub async fn change_password(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<PasswordForm>,
) -> Result<Response, AppError> {
    if let Some(h) = &user.password_hash {
        if !auth::verify_password(&form.current_password, h) {
            return profile_page(&state, user.id, Some("Your current password is incorrect.".into()), None).await;
        }
    }
    if form.new_password.len() < MIN_PASSWORD_LEN {
        return profile_page(
            &state,
            user.id,
            Some(format!("Passwords must be at least {MIN_PASSWORD_LEN} characters.")),
            None,
        )
        .await;
    }
    if form.new_password != form.new_password_confirm {
        return profile_page(&state, user.id, Some("The two passwords do not match.".into()), None).await;
    }
    let hash = auth::hash_password(&form.new_password)?;
    let uid = user.id;
    db::run(&state.pool, move |conn| {
        diesel::update(users::table.find(uid))
            .set(users::password_hash.eq(Some(hash)))
            .execute(conn)?;
        Ok(())
    })
    .await?;
    Ok(Redirect::to("/profile?notice=password").into_response())
}

pub async fn unlink(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(identity_id): Path<i32>,
) -> Result<Response, AppError> {
    let uid = user.id;
    let has_password = user.password_hash.is_some();
    let res = db::run(&state.pool, move |conn| {
        let count: i64 = oauth_identities::table
            .filter(oauth_identities::user_id.eq(uid))
            .count()
            .get_result(conn)?;
        if !has_password && count <= 1 {
            return Ok(false);
        }
        let n = diesel::delete(
            oauth_identities::table
                .filter(oauth_identities::id.eq(identity_id))
                .filter(oauth_identities::user_id.eq(uid)),
        )
        .execute(conn)?;
        if n == 0 {
            return Err(AppError::NotFound);
        }
        Ok(true)
    })
    .await?;
    if !res {
        return profile_page(
            &state,
            user.id,
            Some("Set a password first; otherwise you would have no way to log in.".into()),
            None,
        )
        .await;
    }
    Ok(Redirect::to("/profile?notice=unlinked").into_response())
}

// ------------------------------------------------------------ admin

pub async fn admin_users(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Response, AppError> {
    if !user.is_admin {
        return Err(AppError::Forbidden);
    }
    let me = user.id;
    let rows = db::run(&state.pool, move |conn| {
        let all: Vec<User> = users::table
            .order(users::display_name.asc())
            .select(User::as_select())
            .load(conn)?;
        let idents: Vec<OAuthIdentity> = OAuthIdentity::belonging_to(&all)
            .select(OAuthIdentity::as_select())
            .load(conn)?;
        let grouped = idents.grouped_by(&all);
        Ok(all
            .into_iter()
            .zip(grouped)
            .map(|(u, ids)| {
                let mut methods: Vec<String> = Vec::new();
                if u.password_hash.is_some() {
                    methods.push("password".into());
                }
                methods.extend(ids.into_iter().map(|i| i.provider));
                AdminUserRow {
                    id: u.id,
                    is_self: u.id == me,
                    email: u.email,
                    name: u.display_name,
                    is_admin: u.is_admin,
                    login_methods: methods.join(", "),
                    created: Ts::new(u.created_at),
                }
            })
            .collect::<Vec<_>>())
    })
    .await?;
    render(&AdminUsersPage {
        nav: Nav::of(&user),
        users: rows,
    })
}

pub async fn toggle_admin(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i32>,
) -> Result<Response, AppError> {
    if !user.is_admin {
        return Err(AppError::Forbidden);
    }
    if id == user.id {
        return Err(AppError::BadRequest("You cannot remove your own admin rights.".into()));
    }
    db::run(&state.pool, move |conn| {
        let current: bool = users::table.find(id).select(users::is_admin).first(conn)?;
        diesel::update(users::table.find(id))
            .set(users::is_admin.eq(!current))
            .execute(conn)?;
        if current {
            // Demoted: force a fresh login so the change applies everywhere.
            diesel::delete(sessions::table.filter(sessions::user_id.eq(id))).execute(conn)?;
        }
        Ok(())
    })
    .await?;
    Ok(Redirect::to("/admin/users").into_response())
}
