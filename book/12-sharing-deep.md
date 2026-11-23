# 12. Sharing deep

**What you can do at the end.** Keep a project private, add a project viewer,
and leave an audit trail for membership changes.

**The Rust you learn.** Layered authorization, project-level roles, audit
records, and defending every query with a workspace scope.

**Build it.**

- Add `ProjectRole` with editor and viewer.
- `add_project_member` requires a workspace admin and a target who is already a
  workspace member.
- `can_access_project` allows owners/admins, allows workspace-visible projects
  to members, and requires project membership for private projects.
- Route every task read and write through `require_project_access`.
- Add an audit table and an `AuditService`.
- Record workspace creation, project creation, member adds, invitations, and
  invitation acceptance.

Authorization is a function of the resource, not the route:

```rust
if role.can_manage() || project.visibility == "workspace" {
    return Ok(true);
}
```

**Checkpoint.**

```console
$ cargo test workspaces
$ cargo test private_projects_require_project_membership
$ cargo test audit
```

The tests prove a workspace member cannot read a private project until they are
added, and that admin actions produce audit rows.

**Exercises.**

1. Add an audit page scoped to admins.
2. Add a test that a project viewer cannot write a task.
