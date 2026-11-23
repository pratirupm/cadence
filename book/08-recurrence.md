# 8. Recurrence

A task that says “water the plants every three days” is not a task with a date.
It is a rule. The date is one occurrence; the rule is the series. If the service
stores only a date, completing the task loses the schedule. If it stores only a
string, every caller invents its own parser.

This chapter adds a deliberately small recurrence engine. It supports a subset
of RFC 5545 RRULE syntax, expands it in a named time zone, and returns the next
occurrence after an instant. It does not create tasks yet; chapter 9 connects
completion to the engine. This chapter is the calendar logic by itself.

By the end of this chapter you can:

- parse `FREQ=DAILY`, `FREQ=WEEKLY;BYDAY=MO,WE`, and a monthly month-end rule
- apply `INTERVAL`, `COUNT`, and `UNTIL`
- keep a 9:00 local reminder at 9:00 across a DST change
- explain why a nonexistent local time is skipped
- reject an unknown frequency, a zero interval, or an invalid weekday
- test a calendar rule without sleeping or using the real clock

The Rust lessons are enums with data, key/value parsing, date arithmetic,
`chrono-tz`, and `Option` as “no next occurrence.” The domain lesson is the
difference between a wall-clock time and an instant. Most calendar bugs are a
failure to keep that distinction.

## The grammar we support

The input is an RRULE string:

```text
FREQ=DAILY
FREQ=WEEKLY;BYDAY=MO,WE
FREQ=MONTHLY;BYMONTHDAY=31
FREQ=DAILY;INTERVAL=3
FREQ=DAILY;COUNT=5
FREQ=DAILY;UNTIL=2026-12-31T00:00:00Z
```

Supported keys:

| Key | Meaning |
| --- | --- |
| `FREQ` | `DAILY`, `WEEKLY`, `MONTHLY`, `YEARLY` |
| `INTERVAL` | positive integer, default 1 |
| `BYDAY` | comma-separated `MO,TU,WE,TH,FR,SA,SU` |
| `BYMONTHDAY` | comma-separated days 1–31 |
| `COUNT` | positive number of occurrences |
| `UNTIL` | RFC 3339 instant or `YYYY-MM-DD` (end of day UTC) |
| `WKST` | accepted and ignored |

Unsupported: `BYSETPOS`, `BYYEARDAY`, `BYWEEKNO`, `BYMONTH`, `BYHOUR`,
`BYMINUTE`, and the rest of RRULE. Why not support everything? A full RRULE
implementation is a project. The subset covers the recurring tasks a todo app
actually needs, and every unsupported key fails loudly instead of being ignored.
A user who writes `BYSETPOS=1` gets an error, not a silently different schedule.

`WKST` is accepted because some clients send it; ignoring it is safe only because
the subset does not use week-start-dependent counts. If you add `BYWEEKNO`, you
must implement it.

## Types

The rule is data:

```rust
pub struct Rule {
    pub frequency: Frequency,
    pub interval: u32,
    pub by_day: Vec<Day>,
    pub by_month_day: Vec<u32>,
    pub count: Option<u32>,
    pub until: Option<DateTime<Utc>>,
}

pub struct Recurrence {
    pub rule: Rule,
    pub starts_at: DateTime<Utc>,
    pub timezone: Tz,
}
```

`Frequency` and `Day` are enums, not strings. `Day` maps to chrono’s `Weekday`
for comparisons. `by_day` and `by_month_day` are vectors because a rule can name
several days.

`starts_at` is the first occurrence. It is an instant, but the engine converts
it to the rule’s time zone before extracting the local date and time of day.
That local time is what stays fixed across DST.

`timezone` is an IANA zone such as `America/New_York`, parsed by `chrono-tz`.
A fixed offset such as `-05:00` is not enough: a recurring 9:00 task must follow
the zone’s DST rules, not a constant offset.

`next_after` returns `Option<DateTime<Utc>>`:

- `Some(next)` means there is a next occurrence after the instant.
- `None` means the rule is exhausted (`COUNT` or `UNTIL`) or no candidate was
  found within the search horizon.

There is no “error” for exhaustion. A finished series is a normal state.

## Parsing

`Recurrence::parse` parses the zone and delegates the rule:

