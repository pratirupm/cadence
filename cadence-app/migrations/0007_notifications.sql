CREATE TABLE notifications (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL,
    read_at TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX notifications_user_id_idx ON notifications (user_id, created_at DESC);
