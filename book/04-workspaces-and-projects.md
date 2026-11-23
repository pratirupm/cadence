# 4. Workspaces and projects

The service can now identify a user. It still cannot answer the next question:
**what does this user own?** A user with a session but no tenant boundary can
read every task in the database. That is not a task service; it is a public
bulletin board with passwords.

This chapter adds the ownership layer: workspaces, memberships, roles, projects,
and invitations. A workspace is the tenant. A project is a container inside it.
A role decides what a member may do. The task table will not change much; the
authorization around it will.

By the end of this chapter you can:

- create a workspace and become its owner in one transaction
- invite a person by email and accept only with the matching account
- add a member with a role and reject a viewer who tries to manage members
- create a project that every workspace member can see
- create a private project that only explicit project members can see
- generate a unique slug without a race that produces duplicates
- keep a workspace from existing without an owner

The Rust lessons are role enums, `FromStr`, permission predicates, transactions,
`ON CONFLICT`, and returning domain errors instead of HTTP status codes. The
product lesson is that authorization is not a check at the edge. It is a
property of every service method that touches a workspace.

## The tenancy model

There are three nested ideas:

```text
workspace  -> the tenant and billing/member boundary
  project  -> a container for tasks
    task   -> the thing a person actually edits
```

A user can belong to many workspaces. A workspace can have many members. Every
workspace has at least one owner. A project belongs to exactly one workspace. A
task will belong to a project, and therefore to a workspace.

The first two roles are workspace-level:

- **owner**: everything, including assigning another owner
- **admin**: manage members, projects, and content
- **member**: create and edit content
- **viewer**: read only

A later chapter adds project-level editor/viewer roles for private projects.
This chapter introduces the visibility flag and the membership table so that
chapter has somewhere to stand.

## The schema

Create `migrations/0003_workspaces.sql`:

```sql
CREATE TABLE workspaces (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    owner_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at TEXT NOT NULL
);

CREATE TABLE workspace_members (
    workspace_id TEXT NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member', 'viewer')),
    created_at TEXT NOT NULL,
    PRIMARY KEY (workspace_id, user_id)
);

CREATE INDEX workspace_members_user_id_idx ON workspace_members (user_id);

CREATE TABLE projects (
    id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    visibility TEXT NOT NULL DEFAULT 'workspace'
        CHECK (visibility IN ('workspace', 'private')),
    archived_at TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX projects_workspace_id_idx ON projects (workspace_id);

CREATE TABLE project_members (
    project_id TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('editor', 'viewer')),
    created_at TEXT NOT NULL,
    PRIMARY KEY (project_id, user_id)
);

CREATE TABLE invitations (
    id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('admin', 'member', 'viewer')),
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TEXT NOT NULL,
    accepted_at TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX invitations_workspace_id_idx ON invitations (workspace_id);
```

The primary key on `workspace_members` is `(workspace_id, user_id)`. A user
cannot have two roles in the same workspace. The role check constraint limits
the values the database will accept; Rust still parses into an enum, but the
database refuses nonsense written by another process.

`workspaces.owner_id` is denormalized. The owner is also a member row with role
`owner`. Keeping both is useful for “who created this?” and for a fast ownership
check. They must not disagree. The creation transaction writes both from the
same `owner_id`.

`projects.visibility` is `workspace` or `private`. A workspace-visible project
is readable by every workspace member. A private project requires a row in
`project_members`, or an owner/admin. The visibility string is constrained; the
list query relies on the exact values.

`invitations.role` excludes `owner`. You may invite an admin, member, or viewer.
Ownership transfer is a separate action, not an invitation side effect.

## Roles as types

The role enum lives in Rust, not as a bare string:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Owner,
    Admin,
    Member,
    Viewer,
}

impl Role {
    pub fn can_manage(self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }

