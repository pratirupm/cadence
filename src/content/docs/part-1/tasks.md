---
title: "Tasks"
description: "Move tasks into workspaces, add versions, and make every mutation honest."
---

The workspace and project tables exist, but the task table is still the
single-user table from chapter 2. It has no owner, no project, and no way to
tell a stale edit from a current one. This chapter moves tasks into the tenancy
model and gives every mutation a version.

The task is the first type in the book that is both a domain object and a
collaboration object. Two people can open the same task, one can rename it while
the other completes it, and a third can delete it from a stale tab. The service
has to choose an answer for each race. It chooses optimistic concurrency:
mutations carry the version the client saw, and a mismatch is a conflict, not a
last-write-wins surprise.

By the end of this chapter you can:

- create a task inside a project and workspace
- list a project’s tasks with open items first
- read a task only if you can access its project
- rename, complete, reopen, and soft-delete with a version check
- reject a viewer or an outsider
- keep a deleted task out of reads while preserving its tombstone
- explain why `rows_affected() == 0` means “conflict,” not “not found”

The Rust lessons are `Option` fields, compare-and-set updates, `rows_affected`,
soft deletes, and keeping authorization in the service. The SQL lesson is that a
`WHERE` clause can be a concurrency control, not just a filter.

## From a global list to a owned task

Chapter 2’s task had five fields:

```text
id, title, completed, created_at, updated_at
```

The service now needs to know:

- which workspace and project own it
- who created it and who it is assigned to
- notes, priority, and an optional due date
- an optional recurrence rule and time zone
- a version for conflict detection
- whether it is deleted

The table grows, but the type stays readable because every field has a job. A
task is not a bag of optional columns; it is a record with a small set of
invariants: a title, a project, a workspace, a version, and a deletion state.

## The migration

Create `migrations/0004_tasks_in_workspaces.sql`:

```sql
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
```

`ALTER TABLE ... ADD COLUMN` keeps the chapter-2 rows. The new columns are
nullable because SQLite cannot add a `NOT NULL` column without a default, and
there is no meaningful default workspace for an old task. The service writes
every new row with all four IDs; the schema’s flexibility is for migration, not
for new code.

If this were a production migration with real data, the next step would be a
backfill: create a personal workspace for each existing owner, attach the old
tasks, then add `NOT NULL` constraints with a table rebuild. The book keeps the
simpler path because the task table is still a teaching table, not customer
data. Say that out loud in a real migration plan; silent nullable columns are a
source of orphaned rows.

The indexes match the two reads this chapter cares about: tasks by project and
tasks by workspace. Chapter 22 adds completed/due indexes after measuring.

## The task type

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct Task {
    pub id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub title: String,
    pub notes: String,
    pub completed: bool,
    pub priority: i64,
    pub due_at: Option<DateTime<Utc>>,
    pub recurrence: Option<String>,
    pub recurrence_timezone: Option<String>,
    pub created_by: String,
    pub assigned_to: Option<String>,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}
