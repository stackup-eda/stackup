//! A manifest: `manifest.kdl`, the file that says how a directory of KDL relates to the world
//! outside it. Beside a board's designs it says which stackup they are written against and where
//! each `@prefix/…` import resolves; at a library's root it names the library, states the same
//! version and lists what the library depends on. The same statements serve both.
//!
//! Read here, as syntax; what the statements mean — finding the file, the local override beside
//! it, the rules between them — is the engine's.

use crate::span::Span;

/// `stackup "<major>.<minor>"`: the engine version the files are written against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    /// As written, for a message.
    pub text: String,
    pub span: Span,
}

/// `name <name>`: what a library calls itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Named {
    pub name: String,
    pub span: Span,
}

/// Where a library comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibrarySource {
    /// `path="…"`, relative to the manifest.
    Path(String),
    /// `git="…" rev="…" [dir="…"]`: a repository at an exact commit, and a directory within it.
    Git {
        url: String,
        rev: String,
        dir: Option<String>,
    },
}

/// `library <name> path="…"` or `library <name> git="…" rev="…" [dir="…"]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryDecl {
    pub name: String,
    pub source: LibrarySource,
    pub span: Span,
}

impl LibraryDecl {
    pub fn path(&self) -> Option<&str> {
        match &self.source {
            LibrarySource::Path(p) => Some(p),
            LibrarySource::Git { .. } => None,
        }
    }
}

/// A manifest's statements. Every field is optional because a manifest says only what it has to:
/// a design that uses nothing but the core library needs no `library` line, and a project that
/// is not a library has no `name`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    pub stackup: Option<Version>,
    pub name: Option<Named>,
    pub libraries: Vec<LibraryDecl>,
}

impl Manifest {
    pub fn library(&self, name: &str) -> Option<&LibraryDecl> {
        self.libraries.iter().find(|l| l.name == name)
    }
}

/// `"0.1"` or `"0.1.3"` → the major and minor. Anything else is not a version.
pub fn parse_version(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    for rest in parts {
        rest.parse::<u32>().ok()?;
    }
    Some((major, minor))
}
