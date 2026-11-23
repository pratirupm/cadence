CREATE INDEX IF NOT EXISTS tasks_workspace_status_due_idx
    ON tasks (workspace_id, completed, due_at);

CREATE INDEX IF NOT EXISTS tasks_project_status_idx
    ON tasks (project_id, completed, created_at);

CREATE INDEX IF NOT EXISTS sessions_expires_at_idx
    ON sessions (expires_at);

CREATE INDEX IF NOT EXISTS webhook_deliveries_webhook_idx
    ON webhook_deliveries (webhook_id, created_at);
