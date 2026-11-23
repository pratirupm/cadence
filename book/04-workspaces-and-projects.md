# 4. Workspaces and projects

**What you can do at the end.** Create a workspace, invite a member, assign a
role, and keep private projects private.

**The Rust you learn.** Role enums, authorization checks, transactions, slug
generation, and the difference between authentication and permission.

**Build it.**

- Add workspaces, members, projects, project members, and invitations.
- Give every new workspace an owner membership in the same transaction.
- Parse roles with `FromStr`, and keep the permission predicates on the enum.
- Create projects only for roles that can write.
- Invite by email with a single-use token; accept only when the signed-in
  email matches.
- Generate a slug, and append `-2`, `-3`, … until it is unique.

Authorization belongs in the service, not only in the template:

```rust
let role = self.require_member(workspace_id, actor_id).await?;
if !role.can_write() {
    return Err(WorkspaceError::Forbidden);
}
```

**Checkpoint.**

```console
$ cargo test workspaces
```

The tests cover owner creation, duplicate slugs, member vs viewer permissions,
private project visibility, invitation email matching, single-use invitations,
and viewers being unable to add members.

**Exercises.**

1. Add an `archived_at` project action and hide archived projects from lists.
2. Add a test that the last owner cannot be demoted.
