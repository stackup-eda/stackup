//! Findings across several files.
//!
//! A finding is one diagnostic against the file it points into, plus **related** findings in
//! other files: an assertion that fails is reported where it is written, and the statements that
//! made the values it read are reported beneath it, wherever they live. Rendered plain for a
//! test and a program, or through miette's graphical reporter for a terminal.

use std::{fmt, sync::Arc};

use stackup_kdl::{Diagnostic, Diagnostics, Severity, Source, Span, diag};

/// One finding, with the file it points into.
#[derive(Clone, Debug)]
pub struct Finding {
    pub source: Arc<Source>,
    pub diagnostic: Diagnostic,
    /// Findings this one rests on, in other files or the same one: where a value came from.
    pub related: Vec<Finding>,
}

impl Finding {
    pub fn new(source: &Arc<Source>, diagnostic: Diagnostic) -> Finding {
        Finding {
            source: source.clone(),
            diagnostic,
            related: Vec::new(),
        }
    }

    pub fn with_related(mut self, f: Finding) -> Finding {
        self.related.push(f);
        self
    }

    /// Plain text: the diagnostic, then each related finding indented under it.
    pub fn render(&self) -> String {
        let mut out = diag::render_one(&self.source, &self.diagnostic);
        for r in &self.related {
            for line in r.render().lines() {
                out.push_str("    ");
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.diagnostic.message)
    }
}

impl std::error::Error for Finding {}

impl miette::Diagnostic for Finding {
    fn severity(&self) -> Option<miette::Severity> {
        Some(match self.diagnostic.severity {
            Severity::Error => miette::Severity::Error,
            Severity::Warning => miette::Severity::Warning,
            Severity::Note => miette::Severity::Advice,
        })
    }

    fn source_code(&self) -> Option<&dyn miette::SourceCode> {
        Some(&self.source.text)
    }

    fn help<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        self.diagnostic
            .help
            .as_ref()
            .map(|h| Box::new(h.clone()) as Box<dyn fmt::Display>)
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = miette::LabeledSpan> + '_>> {
        let primary = self
            .diagnostic
            .span
            .map(|s| miette::LabeledSpan::new_primary_with_span(None, s));
        let rest = self
            .diagnostic
            .labels
            .iter()
            .map(|l| miette::LabeledSpan::new_with_span(Some(l.text.clone()), l.span));
        Some(Box::new(primary.into_iter().chain(rest)))
    }

    fn related<'a>(&'a self) -> Option<Box<dyn Iterator<Item = &'a dyn miette::Diagnostic> + 'a>> {
        if self.related.is_empty() {
            return None;
        }
        Some(Box::new(
            self.related.iter().map(|f| f as &dyn miette::Diagnostic),
        ))
    }
}

/// Everything found wrong, in the order it was found. Never stops at the first: a design is
/// judged whole.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn error(&mut self, source: &Arc<Source>, span: Span, message: impl Into<String>) {
        self.findings
            .push(Finding::new(source, Diagnostic::error(span, message)));
    }

    pub fn warning(&mut self, source: &Arc<Source>, span: Span, message: impl Into<String>) {
        self.findings
            .push(Finding::new(source, Diagnostic::warning(span, message)));
    }

    pub fn note(&mut self, source: &Arc<Source>, span: Span, message: impl Into<String>) {
        self.findings
            .push(Finding::new(source, Diagnostic::note(span, message)));
    }

    pub fn push(&mut self, source: &Arc<Source>, diagnostic: Diagnostic) {
        self.findings.push(Finding::new(source, diagnostic));
    }

    pub fn add(&mut self, finding: Finding) {
        self.findings.push(finding);
    }

    pub fn extend(&mut self, diags: Diagnostics) {
        for d in diags.items {
            self.push(&diags.source, d);
        }
    }

    pub fn has_errors(&self) -> bool {
        self.findings
            .iter()
            .any(|f| f.diagnostic.severity == Severity::Error)
    }

    /// Nothing wrong: no errors and no warnings. Notes are allowed, since a note is a check that
    /// held for what was stated.
    pub fn is_clean(&self) -> bool {
        !self
            .findings
            .iter()
            .any(|f| f.diagnostic.severity != Severity::Note)
    }

    pub fn is_empty(&self) -> bool {
        self.findings.is_empty()
    }

    pub fn messages(&self) -> Vec<String> {
        self.findings
            .iter()
            .map(|f| f.diagnostic.message.clone())
            .collect()
    }

    pub fn render(&self) -> String {
        self.findings.iter().map(Finding::render).collect()
    }

    /// miette's graphical rendering: unicode, and colour when asked for.
    pub fn render_fancy(&self, colour: bool) -> String {
        let theme = if colour {
            miette::GraphicalTheme::unicode()
        } else {
            miette::GraphicalTheme::unicode_nocolor()
        };
        let handler = miette::GraphicalReportHandler::new_themed(theme).with_context_lines(2);
        let mut out = String::new();
        for f in &self.findings {
            let _ = handler.render_report(&mut out, f);
            out.push('\n');
        }
        out
    }
}
