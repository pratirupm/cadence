---
title: "The next occurrence"
description: "Create the next occurrence when a recurring task is completed."
---

Chapter 8’s engine can answer a pure question: given a rule, a start instant, and
a time zone, what is the next occurrence after this moment? This chapter connects
that answer to the task table. Completing a recurring task should leave the
completed occurrence in history and create the next one.

That sounds like one operation. It is at least four decisions:

1. Is the task actually recurrent?
2. Is this completion a transition from open to done, or a repeated request?
3. What is the next occurrence, and in which zone?
4. Which fields belong to the new occurrence, and which are history?

The chapter makes each decision visible. It also fixes the first real duplicate
bug in the book: completing an already-completed recurring task must not create a
second “next” occurrence.

By the end of this chapter you can:

- attach a validated RRULE and time zone to a task
- complete a recurring task and see exactly one future occurrence
- complete it twice and still see exactly one future occurrence
- explain why the next occurrence is after `max(now, starts_at)`
- preserve title, notes, priority, assignee, and rule on the new occurrence
- describe what happens to missed occurrences (they are skipped)
- identify the transaction gap between “mark done” and “insert next”

The Rust lessons are cloning a domain value, calling a pure function from a
service method, and guarding an operation with a state transition. The product
lesson is that “recurring” is a series, not a date.

## Two columns, not a table

Create `migrations/0005_recurrence.sql`:

```sql
ALTER TABLE tasks ADD COLUMN recurrence TEXT;
ALTER TABLE tasks ADD COLUMN recurrence_timezone TEXT;
```

A separate `recurrence_rules` table would be justified if a rule could be shared
by many tasks or edited independently. Here a task has at most one rule, and the
rule is copied when an occurrence is created. Two nullable columns are enough.

`recurrence` stores the original RRULE text. `recurrence_timezone` stores the
IANA zone name. Do not store the computed UTC offset; offsets change at DST.

The `due_at` column becomes the first occurrence’s instant when a rule is set.
The rule alone does not choose a time of day; `starts_at` does.

## Setting a rule

`set_recurrence` validates before it writes:

```rust
pub async fn set_recurrence(
    &self,
    actor_id: &str,
    id: &str,
    rrule: &str,
    timezone: &str,
    starts_at: DateTime<Utc>,
) -> Result<Task, TaskError> {
    Recurrence::parse(rrule, timezone, starts_at)?;

    let task = self.find(id).await?;
    self.require_project_write(actor_id, &task.workspace_id, &task.project_id)
        .await?;

    let result = sqlx::query(
        "UPDATE tasks
         SET recurrence = ?, recurrence_timezone = ?, due_at = ?,
             version = version + 1, updated_at = ?
         WHERE id = ? AND version = ? AND deleted_at IS NULL",
    )
    .bind(rrule)
    .bind(timezone)
    .bind(starts_at)
    .bind(Utc::now())
    .bind(id)
    .bind(task.version)
    .execute(&self.pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(TaskError::Conflict);
    }

    self.find(id).await
}
```

The parser runs before authorization. That means an outsider can learn whether a
rule is valid without learning whether the task exists. The leak is small, but it
is a leak. If the rule is large and parsing expensive, the order also lets an
unauthenticated caller spend CPU. A stricter version authorizes first, then
parses. The exercise asks you to choose and test the order.

The version check is the same compare-and-set as every other task mutation. A
stale editor cannot overwrite a rule that changed under them.

Setting a rule updates `due_at` to `starts_at`. That is deliberate: the first
occurrence is the start. If the task already had a due date, this overwrites it.
An alternative API takes a separate “first due” and “rule” and refuses to
overwrite silently. The book keeps one input because the UI sets both together.

## The completion transition

`set_completed` loads the task, remembers whether it was already complete, checks
permissions, and performs the versioned update:

