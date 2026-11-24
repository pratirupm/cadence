---
title: "Query DSL"
description: "Parse task filters into an AST and compile them to bound SQL."
---

The task list can filter by one value at a time. That is not enough. A person
wants “open work due today with priority above 2,” not a form with five
independent controls and a rule about how they combine. Every new combination
would need a new endpoint or a new branch.

This chapter adds a small query language:

```text
due:today & priority:>2
text:"buy milk" | text:"oat milk"
(status:done | status:open) & !priority:0
```

The language is deliberately small. It is not SQL, and it is not a general
expression language. It is a typed filter for one table. The parser turns text
into an AST; the compiler turns the AST into a parameterized SQL clause. The
task service binds the parameters. No user input is concatenated into SQL.

By the end of this chapter you can:

- parse `field:value`, comparisons, `&`, `|`, `!`, and parentheses
- quote a value that contains spaces
- compile a filter to a SQL clause plus a parameter list
- resolve `today`, `tomorrow`, `overdue`, and `none`
- reject an unknown field, a bad number, or trailing input
- explain why the parser and the SQL compiler are separate steps

The Rust lessons are lexing, recursive-descent parsing, an AST with boxed
recursion, `Result` with specific parse errors, and parameterized query
building. The security lesson is that a parser is not a sanitizer; binding is.

## Why a DSL

There are three common alternatives:

1. **One query parameter per filter.** `?status=open&due=today&priority_gt=2`.
   Simple for one filter, awkward for `or`, negation, and nesting. The URL
   becomes a schema.
2. **Raw SQL from the client.** Flexible and unacceptable. It couples the client
   to the schema and invites injection.
3. **A small expression language.** Flexible enough for combinations, small
   enough to reason about, and compilable to bound SQL.

The DSL is the third option. It has a grammar, an AST, and a compiler. The
grammar is the contract; the compiler is the only place that knows column names.

The language is case-sensitive for fields and operators. `Due:today` is an
unknown field. That is a deliberate simplification; a case-insensitive parser is
an exercise.

## The AST

Start with the types, not the parser:

```rust
pub enum Expr {
    Comparison(Comparison),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
}

pub struct Comparison {
    pub field: Field,
    pub op: Op,
    pub value: String,
}

pub enum Field {
    Text,
    Status,
    Priority,
    Due,
    Created,
    Project,
}

pub enum Op {
    Eq,
    NotEq,
    Lt,
    Lte,
    Gt,
    Gte,
    Contains,
}
```

`Expr` is recursive, so the binary variants own `Box<Expr>`. Without the box,
the enum would have an infinite size. This is the first recursive AST in the
book; the box is the standard way to give it a finite representation.

`Field` is an enum, not a string. The parser resolves `"due"` to `Field::Due`
once, and every later stage matches on the enum. A typo becomes a parse error,
not a runtime SQL error.

`Op::Contains` is separate from `Eq` because it compiles to `LIKE` and produces
two parameters for the text field. The parser chooses `Contains` when the field
is text and no explicit operator was written.

`Param` is the bridge to SQLx:

```rust
pub enum Param {
    Text(String),
    Integer(i64),
}
```

A parameter has a type because SQLite is dynamically typed but the bind call is
not. `priority > ?` needs an integer; `title LIKE ?` needs text. Mixing them
produces a query that runs but compares the wrong things.

`SqlFilter` is the compiler’s output:

```rust
pub struct SqlFilter {
    pub clause: String,
    pub params: Vec<Param>,
}
```

The clause contains `?` placeholders. It never contains a user value. The params
are bound in order. If the order is wrong, the query is wrong even though it
compiles; tests must assert both.

`QueryError` distinguishes syntax from semantics:

```rust
pub enum QueryError {
    Expected(&'static str),
    UnknownField(String),
    InvalidValue(&'static str, String),
    Unexpected(String),
}
```

`Expected` is a grammar error. `UnknownField` is a vocabulary error.
`InvalidValue` is a type error. `Unexpected` covers leftover tokens and
unterminated strings. The HTTP layer maps all of them to a 400 with a readable
message; the service does not guess.

## The lexer

The lexer turns characters into tokens:

```rust
enum Token {
    Ident(String),
    String(String),
    LParen,
    RParen,
    And,
    Or,
    Not,
    Colon,
    Eq,
    NotEq,
    Lt,
    Lte,
    Gt,
    Gte,
}
```

