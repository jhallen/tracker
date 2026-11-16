//! Maintenance requests: dashboard, listing, creation, conversation,
//! state/assignment changes and photo download.

use std::collections::HashMap;

use axum::{
    Form,
    extract::{Multipart, Path, Query, State},
    http::header,
    response::{IntoResponse, Redirect, Response},
};
use chrono::Utc;
use diesel::prelude::*;
use serde::Deserialize;

use crate::{
    AppState,
    auth::CurrentUser,
    db,
    error::AppError,
    models::{
        Attachment, COMMENT_KINDS, Comment, Issue, IssueState, NewAttachment, NewComment,
        NewIssue, PRIORITIES, User,
    },
    sanitize::{clean_rich_text, is_blank_html, opt},
    schema::{attachments, comments, issues, users},
    uploads,
    views::*,
};

const MAX_PHOTOS: usize = 10;
const LIST_LIMIT: i64 = 200;

// ------------------------------------------------------------ helpers

fn user_names(conn: &mut SqliteConnection, ids: &[i32]) -> QueryResult<HashMap<i32, String>> {
    Ok(users::table
        .filter(users::id.eq_any(ids))
        .select((users::id, users::display_name))
        .load::<(i32, String)>(conn)?
        .into_iter()
        .collect())
}

fn user_options(conn: &mut SqliteConnection, selected: Option<i32>) -> QueryResult<Vec<UserOption>> {
    Ok(users::table
        .order(users::display_name.asc())
        .select((users::id, users::display_name))
        .load::<(i32, String)>(conn)?
        .into_iter()
        .map(|(id, name)| UserOption {
            id,
            name,
            selected: Some(id) == selected,
        })
        .collect())
}

/// Turn issues into list rows with names, counts and "awaiting reply".
fn to_rows(conn: &mut SqliteConnection, list: Vec<Issue>, viewer: i32) -> QueryResult<Vec<IssueRow>> {
    let ids: Vec<i32> = list.iter().map(|i| i.id).collect();
    let mut people: Vec<i32> = list.iter().map(|i| i.submitter_id).collect();
    people.extend(list.iter().filter_map(|i| i.assignee_id));
    let names = user_names(conn, &people)?;

    // Conversation entries (oldest first) to count and find the latest.
    let convo: Vec<(i32, i32, String)> = comments::table
        .filter(comments::issue_id.eq_any(&ids))
        .filter(comments::kind.ne("event"))
        .order(comments::id.asc())
        .select((comments::issue_id, comments::author_id, comments::kind))
        .load(conn)?;
    let mut counts: HashMap<i32, i64> = HashMap::new();
    let mut last: HashMap<i32, (i32, String)> = HashMap::new();
    for (issue_id, author, kind) in convo {
        *counts.entry(issue_id).or_default() += 1;
        last.insert(issue_id, (author, kind));
    }
    let photos: HashMap<i32, i64> = attachments::table
        .filter(attachments::issue_id.eq_any(&ids))
        .group_by(attachments::issue_id)
        .select((attachments::issue_id, diesel::dsl::count_star()))
        .load::<(i32, i64)>(conn)?
        .into_iter()
        .collect();

    Ok(list
        .into_iter()
        .map(|i| {
            let involved = i.submitter_id == viewer || i.assignee_id == Some(viewer);
            let needs_reply = involved
                && i.state != "closed"
                && matches!(last.get(&i.id), Some((author, kind)) if *author != viewer && kind == "question");
            IssueRow {
                id: i.id,
                submitter_name: names.get(&i.submitter_id).cloned().unwrap_or_default(),
                assignee_name: i.assignee_id.and_then(|a| names.get(&a).cloned()),
                comment_count: counts.get(&i.id).copied().unwrap_or(0),
                photo_count: photos.get(&i.id).copied().unwrap_or(0),
                needs_reply,
                created: Ts::new(i.created_at),
                updated: Ts::new(i.updated_at),
                title: i.title,
                state: i.state,
                priority: i.priority,
                location: i.location,
                equipment_code: i.equipment_code,
            }
        })
        .collect())
}

