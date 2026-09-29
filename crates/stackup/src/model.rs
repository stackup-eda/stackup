//! What elaboration produces: the instance tree and the nets.

use std::collections::HashMap;

use stackup_kdl::Span;

use crate::quantity::Quantity;
use crate::{load::DeclRef, load::FileId, report::Report};

/// One thing placed in the design, at a path.
#[derive(Clone, Debug)]
pub struct Instance {
    pub path: String,
    pub parent: Option<usize>,
    pub kind: Kind,
    /// The file and span of the statement that placed it.
    pub file: FileId,
    pub span: Span,
    /// The designator, once assigned (parts only).
    pub designator: Option<String>,
    /// `designator=` on the placement: printed whole.
    pub designator_word: Option<String>,
    /// `reference=` on the placement: the prefix it is numbered under.
    pub reference_prefix: Option<String>,
    pub value: Option<String>,
    pub footprint: Option<String>,
    /// Board-selected ordering and rating fields, overriding a library part's defaults.
    pub fields: HashMap<String, String>,
    /// Minimum component voltage rating required by this use of the placement.
    pub required_voltage: Option<Quantity>,
    /// Installed by hand after outsourced assembly; retained in the purchasing BOM.
    pub hand: bool,
    pub intent: Option<String>,
    /// Physical pad this part belongs beside, as `designator.pad` for KiCad.
    pub anchor: Option<String>,
    /// Exact offset from the anchor pad in the host's frame: `dx dy rotation`.
    pub spot: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum Kind {
    /// A part, and which of its packages the placement chose.
    Part {
        decl: DeclRef,
        package: Option<usize>,
    },
    Block {
        decl: DeclRef,
    },
    Scope,
}

/// Something a net can hold: a pin of a part instance, or a line of a block or part port that
/// maps to no pin.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Terminal {
    Pin {
        inst: usize,
        pin: String,
    },
    Line {
        inst: usize,
        port: String,
        line: String,
    },
}

#[derive(Clone, Debug)]
pub struct Net {
    pub name: String,
    pub members: Vec<Terminal>,
}

#[derive(Clone, Debug)]
pub struct Model {
    pub name: String,
    pub instances: Vec<Instance>,
    pub nets: Vec<Net>,
    /// `stock { packages imperial="0402" }`.
    pub imperial: Option<String>,
    pub report: Report,
}

impl Model {
    pub fn parts(&self) -> impl Iterator<Item = (usize, &Instance)> {
        self.instances
            .iter()
            .enumerate()
            .filter(|(_, i)| matches!(i.kind, Kind::Part { .. }))
    }
}

/// Union-find over terminals.
#[derive(Default)]
pub struct Nets {
    ids: HashMap<Terminal, usize>,
    terminals: Vec<Terminal>,
    parent: Vec<usize>,
}

impl Nets {
    pub fn id(&mut self, t: Terminal) -> usize {
        if let Some(&id) = self.ids.get(&t) {
            return id;
        }
        let id = self.terminals.len();
        self.ids.insert(t.clone(), id);
        self.terminals.push(t);
        self.parent.push(id);
        id
    }

    pub fn find(&mut self, mut a: usize) -> usize {
        while self.parent[a] != a {
            self.parent[a] = self.parent[self.parent[a]];
            a = self.parent[a];
        }
        a
    }

    pub fn union(&mut self, a: Terminal, b: Terminal) {
        let a = self.id(a);
        let b = self.id(b);
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            // Keep the lower index as the root, so a net's identity is its first terminal.
            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
            self.parent[hi] = lo;
        }
    }

    /// Every set with its members, in first-terminal order.
    pub fn sets(&mut self) -> Vec<Vec<Terminal>> {
        let mut groups: Vec<(usize, Vec<Terminal>)> = Vec::new();
        let mut index: HashMap<usize, usize> = HashMap::new();
        for i in 0..self.terminals.len() {
            let root = self.find(i);
            let g = *index.entry(root).or_insert_with(|| {
                groups.push((root, Vec::new()));
                groups.len() - 1
            });
            groups[g].1.push(self.terminals[i].clone());
        }
        groups.into_iter().map(|(_, members)| members).collect()
    }

    pub fn root_of(&mut self, t: &Terminal) -> Option<usize> {
        let id = *self.ids.get(t)?;
        Some(self.find(id))
    }
}