```rust
let task = self.find(id).await?;
let was_completed = task.completed;
self.require_project_write(actor_id, &task.workspace_id, &task.project_id).await?;

let result = sqlx::query(
    "UPDATE tasks
     SET completed = ?, version = version + 1, updated_at = ?
     WHERE id = ? AND version = ? AND deleted_at IS NULL",
)
// ...
if result.rows_affected() == 0 {
    return Err(TaskError::Conflict);
}
```

The `was_completed` value is the fix for a duplicate bug. The first version of
this chapter created the next occurrence whenever the updated row was completed
and had a rule. Calling complete twice on an already-completed row created a
second future task. The transition guard changes the condition:

```rust
let task = self.find(id).await?;
if completed && !was_completed && task.recurrence.is_some() {
    self.create_next_occurrence(&task).await?;
}
```

Only an open-to-done transition creates the next occurrence. Repeated completion
is a no-op for the series (though it still increments the version; a stricter
implementation would return the row unchanged).

Reopening and completing again re-enters the transition and creates another next
occurrence. That is a separate product decision. Without a series identity or an
occurrence marker, the service cannot tell “I reopened by mistake” from “I want
another occurrence.” The exercise asks for `series_id`; the current contract is
simple and documented.

## Creating the next occurrence

`create_next_occurrence` is private and takes the completed task:

```rust
async fn create_next_occurrence(&self, task: &Task) -> Result<Option<Task>, TaskError> {
    let Some(rrule) = task.recurrence.as_deref() else {
        return Ok(None);
    };
    let timezone = task.recurrence_timezone.as_deref().unwrap_or("UTC");
    let starts_at = task.due_at.unwrap_or(task.created_at);
    let recurrence = Recurrence::parse(rrule, timezone, starts_at)?;
    let after = Utc::now().max(starts_at);
    let Some(due_at) = recurrence.next_after(after) else {
        return Ok(None);
    };
    // build and insert the next task
}
```

Let’s take the choices one at a time.

**Rule missing.** Return `Ok(None)`. A non-recurring task is not an error.

**Time zone missing.** Fall back to UTC. New rows always set the zone when a rule
is set; the fallback is for a legacy row or a manual edit.

**Start missing.** Use `due_at`, then `created_at`. A rule without a start has no
anchor; the creation time is the least surprising fallback. Better: refuse to set
a rule without a start. The current API always passes one.

**`after`.** Use `Utc::now().max(starts_at)`. Two cases:

- The task is completed before its due time. `after` is the start instant.
  `next_after` is strict, so it returns the following occurrence, not the one
  being completed.
- The task is overdue. `after` is now. The engine returns the next occurrence
  after now, skipping every missed one.

Skipping missed occurrences is a policy. The alternatives are to backfill every
missed occurrence (a burst of tasks after a vacation) or to create one task at
the first missed time (a backlog reminder). The book chooses “next after now”
because it keeps the list useful. Document the policy in the UI so a user does
not expect seven overdue “water plants” tasks after a week away.

**No next.** `COUNT` or `UNTIL` can exhaust the rule. Return `Ok(None)`; the
completed occurrence is the end of the series. Do not create a task with a null
due date to represent “finished.”

The new task copies:

```rust
let next = Task {
    id: Ulid::new().to_string(),
    workspace_id: task.workspace_id.clone(),
    project_id: task.project_id.clone(),
    title: task.title.clone(),
    notes: task.notes.clone(),
    completed: false,
    priority: task.priority,
    due_at: Some(due_at),
    recurrence: Some(rrule.to_owned()),
    recurrence_timezone: Some(timezone.to_owned()),
    created_by: task.created_by.clone(),
    assigned_to: task.assigned_to.clone(),
    version: 1,
    created_at: now,
    updated_at: now,
    deleted_at: None,
};
```

The new occurrence gets a new ULID and `version = 1`. The old row keeps its own
ID and completed state. The two rows are separate occurrences, not versions of
one row. That is what makes history and sync simple: completing one task is an
update; creating the next is an insert.

