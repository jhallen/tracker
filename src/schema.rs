// @generated automatically by Diesel CLI.

diesel::table! {
    attachments (id) {
        id -> Integer,
        issue_id -> Integer,
        comment_id -> Nullable<Integer>,
        uploader_id -> Integer,
        original_name -> Text,
        stored_name -> Text,
        content_type -> Text,
        size_bytes -> Integer,
        created_at -> Timestamp,
    }
}

diesel::table! {
    comments (id) {
        id -> Integer,
        issue_id -> Integer,
        author_id -> Integer,
        kind -> Text,
        body_html -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    email_verifications (id) {
        id -> Integer,
        user_id -> Integer,
        email -> Text,
        token_hash -> Text,
        created_at -> Timestamp,
        expires_at -> Timestamp,
        used_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    issues (id) {
        id -> Integer,
        title -> Text,
        description_html -> Text,
        location -> Nullable<Text>,
        equipment_code -> Nullable<Text>,
        equipment_code_format -> Nullable<Text>,
        priority -> Text,
        state -> Text,
        submitter_id -> Integer,
        assignee_id -> Nullable<Integer>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        closed_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    oauth_identities (id) {
        id -> Integer,
        user_id -> Integer,
        provider -> Text,
        subject -> Text,
        email -> Nullable<Text>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    sessions (id) {
        id -> Text,
        user_id -> Integer,
        created_at -> Timestamp,
        expires_at -> Timestamp,
    }
}

diesel::table! {
    users (id) {
        id -> Integer,
        email -> Text,
        display_name -> Text,
        password_hash -> Nullable<Text>,
        is_admin -> Bool,
        created_at -> Timestamp,
        email_verified_at -> Nullable<Timestamp>,
    }
}

diesel::joinable!(attachments -> comments (comment_id));
diesel::joinable!(attachments -> issues (issue_id));
diesel::joinable!(attachments -> users (uploader_id));
diesel::joinable!(comments -> issues (issue_id));
diesel::joinable!(comments -> users (author_id));
diesel::joinable!(email_verifications -> users (user_id));
diesel::joinable!(oauth_identities -> users (user_id));
diesel::joinable!(sessions -> users (user_id));

diesel::allow_tables_to_appear_in_same_query!(
    attachments,
    comments,
    email_verifications,
    issues,
    oauth_identities,
    sessions,
    users,
);
