//! Elaboration: from declarations to an instance tree and nets.
//!
//! A design is walked from its root block. Each `place` makes an instance; a part's child blocks
//! come with it as its features say; `circuit` and `connect` join terminals into nets. Every
//! problem is a finding and elaboration carries on, so a design is judged whole.
//!
//! Placement and wiring are separate passes because order carries no meaning. Port arguments
//! also join after every placement exists, so they may name a later placement.

use std::collections::{HashMap, HashSet};

use stackup_kdl::{
    Property, Reference, Span, Value,
    ast::{
        Aspect, Block, BlockItem, BlockKind, Feature, LineItem, MetaKey, Package, Param, Part,
        Place, PortItem, Require, Rule, TypeDecl,
    },
};

use crate::{
    load::{Decl, DeclRef, FileId, Library},
    model::{Instance, Kind, Model, Net, Nets, Terminal},
    report::Report,
};

mod facts;
mod peripherals;
mod static_values;

/// Elaborates one design.
pub fn elaborate(lib: &Library, file: FileId, design: &Block) -> Model {
    let mut b = Builder {
        lib,
        instances: Vec::new(),
        children: HashMap::new(),
        ports: Vec::new(),
        pin_alias: HashMap::new(),
        pin_port: HashMap::new(),
        nets: Nets::default(),
        contributions: Vec::new(),
        returns: HashSet::new(),
        requirements: Vec::new(),
        ignores: Vec::new(),
        pending_notes: Vec::new(),
        pending_values: Vec::new(),
        pending_required_voltages: Vec::new(),
        pending_anchors: Vec::new(),
        pending_port_joins: Vec::new(),
        nc: Vec::new(),
        frames: Vec::new(),
        report: Report::default(),
        imperial: None,
    };
    let root = b.new_instance(
        String::new(),
        None,
        Kind::Block {
            decl: DeclRef { file, item: 0 },
        },
        file,
        design.span,
    );
    // The design's own declaration is found by span, since a design is a block placed by nobody.
    if let Some(item) = lib.files[file]
        .file
        .items
        .iter()
        .position(|i| matches!(i, stackup_kdl::ast::Item::Block(bl) if bl.span == design.span))
    {
        b.instances[root].kind = Kind::Block {
            decl: DeclRef { file, item },
        };
    }
    let frame = b.new_frame(root, file, None);
    b.block_ports(root, frame, file, design);
    b.body(frame, &design.items);
    b.join_pending_ports();
    let mut model = b.finish(design.name.clone());
    crate::purchasing::apply(lib, file, design, &mut model);
    model
}

// --- Working state ------------------------------------------------------------------------------

struct PortInfo {
    name: String,
    ty: Option<String>,
    lines: Vec<(String, Terminal)>,
}

#[derive(Clone)]
enum Named {
    Instance(usize),
    Port(usize, String),
    Pin(usize, String),
}

/// A block body being elaborated: what its names mean.
struct Frame {
    inst: usize,
    file: FileId,
    parent: Option<usize>,
    params: HashMap<String, Value>,
    /// Part-valued parameters retain the caller's declaration, across file scopes.
    part_params: HashMap<String, Option<DeclRef>>,
    /// For a part's child blocks: the placement's features.
    features: HashMap<String, bool>,
    names: HashMap<String, Named>,
    self_inst: Option<usize>,
    /// Language properties on a block placement, for its `as=self` part.
    self_props: Vec<Property>,
    /// The caller's scope for forwarded placement references such as `anchor=`.
    self_props_frame: Option<usize>,
    /// Caller scope of a block's `anchor` parameter, whose value names a placement or pin.
    anchor_origin: Option<usize>,
}

#[derive(Clone, Debug)]
enum Resolved {
    Instance(usize),
    Pin(usize, String),
    Port(usize, String),
    Line(usize, String, String),
}

enum Shape {
    Point(Terminal),
    Series(Terminal, Terminal),
}

/// A fact stated on a terminal's net: by a port line, by a `circuit`'s `name=`, or by a
/// peripheral answer. Its value is kept as written and evaluated by the checks, in `frame`.
#[derive(Clone, Debug)]
struct Contribution {
    terminal: Terminal,
    aspect: Aspect,
    fact: String,
    rule: Rule,
    /// As written; `None` for a flag (`set signal.dma`) or a range given as `min=`/`max=`.
    value: Option<Value>,
    props: Vec<Property>,
    /// The frame the statement was written in: where its names resolve.
    frame: usize,
    /// The port it was written on, whose facts it may read by bare name.
    port: Option<(usize, String)>,
    file: FileId,
    span: Span,
}

/// What a port or a pin asks of its net.
struct Requirement {
    on: ReqOn,
    require: Require,
    frame: usize,
    file: FileId,
}

struct IgnoreCheck {
    inst: usize,
    target: String,
    reason: String,
    file: FileId,
    span: Span,
    matched: bool,
}

enum ReqOn {
    Port {
        inst: usize,
        port: String,
        lines: Vec<(String, Terminal)>,
    },
    Pin {
        inst: usize,
        pin: String,
        terminal: Terminal,
    },
}

/// The answer a peripheral table gives for one pin on one line: the instances that could carry
/// it, and what each has.
struct Candidate {
    instance: String,
    kind: String,
    caps: Vec<String>,
}

struct LineInfo {
    name: String,
    optional: bool,
    partner: Option<String>,
    ty: Option<String>,
}

struct TypeInfo {
    name: String,
    lines: Vec<LineInfo>,
}

struct Builder<'a> {
    lib: &'a Library,
    instances: Vec<Instance>,
    children: HashMap<(usize, String), usize>,
    ports: Vec<Vec<PortInfo>>,
    /// Every name a pin answers to — its own and its `also` names — to the pin.
    pin_alias: HashMap<(usize, String), String>,
    /// For a pin a part port maps a line onto: the port and line, for naming its net.
    pin_port: HashMap<Terminal, (String, String)>,
    nets: Nets,
    /// Every fact a statement contributed to a terminal's net, combined per net at the end.
    contributions: Vec<Contribution>,
    /// The `gnd` line of every power port: a return, which takes no voltage and rests low.
    returns: HashSet<Terminal>,
    requirements: Vec<Requirement>,
    ignores: Vec<IgnoreCheck>,
    /// Notes to render once facts are known: the instance, the frame, the note as written.
    pending_notes: Vec<(usize, usize, Value, Span)>,
    /// Part values may refer to derives, so resolve them after facts are available.
    pending_values: Vec<(usize, usize, Value, Span)>,
    /// A placement requirement may read a derive or a net fact in its placing block.
    pending_required_voltages: Vec<(usize, usize, Value, Span)>,
    /// Anchor references can name parts placed later in the same block.
    pending_anchors: Vec<(usize, usize, usize, Value, Span)>,
    /// Port arguments can name placements declared later in the same or an enclosing body.
    pending_port_joins: Vec<(usize, usize, String, Value, Span)>,
    nc: Vec<(Terminal, FileId, Span)>,
    frames: Vec<Frame>,
    report: Report,
    imperial: Option<String>,
}

const LANGUAGE_PROPS: [&str; 24] = [
    "value",
    "intent",
    "note",
    "footprint",
    "package",
    "designator",
    "reference",
    "as",
    "anchor",
    "spot",
    "bom_value",
    "hand",
    "manufacturer",
    "mpn",
    "lcsc",
    "mouser",
    "digikey",
    "series",
    "voltage",
    "required_voltage",
    "dissipation",
    "current",
    "tolerance",
    "dielectric",
];

impl<'a> Builder<'a> {
    fn error(&mut self, file: FileId, span: Span, message: impl Into<String>) {
        self.report.error(self.lib.source(file), span, message);
    }

    fn warning(&mut self, file: FileId, span: Span, message: impl Into<String>) {
        self.report.warning(self.lib.source(file), span, message);
    }

    fn add_ignores(&mut self, inst: usize, file: FileId, p: &Place) {
        for i in &p.ignores {
            if self
                .ignores
                .iter()
                .any(|old| old.inst == inst && old.target == i.target)
            {
                self.error(
                    file,
                    i.span,
                    format!("duplicate ignore selector `{}`", i.target),
                );
                continue;
            }
            self.ignores.push(IgnoreCheck {
                inst,
                target: i.target.clone(),
                reason: i.reason.clone(),
                file,
                span: i.span,
                matched: false,
            });
        }
    }

    fn new_instance(
        &mut self,
        path: String,
        parent: Option<usize>,
        kind: Kind,
        file: FileId,
        span: Span,
    ) -> usize {
        self.instances.push(Instance {
            path,
            parent,
            kind,
            file,
            span,
            designator: None,
            designator_word: None,
            reference_prefix: None,
            value: None,
            footprint: None,
            fields: HashMap::new(),
            required_voltage: None,
            hand: false,
            intent: None,
            anchor: None,
            spot: None,
            notes: Vec::new(),
        });
        self.ports.push(Vec::new());
        self.instances.len() - 1
    }

    fn new_frame(&mut self, inst: usize, file: FileId, parent: Option<usize>) -> usize {
        self.frames.push(Frame {
            inst,
            file,
            parent,
            params: HashMap::new(),
            part_params: HashMap::new(),
            features: HashMap::new(),
            names: HashMap::new(),
            self_inst: None,
            self_props: Vec::new(),
            self_props_frame: None,
            anchor_origin: None,
        });
        self.frames.len() - 1
    }

    fn compose(&self, parent: usize, name: &str) -> String {
        let p = &self.instances[parent].path;
        if p.is_empty() {
            name.to_string()
        } else {
            format!("{p}/{name}")
        }
    }

