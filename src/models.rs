//! Diesel models plus the small enums stored as TEXT columns.

use chrono::NaiveDateTime;
use diesel::prelude::*;

use crate::schema::*;

// ---------------------------------------------------------------- users

#[derive(Queryable, Selectable, Identifiable, Debug, Clone)]
#[diesel(table_name = users)]
pub struct User {
    pub id: i32,
    pub email: String,
    pub display_name: String,
    pub password_hash: Option<String>,
    pub is_admin: bool,
    pub created_at: NaiveDateTime,
    /// When the address was confirmed (link clicked, or vouched for by an
    /// OAuth provider). `None` = not verified; the app works either way.
    pub email_verified_at: Option<NaiveDateTime>,
}

impl User {
    pub fn email_verified(&self) -> bool {
        self.email_verified_at.is_some()
    }
}

#[derive(Insertable)]
#[diesel(table_name = users)]
pub struct NewUser<'a> {
    pub email: &'a str,
    pub display_name: &'a str,
    pub password_hash: Option<&'a str>,
    pub is_admin: bool,
    pub created_at: NaiveDateTime,
    pub email_verified_at: Option<NaiveDateTime>,
}

#[derive(Insertable)]
#[diesel(table_name = email_verifications)]
pub struct NewEmailVerification {
    pub user_id: i32,
    pub email: String,
    pub token_hash: String,
    pub created_at: NaiveDateTime,
    pub expires_at: NaiveDateTime,
}

#[derive(Queryable, Selectable, Identifiable, Associations, Debug, Clone)]
#[diesel(table_name = oauth_identities, belongs_to(User))]
pub struct OAuthIdentity {
    pub id: i32,
    pub user_id: i32,
    pub provider: String,
    pub subject: String,
    pub email: Option<String>,
    pub created_at: NaiveDateTime,
}

#[derive(Insertable)]
#[diesel(table_name = oauth_identities)]
pub struct NewOAuthIdentity<'a> {
    pub user_id: i32,
    pub provider: &'a str,
    pub subject: &'a str,
    pub email: Option<&'a str>,
    pub created_at: NaiveDateTime,
}

#[derive(Queryable, Selectable, Insertable, Debug, Clone)]
#[diesel(table_name = sessions)]
pub struct Session {
    pub id: String,
    pub user_id: i32,
    pub created_at: NaiveDateTime,
    pub expires_at: NaiveDateTime,
}

// ---------------------------------------------------------------- issues

#[derive(Queryable, Selectable, Identifiable, Debug, Clone)]
#[diesel(table_name = issues)]
pub struct Issue {
    pub id: i32,
    pub title: String,
    pub description_html: String,
    pub location: Option<String>,
    pub equipment_code: Option<String>,
    pub equipment_code_format: Option<String>,
    pub priority: String,
    pub state: String,
    pub submitter_id: i32,
    pub assignee_id: Option<i32>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub closed_at: Option<NaiveDateTime>,
}

#[derive(Insertable)]
#[diesel(table_name = issues)]
pub struct NewIssue {
    pub title: String,
    pub description_html: String,
    pub location: Option<String>,
    pub equipment_code: Option<String>,
    pub equipment_code_format: Option<String>,
    pub priority: String,
    pub state: String,
    pub submitter_id: i32,
    pub assignee_id: Option<i32>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Queryable, Selectable, Identifiable, Associations, Debug, Clone)]
#[diesel(table_name = comments, belongs_to(Issue))]
pub struct Comment {
    pub id: i32,
    pub issue_id: i32,
    pub author_id: i32,
    pub kind: String,
    pub body_html: String,
    pub created_at: NaiveDateTime,
}

#[derive(Insertable)]
#[diesel(table_name = comments)]
pub struct NewComment {
    pub issue_id: i32,
    pub author_id: i32,
    pub kind: String,
    pub body_html: String,
    pub created_at: NaiveDateTime,
}

#[derive(Queryable, Selectable, Identifiable, Associations, Debug, Clone)]
#[diesel(table_name = attachments, belongs_to(Issue))]
pub struct Attachment {
    pub id: i32,
    pub issue_id: i32,
    pub comment_id: Option<i32>,
    pub uploader_id: i32,
    pub original_name: String,
    pub stored_name: String,
    pub content_type: String,
    pub size_bytes: i32,
    pub created_at: NaiveDateTime,
}

#[derive(Insertable)]
#[diesel(table_name = attachments)]
pub struct NewAttachment {
    pub issue_id: i32,
    pub comment_id: Option<i32>,
    pub uploader_id: i32,
    pub original_name: String,
    pub stored_name: String,
    pub content_type: String,
    pub size_bytes: i32,
    pub created_at: NaiveDateTime,
}

// ---------------------------------------------------------------- enums

/// Lifecycle of a maintenance request. Stored as lower-case TEXT.
///
/// * `unassigned` – submitted, nobody is responsible yet (initial state).
/// * `assigned`   – a maintainer has been assigned but has not started.
/// * `open`       – actively being worked on (or re-opened after closing).
/// * `closed`     – resolved; `closed_at` is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueState {
    Unassigned,
    Assigned,
    Open,
    Closed,
}

impl IssueState {
    pub const ALL: [IssueState; 4] = [
        IssueState::Unassigned,
        IssueState::Assigned,
        IssueState::Open,
        IssueState::Closed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            IssueState::Unassigned => "unassigned",
            IssueState::Assigned => "assigned",
            IssueState::Open => "open",
            IssueState::Closed => "closed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

pub const PRIORITIES: [&str; 4] = ["low", "normal", "high", "urgent"];

/// Kinds of conversation entries a person can post.
pub const COMMENT_KINDS: [&str; 3] = ["question", "response", "note"];
