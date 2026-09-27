//! The typed model of a stackup file: one struct per statement the format has.
//!
//! This is **syntax**. It records what a file says, in the order it says it, with a [`Span`] on
//! every statement; it resolves no names, evaluates no expressions and checks no design rule.
//! Expressions are kept as their source text, and references as [`Reference`]s — parsed for their
//! shape, not for what they point at.
//!
//! Where the format is still open (a pin's `role`, a part's `order`, a placement's arguments, a
//! `circuit`'s properties, a design's `stock`) the model keeps the properties or statements as
//! they were written rather than guessing at a schema.

use crate::{reference::Reference, span::Span, value::Property, value::Value};

/// One file's worth of declarations, in source order.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct File {
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Use(Use),
    Part(Part),
    /// A `block` or a `design`, told apart by [`Block::kind`].
    Block(Block),
    Type(TypeDecl),
}

impl File {
    pub fn uses(&self) -> impl Iterator<Item = &Use> {
        self.items.iter().filter_map(|i| match i {
            Item::Use(u) => Some(u),
            _ => None,
        })
    }

    pub fn parts(&self) -> impl Iterator<Item = &Part> {
        self.items.iter().filter_map(|i| match i {
            Item::Part(p) => Some(p),
            _ => None,
        })
    }

    /// Top-level blocks, designs excluded.
    pub fn blocks(&self) -> impl Iterator<Item = &Block> {
        self.items.iter().filter_map(|i| match i {
            Item::Block(b) if b.kind == BlockKind::Block => Some(b),
            _ => None,
        })
    }

    pub fn designs(&self) -> impl Iterator<Item = &Block> {
        self.items.iter().filter_map(|i| match i {
            Item::Block(b) if b.kind == BlockKind::Design => Some(b),
            _ => None,
        })
    }

    pub fn types(&self) -> impl Iterator<Item = &TypeDecl> {
        self.items.iter().filter_map(|i| match i {
            Item::Type(t) => Some(t),
            _ => None,
        })
    }
}

/// `use "<path>"`
#[derive(Clone, Debug, PartialEq)]
pub struct Use {
    pub path: String,
    pub span: Span,
}

// --- Parts ----------------------------------------------------------------------------------------

/// `part <name> { … }`
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub name: String,
    pub items: Vec<PartItem>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PartItem {
    Meta(Meta),
    Order(Order),
    Param(Param),
    Pin(Pin),
    Package(Package),
    Port(Port),
    Peripheral(Peripheral),
    Derive(Derive),
    Text(Text),
    Assert(Assert),
    /// A child block: an application circuit, and a feature of the part.
    Block(Block),
}

impl Part {
    pub fn meta(&self, key: MetaKey) -> Option<&Value> {
        self.items.iter().find_map(|i| match i {
            PartItem::Meta(m) if m.key == key => Some(&m.value),
            _ => None,
        })
    }

    pub fn pins(&self) -> impl Iterator<Item = &Pin> {
        self.items.iter().filter_map(|i| match i {
            PartItem::Pin(p) => Some(p),
            _ => None,
        })
    }

    pub fn packages(&self) -> impl Iterator<Item = &Package> {
        self.items.iter().filter_map(|i| match i {
            PartItem::Package(p) => Some(p),
            _ => None,
        })
    }

    pub fn order(&self) -> Option<&Order> {
        self.items.iter().find_map(|i| match i {
            PartItem::Order(o) => Some(o),
            _ => None,
        })
    }

    /// A metadata value as it stands in one package: the package's own, else the part's.
    pub fn meta_in<'a>(&'a self, package: Option<&'a Package>, key: MetaKey) -> Option<&'a Value> {
        package.and_then(|p| p.meta(key)).or_else(|| self.meta(key))
    }

    /// The orderable numbers as they stand in one package, the same way.
    pub fn order_in<'a>(&'a self, package: Option<&'a Package>) -> Option<&'a Order> {
        package.and_then(|p| p.order()).or_else(|| self.order())
    }

    pub fn ports(&self) -> impl Iterator<Item = &Port> {
        self.items.iter().filter_map(|i| match i {
            PartItem::Port(p) => Some(p),
            _ => None,
        })
    }

    pub fn peripherals(&self) -> impl Iterator<Item = &Peripheral> {
        self.items.iter().filter_map(|i| match i {
            PartItem::Peripheral(p) => Some(p),
            _ => None,
        })
    }

    pub fn blocks(&self) -> impl Iterator<Item = &Block> {
        self.items.iter().filter_map(|i| match i {
            PartItem::Block(b) => Some(b),
            _ => None,
        })
    }