```

`Serialize` is here because the JSON API returns this type. `FromRow` maps the
columns. The `Option` fields are honest: a task may have no due date, no
assignee, no recurrence, and not be deleted.

Do not use empty strings for “no due date” or `0` for “unassigned.” Empty values
are indistinguishable from real empty values and force every consumer to know
the convention. `Option` puts the absence in the type.

`deleted_at` is part of the type even though most reads hide it. The repository
needs to know whether a row is a tombstone; the sync layer needs to send it. A
separate `DeletedTask` type would duplicate every field for no benefit.

## Errors

```rust
#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("task title cannot be empty")]
    EmptyTitle,
    #[error("task not found")]
    NotFound,
    #[error("project not found")]
    ProjectNotFound,
    #[error("you do not have permission to do that")]
    Forbidden,
    #[error("the task changed since you loaded it")]
    Conflict,
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Query(#[from] QueryError),
    #[error(transparent)]
    Recurrence(#[from] RecurrenceError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
```

`Conflict` is the new idea. A missing row is `NotFound`; a row that changed
under you is `Conflict`. They map to different HTTP statuses and different UI
messages. Do not collapse them into “not found” to avoid a branch. The client
needs to know whether to reload or to create.

`Workspace`, `Query`, and `Recurrence` are transparent wrappers. The service can
use `?` on a workspace permission check, a filter parse, or a recurrence rule
without translating every variant. `Error` keeps the chain for logs.

## Authorization before data

Every public method begins with the same two checks:

```rust
self.require_write(workspace_id, actor_id).await?;
self.require_project_access(actor_id, workspace_id, project_id).await?;
```

The first asks “is this user allowed to write in this workspace?” The second
asks “can this user see this project?” They are separate because a workspace
member may be a viewer or may be excluded from a private project.

`require_project_access` in this chapter is:

```rust
async fn require_project_access(
    &self,
    actor_id: &str,
    workspace_id: &str,
    project_id: &str,
) -> Result<(), TaskError> {
    self.require_project(workspace_id, project_id).await?;
    if self.workspaces().can_access_project(actor_id, project_id).await? {
        Ok(())
    } else {
        Err(TaskError::Forbidden)
    }
}
```

`require_project` verifies that the project exists, is not archived, and belongs
to the claimed workspace. That last check prevents a caller from passing a
workspace they can write to and a project from a different tenant. Never trust
two identifiers to agree; join or compare them.

Reads use `require_member` plus the same project check. A viewer can read; a
non-member cannot.

## Creating a task

```rust
pub async fn create(
    &self,
    actor_id: &str,
    workspace_id: &str,
    project_id: &str,
    title: &str,
) -> Result<Task, TaskError> {
    let title = normalize_title(title)?;
    self.require_write(workspace_id, actor_id).await?;
    self.require_project_access(actor_id, workspace_id, project_id).await?;

    let now = Utc::now();
    let task = Task {
        id: Ulid::new().to_string(),
        workspace_id: workspace_id.to_owned(),
        project_id: project_id.to_owned(),
        title,
        notes: String::new(),
        completed: false,
        priority: 0,
        due_at: None,
        recurrence: None,
        recurrence_timezone: None,
        created_by: actor_id.to_owned(),
        assigned_to: None,
        version: 1,
        created_at: now,
        updated_at: now,
        deleted_at: None,
    };

    // INSERT with the same values
    Ok(task)
}
```

`normalize_title` trims and rejects an empty result. It runs before the
authorization checks? In the current code it runs first, which means an
anonymous caller can learn that a title is empty without learning whether the
project exists. That is a minor information leak. If you care, run authorization
first, then validation. The book keeps the current order because the leak is
small and the validation is cheap; exercise 4 asks you to swap them and reason
about the difference.

The insert writes `version = 1`. Every later mutation increments it. The first
version is not zero because a new task already has one committed state.

## Listing

The default list is:

```sql
SELECT id, workspace_id, project_id, title, notes, completed, priority, due_at,
       recurrence, recurrence_timezone, created_by, assigned_to, version,
       created_at, updated_at, deleted_at
FROM tasks
WHERE project_id = ? AND deleted_at IS NULL
ORDER BY completed, created_at DESC, id DESC
```

`completed` sorts `0` before `1`, so open tasks appear first. Within each group,
newest first. The `id` tie-breaker keeps the order stable when timestamps are
equal.

`list_for_project` delegates to `list_filtered(..., "")`. The filter DSL is
chapter 7; the important structure is already here: the SQL is built once, the
project ID is bound first, and the filter parameters are appended in order.

Do not fetch a project’s tasks and then filter by workspace in Rust. The query
should be scoped from the start. A forgotten `WHERE` is the classic multi-tenant
bug.

## Reading one task

```rust
pub async fn get(&self, actor_id: &str, id: &str) -> Result<Task, TaskError> {
    let task = self.find(id).await?;
    self.require_member(&task.workspace_id, actor_id).await?;
    self.require_project_access(actor_id, &task.workspace_id, &task.project_id).await?;
    Ok(task)
}
```

`find` filters `deleted_at IS NULL`. A deleted task is not “found” for ordinary
reads. The sync layer uses the tombstone directly, not this method.

The order matters: find the task first, then authorize with its own workspace
and project. Do not accept workspace/project IDs from the caller for a read;
the row already knows them. Trusting caller-supplied scope is how IDOR bugs
happen.

## Optimistic concurrency

Every mutation uses a compare-and-set:

```rust
UPDATE tasks
SET completed = ?, version = version + 1, updated_at = ?
WHERE id = ? AND version = ? AND deleted_at IS NULL
```

The final `?` is the version the service read at the start of the method. If
another request changed the row, the version no longer matches and zero rows
update. The service returns `Conflict`.

Why not lock the row for the whole request? A lock would serialize every editor
and hold a database connection across the UI round trip. Optimistic concurrency
lets both readers proceed and pays only when they actually collide. The cost is
that the client must handle a conflict, which is a real product requirement, not
an implementation detail.

`rows_affected() == 0` has two possible causes: the row disappeared (deleted)
or the version changed. The method already loaded the task and knows it existed,
so the likely cause is a conflict. It returns `Conflict` rather than re-reading
and guessing. A more precise implementation could distinguish `NotFound` by
re-checking, but that adds a query and a race of its own. Pick one and test it.

## Rename, complete, delete

Rename and completion are the same shape:

```rust
let task = self.find(id).await?;
self.require_write(&task.workspace_id, actor_id).await?;
self.require_project_access(...).await?;

let result = sqlx::query(
    "UPDATE tasks SET title = ?, version = version + 1, updated_at = ?
     WHERE id = ? AND version = ? AND deleted_at IS NULL",
)
.bind(title)
.bind(Utc::now())
.bind(id)
.bind(task.version)
.execute(&self.pool)
.await?;

if result.rows_affected() == 0 {
    return Err(TaskError::Conflict);
}

self.find(id).await
```

Returning the reloaded task gives the caller the new version. The client needs
that version for its next edit. Returning `()` would force a second request and
create another race.

Completion calls `create_next_occurrence` when the task has a recurrence rule.
That is chapter 9; the hook is here so the completion contract does not change
later. A recurring task is completed once and the next occurrence is a new row.

Delete is a soft delete:

```rust
UPDATE tasks
SET deleted_at = ?, version = version + 1, updated_at = ?
WHERE id = ? AND version = ? AND deleted_at IS NULL
```

A hard `DELETE` would remove the row before sync clients learned about it. The
tombstone lets chapter 14 send “this task is gone” with a version. A cleanup job
can eventually remove tombstones after every client has acknowledged them; this
book does not add that job, but the design leaves room.

`delete` returns `()`, not the deleted task. The caller usually redirects to a
list. If the UI needs the title for a flash message, it already had it. Keep the
return type honest.

## The sync update hook

`apply_sync_update` is a variant for the CLI:

```rust
pub async fn apply_sync_update(
    &self,
    actor_id: &str,
    id: &str,
    title: Option<&str>,
    completed: Option<bool>,
    base_version: i64,
) -> Result<Task, TaskError> {
    let task = self.find(id).await?;
    // authorization ...
    if task.version != base_version {
        return Err(TaskError::Conflict);
    }
    // UPDATE with COALESCE(?, title), COALESCE(?, completed)
}
```

It accepts partial fields because a sync operation may only change one of them.
`COALESCE(?, title)` means “keep the old value when the parameter is null.” The
version check is explicit before the update and repeated in the `WHERE` clause.
The first check produces a `Conflict` before any write; the second protects
against a race between the check and the update.

`set_recurrence` is another version-checked mutation. It validates the rule
before writing, so an invalid recurrence never reaches the table. That method is
chapter 9’s entry point; the authorization pattern is identical.

## Tests

The task suite uses a fixture that creates a user, workspace, and project
through the real services:

```rust
async fn fixture() -> Fixture {
    // in-memory DB, migrations
    // owner, workspace, project
}
```

Nine tests currently cover the task contract:

- `creates_and_lists_tasks_in_a_project`: creation, version 1, list.
- `trims_titles_and_rejects_empty_ones`: validation before insert.
- `completes_reopens_and_renames_a_task`: version increments 1 → 2 → 3.
- `soft_deletes_a_task`: `get` returns `NotFound`, list is empty.
- `viewers_cannot_change_tasks`: read allowed, write forbidden.
- `private_projects_require_project_membership`: no access until project member.
- `filters_tasks_with_the_query_language`: chapter 7 at the service boundary.
- `outsiders_cannot_read_the_project`: no membership, no list.
- `completing_a_recurring_task_creates_the_next_occurrence`: chapter 9.

At the chapter-5 checkpoint, the recurrence and filter tests may not exist yet.
The important assertions are the first six. They prove the authorization and
concurrency contract, not the query language.

Run:

```console
$ cargo test tasks
```

## Checkpoint

```console
$ cargo test tasks
$ cargo test
$ cargo run -- migrate
```

There is no task form until chapter 6, and no API until chapter 13. The service
and its tests are the checkpoint. If you want a manual read, use a small test or
the future API; do not add a temporary handler just to see a row.

## Failure modes you will meet

**A task is visible in the wrong workspace.** The read used the task’s project
ID but not its workspace, or the project/workspace IDs came from the caller
independently. Compare the project’s workspace to the task’s workspace and
authorize on the row.

**`Conflict` on the first edit after loading.** The client sent an old version.
Reload the task, show the current values, and let the user retry. Do not
automatically overwrite; that defeats the version.

**A viewer gets `Forbidden` on a read.** Reads call `require_member`, not
`require_write`. If a viewer is forbidden on a read, the code used the wrong
helper or the project is private and the viewer is not a project member.

**A deleted task still appears.** The list query filters `deleted_at IS NULL`;
the detail query must too. A soft delete is only soft if every read agrees.

**A task is created in a project from another workspace.** `require_project`
compares the project’s workspace to the claimed one. Without it, a caller can
use a workspace they own and a project they should not touch.

**The version jumps by two.** A method called `find` and then another method that
also increments, or a recurrence inserted a second row. Trace the mutations;
each public write should increment once.

**`rows_affected() == 0` is treated as `NotFound`.** The row was loaded, so the
likely cause is a version mismatch. Return `Conflict` and test the two-client
case.

## Under the hood

Optimistic concurrency is a compare-and-set. The database checks the version in
the same statement that writes, so there is no window between “check” and
“write” in the service. The `WHERE` clause is the lock.

A soft delete trades storage for recoverability and sync. The row stays, the
version increments, and every ordinary query filters it. A hard delete would be
simpler until a second client needs to learn that the task disappeared.

ULIDs are generated in Rust, so a task has an ID before the insert. That makes
the create response immediate and lets a client queue an operation offline. The
database still enforces primary-key uniqueness.

`Serialize` on the domain type is convenient for the API, but it is also a
coupling. When a field should not be public, add a view type or skip the field;
do not assume the domain type is a safe API contract forever.

## Exercises

1. **Add an assignee.** A `set_assignee` method must verify the target is a
   workspace member and use the same version check. Add tests for a member, a
   non-member, and a stale version.
2. **Protect the last open task.** Add a “cannot delete the last open task in a
   project” rule and test the interaction with soft delete. Is that a good
   product rule? Argue both sides.
3. **Make conflict handling testable.** Write a test that loads a task twice,
   renames one copy, then completes the stale copy. Assert `Conflict` and the
   current version.
4. **Swap validation and authorization.** Move `normalize_title` after the
   permission checks. Add a test that an outsider gets `Forbidden` even with an
   empty title. Which order leaks less?
5. **Add a restore.** A deleted task can be restored by setting `deleted_at` to
   null with a version check. Decide whether restore is a new task or the same
   row, and make the sync log reflect it.

## What comes next

The service can store and mutate owned tasks, but a person cannot use it from a
browser. Chapter 6 adds the server-rendered MPA: login, workspace and project
forms, task add/toggle/delete, CSRF, and the first accessible HTML. The service
contract from this chapter is what the forms call; the forms do not get to
bypass it.
