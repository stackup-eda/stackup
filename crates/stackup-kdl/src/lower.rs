//! KDL nodes → the typed model. Every problem is reported and reading carries on: a statement that
//! cannot be read is left out, and the rest of the file is still returned.

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};

use crate::{
    ast::*,
    diag::Diagnostic,
    manifest::{self, LibraryDecl, LibrarySource, Manifest},
    reference::Reference,
    span::Span,
    value::{Property, Value},
};

pub(crate) fn lower(doc: &KdlDocument) -> (File, Vec<Diagnostic>) {
    let mut cx = Cx { diags: Vec::new() };
    let file = cx.file(doc);
    (file, cx.diags)
}

pub(crate) fn lower_manifest(doc: &KdlDocument) -> (Manifest, Vec<Diagnostic>) {
    let mut cx = Cx { diags: Vec::new() };
    let manifest = cx.manifest(doc);
    (manifest, cx.diags)
}

// --- Node access ------------------------------------------------------------------------------------

fn span(n: &KdlNode) -> Span {
    n.span().into()
}

fn name(n: &KdlNode) -> &str {
    n.name().value()
}

fn args(n: &KdlNode) -> impl Iterator<Item = &KdlEntry> {
    n.entries().iter().filter(|e| e.name().is_none())
}

fn props(n: &KdlNode) -> impl Iterator<Item = &KdlEntry> {
    n.entries().iter().filter(|e| e.name().is_some())
}

fn children(n: &KdlNode) -> &[KdlNode] {
    n.children().map(|c| c.nodes()).unwrap_or(&[])
}

/// Whether the entry was written as a quoted string. A bare identifier is a `KdlValue::String`
/// too; the format is the only thing that tells them apart.
fn quoted(e: &KdlEntry) -> bool {
    match e.format() {
        Some(f) => f.value_repr.starts_with('"') || f.value_repr.starts_with('#'),
        None => true,
    }
}

fn value_of(e: &KdlEntry) -> Value {
    match e.value() {
        KdlValue::String(s) if quoted(e) => Value::String(s.clone()),
        KdlValue::String(s) => Value::Name(s.clone()),
        KdlValue::Integer(i) => Value::Integer(*i),
        KdlValue::Float(f) => Value::Float(*f),
        KdlValue::Bool(b) => Value::Bool(*b),
        KdlValue::Null => Value::Null,
    }
}

fn property_of(e: &KdlEntry) -> Property {
    Property {
        key: e.name().map(|n| n.value().to_string()).unwrap_or_default(),
        value: value_of(e),
        span: e.span().into(),
    }
}

// --- The lowering -------------------------------------------------------------------------------------

struct Cx {
    diags: Vec<Diagnostic>,
}

