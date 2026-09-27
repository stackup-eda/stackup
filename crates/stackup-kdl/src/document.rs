use std::{fmt, path::Path, sync::Arc};

use kdl::{KdlDocument, KdlEntry, KdlEntryFormat, KdlNode};

use crate::{
    ast::File,
    diag::{Diagnostic, Diagnostics, Severity},
    lower,
    manifest::Manifest,
    span::Span,
    value::Value,
};

/// A document's name and the text it was parsed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub name: String,
    pub text: String,
}

impl Source {
    /// 1-based line and column (in characters) of a byte offset.
    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.text.len());
        let before = &self.text[..offset];
        let line = before.matches('\n').count() + 1;
        let col = before
            .rsplit('\n')
            .next()
            .map(|l| l.chars().count())
            .unwrap_or(0)
            + 1;
        (line, col)
    }

    /// The text of a 1-based line, without its newline.
    pub fn line_text(&self, line: usize) -> &str {
        self.text.lines().nth(line.saturating_sub(1)).unwrap_or("")
    }
}

/// A stackup file: its text, parsed as KDL with every comment and space kept.
///
/// A document is read into a [`File`] with [`Document::file`], and written back with
/// [`Document::to_string`], which reproduces the text exactly unless the document was edited.
/// Edits are made through the methods here, which change only the node they name and leave the
/// rest of the text as it was — which is what lets a program write an answer into a `connect` a
/// person wrote, without reformatting their file.
///
/// A node is named by its [`Span`], which every statement in a [`File`] carries. Spans are
/// positions in the text that was parsed, so after an edit the document is [`reparse`]d before
/// its spans are used again.
///
/// [`reparse`]: Document::reparse
#[derive(Debug, Clone)]
pub struct Document {
    source: Arc<Source>,
    kdl: KdlDocument,
}

impl Document {
    pub fn parse(name: impl Into<String>, text: impl Into<String>) -> Result<Self, Diagnostics> {
        let source = Arc::new(Source {
            name: name.into(),
            text: text.into(),
        });
        match KdlDocument::parse(&source.text) {
            Ok(kdl) => Ok(Document { source, kdl }),
            Err(e) => Err(kdl_diagnostics(source, e)),
        }
    }

