//! Askama template structs and the view models they render.

use askama::Template;
use axum::response::{Html, IntoResponse, Response};
use chrono::NaiveDateTime;

use crate::{error::AppError, models::User};

/// Render a template into an HTML response.
pub fn render<T: Template>(t: &T) -> Result<Response, AppError> {
    Ok(Html(t.render()?).into_response())
}

/// What the navigation bar needs to know about the viewer.
pub struct Nav {
    pub id: i32,
    pub name: String,
    pub is_admin: bool,
    pub email_verified: bool,
}

impl Nav {
    pub fn of(u: &User) -> Option<Nav> {
        Some(Nav {
            id: u.id,
            name: u.display_name.clone(),
            is_admin: u.is_admin,
            email_verified: u.email_verified(),
        })
    }
}

/// A timestamp rendered as `<time>`; JavaScript converts it to local time.
#[derive(Clone)]
pub struct Ts {
    pub iso: String,
    pub text: String,
}

impl Ts {
    pub fn new(t: NaiveDateTime) -> Ts {
        Ts {
            iso: t.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            text: t.format("%Y-%m-%d %H:%M UTC").to_string(),
        }
    }
}

pub struct ProviderButton {
    pub slug: String,
    pub name: String,
}

pub struct IssueRow {
    pub id: i32,
    pub title: String,
    pub state: String,
    pub priority: String,
    pub location: Option<String>,
    pub equipment_code: Option<String>,
    pub submitter_name: String,
    pub assignee_name: Option<String>,
    pub created: Ts,
    pub updated: Ts,
    pub comment_count: i64,
    pub photo_count: i64,
    pub needs_reply: bool,
}

pub struct UserOption {
    pub id: i32,
    pub name: String,
    pub selected: bool,
}

pub struct PhotoView {
    pub id: i32,
    pub name: String,
}

pub struct CommentView {
    pub id: i32,
    pub kind: String,
    pub author_name: String,
    pub is_mine: bool,
    pub role: String,
    pub body_html: String,
    pub created: Ts,
    pub photos: Vec<PhotoView>,
}

pub struct IssueView {
    pub id: i32,
    pub title: String,
    pub description_html: String,
    pub location: Option<String>,
    pub equipment_code: Option<String>,
    pub equipment_code_format: Option<String>,
    pub priority: String,
    pub state: String,
    pub submitter_name: String,
    pub assignee_name: Option<String>,
    pub assignee_id: Option<i32>,
    pub created: Ts,
    pub updated: Ts,
    pub closed: Option<Ts>,
}

// ------------------------------------------------------------ pages

#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginPage {
    pub nav: Option<Nav>,
    pub error: Option<String>,
    pub email: String,
    pub providers: Vec<ProviderButton>,
    pub allow_registration: bool,
}

#[derive(Template)]
#[template(path = "register.html")]
pub struct RegisterPage {
    pub nav: Option<Nav>,
    pub error: Option<String>,
    pub email: String,
    pub display_name: String,
    pub providers: Vec<ProviderButton>,
    pub min_password: usize,
}

#[derive(Template)]
#[template(path = "dashboard.html")]
pub struct DashboardPage {
    pub nav: Option<Nav>,
    pub assigned: Vec<IssueRow>,
    pub submitted: Vec<IssueRow>,
    pub unassigned: Vec<IssueRow>,
}

#[derive(Template)]
#[template(path = "issue_list.html")]
pub struct IssueListPage {
    pub nav: Option<Nav>,
    pub issues: Vec<IssueRow>,
    pub q: String,
    pub state: String,
    pub scope: String,
    pub states: Vec<&'static str>,
}

#[derive(Template)]
#[template(path = "issue_new.html")]
pub struct NewIssuePage {
    pub nav: Option<Nav>,
    pub error: Option<String>,
    pub title: String,
    pub description: String,
    pub location: String,
    pub equipment_code: String,
    pub equipment_code_format: String,
    pub priority: String,
    pub priorities: Vec<&'static str>,
    pub users: Vec<UserOption>,
}

#[derive(Template)]
#[template(path = "issue_show.html")]
pub struct IssuePage {
    pub nav: Option<Nav>,
    pub error: Option<String>,
    pub issue: IssueView,
    pub photos: Vec<PhotoView>,
    pub comments: Vec<CommentView>,
    pub can_participate: bool,
    pub can_take: bool,
    pub states: Vec<&'static str>,
    pub priorities: Vec<&'static str>,
    pub users: Vec<UserOption>,
    pub default_kind: String,
}

pub struct IdentityView {
    pub id: i32,
    pub provider_name: String,
    pub email: Option<String>,
}

#[derive(Template)]
#[template(path = "profile.html")]
pub struct ProfilePage {
    pub nav: Option<Nav>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub email: String,
    pub display_name: String,
    pub email_verified: bool,
    pub email_verified_at: Option<Ts>,
    pub mail_delivers: bool,
    pub has_password: bool,
    pub identities: Vec<IdentityView>,
    pub linkable: Vec<ProviderButton>,
    pub min_password: usize,
}

pub struct AdminUserRow {
    pub id: i32,
    pub email: String,
    pub name: String,
    pub is_admin: bool,
    pub email_verified: bool,
    pub is_self: bool,
    pub login_methods: String,
    pub created: Ts,
}

#[derive(Template)]
#[template(path = "admin_users.html")]
pub struct AdminUsersPage {
    pub nav: Option<Nav>,
    pub users: Vec<AdminUserRow>,
}

#[derive(Template)]
#[template(path = "verify_email.html")]
pub struct VerifyEmailPage {
    pub nav: Option<Nav>,
    pub logged_in: bool,
    pub ok: bool,
    pub heading: String,
    pub message: String,
}