    fn part_of(&self, inst: usize) -> Option<&'a Part> {
        match self.instances[inst].kind {
            Kind::Part { decl, .. } => match self.lib.decl(decl) {
                Decl::Part(p) => Some(p),
                _ => None,
            },
            _ => None,
        }
    }

    /// The package the placement chose, where the part lists any.
    fn package_of(&self, inst: usize) -> Option<&'a Package> {
        let Kind::Part { package, .. } = self.instances[inst].kind else {
            return None;
        };
        self.part_of(inst)
            .and_then(|part| package.and_then(|i| part.packages().nth(i)))
    }

    /// Why a name is not a pin of this placement, when the part has such a pin and the chosen
    /// body does not bond it — so the answer names the bodies that do.
    fn unbonded(&self, inst: usize, name: &str) -> Option<String> {
        let part = self.part_of(inst)?;
        let body = self.package_of(inst)?;
        let pin = part
            .pins()
            .find(|p| p.name == name || p.also.iter().any(|a| a.name == name))?;
        let bonded: Vec<String> = part
            .packages()
            .filter(|k| k.bonds(&pin.name))
            .map(|k| format!("`{}`", k.name))
            .collect();
        Some(format!(
            "`{}` is not bonded in `{}`, the package `{}` is placed in; it is in {}",
            pin.name,
            body.name,
            self.instances[inst].path,
            if bonded.is_empty() {
                "no package".to_string()
            } else {
                bonded.join(", ")
            }
        ))
    }

    // --- Names ------------------------------------------------------------------------------------

    fn lookup_name(&self, frame: usize, name: &str) -> Option<Named> {
        let mut f = Some(frame);
        while let Some(i) = f {
            if let Some(n) = self.frames[i].names.get(name) {
                return Some(n.clone());
            }
            f = self.frames[i].parent;
        }
        None
    }

    fn lookup_param(&self, frame: usize, name: &str) -> Option<Value> {
        let mut f = Some(frame);
        while let Some(i) = f {
            if let Some(v) = self.frames[i].params.get(name) {
                return Some(v.clone());
            }
            f = self.frames[i].parent;
        }
        None
    }

    fn lookup_part_param(&self, frame: usize, name: &str) -> Option<Option<DeclRef>> {
        let mut f = Some(frame);
        while let Some(i) = f {
            if let Some(part) = self.frames[i].part_params.get(name) {
                return Some(*part);
            }
            f = self.frames[i].parent;
        }
        None
    }

    fn part_argument(&mut self, frame: usize, value: &Value, span: Span) -> Option<DeclRef> {
        let file = self.frames[frame].file;
        let Value::Name(name) = value else {
            self.error(file, span, "a `part` parameter takes a bare part name");
            return None;
        };
        if let Some(bound) = self.lookup_part_param(frame, name) {
            return bound;
        }
        self.part_named(file, name, span)
    }

    fn part_named(&mut self, file: FileId, name: &str, span: Span) -> Option<DeclRef> {
        let Some(decl) = self.lib.lookup(file, name) else {
            self.error(file, span, format!("`{name}` is not a part in scope"));
            return None;
        };
        if !matches!(self.lib.decl(decl), Decl::Part(_)) {
            self.error(file, span, format!("`{name}` is not a part"));
            return None;
        }
        Some(decl)
    }

    fn bind_part_parameter(
        &mut self,
        caller: usize,
        declared_in: FileId,
        param: &Param,
        args: &[Property],
    ) -> Option<DeclRef> {
        if let Some(arg) = args.iter().find(|a| a.key == param.name) {
            return self.part_argument(caller, &arg.value, arg.span);
        }
        match &param.default {
            Some(Value::Name(name)) => self.part_named(declared_in, name, param.span),
            Some(_) => {
                self.error(
                    declared_in,
                    param.span,
                    "a `part` default must be a bare part name",
                );
                None
            }
            None => None,
        }
    }

    /// A value as the placing frame sees it: a name that is a parameter becomes its value.
    fn resolve_value(&self, frame: usize, v: &Value) -> Value {
        match v {
            Value::Name(n) => self.lookup_param(frame, n).unwrap_or_else(|| v.clone()),
            _ => v.clone(),
        }
    }

    fn text_value(&self, frame: usize, v: &Value) -> String {
        match self.resolve_value(frame, v) {
            Value::String(s) | Value::Name(s) => s,
            Value::Integer(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => String::new(),
        }
    }

    fn resolve(&self, frame: usize, r: &Reference) -> Result<Resolved, String> {
        let head = &r.path[0];
        let mut cur = if head == "self" {
            match self.frames[frame].self_inst {
                Some(i) => Resolved::Instance(i),
                None => {
                    return Err("`self` is only a name in a block with a `place … as=self`".into());
                }
            }
        } else {
            match self.lookup_name(frame, head) {
                Some(Named::Instance(i)) => Resolved::Instance(i),
                Some(Named::Port(i, p)) => Resolved::Port(i, p),
                Some(Named::Pin(i, p)) => Resolved::Pin(i, p),
                None => return Err(format!("`{head}` is not a name in scope")),
            }
        };
        for seg in &r.path[1..] {
            let Resolved::Instance(i) = cur else {
                return Err(format!("`{seg}` follows something that is not a placement"));
            };
            match self.children.get(&(i, seg.clone())) {
                Some(&c) => cur = Resolved::Instance(c),
                None => {
                    return Err(format!(
                        "`{}` has nothing placed as `{seg}`",
                        self.instances[i].path
                    ));
                }
            }
        }
        for m in &r.members {
            cur = match cur {
                Resolved::Instance(i) => {
                    if let Some(pin) = self.pin_alias.get(&(i, m.clone())) {
                        Resolved::Pin(i, pin.clone())
                    } else if self.ports[i].iter().any(|p| &p.name == m) {
                        Resolved::Port(i, m.clone())
                    } else if let Some(e) = self.unbonded(i, m) {
                        return Err(e);
                    } else {
                        let path = if self.instances[i].path.is_empty() {
                            "the design"
                        } else {
                            &self.instances[i].path
                        };
                        return Err(format!("{path} has no pin or port `{m}`"));
                    }
                }
                Resolved::Port(i, p) => {
                    let port = self.ports[i].iter().find(|q| q.name == p).unwrap();
                    if port.lines.iter().any(|(l, _)| l == m) {
                        Resolved::Line(i, p, m.clone())
                    } else {
                        return Err(format!("port `{p}` has no line `{m}`"));
                    }
                }
                Resolved::Pin(_, p) => {
                    return Err(format!("`{m}` follows pin `{p}`, which has no members"));
                }
                Resolved::Line(_, _, l) => {
                    return Err(format!("`{m}` follows line `{l}`, which has no members"));
                }
            };
        }
        if let Some(pad) = &r.pad {
            match &cur {
                Resolved::Pin(i, p) => {
                    if let Some(body) = self.package_of(*i) {
                        let labels: Vec<&str> = body
                            .pads()
                            .filter(|d| &d.pin == p)
                            .map(|d| d.label.as_str())
                            .collect();
                        if !labels.contains(&pad.as_str()) {
                            return Err(format!(
                                "pin `{p}` has no pad `{pad}` in `{}`; its pads are {}",
                                body.name,
                                labels.join(", ")
                            ));
                        }
                    }
                }
                _ => return Err(format!("`@{pad}` selects a pad, and only a pin has pads")),
            }
        }
        Ok(cur)
    }

    /// Resolves a name relative to an instance: one of its pins, ports, or a port's line.
    fn resolve_in(&self, inst: usize, text: &str) -> Result<Resolved, String> {
        let r = Reference::parse(text).map_err(|e| e.message)?;
        if r.path.len() != 1 {
            return Err(format!(
                "`{text}` is a path; an answer names a pin, port or line of the side"
            ));
        }
        let head = &r.path[0];
        let mut cur = if let Some(pin) = self.pin_alias.get(&(inst, head.clone())) {
            Resolved::Pin(inst, pin.clone())
        } else if self.ports[inst].iter().any(|p| &p.name == head) {
            Resolved::Port(inst, head.clone())
        } else if let Some(e) = self.unbonded(inst, head) {
            return Err(e);
        } else {
            return Err(format!(
                "`{}` has no pin or port `{head}`",
                self.instances[inst].path
            ));
        };
        for m in &r.members {
            cur = match cur {
                Resolved::Port(i, p) => {
                    let port = self.ports[i].iter().find(|q| q.name == p).unwrap();
                    if port.lines.iter().any(|(l, _)| l == m) {
                        Resolved::Line(i, p, m.clone())
                    } else {
                        return Err(format!("port `{p}` has no line `{m}`"));
                    }
                }
                _ => return Err(format!("`{m}` follows something with no members")),
            };
        }
        Ok(cur)
    }

    fn port_lines(&self, inst: usize, port: &str) -> Vec<(String, Terminal)> {
        self.ports[inst]
            .iter()
            .find(|p| p.name == port)
            .map(|p| p.lines.clone())
            .unwrap_or_default()
    }

    fn point(&self, r: &Resolved) -> Result<Terminal, String> {
        match r {
            Resolved::Pin(i, p) => Ok(Terminal::Pin {
                inst: *i,
                pin: p.clone(),
            }),
            Resolved::Line(i, p, l) => self
                .port_lines(*i, p)
                .into_iter()
                .find(|(n, _)| n == l)
                .map(|(_, t)| t)
                .ok_or_else(|| format!("port `{p}` has no line `{l}`")),
            Resolved::Port(i, p) => {
                let lines = self.port_lines(*i, p);
                if lines.len() == 1 {
                    Ok(lines[0].1.clone())
                } else {
                    Err(format!(
                        "port `{p}` has lines {}; a point needs one",
                        lines
                            .iter()
                            .map(|(n, _)| format!("`{n}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                }
            }
            Resolved::Instance(i) => match self.shape(*i)? {
                Shape::Point(t) => Ok(t),
                Shape::Series(..) => Err(format!(
                    "`{}` is a series element (it has `a` and `b`), not a point",
                    self.instances[*i].path
                )),
            },
        }
    }

    fn shape(&self, inst: usize) -> Result<Shape, String> {
        let ports = &self.ports[inst];
        let single = |name: &str| {
            ports
                .iter()
                .find(|p| p.name == name && p.lines.len() == 1)
                .map(|p| p.lines[0].1.clone())
        };
        if let (Some(a), Some(b)) = (single("a"), single("b")) {
            return Ok(Shape::Series(a, b));
        }
        if let Some(node) = single("node") {
            return Ok(Shape::Point(node));
        }
        if ports.len() == 1 && ports[0].lines.len() == 1 {
            return Ok(Shape::Point(ports[0].lines[0].1.clone()));
        }
        let path = if self.instances[inst].path.is_empty() {
            "the design".to_string()
        } else {
            self.instances[inst].path.clone()
        };
        Err(format!(
            "`{path}` is neither a point (a `node` port) nor a series element (`a` and `b`); its ports are {}",
            if ports.is_empty() {
                "none".to_string()
            } else {
                ports
                    .iter()
                    .map(|p| format!("`{}`", p.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        ))
    }

    // --- Types ------------------------------------------------------------------------------------

    fn type_info(&self, file: FileId, name: &str) -> Option<TypeInfo> {
        let line = |n: &str| LineInfo {
            name: n.into(),
            optional: false,
            partner: None,
            ty: None,
        };
        let core: Option<Vec<LineInfo>> = match name {
            "terminal" | "output" | "pwm" | "analog" => Some(vec![line("node")]),
            "pwm-pair" => Some(vec![line("high"), line("low")]),
            "i2c" => Some(vec![line("sda"), line("scl")]),
            "spi" => Some(vec![line("sck"), line("mosi"), line("miso")]),
            "uart" => Some(vec![
                LineInfo {
                    name: "tx".into(),
                    optional: false,
                    partner: Some("rx".into()),
                    ty: None,
                },
                LineInfo {
                    name: "rx".into(),
                    optional: false,
                    partner: Some("tx".into()),
                    ty: None,
                },
                LineInfo {
                    name: "rts".into(),
                    optional: true,
                    partner: Some("cts".into()),
                    ty: None,
                },
                LineInfo {
                    name: "cts".into(),
                    optional: true,
                    partner: Some("rts".into()),
                    ty: None,
                },
            ]),
            "swd" => Some(vec![line("swdio"), line("swclk"), line("nrst")]),
            "power" => Some(vec![line("rail"), line("gnd")]),
            "can" => Some(vec![line("tx"), line("rx")]),
            _ => None,
        };
        if let Some(lines) = core {
            return Some(TypeInfo {
                name: name.into(),
                lines,
            });
        }
        let decl = self.lib.lookup(file, name)?;
        let Decl::Type(t) = self.lib.decl(decl) else {
            return None;
        };
        Some(type_of_decl(t))
    }

    fn type_lines(&self, file: FileId, ty: Option<&str>) -> Option<Vec<String>> {
        match ty {
            None => Some(vec!["node".into()]),
            Some(t) => self
                .type_info(file, t)
                .map(|t| t.lines.into_iter().map(|l| l.name).collect()),
        }
    }

    // --- Ports ------------------------------------------------------------------------------------

    fn block_ports(&mut self, inst: usize, frame: usize, file: FileId, block: &Block) {
        for port in block.ports() {
            let Some(lines) = self.type_lines(file, port.ty.as_deref()) else {
                self.error(
                    file,
                    port.span,
                    format!(
                        "`{}` is not a type in scope",
                        port.ty.clone().unwrap_or_default()
                    ),
                );
                continue;
            };
            let lines: Vec<(String, Terminal)> = lines
                .into_iter()
                .map(|l| {
                    (
                        l.clone(),
                        Terminal::Line {
                            inst,
                            port: port.name.clone(),
                            line: l,
                        },
                    )
                })
                .collect();
            self.port_facts(inst, frame, file, port, &lines);
            self.ports[inst].push(PortInfo {
                name: port.name.clone(),
                ty: port.ty.clone(),
                lines,
            });
        }
    }

    /// The facts a port states, contributed to its lines' nets: a fact on the port goes to
    /// every line, a fact inside a `line` to that line alone. A stated name is composed onto the
    /// path of what declares the port, like every name.
    fn port_facts(
        &mut self,
        inst: usize,
        frame: usize,
        file: FileId,
        port: &stackup_kdl::ast::Port,
        lines: &[(String, Terminal)],
    ) {
        let is_power = port.ty.as_deref() == Some("power");
        // A power port's `gnd` is a return.
        if is_power {
            for (n, t) in lines {
                if n == "gnd" {
                    self.returns.insert(t.clone());
                }
            }
        }
        let push = |this: &mut Self, fact: &stackup_kdl::ast::Fact, terminals: Vec<Terminal>| {
            let mut value = fact.value.clone();
            if fact.aspect == Aspect::Net
                && fact.fact == "name"
                && let Some(v) = &value
            {
                let text = this.text_value(frame, v);
                value = Some(Value::String(this.compose(inst, &text)));
            }
            for terminal in terminals {
                this.contributions.push(Contribution {
                    terminal,
                    aspect: fact.aspect,
                    fact: fact.fact.clone(),
                    rule: fact.rule,
                    value: value.clone(),
                    props: fact.props.clone(),
                    frame,
                    port: Some((inst, port.name.clone())),
                    file,
                    span: fact.span,
                });
            }
        };
        for item in &port.items {
            match item {
                PortItem::Fact(f) => {
                    // On the port, a fact goes to every line — except that a rail's voltage is
                    // not its return's.
                    let all = lines
                        .iter()
                        .filter(|(n, _)| !(is_power && n == "gnd" && f.fact == "voltage"))
                        .map(|(_, t)| t.clone())
                        .collect();
                    push(self, f, all);
                }
                PortItem::Line(l) => {
                    for li in &l.items {
                        match li {
                            LineItem::Fact(f) => {
                                let own = lines
                                    .iter()
                                    .filter(|(n, _)| n == &l.name)
                                    .map(|(_, t)| t.clone())
                                    .collect();
                                push(self, f, own);
                            }
                            LineItem::Require(r) => {
                                let own: Vec<(String, Terminal)> = lines
                                    .iter()
                                    .filter(|(n, _)| n == &l.name)
                                    .cloned()
                                    .collect();
                                self.requirements.push(Requirement {
                                    on: ReqOn::Port {
                                        inst,
                                        port: port.name.clone(),
                                        lines: own,
                                    },
                                    require: r.clone(),
                                    frame,
                                    file,
                                });
                            }
                            _ => {}
                        }
                    }
                }
                PortItem::Require(r) => {
                    self.requirements.push(Requirement {
                        on: ReqOn::Port {
                            inst,
                            port: port.name.clone(),
                            lines: lines.to_vec(),
                        },
                        require: r.clone(),
                        frame,
                        file,
                    });
                }
                PortItem::Assert(_) => {}
            }
        }
    }

    fn part_ports(&mut self, inst: usize, frame: usize, file: FileId, part: &Part) {
        for port in part.ports() {
            let mut lines = Vec::new();
            let declared: Vec<_> = port.lines().collect();
            if !declared.is_empty() {
                for line in declared {
                    let t = match &line.pin {
                        Some(pin) => match self.pin_alias.get(&(inst, pin.clone())) {
                            Some(canonical) => Terminal::Pin {
                                inst,
                                pin: canonical.clone(),
                            },
                            None if self.unbonded(inst, pin).is_some() => continue,
                            None => {
                                self.error(
                                    file,
                                    line.span,
                                    format!("`{}` has no pin `{pin}`", part.name),
                                );
                                continue;
                            }
                        },
                        None => Terminal::Line {
                            inst,
                            port: port.name.clone(),
                            line: line.name.clone(),
                        },
                    };
                    lines.push((line.name.clone(), t));
                }
            } else if let Some(pin) = &port.pin {
                match self.pin_alias.get(&(inst, pin.clone())) {
                    Some(canonical) => lines.push((
                        "node".into(),
                        Terminal::Pin {
                            inst,
                            pin: canonical.clone(),
                        },
                    )),
                    // A port over a pin this body does not bond is not a port of this placement.
                    None if self.unbonded(inst, pin).is_some() => continue,
                    None => self.error(
                        file,
                        port.span,
                        format!("`{}` has no pin `{pin}`", part.name),
                    ),
                }
            } else {
                match self.type_lines(file, port.ty.as_deref()) {
                    Some(names) => {
                        for l in names {
                            lines.push((
                                l.clone(),
                                Terminal::Line {
                                    inst,
                                    port: port.name.clone(),
                                    line: l,
                                },
                            ));
                        }
                    }
                    None => self.error(
                        file,
                        port.span,
                        format!(
                            "`{}` is not a type in scope",
                            port.ty.clone().unwrap_or_default()
                        ),
                    ),
                }
            }
            for (line, t) in &lines {
                if let Terminal::Pin { .. } = t {
                    self.pin_port
                        .entry(t.clone())
                        .or_insert((port.name.clone(), line.clone()));
                }
            }
            self.port_facts(inst, frame, file, port, &lines);
            self.ports[inst].push(PortInfo {
                name: port.name.clone(),
                ty: port.ty.clone(),
                lines,
            });
        }
    }

    // --- Bodies -----------------------------------------------------------------------------------

    fn body(&mut self, frame: usize, items: &'a [BlockItem]) {
        let mut scopes: Vec<(usize, &'a [BlockItem])> = Vec::new();
        self.pass1(frame, items, &mut scopes);
        self.pass2(frame, items);
        for (f, items) in scopes {
            self.pass2(f, items);
        }
    }

    fn pass1(
        &mut self,
        frame: usize,
        items: &'a [BlockItem],
        scopes: &mut Vec<(usize, &'a [BlockItem])>,
    ) {
        for item in items {
            match item {
                BlockItem::Place(p) => self.place(frame, p),
                BlockItem::Scope(s) => {
                    let file = self.frames[frame].file;
                    let parent = self.frames[frame].inst;
                    let path = self.compose(parent, &s.name);
                    if self.children.contains_key(&(parent, s.name.clone())) {
                        self.error(file, s.span, format!("`{}` is already placed here", s.name));
                        continue;
                    }
                    let inst = self.new_instance(path, Some(parent), Kind::Scope, file, s.span);
                    self.children.insert((parent, s.name.clone()), inst);
                    self.frames[frame]
                        .names
                        .insert(s.name.clone(), Named::Instance(inst));
                    let f = self.new_frame(inst, file, Some(frame));
                    scopes.push((f, &s.items));
                    self.pass1(f, &s.items, scopes);
                }
                BlockItem::Stock(s) => {
                    for st in &s.items {
                        if st.name == "packages"
                            && let Some(p) = st.props.iter().find(|p| p.key == "imperial")
                        {
                            self.imperial = p.value.as_str().map(str::to_string);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn pass2(&mut self, frame: usize, items: &'a [BlockItem]) {
        for item in items {
            match item {
                BlockItem::Circuit(c) => self.circuit(frame, c),
                BlockItem::Connect(c) => self.connect(frame, c),
                BlockItem::Fact(f) => self.terminal_fact(frame, f),
                BlockItem::Nc(n) => {
                    let file = self.frames[frame].file;
                    for pin in &n.pins {
                        match self.resolve(frame, pin) {
                            Ok(Resolved::Pin(i, p)) => {
                                self.nc
                                    .push((Terminal::Pin { inst: i, pin: p }, file, n.span))
                            }
                            Ok(_) => self.error(file, n.span, format!("`{pin}` is not a pin")),
                            Err(e) => self.error(file, n.span, e),
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // --- Placement --------------------------------------------------------------------------------

    fn place(&mut self, frame: usize, p: &'a Place) {
        let file = self.frames[frame].file;
        let selected = self.lookup_part_param(frame, &p.what);
        let Some(decl) = selected.unwrap_or_else(|| self.lib.lookup(file, &p.what)) else {
            if selected.is_some() {
                return;
            }
            self.error(
                file,
                p.span,
                format!("`{}` is not a part or block in scope", p.what),
            );
            return;
        };
        match self.lib.decl(decl) {
            Decl::Part(part) => self.place_part(frame, p, decl, part),
            Decl::Block(block) if block.kind == BlockKind::Block => {
                self.place_block(frame, p, decl, block)
            }
            other => self.error(
                file,
                p.span,
                format!(
                    "`{}` is a {}, and only a part or block can be placed",
                    p.what,
                    other.what()
                ),
            ),
        }
    }

    fn place_part(&mut self, frame: usize, p: &'a Place, decl: DeclRef, part: &'a Part) {
        let file = self.frames[frame].file;
        let is_self = p.is_self();
        let parent = self.frames[frame].inst;
        let (path, name) = match (&p.name, is_self) {
            (Some(n), false) => (self.compose(parent, n), Some(n.clone())),
            (None, true) => (self.instances[parent].path.clone(), None),
            (Some(_), true) => {
                self.error(
                    file,
                    p.span,
                    "`as=self` takes the block's own path; it cannot also have a name",
                );
                return;
            }
            (None, false) => {
                self.error(file, p.span, format!("`place {}` needs a name", p.what));
                return;
            }
        };
        if let Some(n) = &name
            && self.children.contains_key(&(parent, n.clone()))
        {
            self.error(file, p.span, format!("`{n}` is already placed here"));
            return;
        }
        if is_self && self.frames[frame].self_inst.is_some() {
            self.error(file, p.span, "a block places at most one part `as=self`");
            return;
        }
        let inst = self.new_instance(
            path,
            Some(parent),
            Kind::Part {
                decl,
                package: None,
            },
            file,
            p.span,
        );
        self.add_ignores(inst, file, p);
        if let Some(n) = &name {
            self.children.insert((parent, n.clone()), inst);
            self.frames[frame]
                .names
                .insert(n.clone(), Named::Instance(inst));
        } else {
            self.frames[frame].self_inst = Some(inst);
        }

        // Arguments: the part's parameters, its ports, and the language's properties.
        let mut args: Vec<Property> = p.args.clone();
        let own_args = args.len();
        if is_self {
            let extra = std::mem::take(&mut self.frames[frame].self_props);
            args.extend(extra);
        }

        // The package first, because it decides which pins this placement has: a pin the chosen
        // body does not bond is not on the part. The first listed is the default.
        let package = match args.iter().find(|a| a.key == "package") {
            Some(a) => {
                let n = self.text_value(frame, &a.value);
                match part.packages().position(|k| k.name == n) {
                    Some(i) => Some(i),
                    None => {
                        let bodies = part
                            .packages()
                            .map(|k| format!("`{}`", k.name))
                            .collect::<Vec<_>>()
                            .join(", ");
                        self.error(
                            file,
                            a.span,
                            if bodies.is_empty() {
                                format!("`{}` has no package `{n}`; it names none", part.name)
                            } else {
                                format!(
                                    "`{}` has no package `{n}`; it comes in {bodies}",
                                    part.name
                                )
                            },
                        );
                        None
                    }
                }
            }
            None => part.packages().next().map(|_| 0),
        };
        if let Kind::Part { package: slot, .. } = &mut self.instances[inst].kind {
            *slot = package;
        }
        let body = package.and_then(|i| part.packages().nth(i));

        // Pins, and every name they answer to — those the body bonds. A part with no package
        // at all has every pin it declares.
        for pin in part.pins() {
            if body.is_some_and(|b| !b.bonds(&pin.name)) {
                continue;
            }
            self.pin_alias
                .insert((inst, pin.name.clone()), pin.name.clone());
            for also in &pin.also {
                self.pin_alias
                    .insert((inst, also.name.clone()), pin.name.clone());
            }
        }
        let param_names: HashSet<&str> = part.params().map(|q| q.name.as_str()).collect();
        let port_names: HashSet<&str> = part.ports().map(|q| q.name.as_str()).collect();
        let mut params: HashMap<String, Value> = part
            .params()
            .filter_map(|q| q.default.clone().map(|d| (q.name.clone(), d)))
            .collect();
        let mut port_args: Vec<(String, Value, Span)> = Vec::new();
        for (arg_index, a) in args.iter().enumerate() {
            let v = self.resolve_value(frame, &a.value);
            if param_names.contains(a.key.as_str()) {
                params.insert(a.key.clone(), v);
            } else if port_names.contains(a.key.as_str()) {
                port_args.push((a.key.clone(), a.value.clone(), a.span));
            } else {
                match a.key.as_str() {
                    "value" => self
                        .pending_values
                        .push((inst, frame, a.value.clone(), a.span)),
                    "intent" => {
                        self.instances[inst].intent = Some(self.text_value(frame, &a.value))
                    }
                    "note" => self
                        .pending_notes
                        .push((inst, frame, a.value.clone(), a.span)),
                    "footprint" => {
                        self.instances[inst].footprint = Some(self.text_value(frame, &a.value))
                    }
                    "required_voltage" => {
                        self.pending_required_voltages
                            .push((inst, frame, a.value.clone(), a.span))
                    }
                    "bom_value" | "manufacturer" | "mpn" | "lcsc" | "mouser" | "digikey"
                    | "series" | "voltage" | "dissipation" | "current" | "tolerance"
                    | "dielectric" => {
                        let value = self.text_value(frame, &a.value);
                        self.instances[inst].fields.insert(a.key.clone(), value);
                    }
                    "hand" => match self.resolve_value(frame, &a.value) {
                        Value::Bool(hand) => self.instances[inst].hand = hand,
                        _ => self.error(file, a.span, "`hand=` needs #true or #false"),
                    },
                    "anchor" => {
                        let origin = if matches!(&a.value, Value::Name(n) if n == "anchor") {
                            self.frames[frame].anchor_origin.unwrap_or(frame)
                        } else if arg_index >= own_args {
                            self.frames[frame].self_props_frame.unwrap_or(frame)
                        } else {
                            frame
                        };
                        if self.resolve_value(frame, &a.value) != Value::Null {
                            self.pending_anchors.push((
                                inst,
                                frame,
                                origin,
                                a.value.clone(),
                                a.span,
                            ));
                        }
                    }
                    "spot" => {
                        let spot = self.text_value(frame, &a.value);
                        let numbers: Vec<_> = spot.split_whitespace().collect();
                        if numbers.len() != 3
                            || numbers
                                .iter()
                                .any(|n| !n.parse::<f64>().is_ok_and(f64::is_finite))
                        {
                            self.error(file, a.span, "`spot=` needs three finite numbers: dx dy rotation (mm, mm, degrees)");
                        } else {
                            self.instances[inst].spot = Some(spot);
                        }
                    }
                    "package" => {}
                    "designator" => {
                        if self.resolve_value(frame, &a.value) != Value::Null {
                            self.instances[inst].designator_word =
                                Some(self.text_value(frame, &a.value));
                        }
                    }
                    "reference" => {
                        self.instances[inst].reference_prefix =
                            Some(self.text_value(frame, &a.value))
                    }
                    "as" => {}
                    other => self.error(
                        file,
                        a.span,
                        format!("`{}` has no parameter or port `{other}`", part.name),
                    ),
                }
            }
        }
        for q in part.params() {
            if !params.contains_key(&q.name) {
                self.error(
                    file,
                    p.span,
                    format!("`{}` needs `{}=`, which has no default", part.name, q.name),
                );
            }
        }
        let part_params: HashMap<String, Option<DeclRef>> = part
            .params()
            .filter(|q| q.ty == "part")
            .map(|q| {
                (
                    q.name.clone(),
                    self.bind_part_parameter(frame, decl.file, q, &args),
                )
            })
            .collect();
        // `package` reads as a parameter inside the part, so a child block can be `when
        // "package == lqfp48"` and a `text` can say which body this is.
        if let Some(b) = body {
            params.insert("package".into(), Value::Name(b.name.clone()));
        }

        let features = self.features(file, &part.name, part.blocks().collect(), &p.features);

        // The part's own frame: its pins, ports and parameters by bare name, closed to the
        // outside. Its ports' facts are read there, and its child blocks elaborate under it.
        let pf = self.new_frame(inst, decl.file, None);
        self.frames[pf].params = params;
        self.frames[pf].part_params = part_params;
        self.frames[pf].features = features.clone();
        for pin in part.pins() {
            if !self.pin_alias.contains_key(&(inst, pin.name.clone())) {
                continue;
            }
            self.frames[pf]
                .names
                .insert(pin.name.clone(), Named::Pin(inst, pin.name.clone()));
            for also in &pin.also {
                self.frames[pf]
                    .names
                    .insert(also.name.clone(), Named::Pin(inst, pin.name.clone()));
            }
            for r in &pin.requires {
                self.requirements.push(Requirement {
                    on: ReqOn::Pin {
                        inst,
                        pin: pin.name.clone(),
                        terminal: Terminal::Pin {
                            inst,
                            pin: pin.name.clone(),
                        },
                    },
                    require: r.clone(),
                    frame: pf,
                    file: decl.file,
                });
            }
        }
        for port in part.ports() {
            self.frames[pf]
                .names
                .insert(port.name.clone(), Named::Port(inst, port.name.clone()));
        }
        self.part_ports(inst, pf, decl.file, part);

        for (port, value, span) in port_args {
            self.pending_port_joins
                .push((frame, inst, port, value, span));
        }

        self.child_blocks(inst, pf, decl, part.blocks(), &features);
    }

    /// Which of a placement's features are on: each child block is on by default, or asked for,
    /// or switched off; a `when` block follows its condition and cannot be named.
    fn features(
        &mut self,
        file: FileId,
        what: &str,
        blocks: Vec<&'a Block>,
        asked: &[Feature],
    ) -> HashMap<String, bool> {
        let mut features: HashMap<String, bool> = blocks
            .iter()
            .filter(|b| b.when().is_none())
            .map(|b| (b.name.clone(), b.is_default()))
            .collect();
        for f in asked {
            match features.get_mut(&f.name) {
                Some(on) => *on = f.on,
                None if blocks.iter().any(|b| b.name == f.name) => {
                    self.error(
                        file,
                        f.span,
                        format!(
                            "`{}` is placed by its `when` condition, not by name",
                            f.name
                        ),
                    );
                }
                None => self.error(
                    file,
                    f.span,
                    format!("`{what}` has no feature `{}`", f.name),
                ),
            }
        }
        features
    }

    /// Elaborates the child blocks that are on — by feature or by `when` — under the placement
    /// `inst`, each in a frame that opens onto the placement's own (`pf`), so a child sees the
    /// part's pins and ports, or the block's ports and placements, by bare name.
    fn child_blocks(
        &mut self,
        inst: usize,
        pf: usize,
        decl: DeclRef,
        blocks: impl Iterator<Item = &'a Block>,
        features: &HashMap<String, bool>,
    ) {
        for block in blocks {
            let on = match block.when() {
                Some(w) => match static_values::when(self, pf, &w.expr) {
                    Ok(on) => on,
                    Err(e) => {
                        self.error(decl.file, w.span, format!("in `when`: {e}"));
                        false
                    }
                },
                None => features[&block.name],
            };
            if !on {
                continue;
            }
            let path = self.compose(inst, &block.name);
            let bi = self.new_instance(
                path,
                Some(inst),
                Kind::Block { decl },
                decl.file,
                block.span,
            );
            self.children.insert((inst, block.name.clone()), bi);
            let bf = self.new_frame(bi, decl.file, Some(pf));
            self.block_ports(bi, bf, decl.file, block);
            self.body(bf, &block.items);
        }
    }

    fn place_block(&mut self, frame: usize, p: &'a Place, decl: DeclRef, block: &'a Block) {
        let file = self.frames[frame].file;
        let parent = self.frames[frame].inst;
        if p.is_self() {
            self.error(file, p.span, "only a part can be placed `as=self`");
            return;
        }
        let Some(name) = &p.name else {
            self.error(file, p.span, format!("`place {}` needs a name", p.what));
            return;
        };
        if self.children.contains_key(&(parent, name.clone())) {
            self.error(file, p.span, format!("`{name}` is already placed here"));
            return;
        }
        let path = self.compose(parent, name);
        let inst = self.new_instance(path, Some(parent), Kind::Block { decl }, file, p.span);
        self.add_ignores(inst, file, p);
        self.children.insert((parent, name.clone()), inst);
        self.frames[frame]
            .names
            .insert(name.clone(), Named::Instance(inst));
        let features = self.features(file, &block.name, block.blocks().collect(), &p.features);

        let has_self = block.places().any(Place::is_self);
        let param_names: HashSet<&str> = block.params().map(|q| q.name.as_str()).collect();
        let port_names: HashSet<&str> = block.ports().map(|q| q.name.as_str()).collect();
        let mut params: HashMap<String, Value> = block
            .params()
            .filter_map(|q| q.default.clone().map(|d| (q.name.clone(), d)))
            .collect();
        let mut port_args: Vec<(String, Value, Span)> = Vec::new();
        let mut self_props = Vec::new();
        for a in &p.args {
            let v = self.resolve_value(frame, &a.value);
            if param_names.contains(a.key.as_str()) {
                params.insert(a.key.clone(), v);
            } else if port_names.contains(a.key.as_str()) {
                port_args.push((a.key.clone(), a.value.clone(), a.span));
            } else if a.key == "note" {
                self.pending_notes
                    .push((inst, frame, a.value.clone(), a.span));
            } else if LANGUAGE_PROPS.contains(&a.key.as_str()) && has_self {
                self_props.push(Property {
                    key: a.key.clone(),
                    value: v,
                    span: a.span,
                });
            } else if LANGUAGE_PROPS.contains(&a.key.as_str()) {
                self.error(
                    file,
                    a.span,
                    format!(
                        "`{}=` is for a part; `{}` is a block with no `as=self` part to take it",
                        a.key, block.name
                    ),
                );
            } else {
                self.error(
                    file,
                    a.span,
                    format!("`{}` has no parameter or port `{}`", block.name, a.key),
                );
            }
        }
        for q in block.params() {
            if !params.contains_key(&q.name) {
                self.error(
                    file,
                    p.span,
                    format!("`{}` needs `{}=`, which has no default", block.name, q.name),
                );
            }
        }
        let part_params: HashMap<String, Option<DeclRef>> = block
            .params()
            .filter(|q| q.ty == "part")
            .map(|q| {
                (
                    q.name.clone(),
                    self.bind_part_parameter(frame, decl.file, q, &p.args),
                )
            })
            .collect();

        let bf = self.new_frame(inst, decl.file, None);
        self.frames[bf].params = params;
        self.frames[bf].part_params = part_params;
        self.frames[bf].self_props = self_props;
        self.frames[bf].self_props_frame = Some(frame);
        if param_names.contains("anchor") && p.arg("anchor").is_some() {
            self.frames[bf].anchor_origin = match p.arg("anchor") {
                Some(Value::Name(n)) if n == "anchor" => {
                    Some(self.frames[frame].anchor_origin.unwrap_or(frame))
                }
                _ => Some(frame),
            };
        }
        self.frames[bf].features = features.clone();
        self.block_ports(inst, bf, decl.file, block);
        for port in block.ports() {
            self.frames[bf]
                .names
                .insert(port.name.clone(), Named::Port(inst, port.name.clone()));
        }
        self.body(bf, &block.items);
        for (port, value, span) in port_args {
            self.pending_port_joins
                .push((frame, inst, port, value, span));
        }
        // The block's features, after its body: a child reaches the block's placements by bare
        // name, so they have to exist first.
        self.child_blocks(inst, bf, decl, block.blocks(), &features);
    }

    /// A port argument: the link its type implies, from what the argument names to the child's
    /// port.
    fn join_pending_ports(&mut self) {
        for (frame, child, port, value, span) in std::mem::take(&mut self.pending_port_joins) {
            self.join_port(frame, child, &port, &value, span);
        }
    }

    fn join_port(&mut self, frame: usize, child: usize, port: &str, value: &Value, span: Span) {
        let file = self.frames[frame].file;
        let Some(text) = value.as_str() else {
            self.error(
                file,
                span,
                format!(
                    "`{port}=` must name what the port joins, not {}",
                    value.describe()
                ),
            );
            return;
        };
        let target = match Reference::parse(text)
            .map_err(|e| e.message)
            .and_then(|r| self.resolve(frame, &r))
        {
            Ok(t) => t,
            Err(e) => {
                self.error(file, span, e);
                return;
            }
        };
        let info = self.ports[child].iter().find(|p| p.name == port).unwrap();
        let ty = info.ty.clone();
        let lines = info.lines.clone();
        if lines.len() == 1 {
            match self.point(&target) {
                Ok(t) => self.nets.union(lines[0].1.clone(), t),
                Err(e) => self.error(file, span, e),
            }
            return;
        }
        let Some(ty) = ty.and_then(|t| self.type_info(file, &t)) else {
            self.error(
                file,
                span,
                format!("port `{port}` has a type this file cannot see"),
            );
            return;
        };
        let Some(from) = self.side_lines(frame, &ty, &target, &[], span) else {
            return;
        };
        let to: HashMap<String, Terminal> = lines.into_iter().collect();
        self.join(file, &ty, &from, &to, &HashSet::new(), span);
    }

    // --- Wiring -------------------------------------------------------------------------------------

    fn circuit(&mut self, frame: usize, c: &'a stackup_kdl::ast::Circuit) {
        let file = self.frames[frame].file;
        if let Some(from) = &c.from {
            let start = self.resolve(frame, from).and_then(|r| match r {
                Resolved::Instance(i) => self.shape(i),
                other => self.point(&other).map(Shape::Point),
            });
            let mut cursor = match start {
                Ok(Shape::Point(t)) => t,
                Ok(Shape::Series(..)) => {
                    self.error(
                        file,
                        c.span,
                        "`from` must name a terminal, not a series element",
                    );
                    return;
                }
                Err(err) => {
                    self.error(file, c.span, format!("`{from}`: {err}"));
                    return;
                }
            };
            // Check the whole path before joining any nets. A malformed later line must not
            // leave the earlier lines wired in a model that already has a finding.
            let mut sections = Vec::new();
            for (line, leg) in c.legs.iter().enumerate() {
                let mut shapes = Vec::new();
                for (position, element) in leg.elements.iter().enumerate() {
                    let shape = self.resolve(frame, element).and_then(|r| match r {
                        Resolved::Instance(i) => self.shape(i),
                        other => self.point(&other).map(Shape::Point),
                    });
                    let shape = match shape {
                        Ok(s) => s,
                        Err(err) => {
                            self.error(file, leg.span, format!("`{element}`: {err}"));
                            return;
                        }
                    };
                    let last_element = position + 1 == leg.elements.len();
                    let last_line = line + 1 == c.legs.len();
                    match &shape {
                        Shape::Point(_) if last_element && !last_line => {
                            self.error(
                                file,
                                leg.span,
                                "a `to` ending at a terminal must be the final `to`",
                            );
                            return;
                        }
                        Shape::Series(..) if !last_element => {
                            self.error(file, leg.span, "a series element must end its `to` line");
                            return;
                        }
                        Shape::Series(..) if last_line => {
                            self.error(file, leg.span, "the final `to` must end at a terminal");
                            return;
                        }
                        _ => {}
                    }
                    shapes.push(shape);
                }
                if let Some(name) = leg.props.iter().find(|p| p.key == "name")
                    && name.value.as_str().is_none()
                {
                    self.error(file, name.span, "`name=` must be a name or string");
                    return;
                }
                sections.push(shapes);
            }
            for (leg, shapes) in c.legs.iter().zip(sections) {
                let head = cursor.clone();
                for shape in shapes {
                    match shape {
                        Shape::Point(t) => {
                            self.nets.union(cursor, t.clone());
                            cursor = t;
                        }
                        Shape::Series(a, b) => {
                            self.nets.union(cursor, a);
                            cursor = b;
                        }
                    }
                }
                if let Some(name) = leg.props.iter().find(|p| p.key == "name") {
                    let n = name.value.as_str().expect("checked above");
                    let inst = self.frames[frame].inst;
                    self.contributions.push(Contribution {
                        terminal: head,
                        aspect: Aspect::Net,
                        fact: "name".into(),
                        rule: Rule::Set,
                        value: Some(Value::String(self.compose(inst, n))),
                        props: Vec::new(),
                        frame,
                        port: None,
                        file,
                        span: name.span,
                    });
                }
            }
            return;
        }
        let mut shapes = Vec::new();
        for e in &c.elements {
            let shape = self.resolve(frame, e).and_then(|r| match r {
                Resolved::Instance(i) => self.shape(i),
                other => self.point(&other).map(Shape::Point),
            });
            match shape {
                Ok(s) => shapes.push(s),
                Err(err) => {
                    self.error(file, c.span, format!("`{e}`: {err}"));
                    return;
                }
            }
        }
        // `name=` names the net the path starts on. A circuit through a series element has
        // several nets, and the head is the one a circuit written in flow order begins at the
        // source of, so `circuit mcu.TXD0 Rrgb led.din name="DIN"` names the MCU's side.
        let named = c.props.iter().find(|p| p.key == "name");
        let mut prev: Option<Terminal> = None;
        let mut first: Option<Terminal> = None;
        for s in shapes {
            match s {
                Shape::Point(t) => {
                    if let Some(p) = &prev {
                        self.nets.union(p.clone(), t.clone());
                    }
                    first.get_or_insert(t.clone());
                    prev = Some(t);
                }
                Shape::Series(a, b) => {
                    if let Some(p) = &prev {
                        self.nets.union(p.clone(), a.clone());
                    }
                    first.get_or_insert(a);
                    prev = Some(b);
                }
            }
        }
        if let Some(name) = named
            && let (Some(t), Some(n)) = (first, name.value.as_str())
        {
            let inst = self.frames[frame].inst;
            self.contributions.push(Contribution {
                terminal: t,
                aspect: Aspect::Net,
                fact: "name".into(),
                rule: Rule::Set,
                value: Some(Value::String(self.compose(inst, n))),
                props: Vec::new(),
                frame,
                port: None,
                file,
                span: name.span,
            });
        }
    }

    /// `set <terminal> <aspect>.<fact> …` in a block: the contribution a port line would make,
    /// on whatever the terminal resolves to — a pin, a port line, a block with a `node` port. A
    /// name is composed onto the block's path, as a port line's is.
    fn terminal_fact(&mut self, frame: usize, f: &'a stackup_kdl::ast::TerminalFact) {
        let file = self.frames[frame].file;
        let resolved = self.resolve(frame, &f.terminal).and_then(|r| match r {
            Resolved::Instance(i) => self.shape(i).and_then(|s| match s {
                Shape::Point(t) => Ok(t),
                Shape::Series(..) => {
                    Err("a series element has two nets; state the fact on one of its ends".into())
                }
            }),
            other => self.point(&other),
        });
        let terminal = match resolved {
            Ok(t) => t,
            Err(err) => {
                self.error(file, f.span, format!("`{}`: {err}", f.terminal));
                return;
            }
        };
        let fact = &f.fact;
        let mut value = fact.value.clone();
        if fact.aspect == Aspect::Net
            && fact.fact == "name"
            && let Some(v) = &value
        {
            let text = self.text_value(frame, v);
            let inst = self.frames[frame].inst;
            value = Some(Value::String(self.compose(inst, &text)));
        }
        self.contributions.push(Contribution {
            terminal,
            aspect: fact.aspect,
            fact: fact.fact.clone(),
            rule: fact.rule,
            value,
            props: fact.props.clone(),
            frame,
            port: None,
            file,
            span: fact.span,
        });
    }

    fn connect(&mut self, frame: usize, c: &'a stackup_kdl::ast::Connect) {
        let file = self.frames[frame].file;
        let Some(ty) = self.type_info(file, &c.ty) else {
            self.error(file, c.span, format!("`{}` is not a type in scope", c.ty));
            return;
        };
        let mut from: HashMap<String, Terminal> = HashMap::new();
        let mut to: HashMap<String, Terminal> = HashMap::new();
        for side in &c.sides {
            let target = match self.resolve(frame, &side.target) {
                Ok(t) => t,
                Err(e) => {
                    self.error(file, side.span, e);
                    continue;
                }
            };
            let Some(lines) = self.side_lines(frame, &ty, &target, &side.answers, side.span) else {
                continue;
            };
            let into = match side.role {
                stackup_kdl::ast::SideRole::From => &mut from,
                stackup_kdl::ast::SideRole::To => &mut to,
            };
            for (l, t) in lines {
                if into.insert(l.clone(), t).is_some() {
                    self.error(
                        file,
                        side.span,
                        format!("`{l}` is supplied twice on the same side of this link"),
                    );
                }
            }
        }
        // Lines the link leaves unbound: nothing on the `to` side receives them, and a pin the
        // `from` side answered for one is a declared no-connect — spent on the instance it pins,
        // and joined to nothing else.
        let mut unbound = HashSet::new();
        for u in &c.unbound {
            for l in &u.lines {
                let Some(line) = ty.lines.iter().find(|t| &t.name == l) else {
                    self.error(
                        file,
                        u.span,
                        format!("`{}` has no line `{l}` to leave unbound", ty.name),
                    );
                    continue;
                };
                let partner = line.partner.clone().unwrap_or_else(|| line.name.clone());
                if to.contains_key(&partner) {
                    self.error(
                        file,
                        u.span,
                        format!("`{l}` is declared unbound, but the `to` side receives it"),
                    );
                }
                if let Some(t @ Terminal::Pin { .. }) = from.get(l) {
                    self.nc.push((t.clone(), file, u.span));
                }
                unbound.insert(l.clone());
            }
        }
        self.check_connection_peripherals(file, c, &ty, &from);
        self.join(file, &ty, &from, &to, &unbound, c.span);
    }

    /// What a side offers, line by line.
    fn side_lines(
        &mut self,
        frame: usize,
        ty: &TypeInfo,
        target: &Resolved,
        answers: &[Property],
        span: Span,
    ) -> Option<HashMap<String, Terminal>> {
        let file = self.frames[frame].file;
        let mut out = HashMap::new();
        if !answers.is_empty() {
            let Resolved::Instance(inst) = target else {
                self.error(
                    file,
                    span,
                    "answers (`line=pin`) go on a placement, not on a port or pin",
                );
                return None;
            };
            let mut stamps: Vec<(Terminal, Vec<Candidate>, Span, bool)> = Vec::new();
            for a in answers {
                let Some(line) = ty.lines.iter().find(|l| l.name == a.key) else {
                    self.error(
                        file,
                        a.span,
                        format!("`{}` is not a line of `{}`", a.key, ty.name),
                    );
                    continue;
                };
                let Some(text) = a.value.as_str() else {
                    self.error(
                        file,
                        a.span,
                        format!(
                            "`{}=` names a pin or port, not {}",
                            a.key,
                            a.value.describe()
                        ),
                    );
                    continue;
                };
                match self
                    .resolve_in(*inst, text)
                    .and_then(|r| self.point(&r).map(|t| (r, t)))
                {
                    Ok((r, t)) => {
                        if let Resolved::Pin(_, pin) = &r
                            && let Some(c) =
                                self.check_peripheral(file, a.span, *inst, ty, line, text, pin)
                        {
                            let shared = line.ty.is_none() && ty.lines.len() > 1;
                            stamps.push((t.clone(), c, a.span, shared));
                        }
                        out.insert(a.key.clone(), t);
                    }
                    Err(e) => self.error(file, a.span, e),
                }
            }
            self.stamp(frame, file, span, stamps);
            return Some(out);
        }
        match target {
            Resolved::Instance(inst) => {
                let matching: Vec<usize> = self.ports[*inst]
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| compatible(p.ty.as_deref(), &ty.name))
                    .map(|(i, _)| i)
                    .collect();
                let path = if self.instances[*inst].path.is_empty() {
                    "the design".to_string()
                } else {
                    self.instances[*inst].path.clone()
                };
                match matching.as_slice() {
                    [one] => {
                        let port = &self.ports[*inst][*one];
                        let type_line = ty.lines.len() == 1 && port.lines.len() == 1;
                        for (l, t) in &port.lines {
                            let key = if type_line {
                                ty.lines[0].name.clone()
                            } else {
                                l.clone()
                            };
                            out.insert(key, t.clone());
                        }
                    }
                    [] => {
                        let ports = self.ports[*inst]
                            .iter()
                            .map(|p| format!("`{}`", p.name))
                            .collect::<Vec<_>>()
                            .join(", ");
                        self.error(
                            file,
                            span,
                            format!(
                                "`{path}` has no port of type `{}`; its ports are {}",
                                ty.name,
                                if ports.is_empty() {
                                    "none".into()
                                } else {
                                    ports
                                }
                            ),
                        );
                        return None;
                    }
                    many => {
                        let names = many
                            .iter()
                            .map(|i| format!("`{}`", self.ports[*inst][*i].name))
                            .collect::<Vec<_>>()
                            .join(", ");
                        self.error(
                            file,
                            span,
                            format!(
                                "`{path}` has several ports of type `{}` ({names}); name one",
                                ty.name
                            ),
                        );
                        return None;
                    }
                }
            }
            Resolved::Port(inst, port) => {
                let lines = self.port_lines(*inst, port);
                let type_line = ty.lines.len() == 1 && lines.len() == 1;
                for (l, t) in lines {
                    let key = if type_line {
                        ty.lines[0].name.clone()
                    } else {
                        l
                    };
                    out.insert(key, t);
                }
            }
            Resolved::Pin(..) | Resolved::Line(..) => {
                if ty.lines.len() != 1 {
                    self.error(
                        file,
                        span,
                        format!(
                            "one pin or line cannot satisfy `{}`, which has lines {}",
                            ty.name,
                            ty.lines
                                .iter()
                                .map(|l| format!("`{}`", l.name))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    );
                    return None;
                }
                match self.point(target) {
                    Ok(t) => {
                        if let Resolved::Pin(inst, pin) = target {
                            let line = &ty.lines[0];
                            if let Some(c) =
                                self.check_peripheral(file, span, *inst, ty, line, pin, pin)
                            {
                                self.stamp(frame, file, span, vec![(t.clone(), c, span, false)]);
                            }
                        }
                        out.insert(ty.lines[0].name.clone(), t);
                    }
                    Err(e) => {
                        self.error(file, span, e);
                        return None;
                    }
                }
            }
        }
        Some(out)
    }

    /// Joins each line of `from` to its partner on `to`, and says which lines nobody supplied.
    fn join(
        &mut self,
        file: FileId,
        ty: &TypeInfo,
        from: &HashMap<String, Terminal>,
        to: &HashMap<String, Terminal>,
        unbound: &HashSet<String>,
        span: Span,
    ) {
        for line in &ty.lines {
            if unbound.contains(&line.name) {
                continue;
            }
            let partner = line.partner.clone().unwrap_or_else(|| line.name.clone());
            match (from.get(&line.name), to.get(&partner)) {
                (Some(a), Some(b)) => self.nets.union(a.clone(), b.clone()),
                (None, _) if !line.optional && !from.is_empty() => {
                    self.warning(
                        file,
                        span,
                        format!(
                            "incomplete link: nothing on the `from` side supplies `{}`",
                            line.name
                        ),
                    );
                }
                (_, None) if !line.optional && !to.is_empty() => {
                    self.warning(
                        file,
                        span,
                        format!("incomplete link: nothing on the `to` side receives `{partner}`"),
                    );
                }
                _ => {}
            }
        }
        if from.is_empty() {
            self.warning(file, span, "incomplete link: the `from` side is unanswered");
        }
        if to.is_empty() {
            self.warning(file, span, "incomplete link: the `to` side is unanswered");
        }
    }

    /// Whether some instance of the right kind can put this line of the type on this pin. Only
    /// the per-line check: whether one instance serves every line of the link at once is the
    /// checker's, not elaboration's.
    #[allow(clippy::too_many_arguments)]
    fn check_peripheral(
        &mut self,
        file: FileId,
        span: Span,
        inst: usize,
        ty: &TypeInfo,
        line: &LineInfo,
        given: &str,
        pin: &str,
    ) -> Option<Vec<Candidate>> {
        let part = self.part_of(inst)?;
        part.peripherals().next()?;
        let effective = line.ty.as_deref().unwrap_or(&ty.name);
        let (kind, row): (&str, RowMatch) = match effective {
            "i2c" | "spi" | "uart" | "swd" | "can" => {
                let name = line.name.clone();
                (effective, Box::new(move |r: &str| r == name))
            }
            "analog" => ("adc", Box::new(|_| true)),
            "output" => ("gpio", Box::new(|_| true)),
            "pwm" => (
                "timer",
                Box::new(|r: &str| r.starts_with("ch") && !r.ends_with('n')),
            ),
            "pwm-pair" if line.name == "high" => (
                "timer",
                Box::new(|r: &str| r.starts_with("ch") && !r.ends_with('n')),
            ),
            "pwm-pair" => (
                "timer",
                Box::new(|r: &str| r.starts_with("ch") && r.ends_with('n')),
            ),
            _ => return None,
        };
        let candidates: Vec<Candidate> = part
            .peripherals()
            .filter(|p| p.kind == kind)
            .flat_map(|p| {
                p.lines
                    .iter()
                    .filter(|l| row(&l.name))
                    .filter(|l| l.pins.iter().any(|q| q == given || q == pin))
                    .map(move |l| Candidate {
                        instance: p.name.clone(),
                        kind: p.kind.clone(),
                        caps: p
                            .has
                            .iter()
                            .chain(l.has.iter())
                            .filter(|h| !h.props.iter().any(|p| p.key == "when"))
                            .map(|h| h.capability.clone())
                            .collect(),
                    })
            })
            .collect();
        if candidates.is_empty() {
            self.error(
                file,
                span,
                format!("`{given}` cannot carry `{}` of `{}`: no {kind} instance of `{}` puts that line on it", line.name, ty.name, part.name),
            );
        }
        Some(candidates)
    }

    /// One side's answers together: the instances that could carry every answered line at once,
    /// and the facts they stamp on those lines — the counter, where it is one instance, and every
    /// capability the surviving instances all have.
    fn stamp(
        &mut self,
        frame: usize,
        file: FileId,
        span: Span,
        stamps: Vec<(Terminal, Vec<Candidate>, Span, bool)>,
    ) {
        if stamps.is_empty() {
            return;
        }
        // The lines of one kind — an I²C's `sda` and `scl`, a pair's `high` and `low` — are one
        // instance; a composed type's lines each answer for themselves.
        let mut common: Option<HashSet<String>> = None;
        let shared = stamps.iter().filter(|s| s.3).count();
        for (_, cands, _, is_shared) in &stamps {
            if !is_shared {
                continue;
            }
            let names: HashSet<String> = cands.iter().map(|c| c.instance.clone()).collect();
            common = Some(match common {
                None => names,
                Some(c) => c.intersection(&names).cloned().collect(),
            });
        }
        let common = common.unwrap_or_default();
        if common.is_empty() && shared > 1 {
            let kind = stamps
                .iter()
                .flat_map(|(_, c, _, _)| c.iter())
                .map(|c| c.kind.clone())
                .next()
                .unwrap_or_else(|| "peripheral".into());
            let each: Vec<String> = stamps
                .iter()
                .filter(|s| s.3)
                .map(|(t, cands, _, _)| {
                    let name = match t {
                        Terminal::Pin { pin, .. } => pin.clone(),
                        Terminal::Line { line, .. } => line.clone(),
                    };
                    let mut insts: Vec<&str> = cands.iter().map(|c| c.instance.as_str()).collect();
                    insts.sort();
                    insts.dedup();
                    format!("`{name}` is on {}", insts.join(", "))
                })
                .collect();
            self.error(
                file,
                span,
                format!(
                    "no one {kind} instance carries every line of this link: {}",
                    each.join("; ")
                ),
            );
            return;
        }
        for (terminal, cands, at, is_shared) in stamps {
            let surviving: Vec<&Candidate> = cands
                .iter()
                .filter(|c| !is_shared || shared < 2 || common.contains(&c.instance))
                .collect();
            if surviving.is_empty() {
                continue;
            }
            let mut instances: Vec<&str> = surviving.iter().map(|c| c.instance.as_str()).collect();
            instances.sort();
            instances.dedup();
            let mut push = |fact: String, value: Option<Value>| {
                self.contributions.push(Contribution {
                    terminal: terminal.clone(),
                    aspect: Aspect::Signal,
                    fact,
                    rule: Rule::Set,
                    value,
                    props: Vec::new(),
                    frame,
                    port: None,
                    file,
                    span: at,
                });
            };
            // A counter is a timer's: the one fact a composed type can hold two lines to.
            if let [one] = instances.as_slice()
                && surviving.iter().all(|c| c.kind == "timer")
            {
                push("counter".into(), Some(Value::Name(one.to_string())));
            }
            let mut caps: Vec<String> = surviving[0].caps.clone();
            caps.retain(|c| surviving.iter().all(|s| s.caps.contains(c)));
            caps.sort();
            caps.dedup();
            for c in caps {
                push(c, None);
            }
        }
    }

    // --- Finish -------------------------------------------------------------------------------------

    fn finish(mut self, name: String) -> Model {
        // Designators, in placement order: a word printed whole, or a prefix numbered under.
        let reserved: HashSet<String> = self
            .instances
            .iter()
            .enumerate()
            .filter(|(i, _)| self.part_of(*i).is_some())
            .filter_map(|(_, instance)| instance.designator_word.clone())
            .collect();
        let mut counters: HashMap<String, usize> = HashMap::new();
        let mut taken: HashSet<String> = HashSet::new();
        for i in 0..self.instances.len() {
            let Some(part) = self.part_of(i) else {
                continue;
            };
            let designator = match self.instances[i].designator_word.clone() {
                Some(word) => word,
                None => {
                    let prefix = self.instances[i]
                        .reference_prefix
                        .clone()
                        .or_else(|| {
                            part.meta(MetaKey::Reference)
                                .and_then(|v| v.as_str().map(str::to_string))
                        })
                        .unwrap_or_else(|| "U".into());
                    let n = counters.entry(prefix.clone()).or_insert(0);
                    loop {
                        *n += 1;
                        let candidate = format!("{prefix}{n}");
                        if !reserved.contains(&candidate) {
                            break candidate;
                        }
                    }
                }
            };
            if !taken.insert(designator.clone()) {
                let (file, span) = (self.instances[i].file, self.instances[i].span);
                self.error(
                    file,
                    span,
                    format!("`{designator}` is printed on two parts"),
                );
            }
            self.instances[i].designator = Some(designator);
        }

        for (inst, value_frame, origin, value, span) in std::mem::take(&mut self.pending_anchors) {
            let text = self.text_value(value_frame, &value);
            let result = Reference::parse(&text)
                .map_err(|e| e.message)
                .and_then(|reference| {
                    let target = self.resolve(origin, &reference)?;
                    let Resolved::Pin(host, pin) = target else {
                        return Err(format!("`anchor={text}` must name a part's pin"));
                    };
                    if host == inst {
                        return Err("a part cannot anchor on itself".into());
                    }
                    let Some(body) = self.package_of(host) else {
                        return Err(format!(
                            "`{}` has no package with pads",
                            self.instances[host].path
                        ));
                    };
                    let mut pads: Vec<&str> = body
                        .pads()
                        .filter(|p| p.pin == pin)
                        .map(|p| p.label.as_str())
                        .collect();
                    pads.sort_unstable();
                    pads.dedup();
                    let pad = match (&reference.pad, pads.as_slice()) {
                        (Some(pad), _) => pad.clone(),
                        (None, [pad]) => (*pad).to_string(),
                        (None, []) => {
                            return Err(format!(
                                "`{}` has no pad for pin `{pin}`",
                                self.instances[host].path
                            ));
                        }
                        (None, _) => {
                            return Err(format!(
                                "`{}` pin `{pin}` has several pads; select one with `@pad`",
                                self.instances[host].path
                            ));
                        }
                    };
                    Ok(format!(
                        "{}.{}",
                        self.instances[host]
                            .designator
                            .as_deref()
                            .unwrap_or_default(),
                        pad
                    ))
                });
            match result {
                Ok(anchor) => self.instances[inst].anchor = Some(anchor),
                Err(message) => self.error(self.frames[value_frame].file, span, message),
            }
        }
        for i in 0..self.instances.len() {
            if self.instances[i].spot.is_some() && self.instances[i].anchor.is_none() {
                let (file, span) = (self.instances[i].file, self.instances[i].span);
                self.error(file, span, "`spot=` needs a valid `anchor=`");
            }
        }

        // Facts, combined per net. `name` is the one consumed here: `set`, so two distinct
        // names on one net conflict, and a net with none takes a projected name at export.
        let contributions: Vec<(Contribution, String)> = self
            .contributions
            .iter()
            .filter(|c| c.aspect == Aspect::Net && c.fact == "name")
            .map(|c| {
                let text = match &c.value {
                    Some(Value::String(s)) | Some(Value::Name(s)) => s.clone(),
                    _ => String::new(),
                };
                (c.clone(), text)
            })
            .collect();
        let mut names: HashMap<usize, Vec<&(Contribution, String)>> = HashMap::new();
        for c in &contributions {
            if let Some(root) = self.nets.root_of(&c.0.terminal) {
                names.entry(root).or_default().push(c);
            }
        }
        let sets = self.nets.sets();
        let mut nets = Vec::new();
        for members in sets {
            let root = self.nets.root_of(&members[0]).unwrap();
            let stated = names.get(&root).cloned().unwrap_or_default();
            let mut distinct: Vec<&(Contribution, String)> = Vec::new();
            for c in &stated {
                if !distinct.iter().any(|d| d.1 == c.1) {
                    distinct.push(c);
                }
            }
            if distinct.len() > 1 {
                let all = distinct
                    .iter()
                    .map(|d| format!("`{}`", d.1))
                    .collect::<Vec<_>>()
                    .join(" and ");
                for d in &distinct {
                    let (file, span) = (d.0.file, d.0.span);
                    self.error(
                        file,
                        span,
                        format!("one net is named {all}; a name is set once"),
                    );
                }
            }
            let name = distinct
                .first()
                .map(|d| d.1.clone())
                .unwrap_or_else(|| self.derived_net_name(&members));
            nets.push(Net { name, members });
        }

        // Facts, derived values, assertions and requirements, over the finished nets.
        facts::check(&mut self, &nets);

        // A pin declared no-connect that is joined to anything.
        for (t, file, span) in std::mem::take(&mut self.nc) {
            let Terminal::Pin { inst, pin } = &t else {
                continue;
            };
            let joined = self
                .nets
                .root_of(&t)
                .map(|r| {
                    nets.iter()
                        .find(|n| self.nets.root_of(&n.members[0]) == Some(r))
                        .map(|n| n.members.len())
                        .unwrap_or(1)
                })
                .unwrap_or(1);
            if joined > 1 {
                let path = self.instances[*inst].path.clone();
                self.error(
                    file,
                    span,
                    format!("`{path}.{pin}` is declared no-connect but is joined to something"),
                );
            }
        }

        Model {
            name,
            instances: self.instances,
            nets,
            imperial: self.imperial,
            report: self.report,
        }
    }

    /// A name for a net nobody named: the highest block port on it — skipping the ports of a
    /// shunt block, whose `node` is the pin it hangs on — else a pin on a part that is not a
    /// passive, else its first pin.
    fn derived_net_name(&self, members: &[Terminal]) -> String {
        let is_self_block = |inst: usize| {
            let path = &self.instances[inst].path;
            self.instances.iter().enumerate().any(|(j, other)| {
                j != inst && &other.path == path && matches!(other.kind, Kind::Part { .. })
            })
        };
        // Union order can differ from placement order (notably for deferred port arguments).
        // Break equal-depth ties by instance order so the projected name remains stable.
        let mut best: Option<(usize, usize, String)> = None;
        for m in members {
            if let Terminal::Line { inst, port, line } = m {
                if is_self_block(*inst) {
                    continue;
                }
                let path = &self.instances[*inst].path;
                let depth = if path.is_empty() {
                    0
                } else {
                    path.matches('/').count() + 1
                };
                let multi = self.port_lines(*inst, port).len() > 1;
                let name = match (path.is_empty(), multi) {
                    (true, true) => format!("{port}.{line}"),
                    (true, false) => port.clone(),
                    (false, true) => format!("{path}/{port}.{line}"),
                    (false, false) => format!("{path}/{port}"),
                };
                if best
                    .as_ref()
                    .is_none_or(|(d, i, _)| (depth, *inst) < (*d, *i))
                {
                    best = Some((depth, *inst, name));
                }
            }
        }
        if let Some((_, _, name)) = best {
            return name;
        }
        // No block port: a part's own contract port — one with several lines, so a jack's
        // `out.rail` names its rail where a resistor's `a` names nothing.
        for m in members {
            if let Some((port, line)) = self.pin_port.get(m) {
                let Terminal::Pin { inst, .. } = m else {
                    continue;
                };
                if self.port_lines(*inst, port).len() < 2 {
                    continue;
                }
                let path = &self.instances[*inst].path;
                let depth = path.matches('/').count() + 1;
                if best
                    .as_ref()
                    .is_none_or(|(d, i, _)| (depth, *inst) < (*d, *i))
                {
                    best = Some((depth, *inst, format!("{path}/{port}.{line}")));
                }
            }
        }
        if let Some((_, _, name)) = best {
            return name;
        }
        let passive = |inst: usize| {
            self.part_of(inst)
                .and_then(|p| p.meta(MetaKey::Reference))
                .and_then(|v| v.as_str().map(|s| matches!(s, "R" | "C" | "L")))
                .unwrap_or(false)
        };
        let pins: Vec<(usize, &String)> = members
            .iter()
            .filter_map(|m| match m {
                Terminal::Pin { inst, pin } => Some((*inst, pin)),
                _ => None,
            })
            .collect();
        let pick = pins
            .iter()
            .min_by_key(|(inst, pin)| (passive(*inst), *inst, pin.as_str()));
        match pick {
            Some((inst, pin)) => {
                let d = self.instances[*inst]
                    .designator
                    .clone()
                    .unwrap_or_else(|| self.instances[*inst].path.clone());
                format!("Net-({d}-{pin})")
            }
            None => "Net-()".into(),
        }
    }
}

/// Which rows of a peripheral kind a line may use.
type RowMatch = Box<dyn Fn(&str) -> bool>;

fn type_of_decl(t: &TypeDecl) -> TypeInfo {
    TypeInfo {
        name: t.name.clone(),
        lines: t
            .lines()
            .map(|l| LineInfo {
                name: l.name.clone(),
                optional: l.optional,
                partner: l.items.iter().find_map(|i| match i {
                    stackup_kdl::ast::LineItem::Match(m) => m
                        .target
                        .members
                        .last()
                        .cloned()
                        .or_else(|| m.target.path.last().cloned()),
                    _ => None,
                }),
                ty: l.ty.clone(),
            })
            .collect(),
    }
}

/// Whether a port of one type can take a link of another: the same type, or one refining the
/// other (`pwm` → `output` → `terminal`, `analog` → `terminal`).
fn compatible(port: Option<&str>, link: &str) -> bool {
    let port = port.unwrap_or("terminal");
    port == link || refines(link, port) || refines(port, link)
}

fn refines(a: &str, b: &str) -> bool {
    let mut cur = a;
    loop {
        let next = match cur {
            "pwm" => "output",
            "output" | "analog" => "terminal",
            _ => return false,
        };
        if next == b {
            return true;
        }
        cur = next;
    }
}
