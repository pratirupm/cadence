CREATE TABLE reminders (
    id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    remind_at TEXT NOT NULL,
    channel TEXT NOT NULL DEFAULT 'email' CHECK (channel IN ('email', 'in_app')),
    sent_at TEXT,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'sending', 'sent', 'failed')),
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX reminders_due_idx ON reminders (status, remind_at);
CREATE INDEX reminders_task_id_idx ON reminders (task_id);
