-- Email address verification.
-- users.email_verified_at is NULL until the user clicks the link we email
-- them (or until an OAuth provider vouches for the address).
ALTER TABLE users ADD COLUMN email_verified_at TIMESTAMP;

-- One row per verification email sent. Only the SHA-256 of the token in
-- the link is stored. A token is valid until expires_at and only once.
CREATE TABLE email_verifications (
    id         INTEGER PRIMARY KEY NOT NULL,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    email      TEXT    NOT NULL,
    token_hash TEXT    NOT NULL UNIQUE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP NOT NULL,
    used_at    TIMESTAMP
);
CREATE INDEX email_verifications_user_id ON email_verifications(user_id);