`lex` skips whitespace, recognizes single-character punctuation, and reads two
characters for `!=`, `<=`, and `>=`. A quoted value reads until the closing
quote and rejects an unterminated string. An unquoted value reads until
whitespace or punctuation.

Two simplifications are visible:

- A quoted string has no escape sequence. You cannot write a literal `"` inside
  a quoted value. The exercise asks you to add `\"`.
- An unquoted value can contain characters such as `-`, `.`, `/`, and `_`, which
  is what dates and project IDs need. It stops at the operators, so
  `priority:>2` lexes as `Ident(priority)`, `Colon`, `Gt`, `Ident(2)`.

The lexer does not know which field is being parsed. `status:done` and
`priority:2` produce the same token shapes until the parser assigns meaning.

## The parser

The parser is recursive descent with three levels:

```text
or      := and ("|" and)*
and     := unary ("&" unary)*
unary   := "!" unary | primary
primary := "(" or ")" | comparison
```

This gives `&` higher precedence than `|`, and `!` higher than both. A reader
who writes `a | b & c` gets `a | (b & c)`. That is the usual convention, and the
parentheses are available when the reader wants a different grouping.

`parse` returns the AST only if every token was consumed:

```rust
pub fn parse(input: &str) -> Result<Expr, QueryError> {
    let tokens = lex(input)?;
    let mut parser = Parser { tokens, position: 0 };
    let expr = parser.parse_or()?;

    if parser.peek().is_some() {
        return Err(QueryError::Unexpected(parser.remaining()));
    }

    Ok(expr)
}
```

Trailing input is an error. `status:done &` must not silently become
`status:done`. The `remaining` helper renders the tokens that were left, which
makes the error useful in a test or a UI.

`parse_comparison` resolves the field, expects a colon, reads an optional
operator, and reads a value:

```rust
let field = match self.next() {
    Some(Token::Ident(field)) => Field::from_name(&field)?,
    _ => return Err(QueryError::Expected("a field name")),
};
self.expect(Token::Colon, "a colon")?;
```

The default operator is `Eq`:

```rust
let op = if self.consume(&Token::Eq) { Op::Eq }
    else if self.consume(&Token::NotEq) { Op::NotEq }
    // ...
    else { Op::Eq };
```

Then the text special case:

```rust
let op = if field == Field::Text && !explicit && op == Op::Eq {
    Op::Contains
} else {
    op
};
```

Why special-case text? `text:buy` should search for “buy” inside the title and
notes, not require the title to equal “buy.” A user who wants exact equality can
write `text:=buy`; the parser records whether an operator was explicit and only
rewrites the default equality to `Contains`. That is a small but useful escape
hatch.

Field names are mapped in one place:

```rust
match name {
    "text" | "title" => Ok(Field::Text),
    "status" => Ok(Field::Status),
    "priority" => Ok(Field::Priority),
    "due" => Ok(Field::Due),
    "created" => Ok(Field::Created),
    "project" => Ok(Field::Project),
    _ => Err(QueryError::UnknownField(name.to_owned())),
}
```

`title` is an alias for `text`. The AST has one text field, so the compiler does
not care which spelling was used. Aliases are cheap at the parser boundary and
expensive if they leak into the database.

## Compiling to SQL

`Expr::to_sql` walks the tree and appends parameters in the same order as the
placeholders:

```rust
pub fn to_sql(&self) -> Result<SqlFilter, QueryError> {
    let mut params = Vec::new();
    let clause = self.clause(&mut params)?;
    Ok(SqlFilter { clause, params })
}
```

Boolean nodes wrap their children in parentheses:

```rust
Self::And(left, right) => Ok(format!(
    "({} AND {})",
    left.clause(params)?,
    right.clause(params)?
)),
```

The parentheses are not optional. SQL precedence is not the same as the DSL’s,
and a future compiler change should not silently regroup a query. Every generated
boolean expression is explicit.

`Not` becomes `NOT (...)`; the inner expression is already parenthesized when it
is a boolean node, and the extra parentheses around the whole expression are
harmless.

### Text

```rust
if self.op == Op::Contains {
    let pattern = format!("%{}%", self.value);
    params.push(Param::Text(pattern.clone()));
    params.push(Param::Text(pattern));
    Ok("(title LIKE ? OR notes LIKE ?)".to_owned())
} else {
    params.push(Param::Text(self.value.clone()));
    Ok("title = ?".to_owned())
}
```

