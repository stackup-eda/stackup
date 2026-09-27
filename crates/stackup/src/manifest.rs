//! Where `@prefix/…` resolves.
//!
//! A design's project is the nearest directory at or above it holding a `manifest.kdl` or a
//! `manifest.local.kdl`. The **manifest** is committed and says the version: every library it
//! names is a `path` beside it or a repository at an exact commit. The **local** file is one
//! machine's and never committed: it redirects a library the manifest already names to a
//! checkout being worked on beside the design. Three rules keep the second honest — it can only
//! redirect, never introduce; what it redirects is reported on every run ([`Prefixes::overrides`]);
//! and `--locked` ignores it, so a fresh clone and CI see only the manifest.
//!
//! `@stackup` is the core library and is named by every manifest without a line, so the local
//! file may redirect it too. Until the library is embedded in the engine, that redirect (or
//! `--lib`) is how it is found at all.
//!
//! A library resolved to a directory is then read for a manifest of its own: its `name` must be
//! the prefix it was mounted under, because its files import each other by that prefix; its
//! version is held to the engine's; and each library it names must be one the root manifest
//! names too, since the root manifest is authoritative for every prefix and a library never
//! introduces a second source.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use stackup_kdl::{
    Diagnostic, Document, Severity, Source, Span,
    manifest::{LibrarySource, Manifest, Version},
};

use crate::{git_cache, report::Report};

pub const MANIFEST: &str = "manifest.kdl";
pub const LOCAL: &str = "manifest.local.kdl";
/// The core library's prefix, which every manifest names without a line.
pub const CORE: &str = "stackup";

/// Which file put a prefix where it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// `manifest.kdl`, the committed answer.
    Manifest,
    /// `manifest.local.kdl`, this machine's redirect.
    Local,
    /// `--lib` on the command line.
    Flag,
}

/// A library's directory, and how it was named.
#[derive(Clone, Debug)]
pub struct Root {
    pub dir: PathBuf,
    /// The path as the file (or flag) wrote it, for a message.
    pub written: String,
    pub origin: Origin,
}

/// What each `@prefix` resolves to.
#[derive(Clone, Debug, Default)]
pub struct Prefixes {
    pub roots: BTreeMap<String, Root>,
    /// The directory holding the manifest, when one was found.
    pub project: Option<PathBuf>,
}

/// How to resolve: whether the local file is honoured, and a `--lib` for the core library.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options<'a> {
    pub locked: bool,
    pub lib: Option<&'a Path>,
}

impl Prefixes {
    /// One prefix at one directory and nothing else, for a test or a caller that resolves its
    /// own.
    pub fn single(name: &str, dir: impl Into<PathBuf>) -> Prefixes {
        let dir = dir.into();
        let mut p = Prefixes::default();
        p.roots.insert(
            name.to_string(),
            Root {
                written: dir.display().to_string(),
                dir,
                origin: Origin::Manifest,
            },
        );
        p
    }

    pub fn dir(&self, name: &str) -> Option<&Path> {
        self.roots.get(name).map(|r| r.dir.as_path())
    }

    /// Every prefix not answered by the manifest: what a run should say it is redirecting.
    pub fn overrides(&self) -> impl Iterator<Item = (&str, &Root)> {
        self.roots
            .iter()
            .filter(|(_, r)| r.origin != Origin::Manifest)
            .map(|(n, r)| (n.as_str(), r))
    }

    /// The manifest's path, for a message that names it.
    pub fn manifest_path(&self) -> Option<PathBuf> {
        let dir = self.project.as_ref()?;
        let path = dir.join(MANIFEST);
        path.is_file().then_some(path)
    }

