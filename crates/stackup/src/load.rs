//! Loading a design's files: the root file, everything it `use`s, and what each file can see.
//!
//! An import brings every top-level name the imported file declares into scope — its own
//! declarations, not what it imported in turn. Two declarations of one name in scope is an error
//! at the `use` that introduced the second.
//!
//! A `use "./x.kdl"` is relative to the importing file; a `use "@name/x"` is `x.kdl` under the
//! directory the [`Prefixes`] resolved `@name` to, from the manifest beside the design.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use stackup_kdl::{
    Diagnostic, Document, Source, Span,
    ast::{self, Block, BlockKind, Item, Part, TypeDecl},
};

use crate::{
    manifest::{CORE, LOCAL, MANIFEST, Prefixes},
    report::Report,
};

pub type FileId = usize;

/// A declaration, wherever it lives.
#[derive(Clone, Copy, Debug)]
pub enum Decl<'a> {
    Part(&'a Part),
    Block(&'a Block),
    Type(&'a TypeDecl),
}

impl Decl<'_> {
    pub fn name(&self) -> &str {
        match self {
            Decl::Part(p) => &p.name,
            Decl::Block(b) => &b.name,
            Decl::Type(t) => &t.name,
        }
    }

    pub fn what(&self) -> &'static str {
        match self {
            Decl::Part(_) => "part",
            Decl::Block(b) if b.kind == BlockKind::Design => "design",
            Decl::Block(_) => "block",
            Decl::Type(_) => "type",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DeclRef {
    pub file: FileId,
    pub item: usize,
}

pub struct LoadedFile {
    pub path: PathBuf,
    pub source: Arc<Source>,
    pub file: ast::File,
    /// Every name this file can use, and where it is declared.
    pub scope: HashMap<String, DeclRef>,
}

pub struct Library {
    pub files: Vec<LoadedFile>,
    /// Where each `@prefix/…` resolves to.
    pub prefixes: Prefixes,
    /// Whether a file is missing: an import that could not be resolved or read. The files that
    /// did load are still here, but a design over them is not the design that was written, so
    /// it is not one to elaborate.
    pub incomplete: bool,
}

fn item_name(item: &Item) -> Option<(&str, Span)> {
    match item {
        Item::Part(p) => Some((&p.name, p.span)),
        Item::Block(b) => Some((&b.name, b.span)),
        Item::Type(t) => Some((&t.name, t.span)),
        Item::Use(_) => None,
    }
}

impl Library {
    /// Loads `root` and everything it imports. Every problem is reported; a file that cannot be
    /// read is left out and the rest is still loaded. The root file is file 0.
    pub fn load(root: &Path, prefixes: &Prefixes) -> (Library, Report) {
        let mut lib = Library {
            files: Vec::new(),
            prefixes: prefixes.clone(),
            incomplete: false,
        };
        let mut report = Report::default();
        let mut ids: HashMap<PathBuf, Option<FileId>> = HashMap::new();
        // Per loaded file: the canonical path each `use` names, and the `use`'s span.
        let mut imports: Vec<Vec<(PathBuf, Span)>> = Vec::new();
        let mut queue: Vec<PathBuf> = vec![canonical(root)];
        let mut next = 0;

        while next < queue.len() {
            let path = queue[next].clone();
            next += 1;
            if ids.contains_key(&path) {
                continue;
            }
            let doc = match Document::open(&path) {
                Ok(doc) => doc,
                Err(e) => {
                    ids.insert(path, None);
                    report.extend(e);
                    lib.incomplete = true;
                    continue;
                }
            };
            let (file, diags) = doc.file();
            report.extend(diags);
            let mut wanted = Vec::new();
            for u in file.uses() {
                match lib.resolve_import(&path, &u.path, u.span) {
                    Ok(target) => {
                        let target = canonical(&target);
                        queue.push(target.clone());
                        wanted.push((target, u.span));
                    }
                    Err(d) => {
                        report.push(doc.source(), d);
                        lib.incomplete = true;
                    }
                }
            }
            ids.insert(path.clone(), Some(lib.files.len()));
            imports.push(wanted);
            lib.files.push(LoadedFile {
                path,
                source: doc.source().clone(),
                file,
                scope: HashMap::new(),
            });
        }

        // Every file is loaded; now each file's scope is its own declarations plus those of
        // every file it imports.
        for (id, wanted) in imports.iter().enumerate() {
            let mut scope: HashMap<String, DeclRef> = HashMap::new();
            for (i, item) in lib.files[id].file.items.iter().enumerate() {
                let Some((name, span)) = item_name(item) else {
                    continue;
                };
                if scope.contains_key(name) {
                    report.error(
                        &lib.files[id].source,
                        span,
                        format!("`{name}` is declared twice in this file"),
                    );
                    continue;
                }
                scope.insert(name.to_string(), DeclRef { file: id, item: i });
            }
            for (target, span) in wanted {
                let Some(Some(target)) = ids.get(target) else {
                    report.error(
                        &lib.files[id].source,
                        *span,
                        "this import could not be loaded",
                    );
                    continue;
                };
                let target = *target;
                for (i, item) in lib.files[target].file.items.iter().enumerate() {
                    let Some((name, _)) = item_name(item) else {
                        continue;
                    };
                    match scope.get(name) {
                        Some(existing) if existing.file == target => {}
                        Some(existing) => report.error(
                            &lib.files[id].source,
                            *span,
                            format!(
                                "this import brings in `{name}`, which is already in scope from {}",
                                lib.files[existing.file].path.display()
                            ),
                        ),
                        None => {
                            scope.insert(
                                name.to_string(),
                                DeclRef {
                                    file: target,
                                    item: i,
                                },
                            );
                        }
                    }
                }
            }
            lib.files[id].scope = scope;
        }
        (lib, report)
    }

    /// The file a `use` names: `@name/path` is `path.kdl` under the prefix's directory, anything
    /// else is relative to the importing file.
    fn resolve_import(&self, from: &Path, spec: &str, at: Span) -> Result<PathBuf, Diagnostic> {
        let Some(rest) = spec.strip_prefix('@') else {
            let dir = from.parent().unwrap_or(Path::new("."));
            return Ok(dir.join(spec));
        };
        let Some((name, path)) = rest
            .split_once('/')
            .filter(|(n, p)| !n.is_empty() && !p.is_empty())
        else {
            return Err(Diagnostic::error(
                at,
                format!("`{spec}` is not an import: a library import is `@library/path`"),
            ));
        };
        if let Some(dir) = self.prefixes.dir(name) {
            return Ok(dir.join(format!("{path}.kdl")));
        }
        Err(if name == CORE {
            Diagnostic::error(
                at,
                format!("`@{CORE}/…` is the core library, which is not embedded in the engine yet"),
            )
            .with_help(format!(
                "point at a checkout of stackup-parts: `library {CORE} path=\"…\"` in {LOCAL}, or --lib"
            ))
        } else {
            match self.prefixes.manifest_path() {
                Some(m) => Diagnostic::error(
                    at,
                    format!("`@{name}` is not a library {} names", m.display()),
                ),
                None => Diagnostic::error(
                    at,
                    format!(
                        "`@{name}` is not a library: no {MANIFEST} was found at or above {}",
                        from.parent().unwrap_or(Path::new(".")).display()
                    ),
                ),
            }
        })
    }

    pub fn decl(&self, r: DeclRef) -> Decl<'_> {
        match &self.files[r.file].file.items[r.item] {
            Item::Part(p) => Decl::Part(p),
            Item::Block(b) => Decl::Block(b),
            Item::Type(t) => Decl::Type(t),
            Item::Use(_) => unreachable!("a use is never a declaration"),
        }
    }

    /// What `name` means in `file`.
    pub fn lookup(&self, file: FileId, name: &str) -> Option<DeclRef> {
        self.files[file].scope.get(name).copied()
    }

    pub fn source(&self, file: FileId) -> &Arc<Source> {
        &self.files[file].source
    }

    /// Every design in the root file.
    pub fn designs(&self) -> Vec<(FileId, &Block)> {
        let Some(root) = self.files.first() else {
            return Vec::new();
        };
        root.file
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Block(b) if b.kind == BlockKind::Design => Some((0, b)),
                _ => None,
            })
            .collect()
    }
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