    pub fn params(&self) -> impl Iterator<Item = &Param> {
        self.items.iter().filter_map(|i| match i {
            PartItem::Param(p) => Some(p),
            _ => None,
        })
    }
}

/// A part's metadata statements: `symbol "Device:R"`, `reference R`, ….
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetaKey {
    Symbol,
    Reference,
    Value,
    Datasheet,
    Description,
    Manufacturer,
}

impl MetaKey {
    pub const ALL: [MetaKey; 6] = [
        MetaKey::Symbol,
        MetaKey::Reference,
        MetaKey::Value,
        MetaKey::Datasheet,
        MetaKey::Description,
        MetaKey::Manufacturer,
    ];

    pub fn statement(self) -> &'static str {
        match self {
            MetaKey::Symbol => "symbol",
            MetaKey::Reference => "reference",
            MetaKey::Value => "value",
            MetaKey::Datasheet => "datasheet",
            MetaKey::Description => "description",
            MetaKey::Manufacturer => "manufacturer",
        }
    }

    pub fn from_statement(name: &str) -> Option<Self> {
        MetaKey::ALL.into_iter().find(|k| k.statement() == name)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Meta {
    pub key: MetaKey,
    pub value: Value,
    pub span: Span,
}

/// `order mpn=… mouser=… lcsc=…` — orderable identifiers, one property per source.
#[derive(Clone, Debug, PartialEq)]
pub struct Order {
    pub props: Vec<Property>,
    pub span: Span,
}

/// `param <name> <type> [default=<value>]`
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    /// A quantity type (`resistance`, `voltage`, …), `part`, or an enumerated type's name.
    pub ty: String,
    pub default: Option<Value>,
    pub span: Span,
}

/// `pin <name> <kind> [required] [once] [symbol="…"] [symbol-kind=<kind>] { also …; role …; require … }`
#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub name: String,
    /// The electrical kind, in KiCad's vocabulary.
    pub kind: String,
    pub required: bool,
    pub once: bool,
    pub symbol: Option<String>,
    pub symbol_kind: Option<String>,
    pub also: Vec<Also>,
    pub roles: Vec<Role>,
    /// What the pin's own net must satisfy: a strap's rest level, a voltage it tolerates.
    pub requires: Vec<Require>,
    pub span: Span,
}

/// `also <name>` — another port-pin name the same pin can be.
#[derive(Clone, Debug, PartialEq)]
pub struct Also {
    pub name: String,
    pub span: Span,
}

/// `role <role> [<prop>=…]` — what a pin is for. Its vocabulary is open; the properties are kept
/// as written.
#[derive(Clone, Debug, PartialEq)]
pub struct Role {
    pub role: String,
    pub props: Vec<Property>,
    pub span: Span,
}

/// `package <name> footprint="…" { symbol …; order …; pad <pin> <label> … }`
///
/// A package maps pins to pads, and carries what is true of the part in this body and not in
/// another: its symbol, its orderable numbers, its description. What it does not say is the
/// part's.
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    pub name: String,
    pub footprint: Option<String>,
    pub items: Vec<PackageItem>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PackageItem {
    /// `symbol`, `value`, `datasheet` or `description` — never `reference` or `manufacturer`,
    /// which a body does not change.
    Meta(Meta),
    Order(Order),
    Pad(Pad),
}

impl Package {
    pub fn pads(&self) -> impl Iterator<Item = &Pad> {
        self.items.iter().filter_map(|i| match i {
            PackageItem::Pad(p) => Some(p),
            _ => None,
        })
    }

    /// Whether this body bonds the pin at all.
    pub fn bonds(&self, pin: &str) -> bool {
        self.pads().any(|p| p.pin == pin)
    }

    pub fn meta(&self, key: MetaKey) -> Option<&Value> {
        self.items.iter().find_map(|i| match i {
            PackageItem::Meta(m) if m.key == key => Some(&m.value),
            _ => None,
        })
    }

    pub fn order(&self) -> Option<&Order> {
        self.items.iter().find_map(|i| match i {
            PackageItem::Order(o) => Some(o),
            _ => None,
        })
    }
}

/// `pad <pin> <label>`. A label is text even when it looks like a number: `4` and `A1` are both
/// labels.
#[derive(Clone, Debug, PartialEq)]
pub struct Pad {
    pub pin: String,
    pub label: String,
    pub span: Span,
}