    /// Resolves the prefixes for a design file. Every problem is reported and resolution carries
    /// on, so a broken line costs that library and nothing else.
    pub fn resolve(design: &Path, opts: Options) -> (Prefixes, Report) {
        let mut report = Report::default();
        let mut p = Prefixes {
            roots: BTreeMap::new(),
            project: find_project(design),
        };

        // The manifest: the committed answer.
        let manifest = p
            .project
            .as_ref()
            .map(|dir| dir.join(MANIFEST))
            .filter(|path| path.is_file())
            .and_then(|path| read(&path, &mut report));
        let mut pending_git = Vec::new();
        if let Some((m, src)) = &manifest {
            check_version(m, src, &mut report);
            let project = p.project.clone().unwrap_or_default();
            for l in &m.libraries {
                match &l.source {
                    LibrarySource::Path(rel) => {
                        if let Some(dir) = existing_dir(&project, rel, src, l.span, &mut report) {
                            p.roots.insert(
                                l.name.clone(),
                                Root {
                                    dir,
                                    written: rel.clone(),
                                    origin: Origin::Manifest,
                                },
                            );
                        }
                    }
                    LibrarySource::Git { .. } => pending_git.push(l.clone()),
                }
            }
        }

        // The local file: this machine's redirects, honoured unless the run is locked.
        let local = p
            .project
            .as_ref()
            .filter(|_| !opts.locked)
            .map(|dir| dir.join(LOCAL))
            .filter(|path| path.is_file())
            .and_then(|path| read(&path, &mut report));
        if let Some((m, src)) = &local {
            let project = p.project.clone().unwrap_or_default();
            if let Some(v) = &m.stackup {
                report.error(
                    src,
                    v.span,
                    format!(
                        "`stackup` belongs in {MANIFEST}; the local file only redirects libraries"
                    ),
                );
            }
            if let Some(n) = &m.name {
                report.error(
                    src,
                    n.span,
                    format!(
                        "`name` belongs in {MANIFEST}; the local file only redirects libraries"
                    ),
                );
            }
            for l in &m.libraries {
                let named = l.name == CORE
                    || manifest
                        .as_ref()
                        .is_some_and(|(m, _)| m.library(&l.name).is_some());
                if !named {
                    report.push(
                        src,
                        Diagnostic::error(
                            l.span,
                            format!(
                                "`{}` is not a library {MANIFEST} names; a local override redirects a library, it cannot introduce one",
                                l.name
                            ),
                        )
                        .with_help(format!(
                            "name it in {MANIFEST} first, so the design loads for everyone"
                        )),
                    );
                    continue;
                }
                let LibrarySource::Path(rel) = &l.source else {
                    report.error(
                        src,
                        l.span,
                        format!(
                            "a local override is a checkout: `path=`, not `git=`; the pin belongs in {MANIFEST}"
                        ),
                    );
                    continue;
                };
                if let Some(dir) = existing_dir(&project, rel, src, l.span, &mut report) {
                    p.roots.insert(
                        l.name.clone(),
                        Root {
                            dir,
                            written: rel.clone(),
                            origin: Origin::Local,
                        },
                    );
                }
            }
        }

        // `--lib`: the core library, from the command line.
        if let Some(lib) = opts.lib {
            if lib.is_dir() {
                p.roots.insert(
                    CORE.to_string(),
                    Root {
                        dir: canonical(lib),
                        written: lib.display().to_string(),
                        origin: Origin::Flag,
                    },
                );
            } else {
                report.push(
                    &Arc::new(Source {
                        name: "--lib".into(),
                        text: String::new(),
                    }),
                    Diagnostic {
                        severity: Severity::Error,
                        message: format!("--lib {}: no such directory", lib.display()),
                        span: None,
                        help: None,
                        labels: Vec::new(),
                    },
                );
            }
        }

        // Overrides win before any network access. A locally redirected git
        // dependency must not produce a fetch error or touch the cache.
        if let Some((_, src)) = &manifest {
            let project = p.project.clone().unwrap_or_default();
            for l in pending_git {
                if p.roots.contains_key(&l.name) {
                    continue;
                }
                let LibrarySource::Git { url, rev, dir } = &l.source else {
                    unreachable!()
                };
                match git_cache::checkout(&project, url, rev, dir.as_deref()) {
                    Ok(path) => {
                        p.roots.insert(
                            l.name,
                            Root {
                                dir: path,
                                written: format!("{url} at {rev}"),
                                origin: Origin::Manifest,
                            },
                        );
                    }
                    Err(e) => report.error(src, l.span, e),
                }
            }
        }

        // Each library's own manifest, held to the root's.
        let roots: Vec<(String, PathBuf)> = p
            .roots
            .iter()
            .map(|(n, r)| (n.clone(), r.dir.clone()))
            .collect();
        for (name, dir) in roots {
            let path = dir.join(MANIFEST);
            if !path.is_file() {
                continue;
            }
            let Some((m, src)) = read(&path, &mut report) else {
                continue;
            };
            if let Some(n) = &m.name
                && n.name != name
            {
                report.push(
                    &src,
                    Diagnostic::error(
                        n.span,
                        format!(
                            "this library calls itself `{}` and is mounted as `@{name}`",
                            n.name
                        ),
                    )
                    .with_help(format!(
                        "a library is mounted under its own name, because its files import each other as `@{}/…`",
                        n.name
                    )),
                );
            }
            check_version(&m, &src, &mut report);
            for dep in &m.libraries {
                if !p.roots.contains_key(&dep.name) {
                    report.push(
                        &src,
                        Diagnostic::error(
                            dep.span,
                            format!(
                                "`@{name}` needs `@{}`, which {} does not name",
                                dep.name,
                                p.manifest_path()
                                    .map(|m| m.display().to_string())
                                    .unwrap_or_else(|| format!("no {MANIFEST}"))
                            ),
                        )
                        .with_help(format!(
                            "the root {MANIFEST} is authoritative for every prefix; add a `library {}` line to it",
                            dep.name
                        )),
                    );
                }
            }
        }

        (p, report)
    }
}

