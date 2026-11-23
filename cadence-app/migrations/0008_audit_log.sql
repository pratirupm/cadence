CREATE TABLE audit_log (
    id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    actor_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    action TEXT NOT NULL,
    target TEXT NOT NULL,
    metadata TEXT NOT NULL DEFAULT '',
    ip TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX audit_log_workspace_id_idx ON audit_log (workspace_id, created_at DESC);
