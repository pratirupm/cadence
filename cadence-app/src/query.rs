use chrono::{Duration, NaiveDate, TimeZone, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Comparison(Comparison),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    pub field: Field,
    pub op: Op,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Text,
    Status,
    Priority,
    Due,
    Created,
    Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    NotEq,
    Lt,
    Lte,
    Gt,
    Gte,
    Contains,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Param {
    Text(String),
    Integer(i64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlFilter {
    pub clause: String,
    pub params: Vec<Param>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum QueryError {
    #[error("expected {0}")]
    Expected(&'static str),
    #[error("unknown field: {0}")]
    UnknownField(String),
    #[error("invalid value for {0}: {1}")]
    InvalidValue(&'static str, String),
    #[error("unexpected input: {0}")]
    Unexpected(String),
}

pub fn parse(input: &str) -> Result<Expr, QueryError> {
    let tokens = lex(input)?;
    let mut parser = Parser {
        tokens,
        position: 0,
    };
    let expr = parser.parse_or()?;

    if parser.peek().is_some() {
        return Err(QueryError::Unexpected(parser.remaining()));
    }

    Ok(expr)
}

impl Expr {
    pub fn to_sql(&self) -> Result<SqlFilter, QueryError> {
        let mut params = Vec::new();
        let clause = self.clause(&mut params)?;
        Ok(SqlFilter { clause, params })
    }

    fn clause(&self, params: &mut Vec<Param>) -> Result<String, QueryError> {
        match self {
            Self::Comparison(comparison) => comparison.clause(params),
            Self::And(left, right) => Ok(format!(
                "({} AND {})",
                left.clause(params)?,
                right.clause(params)?
            )),
            Self::Or(left, right) => Ok(format!(
                "({} OR {})",
                left.clause(params)?,
                right.clause(params)?
            )),
            Self::Not(expr) => Ok(format!("NOT ({})", expr.clause(params)?)),
        }
    }
}

impl Comparison {
    fn clause(&self, params: &mut Vec<Param>) -> Result<String, QueryError> {
        match self.field {
            Field::Text => {
                if self.op == Op::Contains {
                    let pattern = format!("%{}%", self.value);
                    params.push(Param::Text(pattern.clone()));
                    params.push(Param::Text(pattern));
                    Ok("(title LIKE ? OR notes LIKE ?)".to_owned())
                } else {
                    params.push(Param::Text(self.value.clone()));
                    Ok("title = ?".to_owned())
                }
            }
            Field::Status => {
                let completed = match self.value.as_str() {
                    "done" | "completed" => 1,
                    "open" | "todo" => 0,
                    other => {
                        return Err(QueryError::InvalidValue("status", other.to_owned()));
                    }
                };
                params.push(Param::Integer(completed));
                Ok("completed = ?".to_owned())
            }
            Field::Priority => {
                let priority = self
                    .value
                    .parse::<i64>()
                    .map_err(|_| QueryError::InvalidValue("priority", self.value.clone()))?;
                params.push(Param::Integer(priority));
                Ok(format!("priority {} ?", self.sql_op()))
            }
            Field::Due => self.date_clause("due_at", params, true),
            Field::Created => self.date_clause("created_at", params, false),
            Field::Project => {
                params.push(Param::Text(self.value.clone()));
                Ok("project_id = ?".to_owned())
            }
        }
    }

    fn date_clause(
        &self,
        column: &str,
        params: &mut Vec<Param>,
        due: bool,
    ) -> Result<String, QueryError> {
        let today = Utc::now().date_naive();
        match self.value.as_str() {
            "today" => {
                let start = day_start(today);
                let end = day_start(today + Duration::days(1));
                params.push(Param::Text(start.to_rfc3339()));
                params.push(Param::Text(end.to_rfc3339()));
                Ok(format!("({column} >= ? AND {column} < ?)"))
            }
            "tomorrow" => {
                let start = day_start(today + Duration::days(1));
                let end = day_start(today + Duration::days(2));
                params.push(Param::Text(start.to_rfc3339()));
                params.push(Param::Text(end.to_rfc3339()));
                Ok(format!("({column} >= ? AND {column} < ?)"))
            }
            "overdue" if due => {
                params.push(Param::Text(Utc::now().to_rfc3339()));
                Ok("(due_at < ? AND completed = 0)".to_owned())
            }
            "none" => Ok(format!("{column} IS NULL")),
            value => {
                let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
                    .map_err(|_| QueryError::InvalidValue("date", value.to_owned()))?;
                let start = day_start(date);
                let end = day_start(date + Duration::days(1));
                params.push(Param::Text(start.to_rfc3339()));
                params.push(Param::Text(end.to_rfc3339()));
                Ok(format!("({column} >= ? AND {column} < ?)"))
            }
        }
    }

    fn sql_op(&self) -> &'static str {
        match self.op {
            Op::Eq => "=",
            Op::NotEq => "!=",
            Op::Lt => "<",
            Op::Lte => "<=",
            Op::Gt => ">",
            Op::Gte => ">=",
            Op::Contains => "LIKE",
        }
    }
}

fn day_start(date: NaiveDate) -> chrono::DateTime<Utc> {
    Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn parse_or(&mut self) -> Result<Expr, QueryError> {
        let mut expr = self.parse_and()?;

        while self.consume(&Token::Or) {
            let right = self.parse_and()?;
            expr = Expr::Or(Box::new(expr), Box::new(right));
        }

        Ok(expr)
    }

    fn parse_and(&mut self) -> Result<Expr, QueryError> {
        let mut expr = self.parse_unary()?;

        while self.consume(&Token::And) {
            let right = self.parse_unary()?;
            expr = Expr::And(Box::new(expr), Box::new(right));
        }

        Ok(expr)
    }

    fn parse_unary(&mut self) -> Result<Expr, QueryError> {
        if self.consume(&Token::Not) {
            return Ok(Expr::Not(Box::new(self.parse_unary()?)));
        }

        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, QueryError> {
        if self.consume(&Token::LParen) {
            let expr = self.parse_or()?;
            self.expect(Token::RParen, "a closing parenthesis")?;
            return Ok(expr);
        }

        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Result<Expr, QueryError> {
        let field = match self.next() {
            Some(Token::Ident(field)) => Field::from_name(&field)?,
            _ => return Err(QueryError::Expected("a field name")),
        };
        self.expect(Token::Colon, "a colon")?;

        let (op, explicit) = if self.consume(&Token::Eq) {
            (Op::Eq, true)
        } else if self.consume(&Token::NotEq) {
            (Op::NotEq, true)
        } else if self.consume(&Token::Lte) {
            (Op::Lte, true)
        } else if self.consume(&Token::Lt) {
            (Op::Lt, true)
        } else if self.consume(&Token::Gte) {
            (Op::Gte, true)
        } else if self.consume(&Token::Gt) {
            (Op::Gt, true)
        } else {
            (Op::Eq, false)
        };

        let value = match self.next() {
            Some(Token::Ident(value)) | Some(Token::String(value)) => value,
            _ => return Err(QueryError::Expected("a value")),
        };

        let op = if field == Field::Text && !explicit && op == Op::Eq {
            Op::Contains
        } else {
            op
        };

        Ok(Expr::Comparison(Comparison { field, op, value }))
    }

    fn expect(&mut self, expected: Token, message: &'static str) -> Result<(), QueryError> {
        if self.consume(&expected) {
            Ok(())
        } else {
            Err(QueryError::Expected(message))
        }
    }

    fn consume(&mut self, expected: &Token) -> bool {
        if self.peek() == Some(expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.peek().cloned();
        if token.is_some() {
            self.position += 1;
        }
        token
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn remaining(&self) -> String {
        self.tokens
            .iter()
            .skip(self.position)
            .map(|token| format!("{token:?}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl Field {
    fn from_name(name: &str) -> Result<Self, QueryError> {
        match name {
            "text" | "title" => Ok(Self::Text),
            "status" => Ok(Self::Status),
            "priority" => Ok(Self::Priority),
            "due" => Ok(Self::Due),
            "created" => Ok(Self::Created),
            "project" => Ok(Self::Project),
            _ => Err(QueryError::UnknownField(name.to_owned())),
        }
    }
}

fn lex(input: &str) -> Result<Vec<Token>, QueryError> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(character) = chars.next() {
        match character {
            character if character.is_whitespace() => {}
            '(' => tokens.push(Token::LParen),
            ')' => tokens.push(Token::RParen),
            '&' => tokens.push(Token::And),
            '|' => tokens.push(Token::Or),
            '!' => {
                if chars.peek() == Some(&'=') {
                    chars.next();
                    tokens.push(Token::NotEq);
                } else {
                    tokens.push(Token::Not);
                }
            }
            ':' => tokens.push(Token::Colon),
            '=' => tokens.push(Token::Eq),
            '<' => {
                if chars.peek() == Some(&'=') {
                    chars.next();
                    tokens.push(Token::Lte);
                } else {
                    tokens.push(Token::Lt);
                }
            }
            '>' => {
                if chars.peek() == Some(&'=') {
                    chars.next();
                    tokens.push(Token::Gte);
                } else {
                    tokens.push(Token::Gt);
                }
            }
            '"' => {
                let mut value = String::new();
                let mut closed = false;
                for next in chars.by_ref() {
                    if next == '"' {
                        closed = true;
                        break;
                    }
                    value.push(next);
                }
                if !closed {
                    return Err(QueryError::Unexpected("unterminated string".to_owned()));
                }
                tokens.push(Token::String(value));
            }
            character => {
                let mut value = String::from(character);
                while let Some(next) = chars.peek() {
                    if next.is_whitespace()
                        || matches!(next, '(' | ')' | '&' | '|' | '!' | ':' | '=' | '<' | '>')
                    {
                        break;
                    }
                    value.push(*next);
                    chars.next();
                }
                tokens.push(Token::Ident(value));
            }
        }
    }

    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_an_and_expression() {
        let expr = parse("due:today & priority:>2").unwrap();

        let Expr::And(left, right) = expr else {
            panic!("expected an AND expression");
        };
        assert!(matches!(*left, Expr::Comparison(_)));
        assert!(matches!(*right, Expr::Comparison(_)));
    }

    #[test]
    fn parses_or_and_not_with_parentheses() {
        let expr = parse("(status:done | status:open) & !priority:0").unwrap();

        assert!(matches!(expr, Expr::And(_, _)));
    }

    #[test]
    fn parses_quoted_values() {
        let expr = parse("text:\"buy milk\"").unwrap();
        let Expr::Comparison(comparison) = expr else {
            panic!("expected a comparison");
        };

        assert_eq!(comparison.value, "buy milk");
        assert_eq!(comparison.op, Op::Contains);
    }

    #[test]
    fn text_equality_can_be_requested_explicitly() {
        let expr = parse("text:=buy").unwrap();
        let Expr::Comparison(comparison) = expr else {
            panic!("expected a comparison");
        };

        assert_eq!(comparison.op, Op::Eq);
        assert_eq!(comparison.clause(&mut Vec::new()).unwrap(), "title = ?");
    }

    #[test]
    fn compiles_a_status_filter() {
        let filter = parse("status:done").unwrap().to_sql().unwrap();

        assert_eq!(filter.clause, "completed = ?");
        assert_eq!(filter.params, [Param::Integer(1)]);
    }

    #[test]
    fn compiles_a_priority_filter() {
        let filter = parse("priority:>2").unwrap().to_sql().unwrap();

        assert_eq!(filter.clause, "priority > ?");
        assert_eq!(filter.params, [Param::Integer(2)]);
    }

    #[test]
    fn compiles_a_text_contains_filter_with_two_parameters() {
        let filter = parse("text:milk").unwrap().to_sql().unwrap();

        assert_eq!(filter.clause, "(title LIKE ? OR notes LIKE ?)");
        assert_eq!(
            filter.params,
            [
                Param::Text("%milk%".to_owned()),
                Param::Text("%milk%".to_owned())
            ]
        );
    }

    #[test]
    fn rejects_unknown_fields_and_bad_values() {
        assert!(matches!(
            parse("owner:ada"),
            Err(QueryError::UnknownField(_))
        ));
        assert!(matches!(
            parse("priority:high").map(|expr| expr.to_sql()),
            Ok(Err(QueryError::InvalidValue(_, _)))
        ));
        assert!(matches!(
            parse("status:done &"),
            Err(QueryError::Expected(_))
        ));
    }
}
