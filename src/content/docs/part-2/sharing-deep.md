---
title: "Sharing deep"
description: "Enforce private projects, project roles, and the audit log."
---

Chapter 4 introduced workspaces and roles. That is enough to share a whole
workspace, but not enough to keep one project private inside it. A team may want
everyone in the workspace to see the general project list while keeping “Hiring”
or “Personal” visible to two people. A workspace-wide role cannot express that.

This chapter adds the second layer: project visibility and project membership.
It also adds the audit log, because sharing is the first feature where “who
changed this?” matters as much as “what changed?”

By the end of this chapter you can:

- keep a private project out of the workspace list for non-members
- add a project viewer who can read but not write
- add a project editor who can write
- explain why an owner or admin can access a private project without a project
  membership row
- record membership and invitation actions in an append-only audit log
- test a viewer who is a workspace member but cannot rename a task
- test an editor who can rename the same task

The Rust lessons are layered predicates, a boolean access check that is not the
same as a write check, and an audit service that stays independent of the domain
operations it records. The security lesson is least privilege: a workspace role
is not a blank cheque for every project.

## Three layers of permission

The rules are cumulative:

```text
1. workspace membership      -> can you see the workspace at all?
2. project visibility        -> is the project workspace-wide or private?
3. project role              -> for a private project, can you read/write?
```

The first layer is a `workspace_members` row with a role. The second is
`projects.visibility`. The third is a `project_members` row with `editor` or
`viewer`.

An owner or admin can manage the workspace, so they bypass the project layer.
A workspace member does not. A workspace viewer can read workspace-visible
projects but cannot write. A private project requires an explicit row for a
member, and that row’s role decides read versus write.

This is not a hierarchy where “workspace member” implies “project editor.” It is
two independent scopes. Keep that sentence in mind when you read the predicates.

## Project membership

The schema from chapter 4 already has the table:

```sql
CREATE TABLE project_members (
    project_id TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('editor', 'viewer')),
    created_at TEXT NOT NULL,
    PRIMARY KEY (project_id, user_id)
);
```

The primary key prevents two roles for one user in one project. The check
constraint limits the role values. Rust parses into:

```rust
pub enum ProjectRole {
    Editor,
    Viewer,
}

impl ProjectRole {
    pub fn can_write(self) -> bool {
        self == Self::Editor
    }
}
```

`add_project_member` requires a workspace admin or owner and a target who is
already a workspace member:

```rust
pub async fn add_project_member(
    &self,
    actor_id: &str,
    project_id: &str,
    user_id: &str,
    role: ProjectRole,
) -> Result<(), WorkspaceError> {
    let project = self.find_project(project_id).await?;
    self.require_role(&project.workspace_id, actor_id, Role::can_manage)
        .await?;
    if self.role(&project.workspace_id, user_id).await?.is_none() {
        return Err(WorkspaceError::NotMember);
    }

    sqlx::query(
        "INSERT INTO project_members (project_id, user_id, role, created_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT (project_id, user_id) DO UPDATE SET role = excluded.role",
    )
    // ...
}
```

The target check is not optional. A project member who is not a workspace member
would have a row that no workspace query can authorize. The row would be dead
weight, and a future refactor might accidentally treat it as access. Require both.

The upsert makes “add” and “change role” one operation. If you need an audit trail
that distinguishes them, read the existing role first and write two actions. The
current audit records `project_member.added` for both; that is a known
simplification.

## Read access

`can_access_project` answers the read question:

```rust
pub async fn can_access_project(
    &self,
    user_id: &str,
    project_id: &str,
) -> Result<bool, WorkspaceError> {
    let project = self.find_project(project_id).await?;
    let Some(role) = self.role(&project.workspace_id, user_id).await? else {
        return Ok(false);
    };
    if role.can_manage() || project.visibility == "workspace" {
        return Ok(true);
    }

    let member = sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM project_members
         WHERE project_id = ? AND user_id = ? LIMIT 1",
    )
    .bind(project_id)
    .bind(user_id)
    .fetch_optional(&self.pool)
    .await?
    .is_some();

    Ok(member)
}
```

The order matters:

1. Load the project. It must exist and not be archived.
2. Load the workspace role. No role means no access, even if a stale project row
   exists.
3. Owner/admin gets access.
4. A workspace-visible project gets access for any workspace member.
5. A private project requires a project row.

The `EXISTS`/`LIMIT 1` query is a boolean. It does not need the role; read access
is the same for editor and viewer. Write access is a separate check.

For non-managers, `list_projects` applies the read rule in SQL:

```sql
WHERE p.workspace_id = ? AND p.archived_at IS NULL
  AND (
      p.visibility = 'workspace'
      OR EXISTS (
          SELECT 1 FROM project_members pm
          WHERE pm.project_id = p.id AND pm.user_id = ?
      )
  )
```

