//! The sandboxed `${{ … }}` interpolation evaluator.
//!
//! A **pure** function [`evaluate`]`(expression, scope) -> Result<Value, RunError>` over a bounded,
//! hand-written AST — the JavaScript *subset* the engine accepts, with **no** JS engine and **no**
//! `eval` ([`.specs/04-execution-engine.md` §State & interpolation scopes](../../../.specs/04-execution-engine.md)).
//! The subset is: member access (`a.b`, `a[expr]`), literals (number, string, `true`/`false`/`null`),
//! **strict** equality (`===`/`!==`), boolean/`!` logic (`&&`/`||`/`!`), and JS truthy/falsy. There
//! are **no** function calls, no assignment, and no arbitrary code — the grammar cannot express them.
//!
//! Two bounds keep the sandbox safe (both named constants in [`tmx_schema::limits`], never inline
//! literals): an expression longer than [`EXPR_LEN_MAX_BYTES`] is rejected *before* tokenising with
//! `expr_too_long`, and the recursive-descent parser threads and **asserts** an AST depth of at most
//! [`EXPR_DEPTH_MAX`] at parse time (`expr_too_deep`), so a pathologically nested input can never
//! recurse past the bound and overflow the stack. Every failure — a syntax error, an unknown
//! namespace key, a type mismatch, an unlisted secret — is a typed [`RunError`] in the
//! [`Resolution`](ErrorCategory::Resolution) category, never a panic.
//!
//! [`EXPR_LEN_MAX_BYTES`]: tmx_schema::limits::EXPR_LEN_MAX_BYTES
//! [`EXPR_DEPTH_MAX`]: tmx_schema::limits::EXPR_DEPTH_MAX
//! [`ErrorCategory`]: crate::error::ErrorCategory
//! [`ErrorCategory::Resolution`]: crate::error::ErrorCategory::Resolution

use serde_json::Value;
use tmx_schema::limits::{EXPR_DEPTH_MAX, EXPR_LEN_MAX_BYTES};

use crate::error::RunError;
use crate::model::Scope;

// ---------------------------------------------------------------------------------------------
// Public entry point.
// ---------------------------------------------------------------------------------------------

/// Evaluate a single `${{ … }}` interpolation `expression` against `scope`.
///
/// Returns the resolved [`Value`], or a [`RunError`] in the
/// [`Resolution`](crate::error::ErrorCategory::Resolution) category naming the failure:
///
/// - `expr_too_long` — the expression exceeds [`EXPR_LEN_MAX_BYTES`](tmx_schema::limits::EXPR_LEN_MAX_BYTES) UTF-8 bytes.
/// - `expr_too_deep` — the parsed AST is deeper than [`EXPR_DEPTH_MAX`](tmx_schema::limits::EXPR_DEPTH_MAX).
/// - `expr_parse_error` — the text is not a well-formed expression in the subset.
/// - `unknown_namespace` — an unknown root namespace, or a missing key / out-of-range index.
/// - `unlisted_secret` — a `secrets.NAME` whose `NAME` the task did not list (so it is absent from scope).
/// - `type_mismatch` — a member/index access on a value that is not a container of the right shape.
///
/// The function is pure and total: it never panics, and it never mutates `scope`.
pub fn evaluate(expression: &str, scope: &Scope) -> Result<Value, RunError> {
    let ast = parse(expression)?;
    eval(&ast, scope)
}

// ---------------------------------------------------------------------------------------------
// AST — a bounded, hand-written tree. No node can represent a call or an assignment.
// ---------------------------------------------------------------------------------------------

/// A parsed `${{ }}` expression node.
#[derive(Debug, Clone, PartialEq)]
enum Expr {
    /// A literal JSON scalar (`1`, `"s"`, `true`, `false`, `null`).
    Literal(Value),
    /// A namespace-rooted access chain: `root` then zero or more `steps` (`inputs.a[0].b`).
    Path { root: String, steps: Vec<Step> },
    /// Logical negation `!e`.
    Not(Box<Expr>),
    /// A binary operator applied to two operands.
    Binary(BinOp, Box<Expr>, Box<Expr>),
}

/// One access step in a [`Path`](Expr::Path): a dotted field or a bracketed, evaluated index.
#[derive(Debug, Clone, PartialEq)]
enum Step {
    /// `.name` — read `name` from an object.
    Field(String),
    /// `[expr]` — read the key/index produced by evaluating `expr` (a string field or integer index).
    Index(Box<Expr>),
}

/// A binary operator in the subset — strict equality and short-circuiting boolean logic only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BinOp {
    /// Strict equality `===` (type-and-value; never coercing).
    StrictEq,
    /// Strict inequality `!==`.
    StrictNe,
    /// Short-circuiting logical AND `&&` (returns an operand value, JS-style).
    And,
    /// Short-circuiting logical OR `||` (returns an operand value, JS-style).
    Or,
}