```rust
pub fn parse(
    rrule: &str,
    timezone: &str,
    starts_at: DateTime<Utc>,
) -> Result<Self, RecurrenceError> {
    let timezone = timezone
        .parse::<Tz>()
        .map_err(|_| RecurrenceError::InvalidTimezone(timezone.to_owned()))?;
    let rule = parse_rule(rrule)?;

    Ok(Self { rule, starts_at, timezone })
}
```

`parse_rule` splits on `;`, then on the first `=`:

```rust
for part in rrule.split(';').filter(|part| !part.is_empty()) {
    let Some((key, value)) = part.split_once('=') else {
        return Err(RecurrenceError::InvalidValue("rule", part.to_owned()));
    };
    // match key ...
}
```

Defaults and validation:

- `FREQ` is required. Missing it is `MissingFrequency`.
- `INTERVAL` defaults to 1 and must be greater than zero.
- `BYDAY` parses each code through `Day::parse`; an unknown code is an error.
- `BYMONTHDAY` parses 1–31; zero or 32 is an error.
- `COUNT` must be greater than zero.
- `UNTIL` parses RFC 3339, or a date at 23:59:59 UTC.
- An unknown key is an error, except `WKST`.

The date-only `UNTIL` choice is a small product decision. A date usually means
“through the end of that day,” so the parser uses 23:59:59 UTC. RFC 5545 says
`UNTIL` must be an instant when the rule has a time zone; accepting a date is a
convenience with a documented meaning.

Errors are typed:

```rust
pub enum RecurrenceError {
    MissingFrequency,
    InvalidValue(&'static str, String),
    InvalidTimezone(String),
    Empty,
}
```

`Empty` is reserved for a future “no occurrences at all” case; the current
engine returns `None` instead. Do not leave a variant unused if it confuses the
contract; either use it or remove it. The book keeps it because a stricter
compiler could choose to reject a rule whose first occurrence is before `UNTIL`.

## The expansion loop

The engine iterates one local date at a time from `starts_at`:

```rust
let local_start = self.starts_at.with_timezone(&self.timezone);
let start_date = local_start.date_naive();
let start_time = local_start.time();

for offset in 0..MAX_DAYS {
    let date = start_date + Duration::days(offset);
    if !self.matches_date(start_date, date) {
        continue;
    }

    let naive = NaiveDateTime::new(date, start_time);
    let Some(candidate) = self.timezone.from_local_datetime(&naive).earliest() else {
        continue;
    };
    let candidate = candidate.with_timezone(&Utc);

    if let Some(until) = self.rule.until
        && candidate > until
    {
        return None;
    }

    matches += 1;
    if let Some(count) = self.rule.count
        && matches > count
    {
        return None;
    }

    if candidate > after {
        return Some(candidate);
    }
}
```

Why a day loop instead of adding months directly? Month arithmetic has edge cases:
January 31 plus one month is not February 31. Weekday sets and interval weeks
need a consistent anchor. A day loop is slower but obviously correct. The
horizon `MAX_DAYS = 366 * 100` bounds the work for a rule that never matches
(for example, February 30 encoded as a month day that never exists in the
selected month). `None` after 100 years is a deliberate safety valve.

`matches_date` is the frequency switch:

```rust
match self.rule.frequency {
    Frequency::Daily => {
        let offset = (date - start).num_days();
        offset >= 0 && offset % i64::from(self.rule.interval) == 0
    }
    Frequency::Weekly => {
        let offset = weeks_between(start, date);
        if offset < 0 || offset % i64::from(self.rule.interval) != 0 {
            return false;
        }
        if self.rule.by_day.is_empty() {
            date.weekday() == start.weekday()
        } else {
            self.rule.by_day.iter().any(|day| day.as_chrono() == date.weekday())
        }
    }
    // monthly and yearly ...
}
```

Daily counts days from the start date. Weekly counts Monday-based weeks from the
start week. Monthly counts calendar months. Yearly counts years. The interval
applies to that unit, not to the number of matched days. A weekly rule with
`INTERVAL=2;BYDAY=MO,WE` runs Monday and Wednesday in every other week.

Monthly requires the day to match:

```rust
let day_matches = if self.rule.by_month_day.is_empty() {
    date.day() == start.day()
} else {
    self.rule.by_month_day.contains(&date.day())
};
```

If you set `BYMONTHDAY=31`, February is skipped because it has no 31st. That is
correct for a strict rule. Some calendar apps “clamp” to the last day of the
month; that is a different product decision and must be explicit. The test
`skips_invalid_month_days` encodes the strict behavior.