/// `port <name> [type=<type>] [pin=<pin>] { line …; set/add/claim …; require …; assert … }`
#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub name: String,
    /// `None` is the default type, `terminal`.
    pub ty: Option<String>,
    /// For a single-line port, the pin its line is on.
    pub pin: Option<String>,
    pub items: Vec<PortItem>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PortItem {
    Line(Line),
    Fact(Fact),
    Require(Require),
    Assert(Assert),
}

impl Port {
    pub fn lines(&self) -> impl Iterator<Item = &Line> {
        self.items.iter().filter_map(|i| match i {
            PortItem::Line(l) => Some(l),
            _ => None,
        })
    }

    pub fn facts(&self) -> impl Iterator<Item = &Fact> {
        self.items.iter().filter_map(|i| match i {
            PortItem::Fact(f) => Some(f),
            _ => None,
        })
    }

    pub fn requires(&self) -> impl Iterator<Item = &Require> {
        self.items.iter().filter_map(|i| match i {
            PortItem::Require(r) => Some(r),
            _ => None,
        })
    }
}

/// `line <name> [type=<type>] [pin=<pin>] [optional] { require …; match … }`
///
/// In a port, a line maps onto a pin. In a type, a line has a type of its own, requirements and
/// possibly a `match`. One struct serves both, with the fields the other side leaves empty.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub name: String,
    pub ty: Option<String>,
    pub pin: Option<String>,
    pub optional: bool,
    pub items: Vec<LineItem>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LineItem {
    Require(Require),
    Match(Match),
    /// A fact contributed to this line alone: `line rail { set name "VPWR" }`.
    Fact(Fact),
}

/// `match <type>.<line>` — the line this one meets on the other side of a link.
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub target: Reference,
    pub span: Span,
}

/// What a fact is shared across.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Aspect {
    /// One connection between two endpoints.
    Segment,
    /// Everything at one potential.
    Net,
    /// One piece of information, across every net that carries it.
    Signal,
}

impl Aspect {
    pub fn name(self) -> &'static str {
        match self {
            Aspect::Segment => "segment",
            Aspect::Net => "net",
            Aspect::Signal => "signal",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "segment" => Some(Aspect::Segment),
            "net" => Some(Aspect::Net),
            "signal" => Some(Aspect::Signal),
            _ => None,
        }
    }
}

/// `require <aspect>.<fact> [<prop>=…]` — a constraint on the facts of whatever a port is joined
/// to.
#[derive(Clone, Debug, PartialEq)]
pub struct Require {
    pub aspect: Aspect,
    pub fact: String,
    /// A level or value the fact must equal (`require net.rest high`), where the requirement
    /// is not a range (`min=`/`max=`) or a flag (`require signal.dma`).
    pub value: Option<Value>,
    pub props: Vec<Property>,
    pub span: Span,
}

/// How several contributions to one fact combine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// One writer; two conflict.
    Set,
    /// Accumulates.
    Add,
    /// Distinct by value.
    Claim,
}

impl Rule {
    pub fn statement(self) -> &'static str {
        match self {
            Rule::Set => "set",
            Rule::Add => "add",
            Rule::Claim => "claim",
        }
    }

    pub fn from_statement(name: &str) -> Option<Self> {
        match name {
            "set" => Some(Rule::Set),
            "add" => Some(Rule::Add),
            "claim" => Some(Rule::Claim),
            _ => None,
        }
    }
}

/// `set|add|claim <aspect>.<fact> [<value>] [<prop>=…]`
///
/// On a port it contributes a value (`set net.voltage "3.3V" tolerance="2%"`,
/// `claim signal.address address`). In a type it declares the fact with no value
/// (`claim signal.address`). The aspect — `net`, `segment` or `signal` — is what the fact is
/// shared across, and every contribution names it.
#[derive(Clone, Debug, PartialEq)]
pub struct Fact {
    pub rule: Rule,
    pub aspect: Aspect,
    pub fact: String,
    pub value: Option<Value>,
    pub props: Vec<Property>,
    pub span: Span,
}

/// `peripheral <name> <kind> { has …; <line> <pin>… { has … } … }`
#[derive(Clone, Debug, PartialEq)]
pub struct Peripheral {
    pub name: String,
    pub kind: String,
    /// Capabilities of the instance.
    pub has: Vec<Has>,
    pub lines: Vec<PeripheralLine>,
    pub span: Span,
}

/// `has <capability> [<prop>=…]`
#[derive(Clone, Debug, PartialEq)]
pub struct Has {
    pub capability: String,
    pub props: Vec<Property>,
    pub span: Span,
}