The `created_by` stays the original creator, not the person who completed it.
`assigned_to` stays the same. A future feature might reassign each occurrence;
that is a product rule, not an automatic copy.

The insert uses the same columns as `create`. It does not call `create` because
`create` would validate authorization and generate a title from scratch. The
next occurrence is a system-created row inside an already-authorized operation.

## The transaction gap

The update and the insert are not in one transaction. The sequence is:

```text
UPDATE tasks SET completed = 1, version = version + 1 ...
INSERT INTO tasks (... next occurrence ...)
```

If the process dies between them, the completed task has no next occurrence. The
completion succeeded; the series is stuck. If the insert fails because the
database is full or the rule became invalid, the same state results.

The correct production shape is one transaction:

```text
BEGIN
  UPDATE the occurrence
  INSERT the next occurrence
COMMIT
```

The current code keeps the two operations visible for teaching. The exercise asks
you to make them atomic and to test a forced insert failure. Do not pretend the
gap does not exist; a crash between the statements is exactly the kind of bug
that survives tests and appears in production.

A stronger design also records the operation in the change log inside the same
transaction, so sync clients see both the completion and the new row together.
Chapter 14 adds triggers for task inserts and updates; those triggers run inside
the insert/update statement, but the two statements still need an outer
transaction.

## Idempotency

The transition guard handles repeated `set_completed(true)` calls in sequence.
It does not handle two concurrent completions:

1. Both load the task with `completed = false` and the same version.
2. One update wins; the other sees zero rows and returns `Conflict`.
3. The loser does not insert a next occurrence.

That is correct. The version check is the concurrency guard; the transition
guard is the repeated-request guard. Together they produce one next occurrence
for one successful completion.

Reopening and completing again is a new transition and creates another
occurrence. If that is unacceptable, the series needs a `series_id` and a unique
constraint on `(series_id, occurrence_index)` or a rule that only the first
completion creates a successor. The book leaves that for the exercise.

## Worked traces

The policy is easiest to see with a daily task that starts at 09:00 UTC.

| When completed | Original due | `after` | Next due | Why |
| --- | --- | --- | --- | --- |
| Sep 1 08:00 | Sep 1 09:00 | Sep 1 09:00 | Sep 2 09:00 | Completed before due; start is the anchor |
| Sep 1 10:00 | Sep 1 09:00 | Sep 1 10:00 | Sep 2 09:00 | Completed just after due |
| Sep 4 10:00 | Sep 1 09:00 | Sep 4 10:00 | Sep 5 09:00 | Missed days are skipped |
| Sep 1 08:00 | Sep 1 09:00, `COUNT=1` | — | none | The rule is exhausted |
| Sep 1 08:00 | Sep 1 09:00, `UNTIL=Sep 1 09:00Z` | — | none | The only occurrence is the end |

Completing early is the subtle row. The task is not due yet, but the user has
done it. `after = starts_at` and `next_after` is strict, so the engine returns
the next occurrence, not the one being completed. A naive `after = now` would
return `starts_at` itself when the task is completed before it is due—the
occurrence being completed. Once the start is in the past, `now` is safe.

The skipped-days row is the policy choice. The user sees one open task for
tomorrow, not four overdue tasks. If the product wants a backlog, it must ask
for it explicitly; silently creating one is a surprising kind of data loss in
reverse.

## What the next row is not

It is not a clone. A clone would copy `completed`, the old timestamps, the old
version, and the old ID. The next row is a new occurrence with a new identity and
its own lifecycle.

It is not a new version of the same row. Version numbers describe edits to one
occurrence. The old row stays completed at its final version; the next row starts
at version 1. Mixing the two would make history and sync ambiguous.

It is not a child task. There is no parent/child relationship yet. The two rows
share a rule and a title, but the database does not know they are a series. That
is why the `series_id` exercise exists.

## Tests

The task module has two recurrence tests:

