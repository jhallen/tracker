-- Server-side login sessions. The id is a random token stored in the
-- browser's HttpOnly session cookie.
CREATE TABLE sessions (
    id         TEXT    PRIMARY KEY NOT NULL,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP NOT NULL
);
CREATE INDEX sessions_user_id ON sessions(user_id);
