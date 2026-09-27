//! Reads and writes stackup's KDL design format.
//!
//! This crate is the **syntax** of a design and nothing more: what a file says, statement by
//! statement, with a span on each. It resolves no names, evaluates no expression and judges no
//! design rule — that is the checker's, which sits on top of this.
//!
//! Three layers:
//!
//! - [`Document`] is a file's text parsed as KDL with every comment and space kept. It writes
//!   itself back out unchanged, and takes edits — set a property, insert or replace a statement —
//!   that touch only the statement they name. That is what lets a program answer a `connect` in a
//!   file a person wrote, without reformatting it.
//! - [`ast`] is the typed model, one struct per statement of the format. [`Document::file`] reads
//!   it out, reporting every problem at once and keeping everything it could read.
//! - [`emit`] writes a [`File`](ast::File) as text, for a file a program makes from scratch.
//! - [`manifest`] is the typed model of a `manifest.kdl`, read by [`Document::manifest`]: a
//!   different document, with statements of its own, for what lies outside a design.
//!
//! Two syntactic facts carry meaning in stackup and are kept on both paths: a quoted string is an
//! expression or literal text where a bare identifier is a name ([`Value`]); and a reference's
//! `/`, `.` and `@` select a level, a member and a pad ([`Reference`]).

pub mod ast;
pub mod diag;
mod document;
mod emit;
mod lower;
pub mod manifest;
mod reference;
mod span;
mod value;

pub use diag::{Diagnostic, Diagnostics, Label, Severity};
pub use document::{Document, Source};
pub use emit::{emit, ident, is_bare, quote};
pub use reference::{Reference, ReferenceError};
pub use span::Span;
pub use value::{Property, Value, property};
