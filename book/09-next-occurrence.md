# 9. The next occurrence

**What you can do at the end.** Complete a recurring task and watch the next
occurrence appear with the same title, project, and rule.

**The Rust you learn.** Combining a domain rule with persistence, using a
transactional boundary, and choosing an “after” instant carefully.

**Build it.**

- Add `recurrence` and `recurrence_timezone` columns to tasks.
- Add `set_recurrence` with a validated rule and a start time.
- On `set_completed(..., true)`, load the completed task and insert a new task
  when a rule exists.
- Use `Utc::now().max(task.due_at.unwrap_or(...))` as the lower bound so
  completing an overdue task does not recreate the past.
- Preserve notes, priority, assignee, and the recurrence rule.
- Return the completed task; the next one appears in the list.

**Checkpoint.**

```console
$ cargo test completing_a_recurring_task_creates_the_next_occurrence
$ cargo test tasks
```

The test creates a daily task, completes it, and asserts two rows: one done and
one open with the next day’s due date.

**Exercises.**

1. Add a `series_id` so the UI can show history.
2. Decide what happens when the rule has no next occurrence, and test it.
