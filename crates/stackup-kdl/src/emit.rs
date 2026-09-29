//! The typed model → KDL text.
//!
//! The writer is its own, rather than the `kdl` crate's printer, for one reason: that printer
//! writes any string bare when it can, and stackup gives the difference between `"ra"` and `ra`
//! meaning. Everything is written the way the reader reads it, so what this emits reads back as
//! what was emitted.

use std::fmt::Write as _;

use crate::{ast::*, value::Property};

/// Writes a file as KDL text, four spaces per level, one blank line between top-level items.
pub fn emit(file: &File) -> String {
    let mut w = W {
        out: String::new(),
        depth: 0,
    };
    w.file(file);
    w.out
}

/// Whether `s` can be written as a bare identifier.
///
/// KDL v2's rules: no whitespace, no `\ / ( ) { } ; [ ] " # =`, and it may not start with a digit
/// or look like a number, a keyword or a sign.
pub fn is_bare(s: &str) -> bool {
    if s.is_empty() || matches!(s, "true" | "false" | "null" | "inf" | "-inf" | "nan") {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if first.is_ascii_digit() {
        return false;
    }
    if matches!(first, '.' | '+' | '-') {
        match chars.next() {
            None => return first != '.',
            Some(c) if c.is_ascii_digit() => return false,
            Some('.') if first != '.' => return false,
            _ => {}
        }
    }
    !s.chars().any(|c| {
        c.is_whitespace()
            || c.is_control()
            || matches!(
                c,
                '\\' | '/' | '(' | ')' | '{' | '}' | ';' | '[' | ']' | '"' | '#' | '='
            )
    })
}

/// `s` as a quoted KDL string.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A name in name position: bare when it can be, quoted otherwise (`"Cvdd/1"`). A name that is a
/// number is written as one (`pin 1`), which the reader takes as the same name.
pub fn ident(s: &str) -> String {
    if is_bare(s) || s.parse::<i128>().is_ok_and(|_| !s.starts_with(['+', '-'])) {
        s.to_string()
    } else {
        quote(s)
    }
}

/// A property key: bare when it can be, and never a number.
fn key(s: &str) -> String {
    if is_bare(s) { s.to_string() } else { quote(s) }
}

fn props(head: &mut String, props: &[Property]) {
    for p in props {
        let _ = write!(head, " {}={}", key(&p.key), p.value.repr());
    }
}

struct W {
    out: String,
    depth: usize,
}

impl W {
    fn line(&mut self, s: &str) {
        for _ in 0..self.depth {
            self.out.push_str("    ");
        }
        self.out.push_str(s);
        self.out.push('\n');
    }

    /// Writes `head` alone, or `head {` … `}` around `body` when `open` says there is one.
    fn stmt(&mut self, head: String, open: bool, body: impl FnOnce(&mut W)) {
        if open {
            self.line(&format!("{head} {{"));
            self.depth += 1;
            body(self);
            self.depth -= 1;
            self.line("}");
        } else {
            self.line(&head);
        }
    }

    fn file(&mut self, file: &File) {
        for (i, item) in file.items.iter().enumerate() {
            if i > 0 {
                self.out.push('\n');
            }
            match item {
                Item::Use(u) => self.line(&format!("use {}", quote(&u.path))),
                Item::Part(p) => self.part(p),
                Item::Mpn(m) => {
                    let mut head = format!("mpn {}", ident(&m.name));
                    props(&mut head, &m.props);
                    self.stmt(head, !m.catalog.is_empty(), |w| {
                        if !m.catalog.is_empty() {
                            let mut catalog = "catalog".to_string();
                            props(&mut catalog, &m.catalog);
                            w.line(&catalog);
                        }
                    });
                }
                Item::Block(b) => self.block(b),
                Item::Type(t) => self.type_decl(t),
            }
        }
    }

    // --- Parts --------------------------------------------------------------------------------------

    fn part(&mut self, p: &Part) {
        self.stmt(format!("part {}", ident(&p.name)), true, |w| {
            for item in &p.items {
                match item {
                    PartItem::Meta(m) => {
                        w.line(&format!("{} {}", m.key.statement(), m.value.repr()))
                    }
                    PartItem::Order(o) => {
                        let mut head = "order".to_string();
                        props(&mut head, &o.props);
                        w.line(&head);
                    }
                    PartItem::Param(p) => w.param(p),
                    PartItem::Pin(p) => w.pin(p),
                    PartItem::Package(p) => w.package(p),
                    PartItem::Port(p) => w.port(p),
                    PartItem::Peripheral(p) => w.peripheral(p),
                    PartItem::Derive(d) => w.derive(d),
                    PartItem::Text(t) => w.text(t),
                    PartItem::Assert(a) => w.assert(a),
                    PartItem::Block(b) => w.block(b),
                }
            }
        });
    }

    fn param(&mut self, p: &Param) {
        let mut head = format!("param {} {}", ident(&p.name), ident(&p.ty));
        if let Some(d) = &p.default {
            let _ = write!(head, " default={}", d.repr());
        }
        self.line(&head);
    }

    fn pin(&mut self, p: &Pin) {
        let mut head = format!("pin {} {}", ident(&p.name), ident(&p.kind));
        if p.required {
            head.push_str(" required");
        }
        if p.once {
            head.push_str(" once");
        }
        if let Some(s) = &p.symbol {
            let _ = write!(head, " symbol={}", quote(s));
        }
        if let Some(k) = &p.symbol_kind {
            let _ = write!(head, " symbol-kind={}", ident(k));
        }
        let body = !p.also.is_empty() || !p.roles.is_empty() || !p.requires.is_empty();
        self.stmt(head, body, |w| {
            for r in &p.roles {
                let mut head = format!("role {}", ident(&r.role));
                props(&mut head, &r.props);
                w.line(&head);
            }
            for a in &p.also {
                w.line(&format!("also {}", ident(&a.name)));
            }
            for r in &p.requires {
                w.require(r);
            }
        });
    }

    fn package(&mut self, p: &Package) {
        let mut head = format!("package {}", ident(&p.name));
        if let Some(f) = &p.footprint {
            let _ = write!(head, " footprint={}", quote(f));
        }
        self.stmt(head, !p.items.is_empty(), |w| {
            for item in &p.items {
                match item {
                    PackageItem::Meta(m) => {
                        w.line(&format!("{} {}", m.key.statement(), m.value.repr()))
                    }
                    PackageItem::Order(o) => {
                        let mut head = "order".to_string();
                        props(&mut head, &o.props);
                        w.line(&head);
                    }
                    PackageItem::Pad(pad) => {
                        let label = if pad.label.parse::<i128>().is_ok() {
                            pad.label.clone()
                        } else {
                            ident(&pad.label)
                        };
                        w.line(&format!("pad {} {label}", ident(&pad.pin)));
                    }
                }
            }
        });
    }

    fn port(&mut self, p: &Port) {
        let mut head = format!("port {}", ident(&p.name));
        if let Some(t) = &p.ty {
            let _ = write!(head, " type={}", ident(t));
        }
        if let Some(pin) = &p.pin {
            let _ = write!(head, " pin={}", ident(pin));
        }
        self.stmt(head, !p.items.is_empty(), |w| {
            for item in &p.items {
                match item {
                    PortItem::Line(l) => w.line_(l),
                    PortItem::Fact(f) => w.fact(f),
                    PortItem::Require(r) => w.require(r),
                    PortItem::Assert(a) => w.assert(a),
                }
            }
        });
    }

    fn line_(&mut self, l: &Line) {
        let mut head = format!("line {}", ident(&l.name));
        if l.optional {
            head.push_str(" optional");
        }
        if let Some(t) = &l.ty {
            let _ = write!(head, " type={}", ident(t));
        }
        if let Some(pin) = &l.pin {
            let _ = write!(head, " pin={}", ident(pin));
        }
        self.stmt(head, !l.items.is_empty(), |w| {
            for item in &l.items {
                match item {
                    LineItem::Require(r) => w.require(r),
                    LineItem::Fact(f) => w.fact(f),
                    LineItem::Match(m) => {
                        w.line(&format!("match {}", ident(&m.target.to_string())))
                    }
                }
            }
        });
    }

    fn fact(&mut self, f: &Fact) {
        let mut head = format!(
            "{} {}.{}",
            f.rule.statement(),
            f.aspect.name(),
            ident(&f.fact)
        );
        if let Some(v) = &f.value {
            let _ = write!(head, " {}", v.repr());
        }
        props(&mut head, &f.props);
        self.line(&head);
    }

    fn terminal_fact(&mut self, t: &TerminalFact) {
        let f = &t.fact;
        let mut head = format!(
            "{} {} {}.{}",
            f.rule.statement(),
            ident(&t.terminal.to_string()),
            f.aspect.name(),
            ident(&f.fact)
        );
        if let Some(v) = &f.value {
            let _ = write!(head, " {}", v.repr());
        }
        props(&mut head, &f.props);
        self.line(&head);
    }

    fn require(&mut self, r: &Require) {
        let mut head = format!("require {}.{}", r.aspect.name(), ident(&r.fact));
        if let Some(v) = &r.value {
            let _ = write!(head, " {}", v.repr());
        }
        props(&mut head, &r.props);
        self.line(&head);
    }

    fn assert(&mut self, a: &Assert) {
        let mut head = format!("assert {}", quote(&a.expr));
        if let Some(m) = &a.message {
            let _ = write!(head, " message={}", quote(m));
        }
        self.line(&head);
    }

    fn derive(&mut self, d: &Derive) {
        self.line(&format!("derive {} {}", ident(&d.name), quote(&d.expr)));
    }

    fn text(&mut self, t: &Text) {
        self.line(&format!("text {} {}", ident(&t.name), quote(&t.template)));
    }

    fn has(&mut self, h: &Has) {
        let mut head = format!("has {}", ident(&h.capability));
        props(&mut head, &h.props);
        self.line(&head);
    }

    fn peripheral(&mut self, p: &Peripheral) {
        self.stmt(
            format!("peripheral {} {}", ident(&p.name), ident(&p.kind)),
            true,
            |w| {
                for h in &p.has {
                    w.has(h);
                }
                for l in &p.lines {
                    let mut head = ident(&l.name);
                    for pin in &l.pins {
                        let _ = write!(head, " {}", ident(pin));
                    }
                    w.stmt(head, !l.has.is_empty(), |w| {
                        for h in &l.has {
                            w.has(h);
                        }
                    });
                }
            },
        );
    }

    // --- Blocks -------------------------------------------------------------------------------------

    fn block(&mut self, b: &Block) {
        let keyword = match b.kind {
            BlockKind::Block => "block",
            BlockKind::Design => "design",
        };
        self.stmt(format!("{keyword} {}", ident(&b.name)), true, |w| {
            w.block_items(&b.items)
        });
    }

    fn block_items(&mut self, items: &[BlockItem]) {
        for item in items {
            match item {
                BlockItem::Default(_) => self.line("default"),
                BlockItem::When(c) => self.line(&format!("when {}", quote(&c.expr))),
                BlockItem::Param(p) => self.param(p),
                BlockItem::Port(p) => self.port(p),
                BlockItem::Derive(d) => self.derive(d),
                BlockItem::Text(t) => self.text(t),
                BlockItem::Assert(a) => self.assert(a),
                BlockItem::Place(p) => self.place(p),
                BlockItem::Circuit(c) => self.circuit(c),
                BlockItem::Connect(c) => self.connect(c),
                BlockItem::Fact(f) => self.terminal_fact(f),
                BlockItem::Block(b) => self.block(b),
                BlockItem::Nc(n) => {
                    let mut head = "nc".to_string();
                    for p in &n.pins {
                        let _ = write!(head, " {}", ident(&p.to_string()));
                    }
                    if let Some(note) = &n.note {
                        let _ = write!(head, " note={}", quote(note));
                    }
                    self.line(&head);
                }
                BlockItem::Scope(s) => self.stmt(format!("scope {}", ident(&s.name)), true, |w| {
                    w.block_items(&s.items)
                }),
                BlockItem::Stock(s) => self.stmt("stock".into(), true, |w| {
                    for st in &s.items {
                        w.statement(st);
                    }
                }),
                BlockItem::Match(m) => self.statement(m),
            }
        }
    }

    fn statement(&mut self, s: &Statement) {
        let mut head = ident(&s.name);
        for a in &s.args {
            let _ = write!(head, " {}", a.repr());
        }
        props(&mut head, &s.props);
        self.stmt(head, !s.children.is_empty(), |w| {
            for c in &s.children {
                w.statement(c);
            }
        });
    }

    fn place(&mut self, p: &Place) {
        let mut head = format!("place {}", ident(&p.what));
        if let Some(n) = &p.name {
            let _ = write!(head, " {}", ident(n));
        }
        props(&mut head, &p.args);
        self.stmt(head, !p.features.is_empty() || !p.ignores.is_empty(), |w| {
            for f in &p.features {
                w.line(&format!(
                    "{} {}",
                    if f.on { "with" } else { "without" },
                    ident(&f.name)
                ));
            }
            for i in &p.ignores {
                w.line(&format!(
                    "ignore {} reason={}",
                    ident(&i.target),
                    crate::value::Value::String(i.reason.clone()).repr()
                ));
            }
        });
    }

    fn circuit(&mut self, c: &Circuit) {
        if let Some(from) = &c.from {
            self.stmt("circuit".into(), true, |w| {
                w.line(&format!("from {}", ident(&from.to_string())));
                for leg in &c.legs {
                    let mut head = "to".to_string();
                    for e in &leg.elements {
                        let _ = write!(head, " {}", ident(&e.to_string()));
                    }
                    props(&mut head, &leg.props);
                    w.line(&head);
                }
            });
            return;
        }
        let mut head = "circuit".to_string();
        for e in &c.elements {
            let _ = write!(head, " {}", ident(&e.to_string()));
        }
        props(&mut head, &c.props);
        self.line(&head);
    }

    fn connect(&mut self, c: &Connect) {
        let head = format!("connect {}", ident(&c.ty));
        let one_line = c.sides.len() == 2
            && c.unbound.is_empty()
            && c.requires.is_empty()
            && c.sides.iter().all(|s| s.answers.is_empty())
            && c.sides[0].role == SideRole::From
            && c.sides[1].role == SideRole::To;
        if one_line {
            self.line(&format!(
                "{head} from={} to={}",
                ident(&c.sides[0].target.to_string()),
                ident(&c.sides[1].target.to_string())
            ));
            return;
        }
        self.stmt(head, true, |w| {
            for s in &c.sides {
                let mut head = format!("{} {}", s.role.statement(), ident(&s.target.to_string()));
                props(&mut head, &s.answers);
                w.line(&head);
            }
            for r in &c.requires {
                w.require(r);
            }
            for u in &c.unbound {
                let mut head = format!(
                    "nc {}",
                    u.lines
                        .iter()
                        .map(|l| ident(l))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
                if let Some(note) = &u.note {
                    head.push_str(&format!(" note={}", quote(note)));
                }
                w.line(&head);
            }
        });
    }

    // --- Types --------------------------------------------------------------------------------------

    fn type_decl(&mut self, t: &TypeDecl) {
        self.stmt(format!("type {}", ident(&t.name)), true, |w| {
            for item in &t.items {
                match item {
                    TypeItem::Line(l) => w.line_(l),
                    TypeItem::Fact(f) => w.fact(f),
                    TypeItem::Assert(a) => w.assert(a),
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_identifiers() {
        for ok in [
            "u",
            "VDD@4",
            "PA11",
            "V+",
            "reg.out.rail",
            "add0-gnd",
            "J.1",
            "STMicroelectronics",
        ] {
            assert!(is_bare(ok), "{ok} should be bare");
        }
        for not in [
            "",
            "100nF",
            "Cvdd/1",
            "a b",
            "true",
            "-1",
            "+.5",
            "Device:R\"",
            "x=y",
            "#x",
            "~{CS}",
        ] {
            assert!(!is_bare(not), "{not} should be quoted");
        }
    }

    #[test]
    fn quoting() {
        assert_eq!(quote("a\"b\\c\n"), r#""a\"b\\c\n""#);
        assert_eq!(quote("4.7kΩ"), "\"4.7kΩ\"");
    }
}