An owner or admin takes a separate branch that lists every project in the
workspace, matching `can_access_project`. If the list hides a project the detail
page allows, users bookmark a page they cannot see in the sidebar. If the list
shows a project the detail forbids, clicking it gives a 403. Test both directions,
including the owner/admin case.

## Write access

Chapter 5’s task service originally checked only the workspace role. That is
wrong for a private project: a workspace member with a project `viewer` role could
rename a task. The fix is a separate predicate:

```rust
pub async fn can_write_project(
    &self,
    user_id: &str,
    project_id: &str,
) -> Result<bool, WorkspaceError> {
    let project = self.find_project(project_id).await?;
    let Some(role) = self.role(&project.workspace_id, user_id).await? else {
        return Ok(false);
    };
    if role.can_manage() {
        return Ok(true);
    }
    if project.visibility == "workspace" {
        return Ok(role.can_write());
    }

    Ok(role.can_write()
        && self.project_role(project_id, user_id).await? == Some(ProjectRole::Editor))
}
```

Read and write are deliberately separate. A viewer can read and cannot write; a
workspace member can write in a shared project but not in a private project where
they are only a viewer. An owner can do both without a project row.

`TaskService` uses a helper that combines project access with write permission:

```rust
async fn require_project_write(
    &self,
    actor_id: &str,
    workspace_id: &str,
    project_id: &str,
) -> Result<(), TaskError> {
    self.require_project_access(actor_id, workspace_id, project_id)
        .await?;
    if self.workspaces().can_write_project(actor_id, project_id).await? {
        Ok(())
    } else {
        Err(TaskError::Forbidden)
    }
}
```

Every task mutation calls this helper: create, rename, complete, delete, set
recurrence, sync update. A read uses `require_project_access` without the write
check.

The private-project task test now proves both sides:

```rust
// viewer can read
assert!(fixture.tasks.get(&member, &task.id).await.is_ok());

// viewer cannot write
assert!(matches!(
    fixture.tasks.rename(&member, &task.id, "still secret").await,
    Err(TaskError::Forbidden)
));

// editor can write
fixture.workspaces
    .add_project_member(&owner, &private.id, &member, ProjectRole::Editor)
    .await?;
assert!(fixture.tasks.rename(&member, &task.id, "shared secret").await.is_ok());
```

That sequence is the chapter’s core contract. If the viewer write succeeds, the
project role is being ignored.

## The audit log

Create `migrations/0008_audit_log.sql`:

```sql
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

CREATE INDEX audit_log_workspace_id_idx
    ON audit_log (workspace_id, created_at DESC);
```

The table is append-only in practice: the service has `record` and
`list_for_workspace`, no update or delete. A real retention policy may delete old
rows, but no application code should rewrite history.

`action` is a stable verb such as `workspace.created`, `member.added`,
`invitation.created`, `invitation.accepted`, or `project_member.added`.
`target` is the affected ID or email. `metadata` is free text for a role, a
visibility, or a reason. `ip` is nullable and currently always null; chapter 21
can thread it through the request layer.

`AuditService::record` inserts one row:

```rust
pub async fn record(
    &self,
    workspace_id: &str,
    actor_id: &str,
    action: &str,
    target: &str,
    metadata: &str,
) -> Result<(), AuditError> {
    sqlx::query(
        "INSERT INTO audit_log
         (id, workspace_id, actor_id, action, target, metadata, ip, created_at)
         VALUES (?, ?, ?, ?, ?, ?, NULL, ?)",
    )
    // ...
}
```

`list_for_workspace` orders newest first and takes a limit. It does not check
authorization; the caller must be an admin. This is a service primitive, not a
route. A future `GET /workspace/audit` handler checks `Role::can_manage` before
calling it.

The workspace service records:

- workspace creation
- project creation
- member add/change
- invitation creation
- invitation acceptance
- project member add/change

The record is written after the database operation. If the record fails, the
operation has already succeeded and the caller gets an error. That is a real
consistency gap. A stronger design writes the audit row in the same transaction
as the change. The current code keeps the audit call visible and the exercises ask
you to make it atomic.

The audit is not a security boundary. It records what the application chose to
record. A bug that bypasses the service leaves no row. Do not treat an empty
audit log as proof that nothing happened.

## Permission matrix

The combinations are easier to audit as a table. “Shared” means
`visibility = 'workspace'`; “private” means `visibility = 'private'`.

| Workspace role | Project role | Shared read | Shared write | Private read | Private write |
| --- | --- | --- | --- | --- | --- |
| owner | none | yes | yes | yes | yes |
| admin | none | yes | yes | yes | yes |
| member | none | yes | yes | no | no |
| member | viewer | yes | yes | yes | no |
| member | editor | yes | yes | yes | yes |
| viewer | none | yes | no | no | no |
| viewer | viewer | yes | no | yes | no |
| viewer | editor | yes | no | yes | no |

