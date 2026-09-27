use std::{fmt, sync::Arc};

use crate::{document::Source, span::Span};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    /// Something worth knowing that is not wrong: a check that holds only for what is stated.
    Note,
}

impl Severity {
    pub fn name(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        }
    }
}

/// A further place in the same source a diagnostic points at, with what it says there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub span: Span,
    pub text: String,
}

/// One thing wrong with a document, pointing at where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    /// Where in the source; `None` for a problem with the file as a whole (it could not be read).
    pub span: Option<Span>,
    pub help: Option<String>,
    /// Other places in the same source that bear on it: the statement a value came from.
    pub labels: Vec<Label>,
}

impl Diagnostic {
    pub fn new(severity: Severity, span: Span, message: impl Into<String>) -> Self {
        Diagnostic {
            severity,
            message: message.into(),
            span: Some(span),
            help: None,
            labels: Vec::new(),
        }
    }

    pub fn error(span: Span, message: impl Into<String>) -> Self {
        Diagnostic::new(Severity::Error, span, message)
    }

    pub fn warning(span: Span, message: impl Into<String>) -> Self {
        Diagnostic::new(Severity::Warning, span, message)
    }

    pub fn note(span: Span, message: impl Into<String>) -> Self {
        Diagnostic::new(Severity::Note, span, message)
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn with_label(mut self, span: Span, text: impl Into<String>) -> Self {
        self.labels.push(Label {
            span,
            text: text.into(),
        });
        self
    }
}

/// Everything found wrong with one document, together with the source the spans point into.
///
/// Reading never stops at the first problem: a document's diagnostics are all of them, and a
/// statement that could not be read is left out of the [`File`](crate::ast::File) while the rest
/// is kept.
#[derive(Clone, Debug)]
pub struct Diagnostics {
    pub source: Arc<Source>,
    pub items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new(source: Arc<Source>) -> Self {
        Diagnostics {
            source,
            items: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn has_errors(&self) -> bool {
        self.items.iter().any(|d| d.severity == Severity::Error)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic> {
        self.items.iter()
    }

    /// Renders every diagnostic as `name:line:col: severity: message`, with the source line and a
    /// caret under the span, and each further label the same way beneath it. Plain text, the
    /// same for a person and a program.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for d in &self.items {
            out.push_str(&render_one(&self.source, d));
        }
        out
    }
}

/// One diagnostic against its source, as [`Diagnostics::render`] prints it.
pub fn render_one(source: &Source, d: &Diagnostic) -> String {
    let mut out = String::new();
    let severity = d.severity.name();
    match d.span {
        None => out.push_str(&format!("{}: {severity}: {}\n", source.name, d.message)),
        Some(span) => {
            let (line, col) = source.line_col(span.offset);
            out.push_str(&format!(
                "{}:{line}:{col}: {severity}: {}\n",
                source.name, d.message
            ));
            out.push_str(&caret(source, span));
        }
    }
    for l in &d.labels {
        let (line, col) = source.line_col(l.span.offset);
        out.push_str(&format!("    {}:{line}:{col}: {}\n", source.name, l.text));
        out.push_str(&caret(source, l.span));
    }
    if let Some(help) = &d.help {
        out.push_str(&format!("    help: {help}\n"));
    }
    out
}

/// The source line a span is on, and a caret under the span.
pub fn caret(source: &Source, span: Span) -> String {
    let (line, col) = source.line_col(span.offset);
    let text = source.line_text(line);
    let width = span
        .len
        .clamp(1, text.chars().count().saturating_sub(col - 1).max(1));
    format!(
        "    {text}\n    {}{}\n",
        " ".repeat(col - 1),
        "^".repeat(width)
    )
}

impl fmt::Display for Diagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

impl std::error::Error for Diagnostics {}

impl miette::Diagnostic for Diagnostics {
    fn source_code(&self) -> Option<&dyn miette::SourceCode> {
        Some(&self.source.text)
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = miette::LabeledSpan> + '_>> {
        Some(Box::new(self.items.iter().filter_map(|d| {
            d.span
                .map(|s| miette::LabeledSpan::new_with_span(Some(d.message.clone()), s))
        })))
    }
}