/// `<line> <pin> <pin>… { has … }` — every port pin a line of the instance can appear on.
#[derive(Clone, Debug, PartialEq)]
pub struct PeripheralLine {
    pub name: String,
    pub pins: Vec<String>,
    pub has: Vec<Has>,
    pub span: Span,
}

/// `derive <name> "<expr>"`
#[derive(Clone, Debug, PartialEq)]
pub struct Derive {
    pub name: String,
    pub expr: String,
    pub span: Span,
}

/// `text <name> "<template>"`
#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub name: String,
    pub template: String,
    pub span: Span,
}

/// `assert "<expr>" [message="<template>"]`
#[derive(Clone, Debug, PartialEq)]
pub struct Assert {
    pub expr: String,
    pub message: Option<String>,
    pub span: Span,
}

// --- Blocks ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Block,
    Design,
}

/// `block <name> { … }` or `design <name> { … }`.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub kind: BlockKind,
    pub name: String,
    pub items: Vec<BlockItem>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BlockItem {
    /// `default` — a part's child block placed unless the placement turns it off.
    Default(Span),
    /// `when "<expr>"` — a part's child block placed when its condition holds.
    When(When),
    Param(Param),
    Port(Port),
    Derive(Derive),
    Text(Text),
    Assert(Assert),
    Place(Place),
    Circuit(Circuit),
    Connect(Connect),
    Nc(Nc),
    /// `set <terminal> <aspect>.<fact> …` — a fact on a terminal's net.
    Fact(TerminalFact),
    Scope(Scope),
    Stock(Stock),
    /// A child block — a feature of this block, placed as a part's child blocks are (§7).
    Block(Block),
}

impl Block {
    /// The block's child blocks: its features.
    pub fn blocks(&self) -> impl Iterator<Item = &Block> {
        self.items.iter().filter_map(|i| match i {
            BlockItem::Block(b) => Some(b),
            _ => None,
        })
    }

    pub fn is_default(&self) -> bool {
        self.items
            .iter()
            .any(|i| matches!(i, BlockItem::Default(_)))
    }

    pub fn when(&self) -> Option<&When> {
        self.items.iter().find_map(|i| match i {
            BlockItem::When(w) => Some(w),
            _ => None,
        })
    }

    pub fn params(&self) -> impl Iterator<Item = &Param> {
        self.items.iter().filter_map(|i| match i {
            BlockItem::Param(p) => Some(p),
            _ => None,
        })
    }

    pub fn ports(&self) -> impl Iterator<Item = &Port> {
        self.items.iter().filter_map(|i| match i {
            BlockItem::Port(p) => Some(p),
            _ => None,
        })
    }

    pub fn places(&self) -> impl Iterator<Item = &Place> {
        self.items.iter().filter_map(|i| match i {
            BlockItem::Place(p) => Some(p),
            _ => None,
        })
    }

    pub fn circuits(&self) -> impl Iterator<Item = &Circuit> {
        self.items.iter().filter_map(|i| match i {
            BlockItem::Circuit(c) => Some(c),
            _ => None,
        })
    }