The last row is deliberately surprising: a workspace viewer with a project
editor row can read the private project but still cannot write tasks, because the
workspace role has no write permission. Is that the product you want? If project
roles are supposed to override the workspace role, the write predicate must say
so. The current code says the workspace role is a ceiling; project membership
only narrows access, never expands it. Document that rule before someone adds a
“project editor can write” test that contradicts the service.

The table also shows why one boolean is not enough. Read and write differ, and
admin bypass is a third branch. Test the corners, not just the happy row.

## Audit timeline

A sharing action leaves a short trail:

```text
workspace.created   target=workspace-id   metadata=""
member.added        target=user-id        metadata="member"
invitation.created  target=person@email   metadata="member"
invitation.accepted target=workspace-id   metadata="member"
project.created     target=project-id     metadata="private"
project_member.added target=user-id       metadata="viewer"
```

The rows are ordered by time, then ID. An admin can reconstruct who invited whom
and when the private project gained a viewer. The audit does not store the
invitation token or the project name; the target ID is enough to join if needed,
and avoiding copied data keeps the log small and less sensitive.

If an action is missing, the first question is “did the service call `record`?”
The second is “did the operation commit before the audit?” The current order is
operation first, audit second, so a successful operation with a failed audit is
possible. Treat that as a defect, not a mystery.

## Tests

Three tests cover the sharing layer:

- `hides_private_projects_from_non_members`: a workspace member sees only the
  shared project; after `add_project_member` as viewer, the private project
  appears.
- `private_projects_require_project_membership`: a member cannot read a private
  task until added; a viewer can read but not rename; an editor can rename.
- `records_and_lists_audit_entries`: a manual user/workspace fixture records
  `member.added` and lists it with the right actor and action.

Run:

```console
$ cargo test hides_private_projects_from_non_members
$ cargo test private_projects_require_project_membership
$ cargo test audit
```

The tests use real service methods for project membership now; they do not insert
the join row directly. That matters: the test would otherwise pass even if
`add_project_member` were broken.

## Checkpoint

```console
$ cargo test workspaces
$ cargo test tasks
$ cargo test audit
$ cargo test
$ cargo fmt --check
$ cargo clippy --all-targets --all-features -- -D warnings
```

There is no members or audit page yet. The service contract is the checkpoint.

## Failure modes you will meet

**A workspace viewer can read a private project.** `can_access_project` returned
true for any workspace member before checking visibility. Put the visibility
branch before the membership branch.

**A project viewer can rename a task.** The task service checks only the workspace
role. Add `can_write_project` and call it from every mutation. The viewer read
test alone will not catch this.

**An admin cannot see a private project.** The bypass checks
`role.can_manage()` before the project membership query. If it is missing, the
admin needs a project row like everyone else. Decide whether admins should bypass;
the book says yes, but it is a product choice.

**The list and detail disagree.** The SQL `EXISTS` and the Rust access check use
different conditions. Test both `list_projects` and `get` for the same user.

**A project member is not a workspace member.** `add_project_member` must require
both. The foreign keys allow the row; the service must reject the state.

**An audit row is missing after a failed operation.** The audit is written after
the change, so a failed change should leave no row. A successful change with a
failed audit leaves a gap. Make both atomic if the audit is a compliance
requirement.

**The audit log leaks sensitive data.** `target` can be an email; `metadata` can
be a role. Do not put tokens or task bodies in it. The log is read by admins and
may be retained longer than the main data.

**A removed project member still has access.** There is no remove method yet. A
deleted row is the removal; add a test before you expose the action. Cached
authorization in a long-lived connection would also delay the change; this
service queries per request.

## Under the hood

Layered authorization is a set of predicates, not a hierarchy of roles. The
workspace role answers “who can manage the tenant?” The project role answers “who
can touch this private container?” A single “admin” boolean cannot express both.

The access check is a boolean because reads do not care whether the project role
is editor or viewer. The write check needs the role. Keeping them separate avoids
the common bug where a read check is reused for a write.

An append-only audit is a log, not a state machine. It records events in order; it
does not enforce the current state. The current state is still the membership
table. If the two disagree, the membership table wins and the audit explains how
it got there.

## Exercises

1. **Add an audit page.** `GET /workspace/audit` for admins, with pagination and
   a stable order. Test that a viewer gets 403.
2. **Remove a project member.** Add `remove_project_member`, audit it, and test
   that the user loses read access immediately.
3. **Make audit atomic.** Write the audit row in the same transaction as the
   membership change. Add a test that forces the audit insert to fail and asserts
   the membership was rolled back.
4. **Record the request IP.** Thread the client IP from the handler into
   `record`. Do not trust `X-Forwarded-For` unless a trusted proxy sets it.
5. **Distinguish add from role change.** Read the existing role before the upsert
   and record `member.added` vs `member.role_changed`. Test both.

## What comes next

Sharing is enforced, but the only clients are the browser and the service tests.
Chapter 13 adds a scoped JSON API with bearer tokens, idempotency keys, and the
same authorization rules. The API is not a second permission system; it calls the
same services.