// ---------------------------------------------------------------------------------------------
// Tokenizer.
// ---------------------------------------------------------------------------------------------

/// A lexical token of the subset.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `[`
    LBracket,
    /// `]`
    RBracket,
    /// `.`
    Dot,
    /// `!`
    Bang,
    /// `===`
    EqEqEq,
    /// `!==`
    BangEqEq,
    /// `&&`
    AmpAmp,
    /// `||`
    PipePipe,
    /// An identifier (`inputs`, `NAME`, `true`, …).
    Ident(String),
    /// A numeric literal, preserving integer-vs-float via [`serde_json::Number`].
    Number(serde_json::Number),
    /// A string literal (quotes stripped, escapes decoded).
    Str(String),
}

/// A parse-category failure (`expr_parse_error`).
fn parse_err(message: impl Into<String>) -> RunError {
    RunError::resolution("expr_parse_error", message)
}

/// Tokenise `src` into the token stream, or an `expr_parse_error` on an illegal character.
fn tokenize(src: &str) -> Result<Vec<Token>, RunError> {
    let bytes = src.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b' ' | b'\t' | b'\r' | b'\n' => {
                i += 1;
            }
            b'(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            b')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            b'[' => {
                tokens.push(Token::LBracket);
                i += 1;
            }
            b']' => {
                tokens.push(Token::RBracket);
                i += 1;
            }
            b'.' => {
                tokens.push(Token::Dot);
                i += 1;
            }
            b'!' => {
                if bytes.get(i + 1) == Some(&b'=') && bytes.get(i + 2) == Some(&b'=') {
                    tokens.push(Token::BangEqEq);
                    i += 3;
                } else {
                    tokens.push(Token::Bang);
                    i += 1;
                }
            }
            b'=' => {
                if bytes.get(i + 1) == Some(&b'=') && bytes.get(i + 2) == Some(&b'=') {
                    tokens.push(Token::EqEqEq);
                    i += 3;
                } else {
                    // A lone `=` or `==` is not in the subset — only strict `===` is.
                    return Err(parse_err(
                        "only strict equality `===`/`!==` is supported, not `=`/`==`",
                    ));
                }
            }
            b'&' => {
                if bytes.get(i + 1) == Some(&b'&') {
                    tokens.push(Token::AmpAmp);
                    i += 2;
                } else {
                    return Err(parse_err(
                        "a lone `&` is not a valid operator (did you mean `&&`?)",
                    ));
                }
            }
            b'|' => {
                if bytes.get(i + 1) == Some(&b'|') {
                    tokens.push(Token::PipePipe);
                    i += 2;
                } else {
                    return Err(parse_err(
                        "a lone `|` is not a valid operator (did you mean `||`?)",
                    ));
                }
            }
            b'"' | b'\'' => {
                let (s, next) = lex_string(src, i)?;
                tokens.push(Token::Str(s));
                i = next;
            }
            b'-' | b'0'..=b'9' => {
                let (num, next) = lex_number(src, i)?;
                tokens.push(Token::Number(num));
                i = next;
            }
            _ if is_ident_start(b) => {
                let start = i;
                i += 1;
                while i < bytes.len() && is_ident_continue(bytes[i]) {
                    i += 1;
                }
                // `start..i` is ASCII by construction (is_ident_* accept ASCII only), so this slice
                // is always a valid str boundary.
                tokens.push(Token::Ident(src[start..i].to_string()));
            }
            _ => {
                return Err(parse_err(format!(
                    "unexpected character {:?} in expression",
                    b as char
                )));
            }
        }
    }
    Ok(tokens)
}

/// Whether `b` may start an identifier (ASCII letter or `_`).
fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

/// Whether `b` may continue an identifier (letter, digit, or `_`).
fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Lex a quoted string literal starting at `start` (the opening quote). Returns the decoded contents
/// and the index just past the closing quote.
fn lex_string(src: &str, start: usize) -> Result<(String, usize), RunError> {
    let bytes = src.as_bytes();
    let quote = bytes[start];
    let mut out = String::new();
    let mut i = start + 1;
    while i < bytes.len() {
        let b = bytes[i];
        if b == quote {
            return Ok((out, i + 1));
        }
        if b == b'\\' {
            let esc = bytes
                .get(i + 1)
                .copied()
                .ok_or_else(|| parse_err("string ends with a dangling backslash"))?;
            let decoded = match esc {
                b'\\' => '\\',
                b'"' => '"',
                b'\'' => '\'',
                b'/' => '/',
                b'n' => '\n',
                b't' => '\t',
                b'r' => '\r',
                other => {
                    return Err(parse_err(format!(
                        "unsupported string escape `\\{}`",
                        other as char
                    )));
                }
            };
            out.push(decoded);
            i += 2;
            continue;
        }
        // Copy one whole UTF-8 char so multibyte content survives intact.
        let ch = src[i..]
            .chars()
            .next()
            .ok_or_else(|| parse_err("malformed UTF-8 in string literal"))?;
        out.push(ch);
        i += ch.len_utf8();
    }
    Err(parse_err("unterminated string literal"))
}