    /// Reads and parses a file. A file that cannot be read is a diagnostic with no span.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Diagnostics> {
        let path = path.as_ref();
        let name = path.display().to_string();
        match std::fs::read_to_string(path) {
            Ok(text) => Document::parse(name, text),
            Err(e) => Err(Diagnostics {
                source: Arc::new(Source {
                    name: name.clone(),
                    text: String::new(),
                }),
                items: vec![Diagnostic {
                    labels: Vec::new(),
                    severity: Severity::Error,
                    message: format!("can't read {name}: {e}"),
                    span: None,
                    help: None,
                }],
            }),
        }
    }

    pub fn name(&self) -> &str {
        &self.source.name
    }

    /// The text that was parsed. After an edit, [`Document::to_string`] is the current text.
    pub fn text(&self) -> &str {
        &self.source.text
    }

    pub fn source(&self) -> &Arc<Source> {
        &self.source
    }

    /// The underlying KDL, for what the typed API does not cover.
    pub fn kdl(&self) -> &KdlDocument {
        &self.kdl
    }

    pub fn kdl_mut(&mut self) -> &mut KdlDocument {
        &mut self.kdl
    }

    /// Reads the document into the typed model, with everything found wrong with it. The file is
    /// as complete as the diagnostics allow: a statement that could not be read is left out.
    pub fn file(&self) -> (File, Diagnostics) {
        let (file, items) = lower::lower(&self.kdl);
        (
            file,
            Diagnostics {
                source: self.source.clone(),
                items,
            },
        )
    }

    /// Reads the document as a manifest — `manifest.kdl`, a different document from a design
    /// file with statements of its own — with everything found wrong with it.
    pub fn manifest(&self) -> (Manifest, Diagnostics) {
        let (manifest, items) = lower::lower_manifest(&self.kdl);
        (
            manifest,
            Diagnostics {
                source: self.source.clone(),
                items,
            },
        )
    }

    /// Parses and reads in one step, failing on any error.
    pub fn read(name: impl Into<String>, text: impl Into<String>) -> Result<File, Diagnostics> {
        let doc = Document::parse(name, text)?;
        let (file, diags) = doc.file();
        if diags.has_errors() {
            Err(diags)
        } else {
            Ok(file)
        }
    }

    /// The document as it now stands, parsed afresh so its spans are current.
    pub fn reparse(&self) -> Result<Document, Diagnostics> {
        Document::parse(self.source.name.clone(), self.to_string())
    }

    // --- Nodes by span --------------------------------------------------------------------------------

    /// The KDL node a statement came from.
    pub fn node(&self, at: Span) -> Option<&KdlNode> {
        find(self.kdl.nodes(), at, 0).map(|(n, _)| n)
    }

    pub fn node_mut(&mut self, at: Span) -> Option<&mut KdlNode> {
        find_mut(self.kdl.nodes_mut(), at, 0).map(|(n, _)| n)
    }

    // --- Edits ------------------------------------------------------------------------------------------

    /// Sets a property on the statement at `at`, replacing its value if it has one and adding it
    /// after the last entry otherwise. Nothing else on the line moves.
    pub fn set_property(&mut self, at: Span, key: &str, value: &Value) -> Result<(), Diagnostics> {
        let missing = self.no_statement(at);
        let node = self.node_mut(at).ok_or(missing)?;
        let repr = value.repr();
        if let Some(entry) = node.entry_mut(key) {
            entry.set_value(value.to_kdl());
            let mut format = entry.format().cloned().unwrap_or_default();
            format.value_repr = repr;
            if format.leading.is_empty() {
                format.leading = " ".into();
            }
            entry.set_format(format);
        } else {
            let mut entry = KdlEntry::new_prop(key, value.to_kdl());
            entry.set_format(KdlEntryFormat {
                value_repr: repr,
                leading: " ".into(),
                ..Default::default()
            });
            node.entries_mut().push(entry);
        }
        Ok(())
    }

    /// Removes a property from the statement at `at`. `Ok(false)` if it had none.
    pub fn remove_property(&mut self, at: Span, key: &str) -> Result<bool, Diagnostics> {
        let missing = self.no_statement(at);
        let node = self.node_mut(at).ok_or(missing)?;
        Ok(node.remove(key).is_some())
    }

    /// Inserts a statement, written as KDL text, as a child of the statement at `parent` (or at
    /// the top level for `None`), at index `at` among its children or last. The text is indented
    /// to fit where it lands; a multi-line statement is written as it would be at the top level.
    pub fn insert(
        &mut self,
        parent: Option<Span>,
        at: Option<usize>,
        statement: &str,
    ) -> Result<(), Diagnostics> {
        let depth = match parent {
            None => 0,
            Some(p) => {
                find(self.kdl.nodes(), p, 0)
                    .ok_or_else(|| self.no_statement(p))?
                    .1
                    + 1
            }
        };
        let node = parse_statement(statement, depth, None)?;
        let nodes = match parent {
            None => self.kdl.nodes_mut(),
            Some(p) => {
                let (n, _) = find_mut(self.kdl.nodes_mut(), p, 0).expect("found above");
                if n.children().is_none()
                    && let Some(f) = n.format_mut()
                    && f.before_children.is_empty()
                {
                    f.before_children = " ".into();
                }
                n.ensure_children().nodes_mut()
            }
        };
        let at = at.unwrap_or(nodes.len()).min(nodes.len());
        nodes.insert(at, node);
        Ok(())
    }

    /// Replaces the statement at `at` with one written as KDL text, keeping the comments and blank
    /// lines around the old one.
    pub fn replace(&mut self, at: Span, statement: &str) -> Result<(), Diagnostics> {
        let (_, depth) = find(self.kdl.nodes(), at, 0).ok_or_else(|| self.no_statement(at))?;
        let (nodes, i) = find_parent_mut(self.kdl.nodes_mut(), at).expect("found above");
        let old = nodes[i].format().cloned();
        nodes[i] = parse_statement(statement, depth, old)?;
        Ok(())
    }

    /// Removes the statement at `at`, with the comments that led into it.
    pub fn remove(&mut self, at: Span) -> Result<(), Diagnostics> {
        let missing = self.no_statement(at);
        let (nodes, i) = find_parent_mut(self.kdl.nodes_mut(), at).ok_or(missing)?;
        nodes.remove(i);
        Ok(())
    }

    fn no_statement(&self, at: Span) -> Diagnostics {
        Diagnostics {
            source: self.source.clone(),
            items: vec![
                Diagnostic::error(at, "no statement starts here")
                    .with_help("spans are positions in the text as parsed; reparse after editing"),
            ],
        }
    }
}