/// The nearest directory at or above `design` holding a manifest or a local file.
fn find_project(design: &Path) -> Option<PathBuf> {
    let start = canonical(design);
    let mut dir = start.parent().map(Path::to_path_buf);
    while let Some(d) = dir {
        if d.join(MANIFEST).is_file() || d.join(LOCAL).is_file() {
            return Some(d);
        }
        dir = d.parent().map(Path::to_path_buf);
    }
    None
}

fn read(path: &Path, report: &mut Report) -> Option<(Manifest, Arc<Source>)> {
    let doc = match Document::open(path) {
        Ok(doc) => doc,
        Err(e) => {
            report.extend(e);
            return None;
        }
    };
    let (m, diags) = doc.manifest();
    report.extend(diags);
    Some((m, doc.source().clone()))
}

/// `rel` under `base`, which must exist as a directory.
fn existing_dir(
    base: &Path,
    rel: &str,
    src: &Arc<Source>,
    span: Span,
    report: &mut Report,
) -> Option<PathBuf> {
    let dir = base.join(rel);
    if dir.is_dir() {
        Some(canonical(&dir))
    } else {
        report.error(
            src,
            span,
            format!(
                "no such directory: {} ({rel} from {})",
                dir.display(),
                base.display()
            ),
        );
        None
    }
}

/// The engine's own version, as `(major, minor)`.
pub fn engine_version() -> (u32, u32) {
    stackup_kdl::manifest::parse_version(env!("CARGO_PKG_VERSION"))
        .expect("the crate version is a version")
}

/// A manifest written against a newer stackup than this is refused: it may say things this
/// engine cannot read, and reading past them would be a silently different design.
fn check_version(m: &Manifest, src: &Arc<Source>, report: &mut Report) {
    let Some(Version {
        major,
        minor,
        text,
        span,
    }) = &m.stackup
    else {
        return;
    };
    let engine = engine_version();
    if (*major, *minor) > engine {
        report.push(
            src,
            Diagnostic::error(
                *span,
                format!(
                    "written against stackup {text}; this is stackup {}",
                    env!("CARGO_PKG_VERSION")
                ),
            )
            .with_help("a newer stackup is needed to read it"),
        );
    }
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
