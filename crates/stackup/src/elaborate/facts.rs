//! The checks over a finished design: facts combined per net, derived values, assertions and
//! requirements.
//!
//! Elaboration records what every port states (a [`Contribution`]) and what every port and pin
//! asks (a [`Requirement`]); this pass is where they meet. A fact's value on a net is its
//! contributions combined by the fact's rule — one writer for `set`, a sum for `add`, distinct
//! values for `claim` — each contribution an expression evaluated in the frame it was written in,
//! so it can read parameters, derived values and the facts on other ports. Facts and derived
//! values are evaluated lazily and memoised, in whatever order the reads dictate; a read that
//! comes back to itself is a loop, and a finding.
//!
//! Three outcomes, not two. A check **holds**, **fails**, or holds **for what is stated**: a sum
//! with a contributor that states nothing is a lower bound (§9.7), and a fact nothing states is
//! unknown. Neither is a pass, and neither is treated as one — a note says exactly what was not
//! stated, and an unknown fact makes a check a note rather than a finding.

use std::collections::{HashMap, HashSet};

use stackup_kdl::{
    Diagnostic, Span, Value,
    ast::{Aspect, Assert, Block, Part, Rule},
};

use super::{Builder, Contribution, Named, ReqOn, Requirement};
use crate::{
    expr::{self, Eval, Origin, Scope, Trace, Val},
    load::{Decl, FileId},
    model::{Kind, Net, Terminal},
    quantity::Quantity,
    report::Finding,
};

/// Runs every check over the finished nets.
pub(super) fn check(b: &mut Builder<'_>, nets: &[Net]) {
    let mut net_of = HashMap::new();
    for (i, n) in nets.iter().enumerate() {
        for m in &n.members {
            net_of.insert(m.clone(), i);
        }
    }
    let mut by_fact: HashMap<(usize, Aspect, String), Vec<usize>> = HashMap::new();
    for (ci, c) in b.contributions.iter().enumerate() {
        if let Some(&net) = net_of.get(&c.terminal) {
            by_fact
                .entry((net, c.aspect, c.fact.clone()))
                .or_default()
                .push(ci);
        }
    }
    // Every part port on each net, for the consumers that state nothing.
    let mut ports_on_net: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
    for (inst, ports) in b.ports.iter().enumerate() {
        if !matches!(b.instances[inst].kind, Kind::Part { .. }) {
            continue;
        }
        for (pi, p) in ports.iter().enumerate() {
            let mut seen = HashSet::new();
            for (_, t) in &p.lines {
                if let Some(&net) = net_of.get(t)
                    && seen.insert(net)
                {
                    ports_on_net.entry(net).or_default().push((inst, pi));
                }
            }
        }
    }
    let mut c = Checker {
        b,
        nets,
        net_of,
        by_fact,
        ports_on_net,
        memo: HashMap::new(),
        stack: Vec::new(),
        reported: HashSet::new(),
    };
    c.run();
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key {
    Derive(usize, String),
    Fact(usize, Aspect, String),
}

struct Checker<'c, 'a> {
    b: &'c mut Builder<'a>,
    nets: &'c [Net],
    net_of: HashMap<Terminal, usize>,
    by_fact: HashMap<(usize, Aspect, String), Vec<usize>>,
    ports_on_net: HashMap<usize, Vec<(usize, usize)>>,
    memo: HashMap<Key, Eval>,
    stack: Vec<Key>,
    /// Conflicts already reported, by the pair of contributions.
    reported: HashSet<(usize, usize)>,
}

/// What a frame's declaration is, for its derives, texts and asserts.
enum Body<'a> {
    Part(&'a Part),
    Block(&'a Block),
}

impl<'a> Body<'a> {
    fn derive(&self, name: &str) -> Option<&'a stackup_kdl::ast::Derive> {
        match self {
            Body::Part(p) => p.items.iter().find_map(|i| match i {
                stackup_kdl::ast::PartItem::Derive(d) if d.name == name => Some(d),
                _ => None,
            }),
            Body::Block(b) => b.items.iter().find_map(|i| match i {
                stackup_kdl::ast::BlockItem::Derive(d) if d.name == name => Some(d),
                _ => None,
            }),
        }
    }

    fn text(&self, name: &str) -> Option<&'a stackup_kdl::ast::Text> {
        match self {
            Body::Part(p) => p.items.iter().find_map(|i| match i {
                stackup_kdl::ast::PartItem::Text(t) if t.name == name => Some(t),
                _ => None,
            }),
            Body::Block(b) => b.items.iter().find_map(|i| match i {
                stackup_kdl::ast::BlockItem::Text(t) if t.name == name => Some(t),
                _ => None,
            }),
        }
    }

    fn asserts(&self) -> Vec<&'a Assert> {
        match self {
            Body::Part(p) => p
                .items
                .iter()
                .filter_map(|i| match i {
                    stackup_kdl::ast::PartItem::Assert(a) => Some(a),
                    _ => None,
                })
                .collect(),
            Body::Block(b) => b
                .items
                .iter()
                .filter_map(|i| match i {
                    stackup_kdl::ast::BlockItem::Assert(a) => Some(a),
                    _ => None,
                })
                .collect(),
        }
    }
}

