ALTER TABLE tasks ADD COLUMN workspace_id TEXT REFERENCES workspaces (id);
ALTER TABLE tasks ADD COLUMN project_id TEXT REFERENCES projects (id);
ALTER TABLE tasks ADD COLUMN created_by TEXT REFERENCES users (id);
ALTER TABLE tasks ADD COLUMN assigned_to TEXT REFERENCES users (id);
ALTER TABLE tasks ADD COLUMN notes TEXT NOT NULL DEFAULT '';
ALTER TABLE tasks ADD COLUMN due_at TEXT;
ALTER TABLE tasks ADD COLUMN priority INTEGER NOT NULL DEFAULT 0;
ALTER TABLE tasks ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE tasks ADD COLUMN deleted_at TEXT;

CREATE INDEX tasks_project_id_idx ON tasks (project_id, created_at DESC);
CREATE INDEX tasks_workspace_id_idx ON tasks (workspace_id, created_at DESC);