/// Lex a numeric literal starting at `start`. Accepts an optional leading `-`, integer and decimal
/// forms, and an exponent — parsed via [`serde_json::Number`] so integers stay integers.
fn lex_number(src: &str, start: usize) -> Result<(serde_json::Number, usize), RunError> {
    let bytes = src.as_bytes();
    let mut i = start;
    if bytes[i] == b'-' {
        i += 1;
    }
    let digits_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i == digits_start {
        return Err(parse_err("`-` is not followed by a number"));
    }
    let lexeme = &src[start..i];
    let num: serde_json::Number = serde_json::from_str(lexeme)
        .map_err(|_| parse_err(format!("`{lexeme}` is not a valid number literal")))?;
    Ok((num, i))
}

// ---------------------------------------------------------------------------------------------
// Parser — precedence-climbing, with an asserted AST-depth bound threaded through every recursion.
// ---------------------------------------------------------------------------------------------

/// Parse `src` into an [`Expr`], enforcing the length bound before tokenising and the depth bound
/// during recursive descent.
fn parse(src: &str) -> Result<Expr, RunError> {
    // Length guard first: reject an over-long expression *before* tokenising, so the parser never
    // sees an unbounded input.
    if src.len() as u64 > EXPR_LEN_MAX_BYTES {
        return Err(RunError::resolution(
            "expr_too_long",
            format!(
                "expression is {} bytes, over the {EXPR_LEN_MAX_BYTES}-byte limit",
                src.len()
            ),
        ));
    }
    let tokens = tokenize(src)?;
    let mut parser = Parser { tokens, pos: 0 };
    // The top level is depth 1; each nested paren, operand, or bracket index adds one.
    let expr = parser.parse_bp(0, 1)?;
    if parser.pos != parser.tokens.len() {
        return Err(parse_err("unexpected trailing tokens after the expression"));
    }
    Ok(expr)
}