/// Pin kinds that drive their net: on a strap's net, one of these from another part makes the
/// rest level that part's business.
const DRIVING_KINDS: [&str; 6] = [
    "output",
    "bidirectional",
    "open_collector",
    "open_emitter",
    "tri_state",
    "power_out",
];

impl<'a> Checker<'_, 'a> {
    fn run(&mut self) {
        // Resolve placed values from parameters and static derives before evaluating facts.
        // Other facts may refer to the resulting value through an `as=self` block.
        let values = std::mem::take(&mut self.b.pending_values);
        for (inst, frame, value, span) in values {
            let rendered = match &value {
                Value::Name(name) => match super::static_values::eval(self.b, frame, name, span) {
                    Ok(value) => match value.val {
                        Val::Unknown(reason) => {
                            self.b.error(self.b.frames[frame].file, span, reason);
                            continue;
                        }
                        other => other.to_string(),
                    },
                    Err(reason) => {
                        self.b.error(self.b.frames[frame].file, span, reason);
                        continue;
                    }
                },
                other => self.b.text_value(frame, other),
            };
            self.b.instances[inst].value = Some(rendered);
        }

        // Requirements: what ports and pins ask of their nets.
        let reqs = std::mem::take(&mut self.b.requirements);
        for r in &reqs {
            self.requirement(r);
        }
        self.b.requirements = reqs;

        // Assertions, in every frame with a body, and on every part port.
        for frame in 0..self.b.frames.len() {
            let Some(body) = self.body_of(frame) else {
                continue;
            };
            let file = self.b.frames[frame].file;
            for a in body.asserts() {
                self.assertion(frame, None, a, file);
            }
            if let Body::Part(part) = body {
                let inst = self.b.frames[frame].inst;
                for port in part.ports() {
                    for item in &port.items {
                        if let stackup_kdl::ast::PortItem::Assert(a) = item {
                            self.assertion(frame, Some((inst, port.name.clone())), a, file);
                        }
                    }
                }
            }
        }
        for i in &self.b.ignores {
            if !i.matched {
                self.b.report.error(
                    self.b.lib.source(i.file),
                    i.span,
                    format!(
                        "ignore selector `{}` matches no check on this placement",
                        i.target
                    ),
                );
            }
        }

        // Every fact stated anywhere is evaluated once, so a conflict nobody reads and an
        // expression nobody uses are still reported.
        let keys: Vec<(usize, Aspect, String)> = self.by_fact.keys().cloned().collect();
        for (net, aspect, fact) in keys {
            self.fact(net, aspect, &fact, None);
        }

        // Notes written as templates, or naming a `text`.
        let notes = std::mem::take(&mut self.b.pending_notes);
        for (inst, frame, value, span) in notes {
            let rendered = match &value {
                Value::Name(n) => {
                    let text = self.frame_chain(frame).into_iter().find_map(|f| {
                        self.body_of(f)
                            .and_then(|b| b.text(n).map(|t| (f, t.template.clone(), t.span)))
                    });
                    match text {
                        Some((f, template, tspan)) => self.render(f, &template, tspan),
                        None => self.b.text_value(frame, &value),
                    }
                }
                Value::String(s) if s.contains('{') => self.render(frame, s, span),
                other => self.b.text_value(frame, other),
            };
            self.b.instances[inst].notes.push(rendered);
        }
    }

    // --- Frames -------------------------------------------------------------------------------

    fn body_of(&self, frame: usize) -> Option<Body<'a>> {
        let inst = self.b.frames[frame].inst;
        match self.b.instances[inst].kind {
            Kind::Part { decl, .. } => match self.b.lib.decl(decl) {
                Decl::Part(p) => Some(Body::Part(p)),
                _ => None,
            },
            Kind::Block { decl } => match self.b.lib.decl(decl) {
                Decl::Block(b) if self.b.frames[frame].parent.is_some() => {
                    let name = self.b.instances[inst].path.rsplit('/').next()?;
                    b.blocks().find(|child| child.name == name).map(Body::Block)
                }
                Decl::Block(b) => Some(Body::Block(b)),
                Decl::Part(p) => {
                    let name = self.b.instances[inst].path.rsplit('/').next()?;
                    p.blocks().find(|b| b.name == name).map(Body::Block)
                }
                _ => None,
            },
            Kind::Scope => None,
        }
    }

    fn frame_chain(&self, frame: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut f = Some(frame);
        while let Some(i) = f {
            out.push(i);
            f = self.b.frames[i].parent;
        }
        out
    }

    fn source(&self, file: FileId) -> std::sync::Arc<stackup_kdl::Source> {
        self.b.lib.source(file).clone()
    }

    fn describe_inst(&self, inst: usize) -> String {
        let p = &self.b.instances[inst].path;
        if p.is_empty() {
            "the design".into()
        } else {
            format!("`{p}`")
        }
    }

    fn render(&mut self, frame: usize, template: &str, span: Span) -> String {
        let (text, _, errors) = {
            let mut ctx = Ctx {
                chk: self,
                frame,
                port: None,
            };
            expr::template(template, &mut ctx, span)
        };
        let file = self.b.frames[frame].file;
        for e in errors {
            self.b.error(file, span, e);
        }
        text
    }

    // --- Values -------------------------------------------------------------------------------

    fn eval_in(
        &mut self,
        frame: usize,
        port: Option<(usize, String)>,
        src: &str,
        span: Span,
    ) -> Result<Eval, String> {
        let mut ctx = Ctx {
            chk: self,
            frame,
            port,
        };
        expr::eval(src, &mut ctx, span)
    }

    fn derive(&mut self, frame: usize, name: &str) -> Option<Eval> {
        let (f, d) = self
            .frame_chain(frame)
            .into_iter()
            .find_map(|f| self.body_of(f).and_then(|b| b.derive(name)).map(|d| (f, d)))?;
        let key = Key::Derive(f, name.to_string());
        if let Some(v) = self.memo.get(&key) {
            return Some(v.clone());
        }
        let file = self.b.frames[f].file;
        if self.stack.contains(&key) {
            self.loop_found(&key, file, d.span);
            return Some(Eval::new(Val::Unknown(format!(
                "`{name}` depends on itself"
            ))));
        }
        self.stack.push(key.clone());
        let v = match self.eval_in(f, None, &d.expr, d.span) {
            Ok(v) => v.with_origin(Origin {
                file,
                span: d.span,
                what: format!("`{name}` is derived here"),
            }),
            Err(e) => {
                self.b
                    .error(file, d.span, format!("in `derive {name}`: {e}"));
                Eval::new(Val::Unknown(format!("`{name}` could not be derived")))
            }
        };
        self.stack.pop();
        self.memo.insert(key, v.clone());
        Some(v)
    }

    fn loop_found(&mut self, key: &Key, file: FileId, span: Span) {
        let start = self.stack.iter().position(|k| k == key).unwrap_or(0);
        let names: Vec<String> = self.stack[start..]
            .iter()
            .chain(std::iter::once(key))
            .map(|k| match k {
                Key::Derive(_, n) => format!("`{n}`"),
                Key::Fact(net, _, f) => format!("`{f}` on {}", self.nets[*net].name),
            })
            .collect();
        self.b.error(
            file,
            span,
            format!(
                "facts depend on one another in a loop: {}",
                names.join(" → ")
            ),
        );
    }

    /// A fact on a net, combined from its contributions. `asking` is the pin reading `rest`.
    fn fact(&mut self, net: usize, aspect: Aspect, fact: &str, asking: Option<&Terminal>) -> Eval {
        if aspect == Aspect::Net && fact == "rest" {
            return self.rest(net, asking);
        }
        let key = Key::Fact(net, aspect, fact.to_string());
        if let Some(v) = self.memo.get(&key) {
            return v.clone();
        }
        let Some(indices) = self.by_fact.get(&key_tuple(&key)).cloned() else {
            return Eval::new(Val::Unknown(format!(
                "nothing on `{}` states `{fact}`",
                self.nets[net].name
            )));
        };
        if self.stack.contains(&key) {
            let c = &self.b.contributions[indices[0]];
            let (file, span) = (c.file, c.span);
            self.loop_found(&key, file, span);
            return Eval::new(Val::Unknown(format!("`{fact}` depends on itself")));
        }
        self.stack.push(key.clone());

        let rule = self.b.contributions[indices[0]].rule;
        let mut values: Vec<(usize, Eval)> = Vec::new();
        let mut partial: Vec<String> = Vec::new();
        for &ci in &indices {
            let c_rule = self.b.contributions[ci].rule;
            if c_rule != rule {
                let (file, span) = (self.b.contributions[ci].file, self.b.contributions[ci].span);
                self.b.error(
                    file,
                    span,
                    format!(
                        "`{fact}` is `{}` here and `{}` elsewhere on the same net; a fact has one rule",
                        c_rule.statement(),
                        rule.statement()
                    ),
                );
            }
            let v = self.contribution(ci);
            match &v.val {
                Val::Unknown(reason) => partial.push(reason.clone()),
                _ => values.push((ci, v)),
            }
        }

        let mut result = match rule {
            Rule::Set => {
                // One writer: every distinct value past the first is a conflict.
                let mut distinct: Vec<(usize, Eval)> = Vec::new();
                for (ci, v) in &values {
                    if !distinct.iter().any(|(_, d)| same(&d.val, &v.val)) {
                        distinct.push((*ci, v.clone()));
                    }
                }
                if distinct.len() > 1 {
                    for (i, (ci, v)) in distinct.iter().enumerate() {
                        for (cj, w) in distinct.iter().skip(i + 1) {
                            self.conflict(*ci, *cj, fact, v, w, "set");
                        }
                    }
                }
                match distinct.into_iter().next() {
                    Some((_, v)) => v,
                    None => Eval::new(Val::Unknown(format!(
                        "nothing on `{}` states `{fact}`",
                        self.nets[net].name
                    ))),
                }
            }
            Rule::Add => {
                let mut sum: Option<Eval> = None;
                for (ci, v) in values {
                    let Val::Num(q) = v.val else {
                        let (file, span) =
                            (self.b.contributions[ci].file, self.b.contributions[ci].span);
                        self.b.error(
                            file,
                            span,
                            format!("`add {fact}` needs a quantity, and this is {}", v.val),
                        );
                        continue;
                    };
                    sum = Some(match sum {
                        None => Eval {
                            val: Val::Num(q),
                            trace: v.trace,
                        },
                        Some(s) => {
                            let Val::Num(sq) = s.val else { unreachable!() };
                            match sq.add(q) {
                                Ok(total) => Eval {
                                    val: Val::Num(total),
                                    trace: s.trace.merge(&v.trace),
                                },
                                Err(e) => {
                                    let (file, span) = (
                                        self.b.contributions[ci].file,
                                        self.b.contributions[ci].span,
                                    );
                                    self.b.error(file, span, format!("in `add {fact}`: {e}"));
                                    s
                                }
                            }
                        }
                    });
                }
                match sum {
                    Some(s) => s,
                    None => Eval::new(Val::Unknown(format!(
                        "nothing on `{}` states `{fact}`",
                        self.nets[net].name
                    ))),
                }
            }
            Rule::Claim => {
                for (i, (ci, v)) in values.iter().enumerate() {
                    for (cj, w) in values.iter().skip(i + 1) {
                        if same(&v.val, &w.val) {
                            self.conflict(*ci, *cj, fact, v, w, "claim");
                        }
                    }
                }
                match values.into_iter().next() {
                    Some((_, v)) => v,
                    None => Eval::new(Val::Unknown(format!(
                        "nothing on `{}` claims `{fact}`",
                        self.nets[net].name
                    ))),
                }
            }
        };

        // A load that states no draw makes the sum a lower bound.
        if rule == Rule::Add && fact == "draw" && aspect == Aspect::Net {
            for &(inst, pi) in self
                .ports_on_net
                .get(&net)
                .cloned()
                .unwrap_or_default()
                .iter()
            {
                let port = &self.b.ports[inst][pi];
                if port.ty.as_deref() != Some("power") {
                    continue;
                }
                let states = self.b.contributions.iter().any(|c| {
                    c.port.as_ref() == Some(&(inst, port.name.clone()))
                        && self.net_of.get(&c.terminal) == Some(&net)
                        && ((c.fact == "draw" && c.rule == Rule::Add)
                            || (c.fact == "voltage" && c.rule == Rule::Set))
                });
                if !states {
                    partial.push(format!(
                        "{}.{} states no draw",
                        self.describe_inst(inst),
                        port.name
                    ));
                }
            }
        }
        for p in partial {
            if !result.trace.partial.contains(&p) {
                result.trace.partial.push(p);
            }
        }

        self.stack.pop();
        self.memo.insert(key, result.clone());
        result
    }

    fn conflict(&mut self, ci: usize, cj: usize, fact: &str, v: &Eval, w: &Eval, verb: &str) {
        let (a, b) = (&self.b.contributions[ci], &self.b.contributions[cj]);
        let pair = (ci.min(cj), ci.max(cj));
        if !self.reported.insert(pair) {
            return;
        }
        let (fa, sa, fb, sb) = (a.file, a.span, b.file, b.span);
        let (oa, ob) = (self.owner_of(a), self.owner_of(b));
        let message = match verb {
            "set" => format!(
                "`{fact}` is set to {} by {oa} and to {} by {ob} on one net; a fact is set once",
                v.val, w.val
            ),
            _ => format!(
                "`{fact}` {} is claimed by {oa} and by {ob} on one net; a claim is distinct",
                v.val
            ),
        };
        let other = Finding::new(
            &self.source(fb),
            Diagnostic::note(sb, format!("{ob} {verb}s `{fact}` to {}", w.val)),
        );
        self.b.report.add(
            Finding::new(&self.source(fa), Diagnostic::error(sa, message)).with_related(other),
        );
    }

    /// Who made a contribution: the port, else the terminal.
    fn owner_of(&self, c: &Contribution) -> String {
        match &c.port {
            Some((inst, port)) => format!("{}.{port}", self.describe_inst(*inst)),
            None => match &c.terminal {
                Terminal::Pin { inst, pin } => format!("{}.{pin}", self.describe_inst(*inst)),
                Terminal::Line { inst, port, .. } => {
                    format!("{}.{port}", self.describe_inst(*inst))
                }
            },
        }
    }

    /// One contribution's value, in the frame it was written in.
    fn contribution(&mut self, ci: usize) -> Eval {
        let c: Contribution = self.b.contributions[ci].clone();
        let owner = self.owner_of(&c);
        let flag_only = c.value.is_none() && c.props.is_empty();
        let mut v = match &c.value {
            None if c.fact == "name" => Eval::text(""),
            None if flag_only => Eval::new(Val::Bool(true)),
            None => {
                // `set net.voltage min="12V" max="18V"`: a range with no nominal.
                let lo = c.props.iter().find(|p| p.key == "min");
                let hi = c.props.iter().find(|p| p.key == "max");
                match (lo, hi) {
                    (Some(lo), Some(hi)) => {
                        let lo = self.eval_prop(&c, &lo.value, lo.span);
                        let hi = self.eval_prop(&c, &hi.value, hi.span);
                        match (lo.as_num(), hi.as_num()) {
                            (Some(l), Some(h)) if l.unit == h.unit => Eval {
                                val: Val::Num(Quantity::range(l.lo, h.hi, l.unit)),
                                trace: lo.trace.merge(&hi.trace),
                            },
                            _ => {
                                self.b.error(
                                    c.file,
                                    c.span,
                                    "`min=` and `max=` must be quantities in one unit",
                                );
                                Eval::new(Val::Unknown(format!("`{}` could not be read", c.fact)))
                            }
                        }
                    }
                    _ => {
                        self.b.error(
                            c.file,
                            c.span,
                            format!(
                                "`{} {}` needs a value, or `min=` and `max=`",
                                c.rule.statement(),
                                c.fact
                            ),
                        );
                        Eval::new(Val::Unknown(format!("`{}` could not be read", c.fact)))
                    }
                }
            }
            Some(Value::String(s)) if c.fact == "name" => Eval::text(s.clone()),
            Some(v) => self.eval_prop(&c, v, c.span),
        };
        if let Some(t) = c.props.iter().find(|p| p.key == "tolerance") {
            let tol = self.eval_prop(&c, &t.value, t.span);
            match (v.as_num(), tol.as_num()) {
                (Some(q), Some(f)) if f.unit == crate::quantity::Unit::NONE => {
                    v.val = Val::Num(q.with_tolerance(f.hi));
                }
                (Some(_), _) => {
                    self.b
                        .error(c.file, t.span, "`tolerance=` is a ratio, such as \"2%\"")
                }
                _ => {}
            }
        }
        if !matches!(v.val, Val::Unknown(_)) {
            let what = format!(
                "{owner} {}s `{}` {} {}",
                c.rule.statement(),
                c.fact,
                if c.rule == Rule::Set { "to" } else { "by" },
                v.val
            );
            v = v.with_origin(Origin {
                file: c.file,
                span: c.span,
                what,
            });
        }
        v
    }

    /// A contribution's value or property as written: a quoted expression, a bare name that is
    /// a parameter, a derived value or a fact of the port's own.
    fn eval_prop(&mut self, c: &Contribution, v: &Value, span: Span) -> Eval {
        let src = match v {
            Value::String(s) | Value::Name(s) => s.clone(),
            Value::Integer(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Bool(b) => return Eval::new(Val::Bool(*b)),
            Value::Null => return Eval::new(Val::Unknown("an empty value".into())),
        };
        match self.eval_in(c.frame, c.port.clone(), &src, span) {
            Ok(v) => v,
            Err(e) => {
                let file = c.file;
                self.b.error(
                    file,
                    span,
                    format!("in `{} {}`: {e}", c.rule.statement(), c.fact),
                );
                Eval::new(Val::Unknown(format!("`{}` could not be evaluated", c.fact)))
            }
        }
    }

    /// What a net sits at with nothing driving it (§9.6).
    fn rest(&mut self, net: usize, asking: Option<&Terminal>) -> Eval {
        let members = self.nets[net].members.clone();
        let mut high: Vec<Origin> = Vec::new();
        let mut low: Vec<Origin> = Vec::new();
        let mut unknown: Vec<String> = Vec::new();
        if members.iter().any(|m| self.b.returns.contains(m)) {
            low.push(Origin {
                file: 0,
                span: Span::default(),
                what: "a return".into(),
            });
        }
        for (fact, into) in [("voltage", 0), ("pull", 0), ("pull-down", 1)] {
            let v = self.fact(net, Aspect::Net, fact, None);
            match &v.val {
                Val::Num(q) if fact != "voltage" || q.hi > 0.0 => {
                    if into == 0 {
                        high.extend(v.trace.from.iter().cloned());
                    } else {
                        low.extend(v.trace.from.iter().cloned());
                    }
                }
                _ => {}
            }
        }
        for m in &members {
            if Some(m) == asking {
                continue;
            }
            let Terminal::Pin { inst, pin } = m else {
                continue;
            };
            let Some(part) = self.b.part_of(*inst) else {
                continue;
            };
            if let Some(p) = part.pins().find(|p| &p.name == pin)
                && DRIVING_KINDS.contains(&p.kind.as_str())
            {
                unknown.push(format!(
                    "{}.{pin} is {} and drives the net when it likes",
                    self.describe_inst(*inst),
                    p.kind
                ));
            }
        }
        let mut trace = Trace::default();
        trace.from.extend(high.iter().chain(low.iter()).cloned());
        trace
            .from
            .retain(|o| !o.what.is_empty() && o.what != "a return");
        let val = match (high.is_empty(), low.is_empty(), unknown.is_empty()) {
            (_, _, false) => Val::Unknown(unknown.join("; ")),
            (false, false, _) => Val::Unknown("the net is pulled both up and down".into()),
            (false, true, _) => Val::Text("high".into()),
            (true, false, _) => Val::Text("low".into()),
            (true, true, _) => {
                // Floating: the asking pin's own pull decides.
                let own = asking.and_then(|t| self.internal_pull(t));
                Val::Text(own.unwrap_or("float").into())
            }
        };
        Eval { val, trace }
    }

    /// The level a pin's own internal pull takes it to, from its `role … pull=`.
    fn internal_pull(&self, t: &Terminal) -> Option<&'static str> {
        let Terminal::Pin { inst, pin } = t else {
            return None;
        };
        let part = self.b.part_of(*inst)?;
        let p = part.pins().find(|p| &p.name == pin)?;
        for role in &p.roles {
            let pull = role
                .props
                .iter()
                .find(|q| q.key == "pull")
                .and_then(|q| q.value.as_str());
            let active = role
                .props
                .iter()
                .find(|q| q.key == "active")
                .and_then(|q| q.value.as_str());
            match pull {
                Some("internal-up") => return Some("high"),
                Some("internal-down") => return Some("low"),
                Some("internal") => {
                    return Some(if active == Some("high") {
                        "low"
                    } else {
                        "high"
                    });
                }
                _ => {}
            }
        }
        None
    }

    // --- Checks -------------------------------------------------------------------------------

    fn requirement(&mut self, r: &Requirement) {
        let file = r.file;
        let (inst, check_name) = match &r.on {
            ReqOn::Port { inst, port, .. } => (*inst, port.as_str()),
            ReqOn::Pin { inst, pin, .. } => (*inst, pin.as_str()),
        };
        let selector = format!(
            "{check_name}.{}.{}",
            r.require.aspect.name(),
            r.require.fact
        );
        let acknowledged = self.acknowledgement(inst, &selector);
        let (subject, terminals): (String, Vec<Terminal>) = match &r.on {
            ReqOn::Port { inst, port, lines } => {
                let non_return: Vec<Terminal> = lines
                    .iter()
                    .filter(|(_, t)| !self.b.returns.contains(t))
                    .map(|(_, t)| t.clone())
                    .collect();
                (
                    format!("{}.{port}", self.describe_inst(*inst)),
                    if non_return.is_empty() {
                        lines.iter().map(|(_, t)| t.clone()).collect()
                    } else {
                        non_return
                    },
                )
            }
            ReqOn::Pin {
                inst,
                pin,
                terminal,
            } => (
                format!("{}.{pin}", self.describe_inst(*inst)),
                vec![terminal.clone()],
            ),
        };
        let asking = match &r.on {
            ReqOn::Pin { terminal, .. } => Some(terminal.clone()),
            _ => None,
        };
        let req = &r.require;
        // The first line whose net knows the fact.
        let mut value: Option<Eval> = None;
        let mut reasons = Vec::new();
        for t in &terminals {
            let Some(&net) = self.net_of.get(t) else {
                // Joined to nothing: a strap's rest is its own pull, or float.
                if req.aspect == Aspect::Net && req.fact == "rest" {
                    let own = self.internal_pull(t).unwrap_or("float");
                    value = Some(Eval::text(own));
                    break;
                }
                continue;
            };
            let v = self.fact(net, req.aspect, &req.fact, asking.as_ref());
            match &v.val {
                Val::Unknown(why) => reasons.push(why.clone()),
                _ => {
                    value = Some(v);
                    break;
                }
            }
        }
        let want = self.requirement_text(r);
        let Some(value) = value else {
            let why = if reasons.is_empty() {
                "it is joined to nothing".to_string()
            } else {
                reasons.join("; ")
            };
            let d = Diagnostic::note(
                req.span,
                format!("{subject} requires {want}, which cannot be checked: {why}"),
            );
            self.b.report.push(&self.source(file), d);
            return;
        };

        // Evaluate the requirement against the value.
        let held: Result<bool, String> = (|| {
            if let Some(expected) = &req.value {
                let e = self.eval_in(r.frame, None, &value_src(expected), req.span)?;
                return Ok(same(&value.val, &e.val));
            }
            if let Some(not) = req.props.iter().find(|p| p.key == "not") {
                let e = self.eval_in(r.frame, None, &value_src(&not.value), req.span)?;
                return Ok(!same(&value.val, &e.val));
            }
            let min = req.props.iter().find(|p| p.key == "min");
            let max = req.props.iter().find(|p| p.key == "max");
            if min.is_some() || max.is_some() {
                let Val::Num(q) = &value.val else {
                    return Err(format!("`{}` is {}, not a quantity", req.fact, value.val));
                };
                let mut ok = true;
                if let Some(m) = min {
                    let m = self.eval_in(r.frame, None, &value_src(&m.value), m.span)?;
                    let mq = m.as_num().ok_or("`min=` must be a quantity")?;
                    if mq.unit != q.unit {
                        return Err(format!(
                            "`min=` is {mq} and the fact is {q}: the units differ"
                        ));
                    }
                    ok &= q.lo >= mq.lo;
                }
                if let Some(m) = max {
                    let m = self.eval_in(r.frame, None, &value_src(&m.value), m.span)?;
                    let mq = m.as_num().ok_or("`max=` must be a quantity")?;
                    if mq.unit != q.unit {
                        return Err(format!(
                            "`max=` is {mq} and the fact is {q}: the units differ"
                        ));
                    }
                    ok &= q.hi <= mq.hi;
                }
                return Ok(ok);
            }
            // A flag: the fact must be there and true.
            Ok(!matches!(value.val, Val::Bool(false)))
        })();
        match held {
            Err(e) => self
                .b
                .error(file, req.span, format!("in `require {}`: {e}", req.fact)),
            Ok(true) if value.trace.partial.is_empty() => {}
            Ok(true) => {
                let d = Diagnostic::note(
                    req.span,
                    format!(
                        "{subject} requires {want}: holds for what is stated ({}); {}",
                        value.val,
                        value.trace.partial.join(", ")
                    ),
                );
                self.b.report.push(&self.source(file), d);
            }
            Ok(false) => {
                let mut finding = Finding::new(
                    &self.source(file),
                    match &acknowledged {
                        Some(reason) => Diagnostic::note(
                            req.span,
                            format!(
                                "{subject} requires {want}, and its net is {} (acknowledged: {reason})",
                                value.val
                            ),
                        ),
                        None => Diagnostic::error(
                            req.span,
                            format!("{subject} requires {want}, and its net is {}", value.val),
                        ),
                    },
                );
                for o in &value.trace.from {
                    finding = finding.with_related(Finding::new(
                        &self.source(o.file),
                        Diagnostic::note(o.span, o.what.clone()),
                    ));
                }
                self.b.report.add(finding);
            }
        }
    }

    fn requirement_text(&self, r: &Requirement) -> String {
        let req = &r.require;
        let mut s = format!("`{}`", req.fact);
        if let Some(v) = &req.value {
            s.push_str(&format!(" {}", value_src(v)));
        }
        for p in &req.props {
            s.push_str(&format!(" {}={}", p.key, value_src(&p.value)));
        }
        s
    }

    fn assertion(&mut self, frame: usize, port: Option<(usize, String)>, a: &Assert, file: FileId) {
        let inst = self.b.frames[frame].inst;
        let selector = match &port {
            Some((_, name)) => format!("{name}.assert:{}", a.expr),
            None => format!("assert:{}", a.expr),
        };
        let acknowledged = self.acknowledgement(inst, &selector);
        let where_ = match &port {
            Some((inst, p)) => format!(" on {}.{p}", self.describe_inst(*inst)),
            None => String::new(),
        };
        let result = self.eval_in(frame, port.clone(), &a.expr, a.span);
        let message = |this: &mut Self| -> String {
            match &a.message {
                Some(m) => {
                    let (text, _, errors) = {
                        let mut ctx = Ctx {
                            chk: this,
                            frame,
                            port: port.clone(),
                        };
                        expr::template(m, &mut ctx, a.span)
                    };
                    for e in errors {
                        this.b
                            .error(file, a.span, format!("in the assertion's message: {e}"));
                    }
                    text
                }
                None => format!("`{}` does not hold{where_}", a.expr),
            }
        };
        match result {
            Err(e) => self.b.error(file, a.span, format!("in `assert`: {e}")),
            Ok(v) => match &v.val {
                Val::Bool(true) if v.trace.partial.is_empty() => {}
                Val::Bool(true) => {
                    let d = Diagnostic::note(
                        a.span,
                        format!(
                            "holds for what is stated{where_}: {}",
                            v.trace.partial.join("; ")
                        ),
                    );
                    self.b.report.push(&self.source(file), d);
                }
                Val::Bool(false) => {
                    let text = message(self);
                    let mut finding = Finding::new(
                        &self.source(file),
                        match &acknowledged {
                            Some(reason) => {
                                Diagnostic::note(a.span, format!("{text} (acknowledged: {reason})"))
                            }
                            None => Diagnostic::error(a.span, text),
                        },
                    );
                    for o in &v.trace.from {
                        finding = finding.with_related(Finding::new(
                            &self.source(o.file),
                            Diagnostic::note(o.span, o.what.clone()),
                        ));
                    }
                    self.b.report.add(finding);
                }
                Val::Unknown(why) => {
                    let d = Diagnostic::note(a.span, format!("cannot be checked{where_}: {why}"));
                    self.b.report.push(&self.source(file), d);
                }
                other => self.b.error(
                    file,
                    a.span,
                    format!("an assertion must be a condition, and this is {other}"),
                ),
            },
        }
    }

    fn acknowledgement(&mut self, inst: usize, selector: &str) -> Option<String> {
        self.b
            .ignores
            .iter_mut()
            .find(|i| i.inst == inst && i.target == selector)
            .map(|i| {
                i.matched = true;
                i.reason.clone()
            })
    }
}