fn can_participate(user: &User, issue: &Issue) -> bool {
    user.is_admin || issue.submitter_id == user.id || issue.assignee_id == Some(user.id)
}

fn can_take(issue: &Issue) -> bool {
    issue.assignee_id.is_none() && issue.state != "closed"
}

fn event(issue_id: i32, author_id: i32, text: String) -> NewComment {
    NewComment {
        issue_id,
        author_id,
        kind: "event".into(),
        body_html: crate::error::html_escape(&text),
        created_at: Utc::now().naive_utc(),
    }
}

// ------------------------------------------------------------ dashboard

pub async fn dashboard(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Response, AppError> {
    let uid = user.id;
    let (assigned, submitted, unassigned) = db::run(&state.pool, move |conn| {
        let assigned = issues::table
            .filter(issues::assignee_id.eq(uid))
            .filter(issues::state.ne("closed"))
            .order(issues::updated_at.desc())
            .limit(LIST_LIMIT)
            .select(Issue::as_select())
            .load(conn)?;
        let submitted = issues::table
            .filter(issues::submitter_id.eq(uid))
            .order((issues::closed_at.is_not_null(), issues::updated_at.desc()))
            .limit(LIST_LIMIT)
            .select(Issue::as_select())
            .load(conn)?;
        let unassigned = issues::table
            .filter(issues::assignee_id.is_null())
            .filter(issues::state.ne("closed"))
            .order(issues::created_at.asc())
            .limit(50)
            .select(Issue::as_select())
            .load(conn)?;
        Ok((
            to_rows(conn, assigned, uid)?,
            to_rows(conn, submitted, uid)?,
            to_rows(conn, unassigned, uid)?,
        ))
    })
    .await?;
    render(&DashboardPage {
        nav: Nav::of(&user),
        assigned,
        submitted,
        unassigned,
    })
}

// ------------------------------------------------------------ list / search

#[derive(Deserialize, Default)]
pub struct ListQuery {
    q: Option<String>,
    state: Option<String>,
    scope: Option<String>,
}

pub async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(query): Query<ListQuery>,
) -> Result<Response, AppError> {
    let q = query.q.unwrap_or_default().trim().to_string();
    let st = query.state.unwrap_or_else(|| "active".into());
    let scope = query.scope.unwrap_or_else(|| "all".into());
    let uid = user.id;
    let (qq, stt, sc) = (q.clone(), st.clone(), scope.clone());
    let rows = db::run(&state.pool, move |conn| {
        let mut sel = issues::table.select(Issue::as_select()).into_boxed();
        if !qq.is_empty() {
            let like = format!("%{}%", qq.replace('%', "\\%").replace('_', "\\_"));
            sel = sel.filter(
                issues::title
                    .like(like.clone())
                    .escape('\\')
                    .or(issues::location.like(like.clone()).escape('\\'))
                    .or(issues::equipment_code.like(like).escape('\\')),
            );
        }
        match stt.as_str() {
            "active" => sel = sel.filter(issues::state.ne("closed")),
            s if IssueState::parse(s).is_some() => sel = sel.filter(issues::state.eq(s.to_string())),
            _ => {}
        }
        match sc.as_str() {
            "mine" => sel = sel.filter(issues::submitter_id.eq(uid)),
            "assigned" => sel = sel.filter(issues::assignee_id.eq(uid)),
            _ => {}
        }
        let list = sel.order(issues::updated_at.desc()).limit(LIST_LIMIT).load(conn)?;
        Ok(to_rows(conn, list, uid)?)
    })
    .await?;
    render(&IssueListPage {
        nav: Nav::of(&user),
        issues: rows,
        q,
        state: st,
        scope,
        states: IssueState::ALL.iter().map(|s| s.as_str()).collect(),
    })
}

// ------------------------------------------------------------ create