/// The recursive-descent parser state.
struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    /// Peek the current token without consuming it.
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    /// Consume and return the current token.
    fn advance(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    /// Consume a token that must equal `expected`, or fail with a parse error.
    fn expect(&mut self, expected: &Token, what: &str) -> Result<(), RunError> {
        match self.advance() {
            Some(ref t) if t == expected => Ok(()),
            _ => Err(parse_err(format!("expected {what}"))),
        }
    }

    /// The depth guard: reject an AST deeper than [`EXPR_DEPTH_MAX`] *before* recursing further, so
    /// a pathologically nested input cannot overflow the parser stack.
    fn check_depth(depth: u32) -> Result<(), RunError> {
        if depth > EXPR_DEPTH_MAX {
            return Err(RunError::resolution(
                "expr_too_deep",
                format!("expression AST is deeper than the {EXPR_DEPTH_MAX}-level limit"),
            ));
        }
        Ok(())
    }

    /// Precedence-climbing parse: parse an expression whose operators bind at least as tightly as
    /// `min_bp`, at nesting level `depth`.
    fn parse_bp(&mut self, min_bp: u8, depth: u32) -> Result<Expr, RunError> {
        Self::check_depth(depth)?;

        // Prefix / primary.
        let mut lhs = match self.advance() {
            Some(Token::Bang) => {
                // `!` binds tighter than any binary operator here.
                let operand = self.parse_bp(BANG_BP, depth + 1)?;
                Expr::Not(Box::new(operand))
            }
            Some(Token::LParen) => {
                let inner = self.parse_bp(0, depth + 1)?;
                self.expect(&Token::RParen, "a closing `)`")?;
                inner
            }
            Some(Token::Number(n)) => Expr::Literal(Value::Number(n)),
            Some(Token::Str(s)) => Expr::Literal(Value::String(s)),
            Some(Token::Ident(name)) => match name.as_str() {
                "true" => Expr::Literal(Value::Bool(true)),
                "false" => Expr::Literal(Value::Bool(false)),
                "null" => Expr::Literal(Value::Null),
                _ => {
                    let steps = self.parse_steps(depth)?;
                    Expr::Path { root: name, steps }
                }
            },
            Some(other) => {
                return Err(parse_err(format!("unexpected token {other:?}")));
            }
            None => {
                return Err(parse_err("unexpected end of expression"));
            }
        };

        // Infix loop.
        loop {
            let op = match self.peek() {
                Some(Token::PipePipe) => BinOp::Or,
                Some(Token::AmpAmp) => BinOp::And,
                Some(Token::EqEqEq) => BinOp::StrictEq,
                Some(Token::BangEqEq) => BinOp::StrictNe,
                _ => break,
            };
            let (lbp, rbp) = infix_bp(op);
            if lbp < min_bp {
                break;
            }
            self.pos += 1; // consume the operator
            let rhs = self.parse_bp(rbp, depth + 1)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// Parse a (possibly empty) run of `.field` / `[index]` access steps after a namespace root.
    /// Member chaining is iterative — only a bracketed index recurses (guarded by `depth`).
    fn parse_steps(&mut self, depth: u32) -> Result<Vec<Step>, RunError> {
        let mut steps = Vec::new();
        loop {
            match self.peek() {
                Some(Token::Dot) => {
                    self.pos += 1;
                    match self.advance() {
                        Some(Token::Ident(name)) => steps.push(Step::Field(name)),
                        _ => return Err(parse_err("expected a field name after `.`")),
                    }
                }
                Some(Token::LBracket) => {
                    self.pos += 1;
                    let index = self.parse_bp(0, depth + 1)?;
                    self.expect(&Token::RBracket, "a closing `]`")?;
                    steps.push(Step::Index(Box::new(index)));
                }
                _ => break,
            }
        }
        Ok(steps)
    }
}

/// The binding power of the prefix `!` — tighter than every binary operator.
const BANG_BP: u8 = 7;

/// The left/right binding powers of a binary operator (all left-associative).
fn infix_bp(op: BinOp) -> (u8, u8) {
    match op {
        BinOp::Or => (1, 2),
        BinOp::And => (3, 4),
        BinOp::StrictEq | BinOp::StrictNe => (5, 6),
    }
}

// ---------------------------------------------------------------------------------------------
// Evaluator.
// ---------------------------------------------------------------------------------------------

/// Evaluate `expr` against `scope`.
fn eval(expr: &Expr, scope: &Scope) -> Result<Value, RunError> {
    match expr {
        Expr::Literal(v) => Ok(v.clone()),
        Expr::Not(inner) => {
            let v = eval(inner, scope)?;
            Ok(Value::Bool(!is_truthy(&v)))
        }
        Expr::Binary(op, lhs, rhs) => eval_binary(*op, lhs, rhs, scope),
        Expr::Path { root, steps } => resolve_path(root, steps, scope),
    }
}

/// Evaluate a binary operator, short-circuiting `&&`/`||` JS-style (returning an operand value).
fn eval_binary(op: BinOp, lhs: &Expr, rhs: &Expr, scope: &Scope) -> Result<Value, RunError> {
    match op {
        BinOp::And => {
            let l = eval(lhs, scope)?;
            if is_truthy(&l) {
                eval(rhs, scope)
            } else {
                Ok(l)
            }
        }
        BinOp::Or => {
            let l = eval(lhs, scope)?;
            if is_truthy(&l) {
                Ok(l)
            } else {
                eval(rhs, scope)
            }
        }
        BinOp::StrictEq => {
            let l = eval(lhs, scope)?;
            let r = eval(rhs, scope)?;
            Ok(Value::Bool(strict_eq(&l, &r)))
        }
        BinOp::StrictNe => {
            let l = eval(lhs, scope)?;
            let r = eval(rhs, scope)?;
            Ok(Value::Bool(!strict_eq(&l, &r)))
        }
    }
}

/// JS truthiness: `false`, `0`, `""`, `null` are falsy; everything else — including `[]` and `{}` —
/// is truthy.
fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// JS strict equality (`===`): equal only when the two values share a type. Numbers compare by
/// numeric value (`1 === 1.0`); a number and a string are never equal (`1 === "1"` is `false`);
/// composites compare structurally (the sandbox has no object identity to distinguish otherwise).
fn strict_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(fx), Some(fy)) => fx == fy,
            _ => false,
        },
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(l, r)| strict_eq(l, r))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, lv)| y.get(k).is_some_and(|rv| strict_eq(lv, rv)))
        }
        // Any cross-type pair — number vs string included — is not strictly equal.
        _ => false,
    }
}

/// Resolve a namespace-rooted access chain against `scope`.
fn resolve_path(root: &str, steps: &[Step], scope: &Scope) -> Result<Value, RunError> {
    let root_is_secret = root == "secrets";
    let base: &Value = match root {
        "inputs" => scope.inputs,
        "env" => scope.env,
        "secrets" => scope.secrets,
        "tasks" => scope.tasks,
        "matrix" => scope.matrix,
        "item" => scope.item.ok_or_else(|| unbound(root))?,
        "case" => scope.case.ok_or_else(|| unbound(root))?,
        "output" => scope.output.ok_or_else(|| unbound(root))?,
        other => {
            return Err(RunError::resolution(
                "unknown_namespace",
                format!("`{other}` is not a known interpolation namespace"),
            ));
        }
    };

    let mut current = base.clone();
    for (i, step) in steps.iter().enumerate() {
        let first = i == 0;
        current = apply_step(current, step, root, root_is_secret && first, scope)?;
    }
    Ok(current)
}