    pub fn can_write(self) -> bool {
        matches!(self, Self::Owner | Self::Admin | Self::Member)
    }
}
```

The predicates are methods on the role, so a handler cannot invent a different
interpretation. A viewer cannot write because `can_write` is false, not because
a route forgot to check a string.

`FromStr` parses the database value:

```rust
impl FromStr for Role {
    type Err = WorkspaceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "owner" => Ok(Self::Owner),
            "admin" => Ok(Self::Admin),
            "member" => Ok(Self::Member),
            "viewer" => Ok(Self::Viewer),
            _ => Err(WorkspaceError::InvalidRole),
        }
    }
}
```

The database check constraint and the parser agree. If they drift, a row can
exist that the service cannot read. Treat the pair as one contract.

`ProjectVisibility` and the later `ProjectRole` follow the same pattern. The
`as_str` method is the write side; `FromStr` is the read side. Keep both in the
same module as the enum.

## Creating a workspace

Creation is two inserts that must succeed together:

```rust
pub async fn create_workspace(
    &self,
    owner_id: &str,
    name: &str,
) -> Result<Workspace, WorkspaceError> {
    let name = normalize_name(name)?;
    let slug = self.unique_slug(&name).await?;
    let workspace = Workspace {
        id: Ulid::new().to_string(),
        name,
        slug,
        owner_id: owner_id.to_owned(),
        created_at: Utc::now(),
    };

    let mut transaction = self.pool.begin().await?;
    sqlx::query("INSERT INTO workspaces ...").execute(&mut *transaction).await?;
    sqlx::query("INSERT INTO workspace_members ... VALUES (?, ?, 'owner', ?)")
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;

    Ok(workspace)
}
```

If the member insert fails, the workspace insert is rolled back. A workspace
without an owner is not a valid state, so it must never be committed.

The slug is generated before the transaction. `unique_slug` queries for a
candidate and appends `-2`, `-3`, and so on:

```rust
async fn unique_slug(&self, name: &str) -> Result<String, WorkspaceError> {
    let base = slugify(name);
    for suffix in 1..=100 {
        let candidate = if suffix == 1 {
            base.clone()
        } else {
            format!("{base}-{suffix}")
        };
        let exists = sqlx::query_scalar::<_, i64>(
            "SELECT 1 FROM workspaces WHERE slug = ? LIMIT 1",
        )
        .bind(&candidate)
        .fetch_optional(&self.pool)
        .await?
        .is_some();
        if !exists { return Ok(candidate); }
    }
    Err(WorkspaceError::InvalidName)
}
```

This is a check-then-insert race. Two concurrent creates can both see `home`
free and both try to insert it. The unique index lets one win; the other gets a
database error. The service does not currently retry the loser with `home-3`.
That is a known limitation, and the exercise asks you to close it. The unique
index is the safety net; the loop is the convenience.

`slugify` lowercases ASCII letters, turns other runs into `-`, trims trailing
dashes, and falls back to `"workspace"` for a name with no usable characters.
Slugs are for URLs and logs, not for identity. The ULID is the identity.

## Listing workspaces

A user should see only workspaces where they are a member:

```rust
pub async fn list_workspaces(&self, user_id: &str) -> Result<Vec<Workspace>, WorkspaceError> {
    let workspaces = sqlx::query_as::<_, Workspace>(
        "SELECT w.id, w.name, w.slug, w.owner_id, w.created_at
         FROM workspaces w
         JOIN workspace_members m ON m.workspace_id = w.id
         WHERE m.user_id = ?
         ORDER BY w.created_at, w.id",
    )
    .bind(user_id)
    .fetch_all(&self.pool)
    .await?;

    Ok(workspaces)
}
```

The join is the authorization. There is no separate `if is_member` that a new
caller might forget. A workspace that has no membership row is invisible.

The ordering is stable: creation time, then ID. The UI can rely on it. Do not
use `ORDER BY name` unless the product explicitly wants alphabetical order; a
rename would then reorder the sidebar unexpectedly.

## Projects

A project is created by a role that can write:

```rust
pub async fn create_project(
    &self,
    actor_id: &str,
    workspace_id: &str,
    name: &str,
    visibility: ProjectVisibility,
) -> Result<Project, WorkspaceError> {
    self.require_role(workspace_id, actor_id, Role::can_write)
        .await?;

    let name = normalize_name(name)?;
    let project = Project {
        id: Ulid::new().to_string(),
        workspace_id: workspace_id.to_owned(),
        name,
        visibility: visibility.as_str().to_owned(),
        archived_at: None,
        created_at: Utc::now(),
    };
    // insert ...
}
```

The name is trimmed and validated by `normalize_name`. A viewer gets
`Forbidden`. A non-member gets `NotMember`.

The list query combines workspace visibility and project membership:

```sql
SELECT p.id, p.workspace_id, p.name, p.visibility, p.archived_at, p.created_at
FROM projects p
WHERE p.workspace_id = ? AND p.archived_at IS NULL
  AND (
      p.visibility = 'workspace'
      OR EXISTS (
          SELECT 1 FROM project_members pm
          WHERE pm.project_id = p.id AND pm.user_id = ?
      )
  )
