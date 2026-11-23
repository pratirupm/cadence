# 7. Query DSL

**What you can do at the end.** Filter tasks with `due:today & priority:>2`
instead of adding a new endpoint for every combination.

**The Rust you learn.** Lexing, recursive-descent parsing, an AST, lifetimes in
a compiler-shaped program, and parameterized SQL.

**Build it.**

- Define `Expr`, `Comparison`, `Field`, `Op`, and `Param`.
- Tokenize identifiers, strings, parentheses, `&`, `|`, `!`, `:`, and
  comparison operators.
- Parse `or` → `and` → `unary` → parenthesis or comparison.
- Compile the AST to a SQL clause and a `Vec<Param>`.
- Whitelist fields and operators. Bind values; never concatenate input.
- Treat `text:foo` as `title LIKE ? OR notes LIKE ?`.

The parser returns a tree, not SQL:

```rust
pub fn parse(input: &str) -> Result<Expr, QueryError> {
    let tokens = lex(input)?;
    let mut parser = Parser { tokens, position: 0 };
    let expr = parser.parse_or()?;
    // ...
    Ok(expr)
}
```

**Checkpoint.**

```console
$ cargo test query
$ cargo test filters_tasks_with_the_query_language
```

The tests cover precedence, parentheses, quoted values, status, priority,
text, unknown fields, and invalid values.

**Exercises.**

1. Add a `sort:` directive that maps to a whitelisted `ORDER BY`.
2. Add relative date tests with a fixed clock.