- `completing_a_recurring_task_creates_the_next_occurrence`: creates a task,
  sets `FREQ=DAILY` with a start one hour in the future, completes it, and
  asserts two rows: one completed and one open with the next day’s due date.
- `completing_twice_does_not_duplicate_the_next_occurrence`: the regression test
  for the transition guard. It completes the same task twice and asserts the
  project still has two rows.

The first test also proves the field copy: title is preserved, completed is
false on the new row, and due date advances by one day.

Run:

```console
$ cargo test completing_a_recurring_task_creates_the_next_occurrence
$ cargo test completing_twice_does_not_duplicate_the_next_occurrence
$ cargo test tasks
```

## Checkpoint

```console
$ cargo test tasks
$ cargo test
$ cargo fmt --check
$ cargo clippy --all-targets --all-features -- -D warnings
```

There is no recurrence UI yet. The service contract is the checkpoint. A manual
demo can call the test fixture or the future API; do not add a temporary route.

## Failure modes you will meet

**Two future occurrences after one completion.** The transition guard is missing,
or the update ran twice without a version check. Complete the same task twice in
a test; the row count must stay two.

**No future occurrence after completing.** Check the rule, the time zone, and
`UNTIL`/`COUNT`. A rule whose next occurrence is `None` is exhausted, not broken.
Also check that `due_at` is set; a null start falls back to `created_at`.

**The next occurrence is in the past.** `after` used `starts_at` instead of
`max(now, starts_at)`, or the engine returned the occurrence being completed.
The comparison in `next_after` is strict; confirm it.

**The next occurrence has the wrong time of day.** `starts_at` changed, or the
rule’s zone is being treated as UTC. A 9:00 New York rule is not 9:00 UTC.

**A completed task has no next because the insert failed.** You found the
transaction gap. Make the two statements atomic and add a failure test.

**Reopening creates a duplicate series.** The service has no series identity. If
this matters, add `series_id` and a unique occurrence key, or forbid reopening a
completed occurrence.

**A missed week produces seven tasks.** The service backfilled instead of
skipping to now. Decide the policy, then test it. The current policy is “next
after now.”

## Under the hood

An occurrence is a row. A series is the rule plus the history of rows. The
service has no explicit series object, so “series” is an emergent property of
copied rules and titles. That is enough until a user wants to edit the series
(“change every future occurrence”). At that point you need a `series_id` and a
rule-editing operation that updates future rows.

Idempotency for a completion is a state transition plus a version check. It is
not a unique index on “next occurrence” because there is no series key. The
version check is what prevents two concurrent completions from both inserting.

The clock is read twice: once for `updated_at`, once for `after` and the new
row’s timestamps. In a long request those can differ by milliseconds. That is
fine for a calendar day, but it is a reminder that `Utc::now()` is not a fixed
input. Tests should use a fixed `starts_at` and assert relative dates, not an
exact “now.”

## Exercises

1. **Make completion atomic.** Wrap the update and the next-occurrence insert in
   one transaction. Add a test that forces the insert to fail and asserts the
   task is still open.
2. **Add `series_id`.** Give every occurrence the same series ID and a unique
   occurrence index. Use it to prevent duplicate successors and to list history.
3. **Make missed occurrences explicit.** Add a policy enum: `skip`, `backfill`,
   or `one_overdue`. Test the behavior for a task completed a week late.
4. **Do not overwrite `due_at` silently.** Add a `set_recurrence` variant that
   refuses when `due_at` is already set unless `replace_due` is true.
5. **Complete early.** Start a daily task tomorrow, complete it today, and assert
   the next occurrence is the day after tomorrow, not today. Explain why
   `max(now, starts_at)` is the right lower bound.

## What comes next

The series can produce its next occurrence, but nothing wakes up to deliver a
reminder. Chapter 10 adds a scheduler: a due-time queue, a claim protocol,
retries, and a worker loop. The recurrence rule will decide *when* an occurrence
is due; the scheduler will decide *when to act*.
