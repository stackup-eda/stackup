//! The expression language: what a `derive`, an `assert`, a fact's value, a `when` and a text
//! template are written in.
//!
//! Expressions are pure. Arithmetic is over [`Quantity`]s with unit checking, comparisons hold
//! for every value of a range (`a <= b` means the top of `a` is under the bottom of `b`), and the
//! functions are core's. Names are resolved by a [`Scope`], which is where a parameter, a derived
//! value or a port's fact comes from — this module knows only the shape of a path.
//!
//! Every value carries a **trace**: what it was computed from (`from`), for a diagnostic to point
//! at, and what it does not know (`partial`) — a sum with an unstated contribution is a lower
//! bound, and a comparison over one holds only for what is stated.

use std::fmt;

use stackup_kdl::Span;

use crate::quantity::{E3, E6, E12, E24, E96, Quantity};

/// Where a value came from: a statement in some file, for a diagnostic's label.
#[derive(Clone, Debug, PartialEq)]
pub struct Origin {
    pub file: usize,
    pub span: Span,
    pub what: String,
}

/// What a value carries besides itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trace {
    /// What states nothing: a value with any of these is a lower bound.
    pub partial: Vec<String>,
    pub from: Vec<Origin>,
}

impl Trace {
    pub fn merge(mut self, o: &Trace) -> Trace {
        for p in &o.partial {
            if !self.partial.contains(p) {
                self.partial.push(p.clone());
            }
        }
        for f in &o.from {
            if !self.from.contains(f) {
                self.from.push(f.clone());
            }
        }
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Num(Quantity),
    Bool(bool),
    Text(String),
    /// A bare word that named nothing in scope — an enumerated member (`gnd`), or a typo.
    Name(String),
    /// A fact nothing states, and anything computed from one. Carries why.
    Unknown(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Eval {
    pub val: Val,
    pub trace: Trace,
}

impl Eval {
    pub fn new(val: Val) -> Eval {
        Eval {
            val,
            trace: Trace::default(),
        }
    }

    pub fn num(q: Quantity) -> Eval {
        Eval::new(Val::Num(q))
    }

    pub fn text(s: impl Into<String>) -> Eval {
        Eval::new(Val::Text(s.into()))
    }

    pub fn with_origin(mut self, o: Origin) -> Eval {
        if !self.trace.from.contains(&o) {
            self.trace.from.push(o);
        }
        self
    }

    /// From a KDL value as written: a quoted quantity, a number, a name.
    pub fn from_value(v: &stackup_kdl::Value) -> Eval {
        use stackup_kdl::Value as K;
        Eval::new(match v {
            K::String(s) => match Quantity::parse(s) {
                Ok(q) => Val::Num(q),
                Err(_) => Val::Text(s.clone()),
            },
            K::Name(n) => Val::Name(n.clone()),
            K::Integer(i) => Val::Num(Quantity::plain(*i as f64)),
            K::Float(f) => Val::Num(Quantity::plain(*f)),
            K::Bool(b) => Val::Bool(*b),
            K::Null => Val::Text(String::new()),
        })
    }

    pub fn as_bool(&self) -> Option<bool> {
        match &self.val {
            Val::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_num(&self) -> Option<Quantity> {
        match &self.val {
            Val::Num(q) => Some(*q),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match &self.val {
            Val::Text(s) | Val::Name(s) => Some(s),
            _ => None,
        }
    }
}

impl fmt::Display for Val {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Val::Num(q) => write!(f, "{q}"),
            Val::Bool(b) => write!(f, "{b}"),
            Val::Text(s) | Val::Name(s) => f.write_str(s),
            Val::Unknown(_) => f.write_str("unknown"),
        }
    }
}

/// What names mean while an expression is evaluated.
pub trait Scope {
    /// A feature's state, if `name` is one (for `when`).
    fn feature(&mut self, _name: &str) -> Option<bool> {
        None
    }
    /// A dotted path: a parameter, a derived value, `port.fact`, `port.line.fact`. `Ok(None)`
    /// means the path names nothing; `Err` means it names something that could not be evaluated.
    fn lookup(&mut self, path: &[String], span: Span) -> Result<Option<Eval>, String>;
}

// --- Lexing -------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Num(String),
    Ident(String),
    Str(String),
    Op(&'static str),
    Open,
    Close,
    Comma,
    Colon,
}

fn lex(src: &str) -> Result<Vec<(Token, usize)>, String> {
    let mut out = Vec::new();
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (at, c) = chars[i];
        let two = |i: usize, s: &str| -> bool {
            let mut it = s.chars();
            chars.get(i).map(|c| c.1) == it.next() && chars.get(i + 1).map(|c| c.1) == it.next()
        };
        match c {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '(' => {
                out.push((Token::Open, at));
                i += 1;
            }
            ')' => {
                out.push((Token::Close, at));
                i += 1;
            }
            ',' => {
                out.push((Token::Comma, at));
                i += 1;
            }
            ':' => {
                out.push((Token::Colon, at));
                i += 1;
            }
            '"' => {
                let start = i + 1;
                let mut j = start;
                while j < chars.len() && chars[j].1 != '"' {
                    j += 1;
                }
                out.push((
                    Token::Str(chars[start..j].iter().map(|c| c.1).collect()),
                    at,
                ));
                i = j + 1;
            }
            _ if two(i, "<=") => {
                out.push((Token::Op("<="), at));
                i += 2;
            }
            _ if two(i, ">=") => {
                out.push((Token::Op(">="), at));
                i += 2;
            }
            _ if two(i, "==") => {
                out.push((Token::Op("=="), at));
                i += 2;
            }
            _ if two(i, "!=") => {
                out.push((Token::Op("!="), at));
                i += 2;
            }
            _ if two(i, "&&") => {
                out.push((Token::Op("&&"), at));
                i += 2;
            }
            _ if two(i, "||") => {
                out.push((Token::Op("||"), at));
                i += 2;
            }
            '<' | '>' | '+' | '-' | '*' | '/' | '!' => {
                out.push((
                    Token::Op(match c {
                        '<' => "<",
                        '>' => ">",
                        '+' => "+",
                        '-' => "-",
                        '*' => "*",
                        '/' => "/",
                        _ => "!",
                    }),
                    at,
                ));
                i += 1;
            }
            c if c.is_ascii_digit() => {
                let start = i;
                let mut j = i + 1;
                while j < chars.len() {
                    let d = chars[j].1;
                    let numeric_so_far = chars[start..j]
                        .iter()
                        .all(|c| c.1.is_ascii_digit() || matches!(c.1, '.' | 'e' | 'E' | 'x'));
                    let ok = d.is_alphanumeric()
                        || matches!(d, '.' | 'µ' | 'Ω' | '%')
                        || (matches!(d, '-' | '+')
                            && numeric_so_far
                            && matches!(chars[j - 1].1, 'e' | 'E')
                            && !src.starts_with("0x"));
                    if !ok {
                        break;
                    }
                    j += 1;
                }
                out.push((
                    Token::Num(chars[start..j].iter().map(|c| c.1).collect()),
                    at,
                ));
                i = j;
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                let mut j = i + 1;
                while j < chars.len() {
                    let d = chars[j].1;
                    let ok = d.is_alphanumeric()
                        || matches!(d, '_' | '.')
                        || (d == '-'
                            && chars
                                .get(j + 1)
                                .is_some_and(|n| n.1.is_alphabetic() || n.1 == '_'));
                    if !ok {
                        break;
                    }
                    j += 1;
                }
                out.push((
                    Token::Ident(chars[start..j].iter().map(|c| c.1).collect()),
                    at,
                ));
                i = j;
            }
            c => return Err(format!("unexpected `{c}` in an expression")),
        }
    }
    Ok(out)
}

// --- Parsing ------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Num(String, usize),
    Str(String),
    Path(Vec<String>, usize),
    Call(String, Vec<Arg>, usize),
    Unary(&'static str, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
struct Arg {
    key: Option<String>,
    value: Expr,
}

struct Parser {
    tokens: Vec<(Token, usize)>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at).map(|t| &t.0)
    }

    fn next(&mut self) -> Option<(Token, usize)> {
        let t = self.tokens.get(self.at).cloned();
        self.at += 1;
        t
    }

    fn eat(&mut self, op: &str) -> bool {
        if self.peek()
            == Some(&Token::Op(match op {
                "||" => "||",
                "&&" => "&&",
                "==" => "==",
                "!=" => "!=",
                "<=" => "<=",
                ">=" => ">=",
                "<" => "<",
                ">" => ">",
                "+" => "+",
                "-" => "-",
                "*" => "*",
                "/" => "/",
                _ => "!",
            }))
        {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn or(&mut self) -> Result<Expr, String> {
        let mut left = self.and()?;
        while self.eat("||") {
            let right = self.and()?;
            left = Expr::Binary("||", Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr, String> {
        let mut left = self.compare()?;
        while self.eat("&&") {
            let right = self.compare()?;
            left = Expr::Binary("&&", Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn compare(&mut self) -> Result<Expr, String> {
        let left = self.additive()?;
        for op in ["==", "!=", "<=", ">=", "<", ">"] {
            if self.eat(op) {
                let right = self.additive()?;
                return Ok(Expr::Binary(op_static(op), Box::new(left), Box::new(right)));
            }
        }
        Ok(left)
    }

    fn additive(&mut self) -> Result<Expr, String> {
        let mut left = self.multiplicative()?;
        loop {
            if self.eat("+") {
                let right = self.multiplicative()?;
                left = Expr::Binary("+", Box::new(left), Box::new(right));
            } else if self.eat("-") {
                let right = self.multiplicative()?;
                left = Expr::Binary("-", Box::new(left), Box::new(right));
            } else {
                return Ok(left);
            }
        }
    }

    fn multiplicative(&mut self) -> Result<Expr, String> {
        let mut left = self.unary()?;
        loop {
            if self.eat("*") {
                let right = self.unary()?;
                left = Expr::Binary("*", Box::new(left), Box::new(right));
            } else if self.eat("/") {
                let right = self.unary()?;
                left = Expr::Binary("/", Box::new(left), Box::new(right));
            } else {
                return Ok(left);
            }
        }
    }

    fn unary(&mut self) -> Result<Expr, String> {
        if self.eat("!") {
            return Ok(Expr::Unary("!", Box::new(self.unary()?)));
        }
        if self.eat("-") {
            return Ok(Expr::Unary("-", Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, String> {
        match self.next() {
            Some((Token::Num(n), at)) => Ok(Expr::Num(n, at)),
            Some((Token::Str(s), _)) => Ok(Expr::Str(s)),
            Some((Token::Open, _)) => {
                let e = self.or()?;
                match self.next() {
                    Some((Token::Close, _)) => Ok(e),
                    _ => Err("expected `)`".into()),
                }
            }
            Some((Token::Ident(name), at)) => {
                if self.peek() == Some(&Token::Open) {
                    self.at += 1;
                    let mut args = Vec::new();
                    if self.peek() != Some(&Token::Close) {
                        loop {
                            // `key: value` or a plain expression.
                            let key = match (self.peek(), self.tokens.get(self.at + 1)) {
                                (Some(Token::Ident(k)), Some((Token::Colon, _))) => {
                                    let k = k.clone();
                                    self.at += 2;
                                    Some(k)
                                }
                                _ => None,
                            };
                            let value = self.or()?;
                            args.push(Arg { key, value });
                            match self.next() {
                                Some((Token::Comma, _)) => continue,
                                Some((Token::Close, _)) => break,
                                _ => return Err(format!("expected `,` or `)` in `{name}(…)`")),
                            }
                        }
                    } else {
                        self.at += 1;
                    }
                    return Ok(Expr::Call(name, args, at));
                }
                Ok(Expr::Path(
                    name.split('.').map(str::to_string).collect(),
                    at,
                ))
            }
            Some((t, _)) => Err(format!("unexpected {}", describe(&t))),
            None => Err("the expression ends early".into()),
        }
    }
}

fn op_static(op: &str) -> &'static str {
    match op {
        "==" => "==",
        "!=" => "!=",
        "<=" => "<=",
        ">=" => ">=",
        "<" => "<",
        _ => ">",
    }
}

fn describe(t: &Token) -> String {
    match t {
        Token::Num(n) => format!("`{n}`"),
        Token::Ident(i) => format!("`{i}`"),
        Token::Str(s) => format!("`\"{s}\"`"),
        Token::Op(o) => format!("`{o}`"),
        Token::Open => "`(`".into(),
        Token::Close => "`)`".into(),
        Token::Comma => "`,`".into(),
        Token::Colon => "`:`".into(),
    }
}

fn parse(src: &str) -> Result<Expr, String> {
    let tokens = lex(src)?;
    let mut p = Parser { tokens, at: 0 };
    let e = p.or()?;
    if p.at != p.tokens.len() {
        return Err(format!(
            "unexpected {} after the expression",
            describe(&p.tokens[p.at].0)
        ));
    }
    Ok(e)
}

// --- Evaluation ---------------------------------------------------------------------------------

/// Evaluates `src` with `scope` resolving its names. `span` is where the expression was written,
/// for the trace of what it produces.
pub fn eval(src: &str, scope: &mut dyn Scope, span: Span) -> Result<Eval, String> {
    let e = parse(src)?;
    let mut ev = Evaluator { scope, span };
    ev.eval(&e)
}

struct Evaluator<'a> {
    scope: &'a mut dyn Scope,
    span: Span,
}

impl Evaluator<'_> {
    fn eval(&mut self, e: &Expr) -> Result<Eval, String> {
        match e {
            Expr::Num(n, _) => Ok(Eval::num(Quantity::parse(n)?)),
            Expr::Str(s) => Ok(match Quantity::parse(s) {
                Ok(q) => Eval::num(q),
                Err(_) => Eval::text(s.clone()),
            }),
            Expr::Path(path, _) => self.path(path),
            Expr::Call(name, args, _) => self.call(name, args),
            Expr::Unary(op, x) => {
                let v = self.eval(x)?;
                if let Val::Unknown(_) = v.val {
                    return Ok(v);
                }
                match (*op, &v.val) {
                    ("!", Val::Bool(b)) => Ok(Eval {
                        val: Val::Bool(!b),
                        trace: v.trace,
                    }),
                    ("-", Val::Num(q)) => Ok(Eval {
                        val: Val::Num(q.neg()),
                        trace: v.trace,
                    }),
                    ("!", _) => Err(format!("`!` needs a condition, not {}", kind(&v.val))),
                    _ => Err(format!("`-` needs a quantity, not {}", kind(&v.val))),
                }
            }
            Expr::Binary(op, a, b) => {
                let a = self.eval(a)?;
                let b = self.eval(b)?;
                binary(op, a, b)
            }
        }
    }

    fn path(&mut self, path: &[String]) -> Result<Eval, String> {
        if path.len() == 1 {
            if let Some(on) = self.scope.feature(&path[0]) {
                return Ok(Eval::new(Val::Bool(on)));
            }
            match path[0].as_str() {
                "ln2" => return Ok(Eval::num(Quantity::plain(std::f64::consts::LN_2))),
                "pi" => return Ok(Eval::num(Quantity::plain(std::f64::consts::PI))),
                "true" => return Ok(Eval::new(Val::Bool(true))),
                "false" => return Ok(Eval::new(Val::Bool(false))),
                _ => {}
            }
        }
        if let Some(v) = self.scope.lookup(path, self.span)? {
            return Ok(v);
        }
        // `x.min` / `x.max`: an end of whatever `x` is.
        if path.len() >= 2
            && let end @ ("min" | "max") = path[path.len() - 1].as_str()
            && let Some(v) = self.scope.lookup(&path[..path.len() - 1], self.span)?
        {
            return match v.val {
                Val::Num(q) => Ok(Eval {
                    val: Val::Num(if end == "min" { q.min() } else { q.max() }),
                    trace: v.trace,
                }),
                Val::Unknown(_) => Ok(v),
                other => Err(format!(
                    "`.{end}` reads an end of a quantity, and `{}` is {}",
                    path[..path.len() - 1].join("."),
                    kind(&other)
                )),
            };
        }
        if path.len() == 1 {
            // A bare word naming nothing: an enumerated member, or a mistake the comparison
            // will report.
            return Ok(Eval::new(Val::Name(path[0].clone())));
        }
        Err(format!("`{}` is not a name in scope", path.join(".")))
    }

    fn call(&mut self, name: &str, args: &[Arg]) -> Result<Eval, String> {
        // An unknown argument makes the result unknown, whatever the function.
        let evaluated: Vec<Eval> = args
            .iter()
            .map(|a| self.eval(&a.value))
            .collect::<Result<_, _>>()?;
        if let Some(u) = evaluated.iter().find(|v| matches!(v.val, Val::Unknown(_))) {
            let trace = evaluated
                .iter()
                .fold(Trace::default(), |t, v| t.merge(&v.trace));
            return Ok(Eval {
                val: u.val.clone(),
                trace,
            });
        }
        let plain = |args: &[Arg]| -> Result<(), String> {
            match args.iter().find(|a| a.key.is_some()) {
                Some(a) => Err(format!(
                    "`{name}` takes no `{}:` argument",
                    a.key.clone().unwrap()
                )),
                None => Ok(()),
            }
        };
        match name {
            "clamp" => {
                plain(args)?;
                if args.len() != 3 {
                    return Err("`clamp` takes `x, lo, hi`".into());
                }
                let x = self.eval(&args[0].value)?;
                let lo = self.eval(&args[1].value)?;
                let hi = self.eval(&args[2].value)?;
                let (xq, lq, hq) = (
                    need_num("clamp", &x)?,
                    need_num("clamp", &lo)?,
                    need_num("clamp", &hi)?,
                );
                if xq.unit != lq.unit || xq.unit != hq.unit {
                    return Err("`clamp` needs its three arguments in one unit".into());
                }
                let trace = x.trace.merge(&lo.trace).merge(&hi.trace);
                Ok(Eval {
                    val: Val::Num(Quantity::range(
                        xq.lo.clamp(lq.lo, hq.hi),
                        xq.hi.clamp(lq.lo, hq.hi),
                        xq.unit,
                    )),
                    trace,
                })
            }
            "e3" | "e6" | "e12" | "e24" | "e96" => {
                plain(args)?;
                if args.len() != 1 {
                    return Err(format!("`{name}` takes one argument"));
                }
                let x = self.eval(&args[0].value)?;
                let q = need_num(name, &x)?;
                let series: &[f64] = match name {
                    "e3" => &E3,
                    "e6" => &E6,
                    "e12" => &E12,
                    "e24" => &E24,
                    _ => &E96,
                };
                Ok(Eval {
                    val: Val::Num(q.e_series(series)),
                    trace: x.trace,
                })
            }
            "min" | "max" => {
                plain(args)?;
                if args.len() != 2 {
                    return Err(format!("`{name}` takes two arguments"));
                }
                let a = self.eval(&args[0].value)?;
                let b = self.eval(&args[1].value)?;
                let (aq, bq) = (need_num(name, &a)?, need_num(name, &b)?);
                if aq.unit != bq.unit {
                    return Err(format!("`{name}` needs both arguments in one unit"));
                }
                let q = if name == "min" {
                    Quantity::range(aq.lo.min(bq.lo), aq.hi.min(bq.hi), aq.unit)
                } else {
                    Quantity::range(aq.lo.max(bq.lo), aq.hi.max(bq.hi), aq.unit)
                };
                Ok(Eval {
                    val: Val::Num(q),
                    trace: a.trace.merge(&b.trace),
                })
            }
            "abs" => {
                plain(args)?;
                if args.len() != 1 {
                    return Err("`abs` takes one argument".into());
                }
                let x = self.eval(&args[0].value)?;
                let q = need_num("abs", &x)?;
                let (lo, hi) = if q.lo <= 0.0 && q.hi >= 0.0 {
                    (0.0, q.lo.abs().max(q.hi.abs()))
                } else {
                    (q.lo.abs().min(q.hi.abs()), q.lo.abs().max(q.hi.abs()))
                };
                Ok(Eval {
                    val: Val::Num(Quantity::range(lo, hi, q.unit)),
                    trace: x.trace,
                })
            }
            "match" => {
                let Some(first) = args.first() else {
                    return Err("`match` takes a key and then `member: value` pairs".into());
                };
                if first.key.is_some() {
                    return Err("`match`'s first argument is the key to match".into());
                }
                let k = self.eval(&first.value)?;
                let key = match &k.val {
                    Val::Text(s) | Val::Name(s) => s.clone(),
                    Val::Num(q) => q.to_string(),
                    Val::Bool(b) => b.to_string(),
                    Val::Unknown(_) => unreachable!("unknown arguments return early"),
                };
                for a in &args[1..] {
                    let Some(member) = &a.key else {
                        return Err("`match` arms are `member: value`".into());
                    };
                    if *member == key {
                        let v = self.eval(&a.value)?;
                        return Ok(Eval {
                            val: v.val,
                            trace: v.trace.merge(&k.trace),
                        });
                    }
                }
                Err(format!(
                    "`match` has no arm for `{key}`; it has {}",
                    args[1..]
                        .iter()
                        .filter_map(|a| a.key.as_ref().map(|k| format!("`{k}`")))
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
            _ => Err(format!("`{name}` is not a function stackup has")),
        }
    }
}

fn kind(v: &Val) -> String {
    match v {
        Val::Num(q) => format!("the quantity {q}"),
        Val::Bool(_) => "a condition".into(),
        Val::Text(s) => format!("the text \"{s}\""),
        Val::Name(n) => format!("`{n}`, which is not a name in scope"),
        Val::Unknown(why) => format!("unknown ({why})"),
    }
}

fn need_num(what: &str, v: &Eval) -> Result<Quantity, String> {
    match &v.val {
        Val::Num(q) => Ok(*q),
        other => Err(format!("`{what}` needs a quantity, not {}", kind(other))),
    }
}

fn binary(op: &str, a: Eval, b: Eval) -> Result<Eval, String> {
    let trace = a.trace.clone().merge(&b.trace);
    // Unknown is absorbing, except where the other side decides alone.
    match (op, &a.val, &b.val) {
        ("&&", Val::Bool(false), _) | ("&&", _, Val::Bool(false)) => {
            return Ok(Eval {
                val: Val::Bool(false),
                trace,
            });
        }
        ("||", Val::Bool(true), _) | ("||", _, Val::Bool(true)) => {
            return Ok(Eval {
                val: Val::Bool(true),
                trace,
            });
        }
        (_, Val::Unknown(_), _) => {
            return Ok(Eval { val: a.val, trace });
        }
        (_, _, Val::Unknown(_)) => {
            return Ok(Eval { val: b.val, trace });
        }
        _ => {}
    }
    let val = match (op, &a.val, &b.val) {
        ("&&", Val::Bool(x), Val::Bool(y)) => Val::Bool(*x && *y),
        ("||", Val::Bool(x), Val::Bool(y)) => Val::Bool(*x || *y),
        ("&&" | "||", x, y) => {
            let bad = if matches!(x, Val::Bool(_)) { y } else { x };
            return Err(format!("`{op}` needs conditions, not {}", kind(bad)));
        }
        ("+", Val::Num(x), Val::Num(y)) => Val::Num(x.add(*y)?),
        ("-", Val::Num(x), Val::Num(y)) => Val::Num(x.sub(*y)?),
        ("*", Val::Num(x), Val::Num(y)) => Val::Num(x.mul(*y)),
        ("/", Val::Num(x), Val::Num(y)) => Val::Num(x.div(*y)?),
        ("+" | "-" | "*" | "/", x, y) => {
            let bad = if matches!(x, Val::Num(_)) { y } else { x };
            return Err(format!("`{op}` needs quantities, not {}", kind(bad)));
        }
        ("==" | "!=" | "<" | "<=" | ">" | ">=", Val::Num(x), Val::Num(y)) => {
            if x.unit != y.unit {
                return Err(format!("cannot compare {x} with {y}: the units differ"));
            }
            // Over ranges, a comparison holds when it holds for every value.
            let r = match op {
                "==" => x.same(y),
                "!=" => !x.same(y),
                "<" => x.hi < y.lo,
                "<=" => x.hi <= y.lo,
                ">" => x.lo > y.hi,
                _ => x.lo >= y.hi,
            };
            Val::Bool(r)
        }
        ("==", Val::Bool(x), Val::Bool(y)) => Val::Bool(x == y),
        ("!=", Val::Bool(x), Val::Bool(y)) => Val::Bool(x != y),
        ("==", Val::Text(x) | Val::Name(x), Val::Text(y) | Val::Name(y)) => Val::Bool(x == y),
        ("!=", Val::Text(x) | Val::Name(x), Val::Text(y) | Val::Name(y)) => Val::Bool(x != y),
        (_, x, y) => {
            let bad = match (x, y) {
                (Val::Name(_), _) => x,
                (_, Val::Name(_)) => y,
                _ => y,
            };
            return Err(format!(
                "cannot compare {} with {}",
                kind(x),
                if std::ptr::eq(bad, y) {
                    kind(y)
                } else {
                    kind(x)
                }
            ));
        }
    };
    Ok(Eval { val, trace })
}

/// Evaluates a `when` condition: an expression that must come out as a condition.
pub fn when(src: &str, scope: &mut dyn Scope) -> Result<bool, String> {
    let v = eval(src, scope, Span::default())?;
    match v.val {
        Val::Bool(b) => Ok(b),
        Val::Unknown(why) => Err(format!("the condition cannot be decided: {why}")),
        other => Err(format!(
            "a `when` condition must be true or false, and this is {}",
            kind(&other)
        )),
    }
}

/// Renders a template: `{expr}` substitutes a value, `{expr:.0%}` formats it as a percentage
/// with that many decimals, `{expr:.2}` to that many decimals. What cannot be evaluated is left
/// as written, between its braces, so a note never loses its words.
pub fn template(src: &str, scope: &mut dyn Scope, span: Span) -> (String, Trace, Vec<String>) {
    let mut out = String::new();
    let mut trace = Trace::default();
    let mut errors = Vec::new();
    let mut rest = src;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('}') else {
            out.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let inner = &rest[open + 1..open + close];
        let (expr, fmt) = match inner.rsplit_once(':') {
            Some((e, f)) if f.starts_with('.') => (e, Some(f)),
            _ => (inner, None),
        };
        match eval(expr, scope, span) {
            Ok(v) => {
                trace = trace.merge(&v.trace);
                out.push_str(&format_value(&v.val, fmt));
            }
            Err(e) => {
                errors.push(format!("in `{{{inner}}}`: {e}"));
                out.push_str(&rest[open..open + close + 1]);
            }
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    (out, trace, errors)
}

fn format_value(v: &Val, fmt: Option<&str>) -> String {
    match (v, fmt) {
        (Val::Num(q), Some(f)) => {
            let percent = f.ends_with('%');
            let digits: usize = f
                .trim_start_matches('.')
                .trim_end_matches('%')
                .parse()
                .unwrap_or(0);
            let one = |x: f64| {
                if percent {
                    format!("{:.*}%", digits, x * 100.0)
                } else {
                    format!("{:.*}", digits, x)
                }
            };
            if q.is_point() {
                one(q.lo)
            } else {
                format!("{}–{}", one(q.lo), one(q.hi))
            }
        }
        (v, _) => v.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct S(HashMap<String, Eval>);

    impl Scope for S {
        fn feature(&mut self, name: &str) -> Option<bool> {
            match name {
                "emc-class-b" => Some(true),
                "other" => Some(false),
                _ => None,
            }
        }
        fn lookup(&mut self, path: &[String], _: Span) -> Result<Option<Eval>, String> {
            Ok(self.0.get(&path.join(".")).cloned())
        }
    }

    fn scope() -> S {
        let mut m = HashMap::new();
        let q = |s: &str| Eval::num(Quantity::parse(s).unwrap());
        m.insert("add0".into(), Eval::new(Val::Name("gnd".into())));
        m.insert("package".into(), Eval::new(Val::Name("soic8".into())));
        m.insert("ballast".into(), q("4.3kΩ"));
        m.insert("forward".into(), q("2V"));
        m.insert("limit".into(), q("4mA"));
        m.insert(
            "rail.voltage".into(),
            Eval::num(Quantity::range(12.0, 18.0, crate::quantity::Unit::VOLT)),
        );
        m.insert("freq".into(), q("2Hz"));
        m.insert("duty".into(), q("0.5"));
        S(m)
    }

    #[test]
    fn conditions() {
        let s = &mut scope();
        assert_eq!(when("emc-class-b", s), Ok(true));
        assert_eq!(when("!emc-class-b", s), Ok(false));
        assert_eq!(when("other || emc-class-b", s), Ok(true));
        assert_eq!(when("add0 == gnd", s), Ok(true));
        assert_eq!(when("add0 == vplus", s), Ok(false));
        assert_eq!(when("add0 != vplus && package == soic8", s), Ok(true));
        assert_eq!(when("!(other || add0 == gnd)", s), Ok(false));
        assert!(when("add0", s).is_err());
        assert!(when("emc-class-b &&", s).is_err());
    }

    #[test]
    fn arithmetic_with_units_and_ranges() {
        let s = &mut scope();
        let sp = Span::default();
        let v = eval("(rail.voltage.max - forward) / ballast", s, sp).unwrap();
        assert_eq!(v.val.to_string(), "3.72mA");
        let v = eval("rail.voltage - forward", s, sp).unwrap();
        assert_eq!(v.val.to_string(), "10V–16V");
        assert_eq!(
            eval("(rail.voltage.max - forward) / ballast <= limit", s, sp)
                .unwrap()
                .as_bool(),
            Some(true)
        );
        assert!(eval("rail.voltage + limit", s, sp).is_err());
        assert!(eval("rail.voltage <= limit", s, sp).is_err());
        assert!(
            eval("curent <= limit", s, sp)
                .unwrap_err()
                .contains("not a name")
        );
    }

    #[test]
    fn functions() {
        let s = &mut scope();
        let sp = Span::default();
        assert_eq!(
            eval("match(add0, gnd: 0x48, vplus: 0x49)", s, sp)
                .unwrap()
                .val
                .to_string(),
            "72"
        );
        assert!(eval("match(add0, vplus: 0x49)", s, sp).is_err());
        assert_eq!(
            eval("clamp((2 * duty - 1) / (1 - duty), 0.05, 50)", s, sp)
                .unwrap()
                .val
                .to_string(),
            "0.05"
        );
        let c = eval(
            "clamp(e3(1 / (ln2 * freq * (0.05 + 2) * 100kΩ)), 1nF, 10µF)",
            s,
            sp,
        )
        .unwrap();
        assert_eq!(c.val.to_string(), "4.7µF");
        assert_eq!(eval("e24(4400Ω)", s, sp).unwrap().val.to_string(), "4.3kΩ");
        assert_eq!(eval("e96(4400Ω)", s, sp).unwrap().val.to_string(), "4.42kΩ");
        assert_eq!(eval("max(1V, 2V)", s, sp).unwrap().val.to_string(), "2V");
    }

    #[test]
    fn templates() {
        let s = &mut scope();
        let (t, _, errs) = template(
            "{ballast} lets {(rail.voltage.max - forward) / ballast} through at {rail.voltage.max}; {duty:.0%}",
            s,
            Span::default(),
        );
        assert_eq!(t, "4.3kΩ lets 3.72mA through at 18V; 50%");
        assert!(errs.is_empty());
        let (t, _, errs) = template("{nope.x} stays", s, Span::default());
        assert_eq!(t, "{nope.x} stays");
        assert_eq!(errs.len(), 1);
    }
}
