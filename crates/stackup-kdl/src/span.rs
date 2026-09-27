use miette::SourceSpan;

/// A byte range in a document's source text.
///
/// Every statement the reader produces carries the span of the KDL node it came from, and a span
/// is also how an edit names the node it applies to (see [`Document`](crate::Document)). Spans
/// are positions in the text that was *parsed*: after an edit, the text has moved, and a document
/// is reparsed before its spans are trusted again.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    pub offset: usize,
    pub len: usize,
}

impl Span {
    pub fn new(offset: usize, len: usize) -> Self {
        Span { offset, len }
    }

    pub fn end(self) -> usize {
        self.offset + self.len
    }
}

impl From<SourceSpan> for Span {
    fn from(s: SourceSpan) -> Self {
        Span {
            offset: s.offset(),
            len: s.len(),
        }
    }
}

impl From<Span> for SourceSpan {
    fn from(s: Span) -> Self {
        SourceSpan::new(s.offset.into(), s.len)
    }
}
