-- Maintenance requests ("issues").
CREATE TABLE issues (
    id                    INTEGER PRIMARY KEY NOT NULL,
    title                 TEXT    NOT NULL,
    description_html      TEXT    NOT NULL,
    location              TEXT,
    equipment_code        TEXT,
    equipment_code_format TEXT,
    priority              TEXT    NOT NULL DEFAULT 'normal'
                          CHECK (priority IN ('low', 'normal', 'high', 'urgent')),
    state                 TEXT    NOT NULL DEFAULT 'unassigned'
                          CHECK (state IN ('open', 'closed', 'unassigned', 'assigned')),
    submitter_id          INTEGER NOT NULL REFERENCES users(id),
    assignee_id           INTEGER REFERENCES users(id),
    created_at            TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at            TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    closed_at             TIMESTAMP
);
CREATE INDEX issues_submitter_id ON issues(submitter_id);
CREATE INDEX issues_assignee_id ON issues(assignee_id);
CREATE INDEX issues_state ON issues(state);
CREATE INDEX issues_equipment_code ON issues(equipment_code);

-- The conversation on an issue. kind: question | response | note | event
-- ('event' rows are written by the system, e.g. "state changed to closed").
CREATE TABLE comments (
    id         INTEGER PRIMARY KEY NOT NULL,
    issue_id   INTEGER NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    author_id  INTEGER NOT NULL REFERENCES users(id),
    kind       TEXT    NOT NULL DEFAULT 'note'
               CHECK (kind IN ('question', 'response', 'note', 'event')),
    body_html  TEXT    NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX comments_issue_id ON comments(issue_id);

-- Photos attached to an issue (and optionally to a specific comment).
-- Files live in UPLOAD_DIR under stored_name.
CREATE TABLE attachments (
    id            INTEGER PRIMARY KEY NOT NULL,
    issue_id      INTEGER NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    comment_id    INTEGER REFERENCES comments(id) ON DELETE CASCADE,
    uploader_id   INTEGER NOT NULL REFERENCES users(id),
    original_name TEXT    NOT NULL,
    stored_name   TEXT    NOT NULL UNIQUE,
    content_type  TEXT    NOT NULL,
    size_bytes    INTEGER NOT NULL,
    created_at    TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX attachments_issue_id ON attachments(issue_id);