fn key_tuple(k: &Key) -> (usize, Aspect, String) {
    match k {
        Key::Fact(n, a, f) => (*n, *a, f.clone()),
        Key::Derive(..) => unreachable!(),
    }
}

/// A KDL value as expression source.
fn value_src(v: &Value) -> String {
    match v {
        Value::String(s) | Value::Name(s) => s.clone(),
        Value::Integer(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
    }
}

fn same(a: &Val, b: &Val) -> bool {
    match (a, b) {
        (Val::Num(x), Val::Num(y)) => x.same(y),
        (Val::Bool(x), Val::Bool(y)) => x == y,
        (Val::Text(x) | Val::Name(x), Val::Text(y) | Val::Name(y)) => x == y,
        _ => false,
    }
}

// --- Name resolution for expressions -----------------------------------------------------------

/// What names mean inside one frame, and — for a fact or an assertion written on a port — that
/// port's own facts by bare name.
struct Ctx<'c, 'b, 'a> {
    chk: &'c mut Checker<'b, 'a>,
    frame: usize,
    port: Option<(usize, String)>,
}

impl Scope for Ctx<'_, '_, '_> {
    fn feature(&mut self, name: &str) -> Option<bool> {
        for f in self.chk.frame_chain(self.frame) {
            if let Some(on) = self.chk.b.frames[f].features.get(name) {
                return Some(*on);
            }
        }
        None
    }

    fn lookup(&mut self, path: &[String], _span: Span) -> Result<Option<Eval>, String> {
        let head = path[0].as_str();
        // A parameter.
        if let Some(v) = self.chk.b.lookup_param(self.frame, head) {
            if path.len() > 1 {
                return Ok(None);
            }
            return Ok(Some(Eval::from_value(&v)));
        }
        // The `as=self` part's value.
        if head == "value" && path.len() == 1 {
            for f in self.chk.frame_chain(self.frame) {
                if let Some(si) = self.chk.b.frames[f].self_inst
                    && let Some(v) = self.chk.b.instances[si].value.clone()
                {
                    return Ok(Some(Eval::from_value(&Value::String(v))));
                }
            }
        }
        // A derived value or a text, in this frame or one above it.
        if path.len() == 1 {
            if let Some(v) = self.chk.derive(self.frame, head) {
                return Ok(Some(v));
            }
            let text = self.chk.frame_chain(self.frame).into_iter().find_map(|f| {
                self.chk
                    .body_of(f)
                    .and_then(|b| b.text(head).map(|t| (f, t.template.clone(), t.span)))
            });
            if let Some((f, template, tspan)) = text {
                return Ok(Some(Eval::text(self.chk.render(f, &template, tspan))));
            }
            // A bare fact of the port this is written on — after the names above, so that
            // `set net.voltage voltage` reads the parameter and not itself.
            if let Some((inst, port)) = self.port.clone()
                && let Some(v) = self.chk.port_fact(inst, &port, None, None, head)?
            {
                return Ok(Some(v));
            }
        }
        // A port's fact: `port.fact`, `port.line.fact`, `port.aspect.fact`, or through a
        // placement: `child.port.fact`.
        let (inst, port, rest): (usize, String, &[String]) =
            match self.chk.b.lookup_name(self.frame, head) {
                Some(Named::Port(i, p)) => (i, p, &path[1..]),
                Some(Named::Instance(i)) if path.len() >= 2 => {
                    let p = &path[1];
                    if self.chk.b.ports[i].iter().any(|q| &q.name == p) {
                        (i, p.clone(), &path[2..])
                    } else {
                        return Ok(None);
                    }
                }
                Some(Named::Pin(i, p)) if path.len() == 2 => {
                    let t = Terminal::Pin { inst: i, pin: p };
                    let Some(&net) = self.chk.net_of.get(&t) else {
                        return Ok(None);
                    };
                    let fact = path[1].as_str();
                    return Ok(Some(self.chk.fact_any_aspect(net, fact, Some(&t))));
                }
                _ => return Ok(None),
            };
        let (line, aspect, fact) = match rest {
            [fact] => (None, None, fact.as_str()),
            [a, fact] if Aspect::from_name(a).is_some() => {
                (None, Aspect::from_name(a), fact.as_str())
            }
            [line, fact] => (Some(line.as_str()), None, fact.as_str()),
            [line, a, fact] => (Some(line.as_str()), Aspect::from_name(a), fact.as_str()),
            _ => return Ok(None),
        };
        self.chk.port_fact(inst, &port, line, aspect, fact)
    }
}