The pattern is built from the value, but the value is bound, not concatenated
into SQL. A value containing `%` or `_` is treated as a wildcard by SQLite’s
`LIKE`. That is a known limitation: `text:100%` matches more than a literal
“100%.” A production implementation escapes `%`, `_`, and the escape character,
then adds `ESCAPE '\'`. The exercise asks for it.

`LIKE` is case-insensitive for ASCII in SQLite by default. That is usually what a
search box wants. It is not Unicode case folding; a title in a non-Latin script
may need a different collation. Do not promise more than the database does.

### Status

```rust
let completed = match self.value.as_str() {
    "done" | "completed" => 1,
    "open" | "todo" => 0,
    other => return Err(QueryError::InvalidValue("status", other.to_owned())),
};
params.push(Param::Integer(completed));
Ok("completed = ?".to_owned())
```

The DSL uses words; the database uses `0`/`1`. The mapping is explicit and
tested. An unknown status is an error, not a silent “open.” Silently defaulting a
filter is worse than rejecting it: the user thinks they filtered and they did
not.

### Priority

```rust
let priority = self.value.parse::<i64>()
    .map_err(|_| QueryError::InvalidValue("priority", self.value.clone()))?;
params.push(Param::Integer(priority));
Ok(format!("priority {} ?", self.sql_op()))
```

The operator is chosen from a closed set. `sql_op` returns one of `=`, `!=`,
`<`, `<=`, `>`, `>=`, `LIKE`. No user text reaches the SQL operator position.

`priority:>2` is the common form. `priority:2` means equality. `priority:>=0`
includes unprioritized tasks.

### Dates

`Due` and `Created` share a date helper. `today` expands to a half-open day:

```text
due_at >= today 00:00 UTC AND due_at < tomorrow 00:00 UTC
```

Two parameters, not string interpolation. The half-open range is deliberate:
`>= start AND < end` includes every instant in the day exactly once and avoids
the `23:59:59` gap.

`tomorrow` uses the next day. `none` becomes `IS NULL`. `overdue` is only allowed
for `due`:

```rust
"overdue" if due => {
    params.push(Param::Text(Utc::now().to_rfc3339()));
    Ok("(due_at < ? AND completed = 0)".to_owned())
}
```

An overdue task is past its due date and not completed. A completed task with a
past due date is not overdue. The extra `completed = 0` is part of the meaning,
not an optimization.

An explicit date is `YYYY-MM-DD`:

```rust
let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
    .map_err(|_| QueryError::InvalidValue("date", value.to_owned()))?;
```

The date is interpreted as UTC midnight. This is a limitation: a user in
`Asia/Kolkata` asking for “today” gets the UTC day, which may start and end at
the wrong local times. A recurring task stores its own zone, but the filter
compiler does not know the *viewer’s* zone. A production query needs a viewer
time zone parameter and a local-day range converted to UTC. The chapter keeps
the UTC contract and makes the limitation explicit; the exercise asks you to thread a
zone through.

### Project

```rust
Field::Project => {
    params.push(Param::Text(self.value.clone()));
    Ok("project_id = ?".to_owned())
}
```

The task list is already scoped to one project, so this filter is redundant in
the current UI. It becomes useful in a cross-project view or the CLI. The
compiler still supports it because the AST field exists; do not add a field that
the compiler silently ignores.

## Binding in the task service

`TaskService::list_filtered` is the only caller that assembles the SQL:

```rust
let compiled = query::parse(filter)?.to_sql()?;
let sql = format!(
    "SELECT ... FROM tasks
     WHERE project_id = ? AND deleted_at IS NULL AND ({})
     ORDER BY completed, created_at DESC, id DESC",
    compiled.clause
);

let mut request = sqlx::query_as::<_, Task>(&sql).bind(project_id);
for param in compiled.params {
    request = match param {
        Param::Text(value) => request.bind(value),
        Param::Integer(value) => request.bind(value),
    };
}
```

The project ID is bound first, before the filter parameters. The clause’s `?`
placeholders come after it in the same order. If the order is wrong, the project
ID is compared to `completed` or a title, and the results are nonsense. A test
that only checks the number of rows will not catch a type-compatible swap; assert
the actual task titles.

The SQL string uses `format!` only for the compiled clause, which is built from
whitelisted fragments (`AND`, `OR`, `NOT`, column names, operators). Values are
always bound. This is the line between compiling and injecting. If you add a new
field, add it to the compiler’s whitelist; never interpolate a raw value.

