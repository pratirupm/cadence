CREATE TABLE changes (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_id TEXT NOT NULL,
    entity TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    op TEXT NOT NULL,
    version INTEGER NOT NULL,
    payload TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX changes_workspace_seq_idx ON changes (workspace_id, seq);

CREATE TABLE sync_operations (
    op_id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    response TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TRIGGER tasks_after_insert
AFTER INSERT ON tasks
BEGIN
    INSERT INTO changes (workspace_id, entity, entity_id, op, version, payload, created_at)
    VALUES (
        NEW.workspace_id,
        'task',
        NEW.id,
        'create',
        NEW.version,
        json_object(
            'id', NEW.id,
            'workspace_id', NEW.workspace_id,
            'project_id', NEW.project_id,
            'title', NEW.title,
            'notes', NEW.notes,
            'completed', NEW.completed,
            'priority', NEW.priority,
            'due_at', NEW.due_at,
            'version', NEW.version,
            'deleted_at', NEW.deleted_at
        ),
        NEW.updated_at
    );
END;

CREATE TRIGGER tasks_after_update
AFTER UPDATE ON tasks
BEGIN
    INSERT INTO changes (workspace_id, entity, entity_id, op, version, payload, created_at)
    VALUES (
        NEW.workspace_id,
        'task',
        NEW.id,
        CASE
            WHEN NEW.deleted_at IS NOT NULL AND OLD.deleted_at IS NULL THEN 'delete'
            ELSE 'update'
        END,
        NEW.version,
        json_object(
            'id', NEW.id,
            'workspace_id', NEW.workspace_id,
            'project_id', NEW.project_id,
            'title', NEW.title,
            'notes', NEW.notes,
            'completed', NEW.completed,
            'priority', NEW.priority,
            'due_at', NEW.due_at,
            'version', NEW.version,
            'deleted_at', NEW.deleted_at
        ),
        NEW.updated_at
    );
END;