/// The `ResolutionError` for an optional namespace referenced outside its construct
/// (`item`/`case`/`output`).
fn unbound(root: &str) -> RunError {
    RunError::resolution(
        "unknown_namespace",
        format!("`{root}` is not bound in this scope"),
    )
}

/// Apply one access `step` to `current`. `secret_first` is true only for the first step under the
/// `secrets` root, where a missing key is the distinct `unlisted_secret` failure.
fn apply_step(
    current: Value,
    step: &Step,
    root: &str,
    secret_first: bool,
    scope: &Scope,
) -> Result<Value, RunError> {
    match step {
        Step::Field(name) => access_key(current, name, root, secret_first),
        Step::Index(expr) => {
            let key = eval(expr, scope)?;
            match key {
                Value::String(s) => access_key(current, &s, root, secret_first),
                Value::Number(n) => {
                    let idx = n.as_u64().ok_or_else(|| {
                        RunError::resolution(
                            "type_mismatch",
                            format!("index must be a non-negative integer, got `{n}`"),
                        )
                    })?;
                    access_index(current, idx, root)
                }
                other => Err(RunError::resolution(
                    "type_mismatch",
                    format!(
                        "an index must be a string or number, got {}",
                        type_name(&other)
                    ),
                )),
            }
        }
    }
}

/// Read `key` from `current`, which must be an object.
fn access_key(
    current: Value,
    key: &str,
    root: &str,
    secret_first: bool,
) -> Result<Value, RunError> {
    match current {
        Value::Object(map) => map.get(key).cloned().ok_or_else(|| {
            if secret_first {
                RunError::resolution(
                    "unlisted_secret",
                    format!(
                        "secret `{key}` is not available (the task did not list it in `secrets`)"
                    ),
                )
            } else {
                RunError::resolution("unknown_namespace", format!("`{root}` has no key `{key}`"))
            }
        }),
        other => Err(RunError::resolution(
            "type_mismatch",
            format!(
                "cannot read property `{key}` of {} (in `{root}`)",
                type_name(&other)
            ),
        )),
    }
}

/// Read numeric `index` from `current`, which must be an array.
fn access_index(current: Value, index: u64, root: &str) -> Result<Value, RunError> {
    match current {
        Value::Array(items) => {
            let idx = usize::try_from(index).map_err(|_| {
                RunError::resolution("type_mismatch", format!("index `{index}` is out of range"))
            })?;
            items.get(idx).cloned().ok_or_else(|| {
                RunError::resolution(
                    "unknown_namespace",
                    format!("index `{index}` is out of range in `{root}`"),
                )
            })
        }
        other => Err(RunError::resolution(
            "type_mismatch",
            format!(
                "cannot index {} with `[{index}]` (in `{root}`)",
                type_name(&other)
            ),
        )),
    }
}