Yearly uses the start month and either the start day or `BYMONTHDAY`. It does not
support `BYMONTH` yet. A yearly rule that starts in February stays in February.

## Count and until

`COUNT` counts occurrences from the start, including the start occurrence. The
loop increments `matches` for every matching candidate, even those before
`after`, then returns `None` once `matches > count`. That means `next_after`
can return `None` without scanning forever, and `COUNT=3` really means three
occurrences total.

`UNTIL` is inclusive in the comparison `candidate > until`; a candidate exactly
at `until` is allowed. This is the usual “through this instant” interpretation.
If you want exclusive, use `>=`.

The order of checks matters. `UNTIL` is checked before the count increment, so a
candidate after `UNTIL` cannot consume a count. The count is checked before the
`after` comparison, so an exhausted series does not return a value.

## Time zones and DST

A recurring task is a wall-clock rule, not a fixed interval in UTC. “Every day
at 9:00” in New York is 14:00 UTC in winter and 13:00 UTC in summer. The engine
keeps the local time and converts each date:

```rust
let naive = NaiveDateTime::new(date, start_time);
let candidate = self.timezone.from_local_datetime(&naive).earliest()?;
```

`from_local_datetime` can return:

- a single instant (the normal case),
- two instants (the ambiguous hour when clocks go back),
- no instant (the skipped hour when clocks go forward).

The engine uses `.earliest()` for the ambiguous hour. For the skipped hour it
gets `None` and skips that occurrence. This is a policy, not a law. The
alternatives are to shift forward to the first valid instant, shift backward, or
skip; each produces a different series. Document the choice and test it.

The DST test:

```rust
let recurrence = Recurrence::parse(
    "FREQ=DAILY",
    "America/New_York",
    utc("2026-03-07T14:00:00Z"),
).unwrap();

assert_eq!(
    recurrence.next_after(utc("2026-03-07T15:00:00Z")),
    Some(utc("2026-03-08T13:00:00Z"))
);
```

The local time is 09:00 on both days. The UTC instant moves from 14:00 to 13:00
because the offset changes. If the engine stored a fixed offset, the second
occurrence would be 09:00 EST, which is 10:00 local after the spring-forward.

The `starts_at` instant is converted to local time once. If the start falls in a
skipped hour, `.with_timezone` still gives a valid local representation for the
instant, but the reconstructed candidate may be skipped. That is an edge case
worth a test if you support starts in the DST gap.

## Worked examples

A few traces make the rules concrete. All examples start at `2026-01-01T09:00Z`
in `UTC` unless noted.

| Rule | After | Next | Why |
| --- | --- | --- | --- |
| `FREQ=DAILY` | Jan 1 10:00 | Jan 2 09:00 | The next local 09:00 |
| `FREQ=DAILY;INTERVAL=2` | Jan 1 10:00 | Jan 3 09:00 | Every other day |
| `FREQ=WEEKLY;BYDAY=MO,WE` | Mon 10:00 | Wed 09:00 | The next selected weekday |
| `FREQ=WEEKLY;BYDAY=MO,WE` | Wed 10:00 | next Mon 09:00 | The week rolls over |
| `FREQ=MONTHLY;BYMONTHDAY=31` | Feb 1 | Mar 31 09:00 | February has no 31st |
| `FREQ=DAILY;COUNT=3` | Jan 3 09:00 | `None` | Three total, start included |
| `FREQ=DAILY;UNTIL=Jan 2 09:00Z` | Jan 2 09:00 | `None` | The instant is the last allowed |

The `after` comparison is strict: if `after` is exactly an occurrence, the
engine returns the following one. That is what a completed task needs. The
`UNTIL` comparison allows an occurrence exactly at the boundary; that is the
“through this instant” convention described above.

`starts_at` is not merely a display value. It anchors the interval and the time
of day. Two daily rules with the same `FREQ` but starts at 08:00 and 09:00
produce different instants. It also counts toward `COUNT`, so changing the start
can change how many future occurrences remain.

## Recurrence is not a scheduler

Three related ideas are easy to conflate:

- **Recurrence** answers “what is the next occurrence after this one?”
- **A scheduler** decides when to run a job that checks due work.
- **A reminder** is a delivery at a specific instant.

A recurrence rule has no clock. It is a pure function of `starts_at`, the rule,
and the zone. A scheduler wakes up periodically and asks the database which
reminders are due. A reminder points at a task or occurrence and has its own
delivery state. Chapter 10 builds the scheduler; chapter 11 builds delivery.