The default empty filter skips parsing entirely and uses a fixed query. That
avoids a parser allocation on the common path and keeps the default ordering
obvious.

## Tests

The query module has eight unit tests:

- `parses_an_and_expression`: `due:today & priority:>2`.
- `parses_or_and_not_with_parentheses`: precedence and grouping.
- `parses_quoted_values`: `text:"buy milk"` becomes `Contains`.
- `text_equality_can_be_requested_explicitly`: `text:=buy` stays exact.
- `compiles_a_status_filter`: `completed = ?` with `1`.
- `compiles_a_priority_filter`: `priority > ?` with `2`.
- `compiles_a_text_contains_filter_with_two_parameters`: title and notes.
- `rejects_unknown_fields_and_bad_values`: `owner`, `priority:high`, trailing
  `&`.

The task service adds an integration test that creates two tasks, completes one,
and filters with `status:open`, `status:done`, `text:milk`, and an invalid field.
That test proves the clause composes with the project scope and the task mapper.

Run:

```console
$ cargo test query
$ cargo test filters_tasks_with_the_query_language
```

## Checkpoint

```console
$ cargo test query
$ cargo test tasks
$ cargo test
$ cargo fmt --check
$ cargo clippy --all-targets --all-features -- -D warnings
```

There is no separate query UI yet; chapter 6’s app page already has a `filter`
input for the service to use. The checkpoint is the parser, compiler, and binding
contract.

## Failure modes you will meet

**`unknown field: owner`.** The field whitelist does not include `owner`. Add a
field to the enum and compiler, or fix the query. Do not fall back to a text
search; that hides the spelling error.

**`expected a value` after a colon.** The value is missing, or whitespace
separated it from the colon. Write `status:done`, not `status: done`. If you want
optional whitespace, change the grammar and test it.

**`unexpected input: ...` after a valid expression.** There are trailing tokens.
Common causes are a double `&&`, an unclosed quote, or a stray `)`. The error
prints the remaining tokens; use them.

**`invalid value for priority: high`.** Priority is an integer. The parser does
not translate words such as `high` to numbers; that would be a second vocabulary
with no clear mapping.

**A filter returns too many rows.** Check the `LIKE` pattern first. `%` and `_`
are wildcards unless escaped. Then check the date range: a UTC day may not match
the user’s local day.

**A filter returns too few rows.** Check the operator. `status:done` is
`completed = 1`; `status:open` is `0`. `due:today` is a range, not an exact
timestamp. A task due at 23:59 UTC is not “today” for a user in another zone.

**The query returns rows from another project.** The project ID bind is missing
or in the wrong position. The filter is not the authorization boundary; the
project scope is.

## Under the hood

Recursive descent is a direct translation of the grammar. `parse_or` calls
`parse_and`, which calls `parse_unary`, which calls `parse_primary`. Each level
handles one precedence. The parser has a token vector and a position; there is no
backtracking and no parser combinator library.

The AST is the boundary between syntax and SQL. A formatter, a debug view, or a
second compiler (for example, an in-memory evaluator for the CLI) can reuse it.
The CLI sync client does not evaluate filters locally in this book; it sends the
string to the server. That is a future exercise.

`SqlFilter` is not a `sqlx::Query`. It is a small description of a clause and its
parameters. Keeping it independent of SQLx makes the compiler unit-testable
without a database.

## Exercises

1. **Escape wildcards.** In the text compiler, escape `%`, `_`, and `\` in the
   value, then add `ESCAPE '\'` to the clause. Add tests for `text:100%` and
   `text:a_b`.
2. **Add relative dates.** Support `due:+7d` and `due:-1d` relative to today.
   Decide whether the unit is days, hours, or both, and test the boundaries.
3. **Thread a time zone.** Change `to_sql` to accept a `Tz` and compute the local
   day range, then convert to UTC. Add a test for a non-UTC zone.
4. **Add sorting.** Parse `sort:due` and `sort:-created`, and emit a whitelisted
   `ORDER BY` instead of a clause. Decide how multiple sort keys combine.
5. **Add a label field.** Add a `label` field to the AST and compiler. The labels
   table is not in the task query yet; decide whether to join or to use a
   subquery, and test both a match and no match.

## What comes next

The filter language can ask for tasks due today, but nothing schedules them.
Chapter 8 adds recurrence rules: a rule parser, local-time expansion, count and
until limits, and the DST behavior that makes calendar software hard.