fn blank_new_page(user: &User, users: Vec<UserOption>) -> NewIssuePage {
    NewIssuePage {
        nav: Nav::of(user),
        error: None,
        title: String::new(),
        description: String::new(),
        location: String::new(),
        equipment_code: String::new(),
        equipment_code_format: String::new(),
        priority: "normal".into(),
        priorities: PRIORITIES.to_vec(),
        users,
    }
}

pub async fn new_form(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Response, AppError> {
    let users = db::run(&state.pool, |conn| Ok(user_options(conn, None)?)).await?;
    render(&blank_new_page(&user, users))
}

pub async fn create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let form = uploads::parse_multipart(multipart, state.config.max_upload_bytes, MAX_PHOTOS).await;
    let form = match form {
        Ok(f) => f,
        Err(AppError::BadRequest(msg)) => {
            let users = db::run(&state.pool, |conn| Ok(user_options(conn, None)?)).await?;
            let mut page = blank_new_page(&user, users);
            page.error = Some(msg);
            return render(&page);
        }
        Err(e) => return Err(e),
    };

    let title = opt(form.get("title")).unwrap_or_default();
    let raw_description = form.get("description").cloned().unwrap_or_default();
    let description = clean_rich_text(&raw_description);
    let location = opt(form.get("location"));
    let equipment_code = opt(form.get("equipment_code"));
    let equipment_code_format = opt(form.get("equipment_code_format")).filter(|_| equipment_code.is_some());
    let priority = opt(form.get("priority"))
        .filter(|p| PRIORITIES.contains(&p.as_str()))
        .unwrap_or_else(|| "normal".into());
    let assignee_id = opt(form.get("assignee_id")).and_then(|v| v.parse::<i32>().ok());

    let error = if title.is_empty() {
        Some("Please give the request a short title.")
    } else if title.chars().count() > 200 {
        Some("The title is too long (200 characters max).")
    } else if is_blank_html(&description) {
        Some("Please describe the problem.")
    } else {
        None
    };
    if let Some(msg) = error {
        let users = db::run(&state.pool, move |conn| Ok(user_options(conn, assignee_id)?)).await?;
        return render(&NewIssuePage {
            nav: Nav::of(&user),
            error: Some(format!("{msg} (Photos need to be re-attached.)")),
            title,
            description: if is_blank_html(&description) { String::new() } else { description },
            location: location.unwrap_or_default(),
            equipment_code: equipment_code.unwrap_or_default(),
            equipment_code_format: equipment_code_format.unwrap_or_default(),
            priority,
            priorities: PRIORITIES.to_vec(),
            users,
        });
    }

    let dir = state.config.upload_dir.clone();
    let photo_rows = uploads::store_photos(&dir, form.photos, user.id).await?;
    let uid = user.id;
    let result = {
        let rows: Vec<NewAttachment> = photo_rows.iter().map(clone_attachment).collect();
        db::run(&state.pool, move |conn| {
            conn.transaction::<_, AppError, _>(|conn| {
                let now = Utc::now().naive_utc();
                if let Some(a) = assignee_id {
                    // Make sure the assignee exists.
                    users::table.find(a).select(users::id).first::<i32>(conn)?;
                }
                let issue: Issue = diesel::insert_into(issues::table)
                    .values(NewIssue {
                        title,
                        description_html: description,
                        location,
                        equipment_code,
                        equipment_code_format,
                        priority,
                        state: if assignee_id.is_some() { "assigned" } else { "unassigned" }.into(),
                        submitter_id: uid,
                        assignee_id,
                        created_at: now,
                        updated_at: now,
                    })
                    .returning(Issue::as_returning())
                    .get_result(conn)?;
                let rows: Vec<NewAttachment> = rows
                    .into_iter()
                    .map(|mut r| {
                        r.issue_id = issue.id;
                        r
                    })
                    .collect();
                diesel::insert_into(attachments::table).values(&rows).execute(conn)?;
                Ok(issue.id)
            })
        })
        .await
    };
    match result {
        Ok(id) => Ok(Redirect::to(&format!("/issues/{id}")).into_response()),
        Err(e) => {
            uploads::discard(&dir, &photo_rows).await;
            Err(e)
        }
    }
}