Keeping the functions separate makes each one testable. `next_after` needs no
database and no sleeping. The scheduler needs a clock and a claim protocol. The
reminder sender needs a mailer and retries. Do not hide all three in one
“background task” function.

## Tests

Six unit tests cover the engine:

- `advances_daily_with_an_interval`: `FREQ=DAILY;INTERVAL=2`.
- `advances_weekly_on_selected_days`: Monday/Wednesday, including the next week.
- `skips_invalid_month_days`: January 31 → March 31.
- `respects_count_and_until`: `COUNT=3` stops; `UNTIL` stops.
- `keeps_local_wall_time_across_dst`: the New York spring-forward case.
- `rejects_invalid_rules`: missing `FREQ`, zero `INTERVAL`, unknown `FREQ`.

Run:

```console
$ cargo test recurrence
```

The tests use fixed instants and no sleeps. A calendar test that depends on the
current time is a flaky test with a delay.

## Checkpoint

```console
$ cargo test recurrence
$ cargo test
$ cargo fmt --check
$ cargo clippy --all-targets --all-features -- -D warnings
```

There is no task integration yet. The checkpoint is a pure engine with fixed
inputs and outputs.

## Failure modes you will meet

**`missing FREQ`.** The rule is empty or starts with `INTERVAL`. `FREQ` is
required; there is no sensible default.

**`invalid recurrence value for INTERVAL: 0`.** An interval of zero would mean
“every zero days” and loop forever or divide by zero. The parser rejects it.

**A monthly rule never fires.** `BYMONTHDAY=31` in a month without a 31st is
skipped. If you expected clamping, change the policy and test it; do not assume
the engine is broken.

**A daily rule jumps by an hour after DST.** The engine is preserving local wall
time, which is usually correct. If the task is a fixed interval from an instant
(for example, “every 24 hours”), use UTC rather than a named zone and document
the difference.

**A rule with `COUNT=3` returns only two future occurrences.** The start
occurrence counts. If the start is in the past and you have already completed
it, the next call still counts it toward the total. The caller must decide
whether completion advances the series or the rule is anchored at the start.

**`next_after` returns `None` unexpectedly.** Check `UNTIL`, `COUNT`, and
`BYMONTHDAY`. A 100-year horizon also returns `None`; that is a safety valve, not
a valid schedule.

**The time zone is wrong for a user.** The rule’s zone is stored with the task.
Do not silently replace it with the server’s zone. A task created in
`Europe/London` and viewed in `Asia/Tokyo` still recurs at London wall time
unless the user edits the rule.

## Under the hood

`chrono-tz` maps IANA names to transition tables. A `Tz` is not an offset; it is
a rule for converting local date-times to instants and back. Store the name, not
a computed offset.

`NaiveDateTime` has no zone. It is the wall-clock reading. `DateTime<Utc>` is an
instant. The conversion between them is where DST lives. Keeping those types
distinct prevents the most common calendar bug: treating a local time as an
instant.

The day loop is O(days between start and next), not O(occurrences). For a rule
that starts ten years ago and runs daily, the next call scans ten years. A
production engine would jump by interval units and filter, or cache the last
occurrence. The book chooses the simple loop because correctness is easier to
see and the horizon bounds it. Chapter 22 measures before optimizing.

## Exercises

1. **Add `BYMONTH`.** Support `FREQ=YEARLY;BYMONTH=3` and test a March-only
   rule. Decide how it interacts with `BYMONTHDAY`.
2. **Implement `WKST`.** Make week intervals start on the configured day and add
   a test where Monday and Sunday starts differ.
3. **Differential-test the subset.** Add the `rrule` crate as a dev-dependency
   and compare `next_after` for supported rules. Skip unsupported keys
   explicitly, not silently.
4. **Thread the viewer’s time zone.** Add a `next_after_in(&self, after, tz)`
   that renders the next occurrence in a display zone without changing the
   rule’s zone. Test the same instant in two zones.
5. **Clamp month-end.** Add an option that maps the 31st to the last day of a
   shorter month. Test January 31 → February 28/29, and explain why this is a
   different rule, not a bug fix.

## What comes next

The engine can answer “when is the next occurrence?” Chapter 9 connects that
answer to task completion: completing a recurring task creates the next one,
with the same title, project, priority, assignee, and rule. The transaction and
version behavior from chapter 5 become part of the calendar contract.
