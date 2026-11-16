-- Users of the tracker. A user may have a password, one or more OAuth
-- identities, or both. Emails are stored lower-cased.
CREATE TABLE users (
    id            INTEGER PRIMARY KEY NOT NULL,
    email         TEXT    NOT NULL UNIQUE,
    display_name  TEXT    NOT NULL,
    password_hash TEXT,
    is_admin      BOOLEAN NOT NULL DEFAULT 0,
    created_at    TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- External login identities (Google, GitHub, Microsoft, ...).
CREATE TABLE oauth_identities (
    id         INTEGER PRIMARY KEY NOT NULL,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider   TEXT    NOT NULL,
    subject    TEXT    NOT NULL,
    email      TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (provider, subject)
);
CREATE INDEX oauth_identities_user_id ON oauth_identities(user_id);
