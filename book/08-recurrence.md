# 8. Recurrence

**What you can do at the end.** Give a task a rule such as
`FREQ=WEEKLY;BYDAY=MO,WE` and ask for the next occurrence.

**The Rust you learn.** Enums with data, date arithmetic, time zones, DST,
parsing key/value rules, and property-style thinking.

**Build it.**

- Support `FREQ` daily/weekly/monthly/yearly, `INTERVAL`, `BYDAY`,
  `BYMONTHDAY`, `COUNT`, `UNTIL`, and `WKST`.
- Parse the rule into `Rule`; keep `starts_at` and the IANA time zone beside it.
- Iterate candidate dates in local wall time.
- Convert each candidate with `from_local_datetime(...).earliest()`.
  Nonexistent DST times are skipped; ambiguous times choose the first.
- Count matches from the start so `COUNT` and `UNTIL` are correct.
- Reject zero intervals, unknown frequencies, and invalid month days.

The core is a date loop, not a cron expression:

```rust
let candidate = self.timezone
    .from_local_datetime(&naive)
    .earliest()?
    .with_timezone(&Utc);
```

**Checkpoint.**

```console
$ cargo test recurrence
```

The tests cover a two-day interval, Monday/Wednesday weeks, month-end skips,
`COUNT`, `UNTIL`, DST wall-time preservation, and invalid rules.

**Exercises.**

1. Add `BYMONTH` and a test for a yearly rule.
2. Differential-test supported rules against the `rrule` crate.
