//! Git-backed, project-local cache for pinned libraries.

use std::{
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TMP: AtomicU64 = AtomicU64::new(0);

fn git(args: &[&str], dir: Option<&Path>) -> Result<String, String> {
    let mut command = Command::new("git");
    command.args(args).env("GIT_TERMINAL_PROMPT", "0");
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let output = command
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git {}: {}", args.join(" "), stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn git_at(repo: &Path, args: &[&str]) -> Result<String, String> {
    git(args, Some(repo))
}

fn url_key(url: &str) -> String {
    // Reversible and collision-free, unlike a process-dependent hasher.
    url.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

fn is_commit_id(rev: &str) -> bool {
    rev.len() == 40 && rev.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn validate_rev(rev: &str) -> Result<(), String> {
    is_commit_id(rev)
        .then_some(())
        .ok_or_else(|| "`rev=` must be a full 40-character SHA-1 commit ID".into())
}

pub fn validate_dir(dir: &str) -> Result<(), String> {
    let path = Path::new(dir);
    if path.as_os_str().is_empty() || !path.components().all(|c| matches!(c, Component::Normal(_)))
    {
        return Err("`dir=` must be a relative path within the repository".into());
    }
    Ok(())
}

fn temporary_path(parent: &Path) -> PathBuf {
    parent.join(format!(
        ".tmp-{}-{}",
        std::process::id(),
        NEXT_TMP.fetch_add(1, Ordering::Relaxed)
    ))
}

fn cache_repo(project: &Path, url: &str) -> PathBuf {
    let mut root = project.join(".stackup/cache");
    let key = url_key(url);
    // A remote URL may exceed a filesystem's single-component length. Keep
    // short keys where they already are and split longer ones without losing
    // the one-to-one mapping from URL to cache directory.
    for chunk in key.as_bytes().chunks(200) {
        root.push(std::str::from_utf8(chunk).expect("hex is ASCII"));
    }
    root
}

fn ensure_repo(project: &Path, url: &str) -> Result<PathBuf, String> {
    if url.is_empty() {
        return Err("`git=` needs a repository URL".into());
    }
    let root = cache_repo(project, url);
    let bare = root.join("repo.git");
    if !bare.exists() {
        fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
        let temp = temporary_path(&root);
        git(
            &[
                "init",
                "--bare",
                temp.to_str().ok_or("non-UTF-8 cache path")?,
            ],
            None,
        )?;
        match fs::rename(&temp, &bare) {
            Ok(()) => {}
            Err(_) if bare.exists() => {
                let _ = fs::remove_dir_all(temp);
            }
            Err(e) => return Err(format!("{}: {e}", bare.display())),
        }
    }
    Ok(bare)
}

fn has_commit(bare: &Path, rev: &str) -> bool {
    git_at(bare, &["cat-file", "-t", rev]).is_ok_and(|kind| kind == "commit")
}

fn fetch_commit(bare: &Path, url: &str, rev: &str) -> Result<(), String> {
    // Some servers reject a direct request for an unadvertised object. Fetching
    // advertised refs is a fallback; the requested commit still has to exist.
    let direct = git_at(bare, &["fetch", "--no-tags", "--", url, rev]);
    if direct.is_err() && !has_commit(bare, rev) {
        git_at(
            bare,
            &[
                "fetch",
                "--no-tags",
                "--",
                url,
                "+refs/heads/*:refs/cache/heads/*",
                "+refs/tags/*:refs/cache/tags/*",
            ],
        )
        .map_err(|fallback| {
            format!(
                "cannot fetch pinned commit {rev}: {}; {fallback}",
                direct.unwrap_err()
            )
        })?;
    }
    if !has_commit(bare, rev) {
        return Err(format!("commit {rev} is unavailable from {url}"));
    }
    git_at(
        bare,
        &["update-ref", &format!("refs/stackup/pins/{rev}"), rev],
    )?;
    Ok(())
}

/// Make an immutable-by-convention checkout of one exact commit. Cache entries
/// are checked for local edits before reuse; Git submodules are not initialized.
pub fn checkout(
    project: &Path,
    url: &str,
    rev: &str,
    dir: Option<&str>,
) -> Result<PathBuf, String> {
    validate_rev(rev)?;
    if url.is_empty() {
        return Err("`git=` needs a repository URL".into());
    }
    if let Some(dir) = dir {
        validate_dir(dir)?;
    }
    let root = cache_repo(project, url);
    let target = root.join("commits").join(rev);
    if target.exists() {
        let head = git_at(&target, &["rev-parse", "HEAD"])?;
        if !head.eq_ignore_ascii_case(rev)
            || !git_at(&target, &["status", "--porcelain", "--untracked-files=all"])?.is_empty()
        {
            return Err(format!("cached checkout {} was modified", target.display()));
        }
    } else {
        let bare = ensure_repo(project, url)?;
        if !has_commit(&bare, rev) {
            fetch_commit(&bare, url, rev)?;
        }
        let parent = target.parent().expect("commit has a parent");
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        let temp = temporary_path(parent);
        git(
            &[
                "clone",
                "--shared",
                "--no-checkout",
                bare.to_str().ok_or("non-UTF-8 cache path")?,
                temp.to_str().ok_or("non-UTF-8 cache path")?,
            ],
            None,
        )?;
        if let Err(e) = git_at(&temp, &["checkout", "--detach", rev]) {
            let _ = fs::remove_dir_all(&temp);
            return Err(e);
        }
        match fs::rename(&temp, &target) {
            Ok(()) => {}
            Err(_) if target.exists() => {
                let _ = fs::remove_dir_all(temp);
            }
            Err(e) => return Err(format!("{}: {e}", target.display())),
        }
    }
    let selected = dir.map_or_else(|| target.clone(), |d| target.join(d));
    let canonical = selected
        .canonicalize()
        .map_err(|e| format!("{}: {e}", selected.display()))?;
    if !canonical.starts_with(target.canonicalize().map_err(|e| e.to_string())?)
        || !canonical.is_dir()
    {
        return Err(format!(
            "`dir=` does not name a directory inside commit {rev}"
        ));
    }
    Ok(canonical)
}

pub fn latest(project: &Path, url: &str) -> Result<String, String> {
    let bare = ensure_repo(project, url)?;
    let head = git(&["ls-remote", "--symref", "--", url, "HEAD"], None)?;
    let branch = head.lines().find_map(|line| {
        line.strip_prefix("ref: ")?
            .split_once('\t')
            .map(|(name, _)| name)
    });
    let branch = branch.ok_or_else(|| format!("{url} does not advertise a default branch"))?;
    if !branch.starts_with("refs/heads/") {
        return Err(format!("{url} has no default branch"));
    }
    let rev = head
        .lines()
        .find_map(|line| {
            let (sha, name) = line.split_once('\t')?;
            (name == "HEAD" && is_commit_id(sha)).then_some(sha)
        })
        .ok_or_else(|| format!("{url} has no default branch commit"))?;
    validate_rev(rev)?;
    fetch_commit(&bare, url, rev)?;
    Ok(rev.to_owned())
}