impl Checker<'_, '_> {
    /// A fact by name on a net, on whichever aspect states it; `rest` is a net fact.
    fn fact_any_aspect(&mut self, net: usize, fact: &str, asking: Option<&Terminal>) -> Eval {
        for aspect in [Aspect::Net, Aspect::Signal, Aspect::Segment] {
            let known = (aspect == Aspect::Net && fact == "rest")
                || self.by_fact.contains_key(&(net, aspect, fact.to_string()));
            if known {
                return self.fact(net, aspect, fact, asking);
            }
        }
        Eval::new(Val::Unknown(format!(
            "nothing on `{}` states `{fact}`",
            self.nets[net].name
        )))
    }

    /// A fact read through a port: on the first of its lines whose net states it, returns
    /// skipped when the port has other lines.
    fn port_fact(
        &mut self,
        inst: usize,
        port: &str,
        line: Option<&str>,
        aspect: Option<Aspect>,
        fact: &str,
    ) -> Result<Option<Eval>, String> {
        let Some(info) = self.b.ports[inst].iter().find(|p| p.name == port) else {
            return Ok(None);
        };
        let lines: Vec<(String, Terminal)> = info.lines.clone();
        if let Some(l) = line
            && !lines.iter().any(|(n, _)| n == l)
        {
            return Ok(None);
        }
        let mut candidates: Vec<&(String, Terminal)> = lines
            .iter()
            .filter(|(n, _)| line.is_none_or(|l| n == l))
            .filter(|(_, t)| line.is_some() || !self.b.returns.contains(t))
            .collect();
        if candidates.is_empty() {
            candidates = lines.iter().collect();
        }
        let mut unknown: Option<Eval> = None;
        for (_, t) in candidates {
            let Some(&net) = self.net_of.get(t) else {
                continue;
            };
            let v = match aspect {
                Some(a) => self.fact(net, a, fact, None),
                None => self.fact_any_aspect(net, fact, None),
            };
            if matches!(v.val, Val::Unknown(_)) {
                unknown.get_or_insert(v);
            } else {
                return Ok(Some(v));
            }
        }
        Ok(Some(unknown.unwrap_or_else(|| {
            Eval::new(Val::Unknown(format!(
                "`{}.{port}` is joined to nothing that states `{fact}`",
                self.describe_inst(inst)
            )))
        })))
    }
}