fn clone_attachment(a: &NewAttachment) -> NewAttachment {
    NewAttachment {
        issue_id: a.issue_id,
        comment_id: a.comment_id,
        uploader_id: a.uploader_id,
        original_name: a.original_name.clone(),
        stored_name: a.stored_name.clone(),
        content_type: a.content_type.clone(),
        size_bytes: a.size_bytes,
        created_at: a.created_at,
    }
}

// ------------------------------------------------------------ show

async fn issue_page(
    state: &AppState,
    user: &User,
    id: i32,
    error: Option<String>,
) -> Result<Response, AppError> {
    let viewer = user.clone();
    let page = db::run(&state.pool, move |conn| {
        let issue: Issue = issues::table.find(id).select(Issue::as_select()).first(conn)?;
        let convo: Vec<Comment> = comments::table
            .filter(comments::issue_id.eq(id))
            .order(comments::id.asc())
            .select(Comment::as_select())
            .load(conn)?;
        let files: Vec<Attachment> = attachments::table
            .filter(attachments::issue_id.eq(id))
            .order(attachments::id.asc())
            .select(Attachment::as_select())
            .load(conn)?;
        let mut people: Vec<i32> = convo.iter().map(|c| c.author_id).collect();
        people.push(issue.submitter_id);
        people.extend(issue.assignee_id);
        let names = user_names(conn, &people)?;
        let users = user_options(conn, issue.assignee_id)?;

        let mut issue_photos = Vec::new();
        let mut comment_photos: HashMap<i32, Vec<PhotoView>> = HashMap::new();
        for f in files {
            let v = PhotoView { id: f.id, name: f.original_name };
            match f.comment_id {
                Some(c) => comment_photos.entry(c).or_default().push(v),
                None => issue_photos.push(v),
            }
        }
        let role = |uid: i32| {
            if uid == issue.submitter_id {
                "Submitter"
            } else if Some(uid) == issue.assignee_id {
                "Assignee"
            } else {
                ""
            }
            .to_string()
        };
        let comments: Vec<CommentView> = convo
            .into_iter()
            .map(|c| CommentView {
                id: c.id,
                author_name: names.get(&c.author_id).cloned().unwrap_or_default(),
                is_mine: c.author_id == viewer.id,
                role: role(c.author_id),
                photos: comment_photos.remove(&c.id).unwrap_or_default(),
                created: Ts::new(c.created_at),
                kind: c.kind,
                body_html: c.body_html,
            })
            .collect();
        let participate = can_participate(&viewer, &issue);
        // Assignees usually answer; submitters usually ask.
        let default_kind = if issue.assignee_id == Some(viewer.id) { "response" } else { "question" };
        Ok(IssuePage {
            nav: Nav::of(&viewer),
            error,
            can_participate: participate,
            can_take: can_take(&issue),
            states: IssueState::ALL.iter().map(|s| s.as_str()).collect(),
            priorities: PRIORITIES.to_vec(),
            users,
            photos: issue_photos,
            comments,
            default_kind: default_kind.into(),
            issue: IssueView {
                id: issue.id,
                submitter_name: names.get(&issue.submitter_id).cloned().unwrap_or_default(),
                assignee_name: issue.assignee_id.and_then(|a| names.get(&a).cloned()),
                assignee_id: issue.assignee_id,
                created: Ts::new(issue.created_at),
                updated: Ts::new(issue.updated_at),
                closed: issue.closed_at.map(Ts::new),
                title: issue.title,
                description_html: issue.description_html,
                location: issue.location,
                equipment_code: issue.equipment_code,
                equipment_code_format: issue.equipment_code_format,
                priority: issue.priority,
                state: issue.state,
            },
        })
    })
    .await?;
    render(&page)
}

pub async fn show(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i32>,
) -> Result<Response, AppError> {
    issue_page(&state, &user, id, None).await
}

// ------------------------------------------------------------ conversation