impl fmt::Display for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.kdl, f)
    }
}

fn kdl_diagnostics(source: Arc<Source>, e: kdl::KdlError) -> Diagnostics {
    let items = e
        .diagnostics
        .iter()
        .map(|d| Diagnostic {
            labels: Vec::new(),
            severity: match d.severity {
                miette::Severity::Warning | miette::Severity::Advice => Severity::Warning,
                miette::Severity::Error => Severity::Error,
            },
            message: d.message.clone().unwrap_or_else(|| "invalid KDL".into()),
            span: Some(d.span.into()),
            help: d.help.clone(),
        })
        .collect();
    Diagnostics { source, items }
}

/// Parses one statement for insertion at `depth`, indenting its lines to match. With `keep`, the
/// leading and trailing text of the node being replaced is carried over.
fn parse_statement(
    statement: &str,
    depth: usize,
    keep: Option<kdl::KdlNodeFormat>,
) -> Result<KdlNode, Diagnostics> {
    let indent = "    ".repeat(depth);
    let text: String = statement
        .trim()
        .lines()
        .enumerate()
        .map(|(i, l)| {
            if i == 0 || l.trim().is_empty() {
                l.to_string()
            } else {
                format!("{indent}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut node = KdlNode::parse(&text).map_err(|e| {
        kdl_diagnostics(
            Arc::new(Source {
                name: "<statement>".into(),
                text: text.clone(),
            }),
            e,
        )
    })?;
    let mut format = node.format().cloned().unwrap_or_default();
    match keep {
        Some(old) => {
            format.leading = old.leading;
            format.before_terminator = old.before_terminator;
            format.terminator = old.terminator;
            format.trailing = old.trailing;
        }
        None => {
            format.leading = indent;
            format.before_terminator = String::new();
            format.terminator = "\n".into();
            format.trailing = String::new();
        }
    }
    node.set_format(format);
    Ok(node)
}

fn find(nodes: &[KdlNode], at: Span, depth: usize) -> Option<(&KdlNode, usize)> {
    for n in nodes {
        if Span::from(n.span()) == at {
            return Some((n, depth));
        }
        if let Some(c) = n.children()
            && let Some(found) = find(c.nodes(), at, depth + 1)
        {
            return Some(found);
        }
    }
    None
}

fn find_mut(nodes: &mut [KdlNode], at: Span, depth: usize) -> Option<(&mut KdlNode, usize)> {
    for n in nodes.iter_mut() {
        if Span::from(n.span()) == at {
            return Some((n, depth));
        }
        if let Some(c) = n.children_mut().as_mut()
            && let Some(found) = find_mut(c.nodes_mut(), at, depth + 1)
        {
            return Some(found);
        }
    }
    None
}

/// The list holding the node at `at`, and its index in it.
fn find_parent_mut(nodes: &mut Vec<KdlNode>, at: Span) -> Option<(&mut Vec<KdlNode>, usize)> {
    if let Some(i) = nodes.iter().position(|n| Span::from(n.span()) == at) {
        return Some((nodes, i));
    }
    for n in nodes.iter_mut() {
        if let Some(c) = n.children_mut().as_mut()
            && let Some(found) = find_parent_mut(c.nodes_mut(), at)
        {
            return Some(found);
        }
    }
    None
}