impl Cx {
    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diags.push(Diagnostic::error(span, message));
    }

    fn unknown(&mut self, n: &KdlNode, inside: &str) {
        self.error(
            span(n),
            format!("`{}` is not a statement {inside} can contain", name(n)),
        );
    }

    // Arguments ------------------------------------------------------------------------------------

    /// Positional argument `i` as a name or string. A number is a name too: a header's pins are
    /// `1`, `2`, `3`.
    fn text_arg(&mut self, n: &KdlNode, i: usize, what: &str) -> Option<String> {
        match args(n).nth(i) {
            Some(e) => match value_of(e) {
                Value::String(s) | Value::Name(s) => Some(s),
                Value::Integer(i) => Some(i.to_string()),
                v => {
                    self.error(
                        e.span().into(),
                        format!(
                            "the {what} of `{}` must be a name, not {}",
                            name(n),
                            v.describe()
                        ),
                    );
                    None
                }
            },
            None => {
                self.error(span(n), format!("`{}` needs a {what}", name(n)));
                None
            }
        }
    }

    /// Positional argument `i` as a name or string, absent being fine.
    fn opt_text_arg(&mut self, n: &KdlNode, i: usize, what: &str) -> Option<String> {
        args(n).nth(i)?;
        self.text_arg(n, i, what)
    }

    /// Positional argument `i`, which must be a quoted string: an expression or literal text.
    fn quoted_arg(&mut self, n: &KdlNode, i: usize, what: &str) -> Option<String> {
        match args(n).nth(i) {
            Some(e) => match value_of(e) {
                Value::String(s) => Some(s),
                Value::Name(_) => {
                    self.error(
                        e.span().into(),
                        format!("the {what} of `{}` must be quoted", name(n)),
                    );
                    None
                }
                v => {
                    self.error(
                        e.span().into(),
                        format!(
                            "the {what} of `{}` must be a quoted string, not {}",
                            name(n),
                            v.describe()
                        ),
                    );
                    None
                }
            },
            None => {
                self.error(span(n), format!("`{}` needs a {what}", name(n)));
                None
            }
        }
    }

    /// Positional argument `i` as any value.
    fn value_arg(&mut self, n: &KdlNode, i: usize, what: &str) -> Option<Value> {
        match args(n).nth(i) {
            Some(e) => Some(value_of(e)),
            None => {
                self.error(span(n), format!("`{}` needs a {what}", name(n)));
                None
            }
        }
    }

    /// Positional argument `i` as a reference.
    fn reference_arg(&mut self, n: &KdlNode, i: usize, what: &str) -> Option<Reference> {
        let e = args(n).nth(i);
        let text = self.text_arg(n, i, what)?;
        self.reference(&text, e.map(|e| e.span().into()).unwrap_or(span(n)))
    }

    fn reference(&mut self, text: &str, at: Span) -> Option<Reference> {
        match Reference::parse(text) {
            Ok(r) => Some(r),
            Err(err) => {
                self.error(at, format!("`{text}` is not a reference: {}", err.message));
                None
            }
        }
    }

    /// The bare flags among the positional arguments from `from` on, checked against `allowed`.
    fn flags(&mut self, n: &KdlNode, from: usize, allowed: &[&str]) -> Vec<String> {
        let mut found = Vec::new();
        for e in args(n).skip(from) {
            match value_of(e) {
                Value::Name(f) if allowed.contains(&f.as_str()) => {
                    if found.contains(&f) {
                        self.error(e.span().into(), format!("`{f}` is given twice"));
                    } else {
                        found.push(f);
                    }
                }
                v => {
                    let expected = match allowed {
                        [] => String::new(),
                        [one] => format!("; only `{one}` can go here"),
                        many => format!(
                            "; only {} can go here",
                            many.iter()
                                .map(|f| format!("`{f}`"))
                                .collect::<Vec<_>>()
                                .join(" or ")
                        ),
                    };
                    let shown = match &v {
                        Value::String(s) | Value::Name(s) => format!("`{s}`"),
                        v => v.describe().to_string(),
                    };
                    self.error(
                        e.span().into(),
                        format!("unexpected {shown} on `{}`{expected}", name(n)),
                    );
                }
            }
        }
        found
    }

    fn no_more_args(&mut self, n: &KdlNode, from: usize) {
        self.flags(n, from, &[]);
    }

    // Properties -------------------------------------------------------------------------------------

    /// Every property, as written.
    fn properties(&mut self, n: &KdlNode) -> Vec<Property> {
        props(n).map(property_of).collect()
    }

    /// The properties, each of which must be one of `allowed`.
    fn known_properties(&mut self, n: &KdlNode, allowed: &[&str]) -> Vec<Property> {
        let mut out = Vec::new();
        for e in props(n) {
            let p = property_of(e);
            if allowed.contains(&p.key.as_str()) {
                if out.iter().any(|q: &Property| q.key == p.key) {
                    self.error(p.span, format!("`{}=` is given twice", p.key));
                } else {
                    out.push(p);
                }
            } else {
                let expected = match allowed {
                    [] => "it takes none".to_string(),
                    many => format!(
                        "it takes {}",
                        many.iter()
                            .map(|f| format!("`{f}=`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                };
                self.error(
                    p.span,
                    format!("`{}` has no `{}=` property; {expected}", name(n), p.key),
                );
            }
        }
        out
    }

    fn no_properties(&mut self, n: &KdlNode) {
        self.known_properties(n, &[]);
    }

    /// Takes a property out of the list as text.
    fn take_text(&mut self, props: &mut Vec<Property>, key: &str) -> Option<String> {
        let i = props.iter().position(|p| p.key == key)?;
        let p = props.remove(i);
        match p.value {
            Value::String(s) | Value::Name(s) => Some(s),
            Value::Integer(i) => Some(i.to_string()),
            v => {
                self.error(
                    p.span,
                    format!("`{key}=` must be a name or a string, not {}", v.describe()),
                );
                None
            }
        }
    }

    fn take_value(&mut self, props: &mut Vec<Property>, key: &str) -> Option<Value> {
        let i = props.iter().position(|p| p.key == key)?;
        Some(props.remove(i).value)
    }

    fn no_children(&mut self, n: &KdlNode) {
        if let Some(c) = n.children()
            && let Some(first) = c.nodes().first()
        {
            self.error(span(first), format!("`{}` has no children", name(n)));
        }
    }

    // --- File -----------------------------------------------------------------------------------------

    fn file(&mut self, doc: &KdlDocument) -> File {
        let mut items = Vec::new();
        for n in doc.nodes() {
            let item = match name(n) {
                "use" => self.use_(n).map(Item::Use),
                "part" => self.part(n).map(Item::Part),
                "block" => self.block(n, BlockKind::Block).map(Item::Block),
                "design" => self.block(n, BlockKind::Design).map(Item::Block),
                "type" => self.type_decl(n).map(Item::Type),
                _ => {
                    self.unknown(n, "a file");
                    None
                }
            };
            items.extend(item);
        }
        File { items }
    }

    fn use_(&mut self, n: &KdlNode) -> Option<Use> {
        let path = self.quoted_arg(n, 0, "path");
        self.no_more_args(n, 1);
        self.no_properties(n);
        self.no_children(n);
        Some(Use {
            path: path?,
            span: span(n),
        })
    }

    // --- Parts ----------------------------------------------------------------------------------------

    fn part(&mut self, n: &KdlNode) -> Option<Part> {
        let part_name = self.text_arg(n, 0, "name");
        self.no_more_args(n, 1);
        self.no_properties(n);
        let mut items = Vec::new();
        for c in children(n) {
            let item = match name(c) {
                k if MetaKey::from_statement(k).is_some() => self.meta(c).map(PartItem::Meta),
                "order" => self.order(c).map(PartItem::Order),
                "param" => self.param(c).map(PartItem::Param),
                "pin" => self.pin(c).map(PartItem::Pin),
                "package" => self.package(c).map(PartItem::Package),
                "port" => self.port(c).map(PartItem::Port),
                "peripheral" => self.peripheral(c).map(PartItem::Peripheral),
                "derive" => self.derive(c).map(PartItem::Derive),
                "text" => self.text(c).map(PartItem::Text),
                "assert" => self.assert(c).map(PartItem::Assert),
                "block" => self.block(c, BlockKind::Block).map(PartItem::Block),
                _ => {
                    self.unknown(c, "a part");
                    None
                }
            };
            items.extend(item);
        }
        Some(Part {
            name: part_name?,
            items,
            span: span(n),
        })
    }

    fn meta(&mut self, n: &KdlNode) -> Option<Meta> {
        let key = MetaKey::from_statement(name(n))?;
        let value = self.value_arg(n, 0, "value");
        self.no_more_args(n, 1);
        self.no_properties(n);
        self.no_children(n);
        Some(Meta {
            key,
            value: value?,
            span: span(n),
        })
    }

    fn order(&mut self, n: &KdlNode) -> Option<Order> {
        self.no_more_args(n, 0);
        let props = self.properties(n);
        self.no_children(n);
        Some(Order {
            props,
            span: span(n),
        })
    }

    fn param(&mut self, n: &KdlNode) -> Option<Param> {
        let param_name = self.text_arg(n, 0, "name");
        let ty = self.text_arg(n, 1, "type");
        self.no_more_args(n, 2);
        let mut props = self.known_properties(n, &["default"]);
        let default = self.take_value(&mut props, "default");
        self.no_children(n);
        Some(Param {
            name: param_name?,
            ty: ty?,
            default,
            span: span(n),
        })
    }

    fn pin(&mut self, n: &KdlNode) -> Option<Pin> {
        let pin_name = self.text_arg(n, 0, "name");
        let kind = self.text_arg(n, 1, "kind");
        let flags = self.flags(n, 2, &["required", "once"]);
        let mut props = self.known_properties(n, &["symbol", "symbol-kind"]);
        let symbol = self.take_text(&mut props, "symbol");
        let symbol_kind = self.take_text(&mut props, "symbol-kind");
        let mut also = Vec::new();
        let mut roles = Vec::new();
        let mut requires = Vec::new();
        for c in children(n) {
            match name(c) {
                "require" => requires.extend(self.require(c)),
                "also" => {
                    let also_name = self.text_arg(c, 0, "name");
                    self.no_more_args(c, 1);
                    self.no_properties(c);
                    self.no_children(c);
                    also.extend(also_name.map(|name| Also {
                        name,
                        span: span(c),
                    }));
                }
                "role" => {
                    let role = self.text_arg(c, 0, "role");
                    self.no_more_args(c, 1);
                    let props = self.properties(c);
                    self.no_children(c);
                    roles.extend(role.map(|role| Role {
                        role,
                        props,
                        span: span(c),
                    }));
                }
                _ => self.unknown(c, "a pin"),
            }
        }
        Some(Pin {
            name: pin_name?,
            kind: kind?,
            required: flags.iter().any(|f| f == "required"),
            once: flags.iter().any(|f| f == "once"),
            symbol,
            symbol_kind,
            also,
            roles,
            requires,
            span: span(n),
        })
    }

    fn package(&mut self, n: &KdlNode) -> Option<Package> {
        let package_name = self.text_arg(n, 0, "name");
        self.no_more_args(n, 1);
        let mut props = self.known_properties(n, &["footprint"]);
        let footprint = self.take_text(&mut props, "footprint");
        let mut items = Vec::new();
        for c in children(n) {
            match name(c) {
                "reference" | "manufacturer" => self.error(
                    span(c),
                    format!(
                        "`{}` is the part's, not a package's: a body changes neither",
                        name(c)
                    ),
                ),
                k if MetaKey::from_statement(k).is_some() => {
                    items.extend(self.meta(c).map(PackageItem::Meta));
                }
                "order" => items.extend(self.order(c).map(PackageItem::Order)),
                "pad" => {
                    let pin = self.text_arg(c, 0, "pin");
                    let label = match args(c).nth(1) {
                        Some(e) => match value_of(e) {
                            Value::String(s) | Value::Name(s) => Some(s),
                            Value::Integer(i) => Some(i.to_string()),
                            v => {
                                self.error(
                                    e.span().into(),
                                    format!(
                                        "a pad label must be a name or a number, not {}",
                                        v.describe()
                                    ),
                                );
                                None
                            }
                        },
                        None => {
                            self.error(span(c), "`pad` needs a pin and a label");
                            None
                        }
                    };
                    self.no_more_args(c, 2);
                    self.no_properties(c);
                    self.no_children(c);
                    if let (Some(pin), Some(label)) = (pin, label) {
                        items.push(PackageItem::Pad(Pad {
                            pin,
                            label,
                            span: span(c),
                        }));
                    }
                }
                _ => self.unknown(c, "a package"),
            }
        }
        Some(Package {
            name: package_name?,
            footprint,
            items,
            span: span(n),
        })
    }

    fn port(&mut self, n: &KdlNode) -> Option<Port> {
        let port_name = self.text_arg(n, 0, "name");
        self.no_more_args(n, 1);
        let mut props = self.known_properties(n, &["type", "pin", "require"]);
        let ty = self.take_text(&mut props, "type");
        let pin = self.take_text(&mut props, "pin");
        let mut items = Vec::new();
        // `require=<fact>` on the port line is the same statement as `require <fact>` inside it.
        if let Some(p) = props.iter().find(|p| p.key == "require").cloned() {
            match p.value.as_str() {
                Some(text) => {
                    if let Some((aspect, fact)) = self.aspect_fact(n, text, p.span) {
                        items.push(PortItem::Require(Require {
                            aspect,
                            fact,
                            value: None,
                            props: Vec::new(),
                            span: p.span,
                        }));
                    }
                }
                None => self.error(
                    p.span,
                    format!("`require=` names a fact, not {}", p.value.describe()),
                ),
            }
        }
        for c in children(n) {
            let item = match name(c) {
                "line" => self.line(c).map(PortItem::Line),
                "set" | "add" | "claim" => self.fact(c).map(PortItem::Fact),
                "require" => self.require(c).map(PortItem::Require),
                "assert" => self.assert(c).map(PortItem::Assert),
                _ => {
                    self.unknown(c, "a port");
                    None
                }
            };
            items.extend(item);
        }
        Some(Port {
            name: port_name?,
            ty,
            pin,
            items,
            span: span(n),
        })
    }

    fn line(&mut self, n: &KdlNode) -> Option<Line> {
        let line_name = self.text_arg(n, 0, "name");
        let flags = self.flags(n, 1, &["optional"]);
        let mut props = self.known_properties(n, &["type", "pin"]);
        let ty = self.take_text(&mut props, "type");
        let pin = self.take_text(&mut props, "pin");
        let mut items = Vec::new();
        for c in children(n) {
            let item = match name(c) {
                "require" => self.require(c).map(LineItem::Require),
                "set" | "add" | "claim" => self.fact(c).map(LineItem::Fact),
                "match" => {
                    let target = self.reference_arg(c, 0, "line");
                    self.no_more_args(c, 1);
                    self.no_properties(c);
                    self.no_children(c);
                    target.map(|target| {
                        LineItem::Match(Match {
                            target,
                            span: span(c),
                        })
                    })
                }
                _ => {
                    self.unknown(c, "a line");
                    None
                }
            };
            items.extend(item);
        }
        Some(Line {
            name: line_name?,
            ty,
            pin,
            optional: flags.iter().any(|f| f == "optional"),
            items,
            span: span(n),
        })
    }

    /// `net.voltage` → the aspect and the fact. A fact names its aspect, always.
    fn aspect_fact(&mut self, n: &KdlNode, text: &str, at: Span) -> Option<(Aspect, String)> {
        let Some((aspect, fact)) = text.split_once('.') else {
            self.error(
                at,
                format!(
                    "`{}` names a fact by its aspect: `net.{text}`, `signal.{text}` or `segment.{text}`",
                    name(n)
                ),
            );
            return None;
        };
        let Some(aspect) = Aspect::from_name(aspect) else {
            self.error(
                at,
                format!("`{aspect}` is not an aspect; a fact is on `net`, `signal` or `segment`"),
            );
            return None;
        };
        if fact.is_empty() || fact.contains('.') {
            self.error(at, format!("`{text}` is not `<aspect>.<fact>`"));
            return None;
        }
        Some((aspect, fact.to_string()))
    }

    fn fact(&mut self, n: &KdlNode) -> Option<Fact> {
        let rule = Rule::from_statement(name(n))?;
        let at = args(n).next().map(|e| e.span().into()).unwrap_or(span(n));
        let fact = self
            .text_arg(n, 0, "fact")
            .and_then(|t| self.aspect_fact(n, &t, at));
        let value = args(n).nth(1).map(value_of);
        self.no_more_args(n, 2);
        let props = self.properties(n);
        self.no_children(n);
        let (aspect, fact) = fact?;
        Some(Fact {
            rule,
            aspect,
            fact,
            value,
            props,
            span: span(n),
        })
    }

    /// `set <terminal> <aspect>.<fact> [<value>]` in a block: a port line's fact statement with
    /// the terminal it is aimed at in front.
    fn terminal_fact(&mut self, n: &KdlNode) -> Option<TerminalFact> {
        let rule = Rule::from_statement(name(n))?;
        let terminal = self.reference_arg(n, 0, "terminal");
        let at = args(n).nth(1).map(|e| e.span().into()).unwrap_or(span(n));
        let fact = self
            .text_arg(n, 1, "fact")
            .and_then(|t| self.aspect_fact(n, &t, at));
        let value = args(n).nth(2).map(value_of);
        self.no_more_args(n, 3);
        let props = self.properties(n);
        self.no_children(n);
        let (aspect, fact) = fact?;
        Some(TerminalFact {
            terminal: terminal?,
            fact: Fact {
                rule,
                aspect,
                fact,
                value,
                props,
                span: span(n),
            },
            span: span(n),
        })
    }

    fn require(&mut self, n: &KdlNode) -> Option<Require> {
        let at = args(n).next().map(|e| e.span().into()).unwrap_or(span(n));
        let fact = self
            .text_arg(n, 0, "fact")
            .and_then(|t| self.aspect_fact(n, &t, at));
        let value = args(n).nth(1).map(value_of);
        self.no_more_args(n, 2);
        let props = self.properties(n);
        self.no_children(n);
        let (aspect, fact) = fact?;
        Some(Require {
            aspect,
            fact,
            value,
            props,
            span: span(n),
        })
    }

    fn assert(&mut self, n: &KdlNode) -> Option<Assert> {
        let expr = self.quoted_arg(n, 0, "expression");
        self.no_more_args(n, 1);
        let mut props = self.known_properties(n, &["message"]);
        let message = self.take_text(&mut props, "message");
        self.no_children(n);
        Some(Assert {
            expr: expr?,
            message,
            span: span(n),
        })
    }

    fn derive(&mut self, n: &KdlNode) -> Option<Derive> {
        let derive_name = self.text_arg(n, 0, "name");
        let expr = self.quoted_arg(n, 1, "expression");
        self.no_more_args(n, 2);
        self.no_properties(n);
        self.no_children(n);
        Some(Derive {
            name: derive_name?,
            expr: expr?,
            span: span(n),
        })
    }

    fn text(&mut self, n: &KdlNode) -> Option<Text> {
        let text_name = self.text_arg(n, 0, "name");
        let template = self.quoted_arg(n, 1, "template");
        self.no_more_args(n, 2);
        self.no_properties(n);
        self.no_children(n);
        Some(Text {
            name: text_name?,
            template: template?,
            span: span(n),
        })
    }

    fn peripheral(&mut self, n: &KdlNode) -> Option<Peripheral> {
        let peripheral_name = self.text_arg(n, 0, "name");
        let kind = self.text_arg(n, 1, "kind");
        self.no_more_args(n, 2);
        self.no_properties(n);
        let mut has = Vec::new();
        let mut lines = Vec::new();
        for c in children(n) {
            if name(c) == "has" {
                has.extend(self.has(c));
            } else {
                lines.extend(self.peripheral_line(c));
            }
        }
        Some(Peripheral {
            name: peripheral_name?,
            kind: kind?,
            has,
            lines,
            span: span(n),
        })
    }

    fn has(&mut self, n: &KdlNode) -> Option<Has> {
        let capability = self.text_arg(n, 0, "capability");
        self.no_more_args(n, 1);
        let props = self.properties(n);
        self.no_children(n);
        Some(Has {
            capability: capability?,
            props,
            span: span(n),
        })
    }

    fn peripheral_line(&mut self, n: &KdlNode) -> Option<PeripheralLine> {
        let mut pins = Vec::new();
        for e in args(n) {
            match value_of(e) {
                Value::String(s) | Value::Name(s) => pins.push(s),
                v => self.error(
                    e.span().into(),
                    format!("a peripheral line lists pins, not {}", v.describe()),
                ),
            }
        }
        self.no_properties(n);
        let mut has = Vec::new();
        for c in children(n) {
            match name(c) {
                "has" => has.extend(self.has(c)),
                _ => self.unknown(c, "a peripheral line"),
            }
        }
        Some(PeripheralLine {
            name: name(n).to_string(),
            pins,
            has,
            span: span(n),
        })
    }

    // --- Blocks ---------------------------------------------------------------------------------------

    fn block(&mut self, n: &KdlNode, kind: BlockKind) -> Option<Block> {
        let block_name = self.text_arg(n, 0, "name");
        self.no_more_args(n, 1);
        self.no_properties(n);
        let what = match kind {
            BlockKind::Block => "a block",
            BlockKind::Design => "a design",
        };
        let items = self.block_items(n, what, kind == BlockKind::Block);
        Some(Block {
            kind,
            name: block_name?,
            items,
            span: span(n),
        })
    }

    fn block_items(&mut self, n: &KdlNode, what: &str, features: bool) -> Vec<BlockItem> {
        let mut items = Vec::new();
        for c in children(n) {
            let item = match name(c) {
                "default" | "when" if !features => {
                    self.error(
                        span(c),
                        format!("`{}` belongs to a part's child block, not {what}", name(c)),
                    );
                    None
                }
                "default" => {
                    self.no_more_args(c, 0);
                    self.no_properties(c);
                    self.no_children(c);
                    Some(BlockItem::Default(span(c)))
                }
                "when" => {
                    let expr = self.quoted_arg(c, 0, "condition");
                    self.no_more_args(c, 1);
                    self.no_properties(c);
                    self.no_children(c);
                    expr.map(|expr| {
                        BlockItem::When(When {
                            expr,
                            span: span(c),
                        })
                    })
                }
                "param" => self.param(c).map(BlockItem::Param),
                "port" => self.port(c).map(BlockItem::Port),
                "derive" => self.derive(c).map(BlockItem::Derive),
                "text" => self.text(c).map(BlockItem::Text),
                "assert" => self.assert(c).map(BlockItem::Assert),
                "place" => self.place(c).map(BlockItem::Place),
                "circuit" => self.circuit(c).map(BlockItem::Circuit),
                "connect" => self.connect(c).map(BlockItem::Connect),
                "nc" => self.nc(c).map(BlockItem::Nc),
                "set" | "add" | "claim" => self.terminal_fact(c).map(BlockItem::Fact),
                // A block's child block is a feature of it, as a part's are; a scope and a
                // design have none, so there the keyword falls through to "unknown".
                "block" if features => self.block(c, BlockKind::Block).map(BlockItem::Block),
                "scope" => {
                    let scope_name = self.text_arg(c, 0, "name");
                    self.no_more_args(c, 1);
                    self.no_properties(c);
                    let items = self.block_items(c, "a scope", false);
                    scope_name.map(|name| {
                        BlockItem::Scope(Scope {
                            name,
                            items,
                            span: span(c),
                        })
                    })
                }
                "stock" => {
                    self.no_more_args(c, 0);
                    self.no_properties(c);
                    let items = children(c).iter().map(|s| self.statement(s)).collect();
                    Some(BlockItem::Stock(Stock {
                        items,
                        span: span(c),
                    }))
                }
                _ => {
                    self.unknown(c, what);
                    None
                }
            };
            items.extend(item);
        }
        items
    }

    fn statement(&mut self, n: &KdlNode) -> Statement {
        Statement {
            name: name(n).to_string(),
            args: args(n).map(value_of).collect(),
            props: self.properties(n),
            children: children(n).iter().map(|c| self.statement(c)).collect(),
            span: span(n),
        }
    }

    fn place(&mut self, n: &KdlNode) -> Option<Place> {
        let what = self.text_arg(n, 0, "part or block");
        let place_name = self.opt_text_arg(n, 1, "name");
        self.no_more_args(n, 2);
        let args = self.properties(n);
        let mut features = Vec::new();
        for c in children(n) {
            match name(c) {
                "with" | "without" => {
                    let feature = self.text_arg(c, 0, "feature");
                    self.no_more_args(c, 1);
                    self.no_properties(c);
                    self.no_children(c);
                    features.extend(feature.map(|name| Feature {
                        on: name_is_with(c),
                        name,
                        span: span(c),
                    }));
                }
                _ => self.unknown(c, "a placement"),
            }
        }
        Some(Place {
            what: what?,
            name: place_name,
            args,
            features,
            span: span(n),
        })
    }

    fn circuit(&mut self, n: &KdlNode) -> Option<Circuit> {
        if n.children().is_some() {
            self.no_more_args(n, 0);
            self.no_properties(n);
            let mut from = None;
            let mut legs = Vec::new();
            for child in children(n) {
                match name(child) {
                    "from" => {
                        if from.is_some() || !legs.is_empty() {
                            self.error(span(child), "`from` must appear once, before every `to`");
                        }
                        let start = self.reference_arg(child, 0, "terminal");
                        self.no_more_args(child, 1);
                        self.no_properties(child);
                        self.no_children(child);
                        if from.is_none() {
                            from = start;
                        }
                    }
                    "to" => {
                        if from.is_none() {
                            self.error(span(child), "`to` needs a preceding `from`");
                        }
                        let mut elements = Vec::new();
                        for entry in args(child) {
                            match value_of(entry) {
                                Value::String(s) | Value::Name(s) => {
                                    elements.extend(self.reference(&s, entry.span().into()));
                                }
                                v => self.error(
                                    entry.span().into(),
                                    format!(
                                        "a `to` element must be a reference, not {}",
                                        v.describe()
                                    ),
                                ),
                            }
                        }
                        if elements.is_empty() {
                            self.error(span(child), "`to` needs at least one element");
                        }
                        let props = self.known_properties(child, &["name"]);
                        self.no_children(child);
                        legs.push(CircuitLeg {
                            elements,
                            props,
                            span: span(child),
                        });
                    }
                    _ => self.unknown(child, "a circuit"),
                }
            }
            if from.is_none() {
                self.error(span(n), "a multiline `circuit` needs `from`");
            }
            if legs.is_empty() {
                self.error(span(n), "a multiline `circuit` needs at least one `to`");
            }
            return from.map(|from| Circuit {
                elements: Vec::new(),
                props: Vec::new(),
                from: Some(from),
                legs,
                span: span(n),
            });
        }
        let mut elements = Vec::new();
        let mut complete = true;
        for e in args(n) {
            match value_of(e) {
                Value::String(s) | Value::Name(s) => match self.reference(&s, e.span().into()) {
                    Some(r) => elements.push(r),
                    None => complete = false,
                },
                v => {
                    self.error(
                        e.span().into(),
                        format!("a circuit's elements are references, not {}", v.describe()),
                    );
                    complete = false;
                }
            }
        }
        if elements.is_empty() && complete {
            self.error(span(n), "`circuit` needs at least one element");
            complete = false;
        }
        let props = self.properties(n);
        self.no_children(n);
        complete.then_some(Circuit {
            elements,
            props,
            from: None,
            legs: Vec::new(),
            span: span(n),
        })
    }

    fn nc(&mut self, n: &KdlNode) -> Option<Nc> {
        let mut pins = Vec::new();
        for e in args(n) {
            match value_of(e) {
                Value::String(s) | Value::Name(s) => {
                    pins.extend(self.reference(&s, e.span().into()))
                }
                v => self.error(
                    e.span().into(),
                    format!("`nc` names pins, not {}", v.describe()),
                ),
            }
        }
        if pins.is_empty() {
            self.error(span(n), "`nc` needs at least one pin");
        }
        let mut props = self.known_properties(n, &["note"]);
        let note = self.take_text(&mut props, "note");
        self.no_children(n);
        Some(Nc {
            pins,
            note,
            span: span(n),
        })
    }

    fn connect(&mut self, n: &KdlNode) -> Option<Connect> {
        let ty = self.text_arg(n, 0, "type");
        self.no_more_args(n, 1);
        let props = self.known_properties(n, &["from", "to"]);
        let mut sides = Vec::new();
        let mut unbound = Vec::new();
        for p in props {
            let role = if p.key == "from" {
                SideRole::From
            } else {
                SideRole::To
            };
            match p.value.as_str() {
                Some(text) => {
                    if let Some(target) = self.reference(text, p.span) {
                        sides.push(Side {
                            role,
                            target,
                            answers: Vec::new(),
                            span: p.span,
                        });
                    }
                }
                None => self.error(
                    p.span,
                    format!("`{}=` names a side, not {}", p.key, p.value.describe()),
                ),
            }
        }
        for c in children(n) {
            match name(c) {
                "from" | "to" => {
                    let role = if name(c) == "from" {
                        SideRole::From
                    } else {
                        SideRole::To
                    };
                    let target = self.reference_arg(c, 0, "side");
                    self.no_more_args(c, 1);
                    let answers = self.properties(c);
                    self.no_children(c);
                    sides.extend(target.map(|target| Side {
                        role,
                        target,
                        answers,
                        span: span(c),
                    }));
                }
                "nc" => {
                    let mut lines = Vec::new();
                    for e in args(c) {
                        match value_of(e) {
                            Value::String(l) | Value::Name(l) => lines.push(l),
                            v => self.error(
                                e.span().into(),
                                format!("`nc` in a connect names lines, not {}", v.describe()),
                            ),
                        }
                    }
                    if lines.is_empty() {
                        self.error(span(c), "`nc` needs at least one line");
                    }
                    let mut props = self.known_properties(c, &["note"]);
                    let note = self.take_text(&mut props, "note");
                    self.no_children(c);
                    unbound.push(Unbound {
                        lines,
                        note,
                        span: span(c),
                    });
                }
                _ => self.unknown(c, "a connect"),
            }
        }
        if !sides.iter().any(|s| s.role == SideRole::From) {
            self.error(span(n), "`connect` needs a `from` side");
        }
        if !sides.iter().any(|s| s.role == SideRole::To) {
            self.error(span(n), "`connect` needs a `to` side");
        }
        Some(Connect {
            ty: ty?,
            sides,
            unbound,
            span: span(n),
        })
    }

    // --- Types ----------------------------------------------------------------------------------------

    fn type_decl(&mut self, n: &KdlNode) -> Option<TypeDecl> {
        let type_name = self.text_arg(n, 0, "name");
        self.no_more_args(n, 1);
        self.no_properties(n);
        let mut items = Vec::new();
        for c in children(n) {
            let item = match name(c) {
                "line" => self.line(c).map(TypeItem::Line),
                "set" | "add" | "claim" => self.fact(c).map(TypeItem::Fact),
                "assert" => self.assert(c).map(TypeItem::Assert),
                _ => {
                    self.unknown(c, "a type");
                    None
                }
            };
            items.extend(item);
        }
        Some(TypeDecl {
            name: type_name?,
            items,
            span: span(n),
        })
    }
}

fn name_is_with(n: &KdlNode) -> bool {
    name(n) == "with"
}

// --- Manifests ------------------------------------------------------------------------------------

impl Cx {
    /// A `manifest.kdl`: `stackup "<version>"`, `name <name>` and `library` lines, each at most
    /// once per name. A different document from a design file, with statements of its own.
    fn manifest(&mut self, doc: &KdlDocument) -> Manifest {
        let mut m = Manifest::default();
        for n in doc.nodes() {
            match name(n) {
                "stackup" => {
                    let text = self.quoted_arg(n, 0, "version");
                    self.no_more_args(n, 1);
                    self.no_properties(n);
                    self.no_children(n);
                    let Some(text) = text else { continue };
                    match manifest::parse_version(&text) {
                        Some(_) if m.stackup.is_some() => {
                            self.error(span(n), "`stackup` is stated twice");
                        }
                        Some((major, minor)) => {
                            m.stackup = Some(manifest::Version {
                                major,
                                minor,
                                text,
                                span: span(n),
                            });
                        }
                        None => self.error(
                            span(n),
                            format!("`stackup` takes a version like \"0.1\", not \"{text}\""),
                        ),
                    }
                }
                "name" => {
                    let lib_name = self.text_arg(n, 0, "name");
                    self.no_more_args(n, 1);
                    self.no_properties(n);
                    self.no_children(n);
                    let Some(lib_name) = lib_name else { continue };
                    if m.name.is_some() {
                        self.error(span(n), "`name` is stated twice");
                    } else {
                        m.name = Some(manifest::Named {
                            name: lib_name,
                            span: span(n),
                        });
                    }
                }
                "library" => {
                    let Some(l) = self.library(n) else { continue };
                    if m.library(&l.name).is_some() {
                        self.error(l.span, format!("library `{}` is declared twice", l.name));
                    } else {
                        m.libraries.push(l);
                    }
                }
                _ => self.unknown(n, "a manifest"),
            }
        }
        m
    }

    fn library(&mut self, n: &KdlNode) -> Option<LibraryDecl> {
        let lib_name = self.text_arg(n, 0, "name");
        self.no_more_args(n, 1);
        let mut props = self.known_properties(n, &["path", "git", "rev", "dir"]);
        self.no_children(n);
        let path = self.take_text(&mut props, "path");
        let git = self.take_text(&mut props, "git");
        let rev = self.take_text(&mut props, "rev");
        let dir = self.take_text(&mut props, "dir");
        let source = match (path, git) {
            (Some(path), None) => {
                if rev.is_some() || dir.is_some() {
                    self.error(
                        span(n),
                        "`rev=` and `dir=` go with `git=`; a `path=` library is a directory as it is",
                    );
                    return None;
                }
                LibrarySource::Path(path)
            }
            (None, Some(url)) => match rev {
                Some(rev) => LibrarySource::Git { url, rev, dir },
                None => {
                    self.error(
                        span(n),
                        "a `git=` library needs `rev=`, the commit it is pinned to",
                    );
                    return None;
                }
            },
            (Some(_), Some(_)) => {
                self.error(span(n), "`library` takes `path=` or `git=`, not both");
                return None;
            }
            (None, None) => {
                self.error(span(n), "`library` needs `path=` or `git=`");
                return None;
            }
        };
        Some(LibraryDecl {
            name: lib_name?,
            source,
            span: span(n),
        })
    }
}
