//! Advance pinned git libraries while preserving manifest formatting.

use std::{
    fs,
    path::{Path, PathBuf},
};

use stackup_kdl::{Document, Value, manifest::LibrarySource};

use crate::{git_cache, manifest::MANIFEST};

pub fn project_from_cwd(cwd: &Path) -> Option<PathBuf> {
    cwd.ancestors()
        .find(|dir| dir.join(MANIFEST).is_file())
        .map(Path::to_path_buf)
}

pub fn local_overrides(project: &Path) -> Vec<String> {
    let Ok(doc) = Document::open(project.join("manifest.local.kdl")) else {
        return Vec::new();
    };
    let (manifest, _) = doc.manifest();
    manifest.libraries.into_iter().map(|l| l.name).collect()
}

pub fn update(project: &Path, name: Option<&str>) -> Result<Vec<(String, String, String)>, String> {
    let path = project.join(MANIFEST);
    let mut doc = Document::open(&path).map_err(|d| d.to_string())?;
    let (manifest, diagnostics) = doc.manifest();
    if diagnostics.has_errors() {
        return Err(diagnostics.to_string());
    }
    let selected: Vec<_> = manifest
        .libraries
        .iter()
        .filter(|l| name.is_none_or(|n| n == l.name))
        .collect();
    if selected.is_empty() {
        return Err(match name {
            Some(n) => format!("no library `{n}` in {}", path.display()),
            None => "no libraries to update".into(),
        });
    }
    let mut changes = Vec::new();
    for lib in selected {
        let LibrarySource::Git { url, rev, dir } = &lib.source else {
            if name.is_some() {
                return Err(format!(
                    "library `{}` uses `path=`, so it cannot be updated from Git",
                    lib.name
                ));
            }
            continue;
        };
        let next = git_cache::latest(project, url)?;
        // Complete every fetch and checkout before touching the manifest.
        git_cache::checkout(project, url, &next, dir.as_deref())?;
        changes.push((lib.name.clone(), rev.clone(), next, lib.span));
    }
    for (_, _, next, span) in &changes {
        doc.set_property(*span, "rev", &Value::String(next.clone()))
            .map_err(|d| d.to_string())?;
    }
    if changes.iter().any(|(_, old, next, _)| old != next) {
        let temp = path.with_extension(format!("kdl.tmp-{}", std::process::id()));
        fs::write(&temp, doc.to_string()).map_err(|e| format!("{}: {e}", temp.display()))?;
        fs::rename(&temp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(changes
        .into_iter()
        .map(|(name, old, next, _)| (name, old, next))
        .collect())
}