pub async fn add_comment(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i32>,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let issue: Issue = db::run(&state.pool, move |conn| {
        Ok(issues::table.find(id).select(Issue::as_select()).first(conn)?)
    })
    .await?;
    if !can_participate(&user, &issue) {
        return Err(AppError::Forbidden);
    }
    let form = match uploads::parse_multipart(multipart, state.config.max_upload_bytes, MAX_PHOTOS).await {
        Ok(f) => f,
        Err(AppError::BadRequest(msg)) => return issue_page(&state, &user, id, Some(msg)).await,
        Err(e) => return Err(e),
    };
    let kind = opt(form.get("kind"))
        .filter(|k| COMMENT_KINDS.contains(&k.as_str()))
        .unwrap_or_else(|| "note".into());
    let body = clean_rich_text(form.get("body").map(String::as_str).unwrap_or(""));
    if is_blank_html(&body) && form.photos.is_empty() {
        return issue_page(&state, &user, id, Some("Please type a message or attach a photo.".into())).await;
    }
    let body = if is_blank_html(&body) { String::new() } else { body };

    let dir = state.config.upload_dir.clone();
    let photo_rows = uploads::store_photos(&dir, form.photos, user.id).await?;
    let rows: Vec<NewAttachment> = photo_rows.iter().map(clone_attachment).collect();
    let uid = user.id;
    let res = db::run(&state.pool, move |conn| {
        conn.transaction::<_, AppError, _>(|conn| {
            let now = Utc::now().naive_utc();
            let comment: Comment = diesel::insert_into(comments::table)
                .values(NewComment {
                    issue_id: id,
                    author_id: uid,
                    kind,
                    body_html: body,
                    created_at: now,
                })
                .returning(Comment::as_returning())
                .get_result(conn)?;
            let rows: Vec<NewAttachment> = rows
                .into_iter()
                .map(|mut r| {
                    r.issue_id = id;
                    r.comment_id = Some(comment.id);
                    r
                })
                .collect();
            diesel::insert_into(attachments::table).values(&rows).execute(conn)?;
            diesel::update(issues::table.find(id))
                .set(issues::updated_at.eq(now))
                .execute(conn)?;
            Ok(comment.id)
        })
    })
    .await;
    match res {
        Ok(cid) => Ok(Redirect::to(&format!("/issues/{id}?posted={cid}#c{cid}")).into_response()),
        Err(e) => {
            uploads::discard(&dir, &photo_rows).await;
            Err(e)
        }
    }
}

// ------------------------------------------------------------ manage

#[derive(Deserialize)]
pub struct UpdateForm {
    state: String,
    #[serde(default)]
    assignee_id: String,
    priority: String,
}