ORDER BY p.created_at, p.id
```

The `EXISTS` subquery is the private-project rule. An owner or admin will also
see private projects through `can_access_project`, which is a separate check
used by tasks. The list and the access check must agree; chapter 12 tests that
they do.

Do not fetch all projects and filter in Rust. That leaks existence through
timing and makes pagination harder. Put the rule in SQL and test it.

## Members and invitations

`add_member` is the direct path for an owner or admin:

```rust
pub async fn add_member(
    &self,
    actor_id: &str,
    workspace_id: &str,
    user_id: &str,
    role: Role,
) -> Result<(), WorkspaceError> {
    let actor_role = self.require_member(workspace_id, actor_id).await?;
    if !actor_role.can_manage() || (role == Role::Owner && actor_role != Role::Owner) {
        return Err(WorkspaceError::Forbidden);
    }

    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, role, created_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT (workspace_id, user_id) DO UPDATE SET role = excluded.role",
    )
    // ...
}
```

An admin can add members and admins, but cannot create another owner. Only an
owner can assign `owner`. The rule is in the service; hiding the option in a
template is not enough.

`ON CONFLICT ... DO UPDATE` makes role changes an update rather than a duplicate
error. It also means “add member” and “change role” share a method. If the
product needs an audit entry for role changes distinct from additions, inspect
the existing role first or write two methods.

`list_members` joins users and parses the role:

```rust
let rows = sqlx::query_as::<_, MemberRow>(
    "SELECT u.id AS user_id, u.email, u.name, m.role
     FROM workspace_members m
     JOIN users u ON u.id = m.user_id
     WHERE m.workspace_id = ?
     ORDER BY u.name, u.id",
)
```

`MemberRow.role` is a `String`; the service parses it into `Role`. The conversion
happens at the boundary, so the rest of the code never sees an untyped role.

## Invitations

An invitation is an email plus a role plus a single-use token. It reuses the
token machinery from chapter 3: 32 random bytes, SHA-256 at rest, expiry, and a
row that records acceptance.

```rust
pub async fn invite(
    &self,
    actor_id: &str,
    workspace_id: &str,
    email: &str,
    role: Role,
) -> Result<String, WorkspaceError> {
    let actor_role = self.require_member(workspace_id, actor_id).await?;
    if !actor_role.can_manage() || role == Role::Owner {
        return Err(WorkspaceError::Forbidden);
    }

    let email = normalize_email(email)?;
    let token = random_token();
    // insert hash, email, role, expires_at = now + 7 days
    Ok(token)
}
```

The raw token is returned to the caller. The email adapter in chapter 11 will
send it; tests can consume it directly. The token is not stored raw.

Acceptance checks the signed-in user’s email:

```rust
let (invitation_id, workspace_id, email, role) = row;
let user_email = sqlx::query_scalar::<_, String>("SELECT email FROM users WHERE id = ?")
    .bind(user_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(WorkspaceError::InvalidInvitation)?;

if user_email != email {
    return Err(WorkspaceError::EmailMismatch);
}
```

This prevents forwarding an invitation to a different account. It assumes
emails are normalized on both rows. Chapter 3 does that; keep it true.

The membership insert and the `accepted_at` update happen in one transaction.
If two requests race, the second finds `accepted_at IS NOT NULL` and fails. The
error is `InvalidInvitation`, not “already accepted”; the caller can render the
same page for both.

Accepting an invitation for an existing member uses `ON CONFLICT DO UPDATE` and
changes their role. Is that desirable? Maybe. A stricter product would refuse to
downgrade an owner through an invitation. Decide and test. The book chooses the
simpler upsert for now.

## Authorization helpers

Every protected method goes through one of these:

```rust
async fn require_member(&self, workspace_id: &str, user_id: &str)
    -> Result<Role, WorkspaceError>
{
    self.role(workspace_id, user_id)
        .await?
        .ok_or(WorkspaceError::NotMember)
}

async fn require_role(&self, workspace_id: &str, user_id: &str, allowed: fn(Role) -> bool)
    -> Result<Role, WorkspaceError>
{
    let role = self.require_member(workspace_id, user_id).await?;
    if allowed(role) { Ok(role) } else { Err(WorkspaceError::Forbidden) }
}
```

`NotMember` and `Forbidden` are distinct on purpose. A missing membership is a
404 or a 403 depending on the product’s disclosure policy; a known member with
the wrong role is unambiguously 403. The HTTP mapping can choose.

Passing `Role::can_write` as a function pointer keeps the predicate in one place.
A closure would work too; the point is that the caller does not write
`role == Owner || role == Admin || role == Member` again.

## Tests

The workspace tests use the auth service to create real users:

```rust
async fn fixture() -> (AuthService, WorkspaceService) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    (AuthService::new(pool.clone()), WorkspaceService::new(pool))
}
```

Six tests cover the chapter:

- `creates_a_workspace_with_an_owner`: slug, owner membership, list.
- `makes_duplicate_slugs_unique`: `home`, then `home-2`.
- `members_can_create_projects_but_viewers_cannot`: role predicates.
- `hides_private_projects_from_non_members`: visibility and project membership.
- `invitations_require_the_matching_email_and_work_once`: email mismatch,
  success, second use fails.
- `viewers_cannot_add_members`: manage permission.

The private-project test inserts a project member directly in the current suite;
chapter 12 adds the service method. The assertion is what matters: a private
project is invisible until membership exists.

Run:

```console
$ cargo test workspaces
```

## Checkpoint

There is no browser form yet. The tests are the client for this chapter. The
checkpoint is:

```console
$ cargo test workspaces
$ cargo test
$ cargo fmt --check
$ cargo clippy --all-targets --all-features -- -D warnings
```

Chapter 6 adds the workspace and project forms. The service contract is stable
before the UI appears, which is the point.

## Failure modes you will meet

**A workspace has no owner.** Creation wrote the workspace but not the member
row. Use one transaction and test the rollback. The schema has no constraint
that enforces “at least one owner”; the service does.

**Two workspaces have the same slug.** The unique index refused one insert, or
the check-then-insert race lost. Catch the unique violation and retry with the
next suffix, or return a clear error. Do not remove the unique index.

**A viewer can create a project.** The route checked membership but not
`can_write`. Put the predicate in `require_role` and test the viewer path.

**A private project appears in the list.** The query is missing the `EXISTS`
subquery or compares the wrong user ID. Test with a workspace member who is not
a project member.

**An invitation works for the wrong email.** Acceptance compared the invitation
to a stale email, or normalization differs between signup and invite. Normalize
both.

**An invitation works twice.** Acceptance did not check `accepted_at IS NULL`
or did not update it in the same transaction. The second request must fail.

**An admin creates an owner.** The role check forgot `role == Owner`. Only an
owner may assign owner; the UI hiding the option is not enough.

## Under the hood

`ON CONFLICT` is SQLite’s upsert syntax. It is not a substitute for authorization;
it only resolves a key collision after the service has decided the actor may
write.

`FromStr` is a parsing boundary. A database string that is not a known role
becomes an error, not a default. Defaulting an unknown role to `Viewer` sounds
safe but hides corruption; defaulting to `Member` is a privilege escalation.

The `owner_id` column and the owner membership row are a denormalization. Every
denormalization needs a rule for keeping the copies consistent. Here the rule is:
only `create_workspace` writes the owner, and a future ownership transfer must
update both in one transaction.

## Exercises

1. **Protect the last owner.** Refuse to demote or remove the only owner of a
   workspace. Add a test that a second owner can be demoted while the first
   remains.
2. **Make slug creation race-safe.** On a unique violation, retry with the next
   suffix a bounded number of times. Test with two concurrent creates.
3. **Add project archival.** Set `archived_at`, hide archived projects from the
   list, and add a restore action. Decide whether tasks in an archived project
   are still readable.
4. **Separate add and change role.** Write `change_role` with an audit entry and
   keep `add_member` for new rows. Update the tests.
5. **Close invitation enumeration.** Make `invite` return the same result whether
   or not the email already has an account. Decide whether the token should be
   issued in both cases.

## What comes next

Workspaces and projects exist, but the task table still has no owner. Chapter 5
moves tasks inside projects, adds optimistic versions and soft deletes, and
makes every task read and write go through the workspace and project checks from
this chapter. The authorization helpers become the gate for the domain.