/// A short human name for a JSON value's type, for error messages.
fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A scope backed by owned JSON values, so tests can build a `Scope` of borrows ergonomically.
    struct ScopeData {
        inputs: Value,
        env: Value,
        secrets: Value,
        tasks: Value,
        item: Option<Value>,
        case: Option<Value>,
        output: Option<Value>,
        matrix: Value,
    }

    impl ScopeData {
        fn scope(&self) -> Scope<'_> {
            Scope {
                inputs: &self.inputs,
                env: &self.env,
                secrets: &self.secrets,
                tasks: &self.tasks,
                item: self.item.as_ref(),
                case: self.case.as_ref(),
                output: self.output.as_ref(),
                matrix: &self.matrix,
            }
        }
    }

    fn sample() -> ScopeData {
        ScopeData {
            inputs: json!({ "name": "ada", "count": 3, "flag": true }),
            env: json!({ "HOME": "/home/ada", "EMPTY": "" }),
            secrets: json!({ "TOKEN": "s3cr3t" }),
            tasks: json!({ "build": { "ok": true, "artifacts": ["a", "b"] } }),
            item: None,
            case: None,
            output: None,
            matrix: json!({ "os": "linux" }),
        }
    }

    fn eval_str(expr: &str, data: &ScopeData) -> Result<Value, RunError> {
        evaluate(expr, &data.scope())
    }

    // --- O1: each namespace resolves to its documented value -----------------------------------

    #[test]
    fn every_namespace_resolves_to_its_documented_value() {
        let mut data = sample();
        data.item = Some(json!({ "index": 2, "sku": "x1" }));
        data.case = Some(json!({ "prompt": "hi" }));
        data.output = Some(json!({ "score": 0.9 }));

        assert_eq!(
            eval_str("inputs.name", &data).unwrap(),
            json!("ada"),
            "inputs.NAME reads a declared input"
        );
        assert_eq!(
            eval_str("env.HOME", &data).unwrap(),
            json!("/home/ada"),
            "env.KEY reads a context env var"
        );
        assert_eq!(
            eval_str("secrets.TOKEN", &data).unwrap(),
            json!("s3cr3t"),
            "secrets.NAME reads a listed secret"
        );
        assert_eq!(
            eval_str("tasks.build.artifacts[1]", &data).unwrap(),
            json!("b"),
            "tasks.NAME.field[i] reads a prior task's nested output"
        );
        assert_eq!(
            eval_str("item.sku", &data).unwrap(),
            json!("x1"),
            "item.* reads the current map element"
        );
        assert_eq!(
            eval_str("item.index", &data).unwrap(),
            json!(2),
            "item.index reads the element's position"
        );
        assert_eq!(
            eval_str("case.prompt", &data).unwrap(),
            json!("hi"),
            "case.* reads the current eval case"
        );
        assert_eq!(
            eval_str("output.score", &data).unwrap(),
            json!(0.9),
            "output reads the subject output"
        );
        assert_eq!(
            eval_str("output", &data).unwrap(),
            json!({ "score": 0.9 }),
            "bare `output` is the whole subject output value"
        );
        assert_eq!(
            eval_str("matrix.os", &data).unwrap(),
            json!("linux"),
            "matrix.KEY reads the current combination"
        );
    }

    #[test]
    fn scope_gated_namespaces_are_unbound_outside_their_construct() {
        // With item/case/output all None (a plain sequential task), each is a ResolutionError, not a
        // value — the negative space of construct-gated binding.
        let data = sample();
        for expr in [
            "item",
            "item.index",
            "case.prompt",
            "output",
            "output.score",
        ] {
            let err = eval_str(expr, &data).expect_err("must be unbound outside its construct");
            assert_eq!(
                err.category,
                crate::error::ErrorCategory::Resolution,
                "{expr} is a resolution error"
            );
            assert_eq!(err.code, "unknown_namespace", "{expr} is unknown_namespace");
        }
        // matrix is always present (it defaults to `{}` when not run via --matrix), so a *missing*
        // key under it is unknown_namespace, but the namespace itself is bound.
        assert!(
            eval_str("matrix", &data).is_ok(),
            "matrix is always a bound namespace"
        );
    }

    // --- O1: strict equality distinguishes 1 from "1" ------------------------------------------

    #[test]
    fn strict_equality_distinguishes_number_from_string() {
        let data = sample();
        assert_eq!(
            eval_str("1 === \"1\"", &data).unwrap(),
            json!(false),
            "a number and a string are never strictly equal"
        );
        assert_eq!(
            eval_str("1 === 1", &data).unwrap(),
            json!(true),
            "equal numbers are strictly equal"
        );
        assert_eq!(
            eval_str("1 === 1.0", &data).unwrap(),
            json!(true),
            "strict equality compares numbers by value, not representation"
        );
        assert_eq!(
            eval_str("1 !== \"1\"", &data).unwrap(),
            json!(true),
            "!== is the negation of ==="
        );
        assert_eq!(
            eval_str("\"a\" === \"a\"", &data).unwrap(),
            json!(true),
            "equal strings are strictly equal"
        );
        assert_eq!(
            eval_str("true === 1", &data).unwrap(),
            json!(false),
            "a boolean and a number are never strictly equal (no coercion)"
        );
        assert_eq!(
            eval_str("null === null", &data).unwrap(),
            json!(true),
            "null is strictly equal to null"
        );
    }

    // --- O1: JS truthy/falsy on every falsy case -----------------------------------------------

    #[test]
    fn truthiness_matches_js_on_the_falsy_table() {
        let data = sample();
        // `x || "F"` returns "F" exactly when x is falsy, and returns x when x is truthy — so it is a
        // direct probe of the truthiness of x (JS `||` yields an operand value).
        let falsy = ["false", "0", "\"\"", "null"];
        for x in falsy {
            assert_eq!(
                eval_str(&format!("{x} || \"F\""), &data).unwrap(),
                json!("F"),
                "{x} is falsy"
            );
        }
        // Truthy values — including empty array/object and the string "0" — are returned unchanged.
        assert_eq!(
            eval_str("\"0\" || \"F\"", &data).unwrap(),
            json!("0"),
            "non-empty string is truthy"
        );
        assert_eq!(
            eval_str("1 || \"F\"", &data).unwrap(),
            json!(1),
            "non-zero number is truthy"
        );
        assert_eq!(
            eval_str("true || \"F\"", &data).unwrap(),
            json!(true),
            "true is truthy"
        );
        assert_eq!(
            eval_str("inputs === inputs", &data).unwrap(),
            json!(true),
            "an object compares structurally equal to itself"
        );
        assert_eq!(
            eval_str("!\"\"", &data).unwrap(),
            json!(true),
            "!empty-string is true (empty string is falsy)"
        );
        assert_eq!(
            eval_str("!inputs.flag", &data).unwrap(),
            json!(false),
            "!true is false"
        );
        // `&&` short-circuits and returns the operand JS-style.
        assert_eq!(
            eval_str("true && \"kept\"", &data).unwrap(),
            json!("kept"),
            "truthy && x returns x"
        );
        assert_eq!(
            eval_str("false && \"dropped\"", &data).unwrap(),
            json!(false),
            "falsy && x short-circuits to the falsy operand"
        );
    }

    // --- O2: unknown namespace key + unlisted secret -------------------------------------------

    #[test]
    fn unknown_namespace_and_unknown_key_are_typed_resolution_errors() {
        let data = sample();
        let bad_root = eval_str("nope.x", &data).expect_err("unknown root namespace");
        assert_eq!(
            bad_root.code, "unknown_namespace",
            "unknown root is unknown_namespace"
        );
        assert_eq!(bad_root.category, crate::error::ErrorCategory::Resolution);

        let bad_key = eval_str("inputs.missing", &data).expect_err("missing key");
        assert_eq!(
            bad_key.code, "unknown_namespace",
            "missing key is unknown_namespace"
        );

        // A type mismatch: reading a property of a scalar.
        let mismatch = eval_str("inputs.name.nope", &data).expect_err("property of a string");
        assert_eq!(
            mismatch.code, "type_mismatch",
            "member of a scalar is type_mismatch"
        );
    }

    #[test]
    fn an_unlisted_secret_reference_is_a_resolution_error() {
        // The runner only places listed secrets into scope.secrets, so an unlisted name is simply
        // absent — the interpolator reports it as the distinct `unlisted_secret` failure, never a value.
        let data = sample();
        let err = eval_str("secrets.MISSING", &data).expect_err("unlisted secret must fail");
        assert_eq!(
            err.category,
            crate::error::ErrorCategory::Resolution,
            "resolution category"
        );
        assert_eq!(
            err.code, "unlisted_secret",
            "unlisted secret has its own distinct code"
        );
        // The listed one still resolves, proving the guard is about presence, not a blanket block.
        assert_eq!(eval_str("secrets.TOKEN", &data).unwrap(), json!("s3cr3t"));
    }

    // --- O2: length boundary — one below / at / one above --------------------------------------

    #[test]
    fn expression_length_boundary_below_at_above() {
        let data = sample();
        let max = EXPR_LEN_MAX_BYTES as usize;
        // A string literal of exact byte length L: `"` + (L-2) 'a's + `"`.
        let expr_of_len = |len: usize| {
            let mut s = String::with_capacity(len);
            s.push('"');
            s.extend(std::iter::repeat_n('a', len - 2));
            s.push('"');
            debug_assert_eq!(s.len(), len);
            s
        };

        let below = expr_of_len(max - 1);
        assert_eq!(below.len(), max - 1, "one below the limit");
        assert!(
            eval_str(&below, &data).is_ok(),
            "one below the length limit parses"
        );

        let at = expr_of_len(max);
        assert_eq!(at.len(), max, "exactly at the limit");
        assert!(
            eval_str(&at, &data).is_ok(),
            "at the length limit is still accepted"
        );

        let above = expr_of_len(max + 1);
        assert_eq!(above.len(), max + 1, "one above the limit");
        let err = eval_str(&above, &data).expect_err("over-length must fail");
        assert_eq!(err.code, "expr_too_long", "over-length is expr_too_long");
        assert_eq!(err.category, crate::error::ErrorCategory::Resolution);
    }

    // --- O2: depth boundary — one below / at / one above ---------------------------------------

    #[test]
    fn expression_depth_boundary_below_at_above() {
        let data = sample();
        // AST depth for `("(" * p) 1 (")" * p)` is p + 1 (the top-level call is depth 1, each paren
        // adds one). So p = target_depth - 1.
        let expr_of_depth = |depth: u32| {
            let p = (depth - 1) as usize;
            format!("{}1{}", "(".repeat(p), ")".repeat(p))
        };
        let max = EXPR_DEPTH_MAX;

        let below = expr_of_depth(max - 1);
        assert!(
            eval_str(&below, &data).is_ok(),
            "one below the depth limit parses"
        );

        let at = expr_of_depth(max);
        assert!(
            eval_str(&at, &data).is_ok(),
            "at the depth limit is still accepted"
        );

        let above = expr_of_depth(max + 1);
        let err = eval_str(&above, &data).expect_err("over-deep must fail");
        assert_eq!(err.code, "expr_too_deep", "over-deep is expr_too_deep");
        assert_eq!(err.category, crate::error::ErrorCategory::Resolution);
    }

    #[test]
    fn a_deeply_nested_input_fails_cleanly_without_overflowing_the_stack() {
        // Far past the depth bound but within the length bound: the parser must return expr_too_deep
        // rather than recursing until the stack overflows (the depth guard fires at parse).
        let data = sample();
        let deep = format!("{}1{}", "(".repeat(1000), ")".repeat(1000));
        assert!(
            deep.len() as u64 <= EXPR_LEN_MAX_BYTES,
            "still within the length bound"
        );
        let err = eval_str(&deep, &data).expect_err("must reject, not overflow");
        assert_eq!(err.code, "expr_too_deep", "deep nesting is caught at parse");
    }

    // --- Parser negative space -----------------------------------------------------------------

    #[test]
    fn malformed_expressions_are_parse_errors_not_panics() {
        let data = sample();
        let malformed = [
            "1 ==",    // incomplete
            "1 == 1",  // loose equality is not in the subset
            "inputs.", // dangling dot
            "inputs[", // unterminated index
            "(1",      // unterminated paren
            "1)",      // stray close paren
            "\"abc",   // unterminated string
            "&",       // lone ampersand
            "|",       // lone pipe
            "1 2",     // trailing token
            "@",       // illegal character
            "",        // empty expression
        ];
        for expr in malformed {
            let err = eval_str(expr, &data).expect_err("must be a typed error, not a panic");
            assert_eq!(
                err.category,
                crate::error::ErrorCategory::Resolution,
                "{expr:?} is a resolution error"
            );
        }
    }

    // --- Property tests (hand-rolled, deterministic) -------------------------------------------

    /// A tiny deterministic LCG so the property loops are reproducible across runs.
    struct Lcg(u64);
    impl Lcg {
        fn next_u64(&mut self) -> u64 {
            // Numerical Recipes LCG constants.
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0
        }
        fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
            &xs[(self.next_u64() as usize) % xs.len()]
        }
    }

    #[test]
    fn property_malformed_input_never_panics() {
        // Fuzz random byte strings drawn from an alphabet rich in operators, quotes, and brackets:
        // every call must return Ok or a typed Err, and none may panic (a panic fails the test).
        let data = sample();
        let alphabet: &[u8] =
            b"inputs.env secrets tasks item case output matrix()[]!=&|\"'0123456789 abc.";
        let mut rng = Lcg(0x1234_5678_9abc_def0);
        for _ in 0..4000 {
            let len = (rng.next_u64() % 40) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| *rng.pick(alphabet)).collect();
            // The alphabet is ASCII, so this is always valid UTF-8.
            let src = std::str::from_utf8(&bytes).expect("ascii alphabet is valid utf-8");
            let result = evaluate(src, &data.scope());
            // Whatever the outcome, an error must be a typed resolution error, never a panic.
            if let Err(e) = result {
                assert_eq!(
                    e.category,
                    crate::error::ErrorCategory::Resolution,
                    "fuzzed input {src:?} produced a non-resolution error"
                );
                assert!(!e.code.is_empty(), "every error carries a code");
            }
        }
    }

    #[test]
    fn property_well_formed_literal_expressions_always_evaluate() {
        // Generate well-formed expressions over literals only (no namespace lookups), bounded well
        // under the depth limit, and assert each one parses and evaluates to a value — the positive
        // "round-trips" side of the property.
        let mut rng = Lcg(0xdead_beef_cafe_0001);
        let data = sample();

        fn gen_expr(rng: &mut Lcg, depth: u32) -> String {
            let atoms = ["1", "0", "42", "true", "false", "null", "\"x\"", "\"\""];
            if depth == 0 || rng.next_u64().is_multiple_of(3) {
                return (*rng.pick(&atoms)).to_string();
            }
            match rng.next_u64() % 5 {
                0 => format!("!{}", gen_expr(rng, depth - 1)),
                1 => format!("({})", gen_expr(rng, depth - 1)),
                2 => format!(
                    "{} === {}",
                    gen_expr(rng, depth - 1),
                    gen_expr(rng, depth - 1)
                ),
                3 => format!(
                    "{} && {}",
                    gen_expr(rng, depth - 1),
                    gen_expr(rng, depth - 1)
                ),
                _ => format!(
                    "{} || {}",
                    gen_expr(rng, depth - 1),
                    gen_expr(rng, depth - 1)
                ),
            }
        }

        for _ in 0..2000 {
            let expr = gen_expr(&mut rng, 5);
            let out = evaluate(&expr, &data.scope());
            assert!(
                out.is_ok(),
                "well-formed literal expression {expr:?} must evaluate, got {out:?}"
            );
        }
    }
}