pub async fn update(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i32>,
    Form(form): Form<UpdateForm>,
) -> Result<Response, AppError> {
    let actor = user.clone();
    let res = db::run(&state.pool, move |conn| {
        conn.transaction::<_, AppError, _>(|conn| {
            let issue: Issue = issues::table.find(id).select(Issue::as_select()).first(conn)?;
            if !can_participate(&actor, &issue) {
                return Err(AppError::Forbidden);
            }
            let old_state = IssueState::parse(&issue.state).unwrap_or(IssueState::Unassigned);
            let mut new_state = IssueState::parse(&form.state)
                .ok_or_else(|| AppError::BadRequest("Unknown state.".into()))?;
            let mut assignee = form.assignee_id.trim().parse::<i32>().ok();
            if let Some(a) = assignee {
                users::table.find(a).select(users::id).first::<i32>(conn)?;
            }
            let priority = if PRIORITIES.contains(&form.priority.as_str()) {
                form.priority.clone()
            } else {
                issue.priority.clone()
            };

            // Keep state and assignee consistent (see THEORY_OF_OPERATION.md).
            let state_changed = new_state != old_state;
            if assignee != issue.assignee_id && !state_changed {
                if assignee.is_some() && new_state == IssueState::Unassigned {
                    new_state = IssueState::Assigned;
                }
                if assignee.is_none() && new_state == IssueState::Assigned {
                    new_state = IssueState::Unassigned;
                }
            }
            if new_state == IssueState::Unassigned {
                assignee = None;
            }
            if new_state == IssueState::Assigned && assignee.is_none() {
                return Ok(Err("Choose who the request is assigned to.".to_string()));
            }

            let now = Utc::now().naive_utc();
            let mut events = Vec::new();
            if new_state != old_state {
                events.push(format!("changed state from {} to {}.", old_state.as_str(), new_state.as_str()));
            }
            if assignee != issue.assignee_id {
                let mut ids = vec![];
                ids.extend(assignee);
                ids.extend(issue.assignee_id);
                let names = user_names(conn, &ids)?;
                events.push(match (assignee, issue.assignee_id) {
                    (Some(a), _) => format!("assigned to {}.", names.get(&a).cloned().unwrap_or_default()),
                    (None, Some(old)) => format!(
                        "removed assignee {}.",
                        names.get(&old).cloned().unwrap_or_default()
                    ),
                    (None, None) => unreachable!(),
                });
            }
            if priority != issue.priority {
                events.push(format!("changed priority from {} to {}.", issue.priority, priority));
            }
            if events.is_empty() {
                return Ok(Ok(()));
            }
            let closed_at = match (old_state, new_state) {
                (_, IssueState::Closed) if old_state != IssueState::Closed => Some(now),
                (_, IssueState::Closed) => issue.closed_at,
                _ => None,
            };
            diesel::update(issues::table.find(id))
                .set((
                    issues::state.eq(new_state.as_str()),
                    issues::assignee_id.eq(assignee),
                    issues::priority.eq(&priority),
                    issues::closed_at.eq(closed_at),
                    issues::updated_at.eq(now),
                ))
                .execute(conn)?;
            let rows: Vec<NewComment> = events.into_iter().map(|t| event(id, actor.id, t)).collect();
            diesel::insert_into(comments::table).values(&rows).execute(conn)?;
            Ok(Ok(()))
        })
    })
    .await?;
    match res {
        Ok(()) => Ok(Redirect::to(&format!("/issues/{id}")).into_response()),
        Err(msg) => issue_page(&state, &user, id, Some(msg)).await,
    }
}

/// "I'll fix it": assign an unassigned request to yourself.
pub async fn take(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i32>,
) -> Result<Response, AppError> {
    let uid = user.id;
    db::run(&state.pool, move |conn| {
        conn.transaction::<_, AppError, _>(|conn| {
            let issue: Issue = issues::table.find(id).select(Issue::as_select()).first(conn)?;
            if !can_take(&issue) {
                return Err(AppError::BadRequest("This request already has an assignee.".into()));
            }
            let new_state = if issue.state == "unassigned" { "assigned" } else { issue.state.as_str() };
            diesel::update(issues::table.find(id))
                .set((
                    issues::assignee_id.eq(Some(uid)),
                    issues::state.eq(new_state),
                    issues::updated_at.eq(Utc::now().naive_utc()),
                ))
                .execute(conn)?;
            diesel::insert_into(comments::table)
                .values(event(id, uid, "took this request.".to_string()))
                .execute(conn)?;
            Ok(())
        })
    })
    .await?;
    Ok(Redirect::to(&format!("/issues/{id}")).into_response())
}

// ------------------------------------------------------------ photos

pub async fn attachment(
    State(state): State<AppState>,
    CurrentUser(_user): CurrentUser,
    Path(id): Path<i32>,
) -> Result<Response, AppError> {
    let a: Attachment = db::run(&state.pool, move |conn| {
        Ok(attachments::table.find(id).select(Attachment::as_select()).first(conn)?)
    })
    .await?;
    let path = state.config.upload_dir.join(&a.stored_name);
    let data = tokio::fs::read(&path).await.map_err(|_| AppError::NotFound)?;
    let disposition = format!(
        "inline; filename=\"{}\"",
        a.original_name.replace(['"', '\\'], "_")
    );
    Ok((
        [
            (header::CONTENT_TYPE, a.content_type),
            (header::CACHE_CONTROL, "private, max-age=86400".to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        data,
    )
        .into_response())
}
