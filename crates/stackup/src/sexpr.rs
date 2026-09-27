//! A minimal S-expression writer, for KiCad's file formats.

pub enum Sexpr {
    Atom { text: String, quoted: bool },
    List(List),
}

pub struct List {
    pub items: Vec<Sexpr>,
}

impl List {
    /// A list whose head is the bare keyword `head`, e.g. `(comp …)`.
    pub fn named(head: &str) -> List {
        List {
            items: vec![Sexpr::Atom {
                text: head.into(),
                quoted: false,
            }],
        }
    }

    pub fn push(&mut self, item: impl Into<Sexpr>) -> &mut Self {
        self.items.push(item.into());
        self
    }

    /// Appends `(name "value")`, the shape most KiCad fields take.
    pub fn field(&mut self, name: &str, value: impl Into<String>) -> &mut Self {
        let mut list = List::named(name);
        list.push(Sexpr::Atom {
            text: value.into(),
            quoted: true,
        });
        self.push(list)
    }

    /// Appends `(name bare)`, for an enum value KiCad writes unquoted.
    pub fn bare(&mut self, name: &str, value: impl Into<String>) -> &mut Self {
        let mut list = List::named(name);
        list.push(Sexpr::Atom {
            text: value.into(),
            quoted: false,
        });
        self.push(list)
    }

    /// Renders as KiCad writes it: one item per line, tab-indented, a list of only atoms on one
    /// line.
    pub fn render(&self) -> String {
        let mut out = String::new();
        write_list(&mut out, self, 0);
        out.push('\n');
        out
    }
}

impl From<List> for Sexpr {
    fn from(list: List) -> Sexpr {
        Sexpr::List(list)
    }
}

fn write_list(out: &mut String, list: &List, depth: usize) {
    let inline = list.items.iter().all(|i| matches!(i, Sexpr::Atom { .. }));
    out.push('(');
    for (i, item) in list.items.iter().enumerate() {
        if i > 0 {
            if inline {
                out.push(' ');
            } else {
                out.push('\n');
                for _ in 0..=depth {
                    out.push('\t');
                }
            }
        }
        match item {
            Sexpr::Atom { text, quoted: true } => write_quoted(out, text),
            Sexpr::Atom {
                text,
                quoted: false,
            } => out.push_str(text),
            Sexpr::List(l) => write_list(out, l, depth + 1),
        }
    }
    if !inline {
        out.push('\n');
        for _ in 0..depth {
            out.push('\t');
        }
    }
    out.push(')');
}

fn write_quoted(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
}