    pub fn connects(&self) -> impl Iterator<Item = &Connect> {
        self.items.iter().filter_map(|i| match i {
            BlockItem::Connect(c) => Some(c),
            _ => None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct When {
    pub expr: String,
    pub span: Span,
}

/// `scope <name> { … }` — a level of hierarchy with nothing placed at it.
#[derive(Clone, Debug, PartialEq)]
pub struct Scope {
    pub name: String,
    pub items: Vec<BlockItem>,
    pub span: Span,
}

/// `stock { … }` — a design's purchasing policy. Its contents are outside the format so far, and
/// are kept as written.
#[derive(Clone, Debug, PartialEq)]
pub struct Stock {
    pub items: Vec<Statement>,
    pub span: Span,
}

/// A statement kept as written: name, arguments, properties and children.
#[derive(Clone, Debug, PartialEq)]
pub struct Statement {
    pub name: String,
    pub args: Vec<Value>,
    pub props: Vec<Property>,
    pub children: Vec<Statement>,
    pub span: Span,
}

/// `place <part-or-block> [<name>] [<arg>=<value>…] { with …; without … }`
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    /// The part or block placed.
    pub what: String,
    /// The placed name. `None` for `as=self`, where the placement takes the block's path.
    pub name: Option<String>,
    /// Parameters, ports and language properties (`value=`, `intent=`, `note=`, `as=`), as written.
    pub args: Vec<Property>,
    pub features: Vec<Feature>,
    pub ignores: Vec<Ignore>,
    pub span: Span,
}

/// An exact, placement-scoped acknowledgement of a requirement or assertion.
#[derive(Clone, Debug, PartialEq)]
pub struct Ignore {
    pub target: String,
    pub reason: String,
    pub span: Span,
}

impl Place {
    pub fn arg(&self, key: &str) -> Option<&Value> {
        self.args.iter().find(|p| p.key == key).map(|p| &p.value)
    }

    /// `as=self`.
    pub fn is_self(&self) -> bool {
        matches!(self.arg("as"), Some(Value::Name(n)) if n == "self")
    }
}

/// `with <feature>` or `without <feature>`.
#[derive(Clone, Debug, PartialEq)]
pub struct Feature {
    pub on: bool,
    pub name: String,
    pub span: Span,
}

/// `set <terminal> <aspect>.<fact> [<value>] [<prop>=…]` (or `add`, `claim`) — the statement
/// a port line uses, written in a block and aimed at a terminal in scope. A net's name is the
/// common case: `set charger.vsys.rail net.name "VSYS"`.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalFact {
    pub terminal: Reference,
    pub fact: Fact,
    pub span: Span,
}

/// A one-line path, or `circuit { from <terminal>; to <elements> [name=…]; … }`.
#[derive(Clone, Debug, PartialEq)]
pub struct Circuit {
    /// One-line form. Empty in the multiline form.
    pub elements: Vec<Reference>,
    /// `name=`, `current=`, … — not yet specified, kept as written.
    pub props: Vec<Property>,
    /// The start of a multiline path. Absent in the one-line form.
    pub from: Option<Reference>,
    pub legs: Vec<CircuitLeg>,
    pub span: Span,
}

/// One equipotential section of a multiline circuit. A series element may occur only last.
#[derive(Clone, Debug, PartialEq)]
pub struct CircuitLeg {
    pub elements: Vec<Reference>,
    pub props: Vec<Property>,
    pub span: Span,
}

/// `nc <pin>… [note="…"]` — pins deliberately joined to nothing. A pin declared no-connect that
/// later lands on a net is a finding; a spare pin nothing declares is only a spare pin.
#[derive(Clone, Debug, PartialEq)]
pub struct Nc {
    pub pins: Vec<Reference>,
    pub note: Option<String>,
    pub span: Span,
}

/// `connect <type> { from …; to … }`, or `connect <type> from=<side> to=<side>`.
#[derive(Clone, Debug, PartialEq)]
pub struct Connect {
    pub ty: String,
    pub sides: Vec<Side>,
    /// Lines of the type this link deliberately leaves unbound: `nc miso`.
    pub unbound: Vec<Unbound>,
    pub span: Span,
}

/// `nc <line> … [note=…]` inside a `connect`: a line of the type that nothing on the `to` side
/// receives. The `from` side may still answer it, and the pin it answers with is then a declared
/// no-connect.
#[derive(Clone, Debug, PartialEq)]
pub struct Unbound {
    pub lines: Vec<String>,
    pub note: Option<String>,
    pub span: Span,
}

impl Connect {
    pub fn from(&self) -> impl Iterator<Item = &Side> {
        self.sides.iter().filter(|s| s.role == SideRole::From)
    }

    pub fn to(&self) -> impl Iterator<Item = &Side> {
        self.sides.iter().filter(|s| s.role == SideRole::To)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideRole {
    From,
    To,
}

impl SideRole {
    pub fn statement(self) -> &'static str {
        match self {
            SideRole::From => "from",
            SideRole::To => "to",
        }
    }
}

/// One end of a link: `from <side> [<line>=<pin>…]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Side {
    pub role: SideRole,
    /// A placement, a port or a line.
    pub target: Reference,
    /// The type's lines, each answered with the side's own pin or line.
    pub answers: Vec<Property>,
    pub span: Span,
}

// --- Types ----------------------------------------------------------------------------------------

/// `type <name> { line …; claim …; assert … }`
#[derive(Clone, Debug, PartialEq)]
pub struct TypeDecl {
    pub name: String,
    pub items: Vec<TypeItem>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TypeItem {
    Line(Line),
    Fact(Fact),
    Assert(Assert),
}

impl TypeDecl {
    pub fn lines(&self) -> impl Iterator<Item = &Line> {
        self.items.iter().filter_map(|i| match i {
            TypeItem::Line(l) => Some(l),
            _ => None,
        })
    }
}
