//! The manifest and the local file beside it: where `@prefix/…` resolves, and the rules that keep
//! a redirect honest.

use std::{path::PathBuf, process::Command};

use stackup::{
    load::Library,
    manifest::{Options, Origin, Prefixes},
};

/// A scratch tree of files.
struct Tree {
    dir: PathBuf,
}

impl Tree {
    fn new(name: &str, files: &[(&str, &str)]) -> Tree {
        let dir =
            std::env::temp_dir().join(format!("stackup-manifest-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, text) in files {
            let full = dir.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, text).unwrap();
        }
        Tree { dir }
    }

    fn resolve(&self, design: &str, opts: Options) -> (Prefixes, Vec<String>) {
        let (p, report) = Prefixes::resolve(&self.dir.join(design), opts);
        (p, report.messages())
    }

    /// Loads a design through the manifest and returns every message, from resolution and
    /// loading both.
    fn load(&self, design: &str, opts: Options) -> (Library, Vec<String>) {
        let (p, mut report) = Prefixes::resolve(&self.dir.join(design), opts);
        let (lib, loaded) = Library::load(&self.dir.join(design), &p);
        report.findings.extend(loaded.findings);
        (lib, report.messages())
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const CORE: &str = "part resistor { symbol \"Device:R\"; reference R; pin A passive; pin B passive; package chip { pad A 1; pad B 2 } }\n";
const LAMP: &str = "use \"@stackup/passives\"\nblock lamp { place resistor as=self }\n";
const BOARD: &str = "use \"@stackup/passives\"\nuse \"@arc/lamp\"\ndesign b { place lamp l }\n";

#[test]
fn a_path_library_resolves_through_the_manifest() {
    let t = Tree::new(
        "path",
        &[
            (
                "manifest.kdl",
                "stackup \"0.1\"\nlibrary arc path=\"lib\"\nlibrary stackup path=\"core\"\n",
            ),
            ("core/passives.kdl", CORE),
            ("lib/lamp.kdl", LAMP),
            ("boards/one/board.kdl", BOARD),
        ],
    );
    // Found by walking up from the design, two levels down.
    let (p, messages) = t.resolve("boards/one/board.kdl", Options::default());
    assert!(messages.is_empty(), "{messages:?}");
    assert_eq!(
        p.project.as_deref(),
        Some(t.dir.canonicalize().unwrap().as_path())
    );
    assert_eq!(
        p.dir("arc"),
        Some(t.dir.join("lib").canonicalize().unwrap().as_path())
    );
    assert_eq!(p.overrides().count(), 0);

    let (lib, messages) = t.load("boards/one/board.kdl", Options::default());
    assert!(messages.is_empty(), "{messages:?}");
    assert!(!lib.incomplete);
    assert_eq!(lib.files.len(), 3);
}

#[test]
fn no_manifest_means_no_libraries_but_the_core() {
    let t = Tree::new("bare", &[("board.kdl", BOARD), ("core/passives.kdl", CORE)]);
    let (lib, messages) = t.load("board.kdl", Options::default());
    assert!(lib.incomplete);
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert!(
        messages[0].contains("core library, which is not embedded"),
        "{}",
        messages[0]
    );
    assert!(
        messages[1].contains("no manifest.kdl was found"),
        "{}",
        messages[1]
    );

    // `--lib` is the core library from the command line, and is reported as a redirect.
    let core = t.dir.join("core");
    let (p, messages) = t.resolve(
        "board.kdl",
        Options {
            locked: false,
            lib: Some(&core),
        },
    );
    assert!(messages.is_empty(), "{messages:?}");
    let overrides: Vec<_> = p.overrides().collect();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].0, "stackup");
    assert_eq!(overrides[0].1.origin, Origin::Flag);
}

#[test]
fn the_local_file_redirects_and_is_reported() {
    let t = Tree::new(
        "local",
        &[
            (
                "manifest.kdl",
                "stackup \"0.1\"\nlibrary arc path=\"lib\"\n",
            ),
            (
                "manifest.local.kdl",
                "library arc path=\"../elsewhere\"\nlibrary stackup path=\"core\"\n",
            ),
            ("core/passives.kdl", CORE),
            ("lib/lamp.kdl", "block wrong {}\n"),
            ("../elsewhere/lamp.kdl", LAMP),
            ("board.kdl", BOARD),
        ],
    );
    let (p, messages) = t.resolve("board.kdl", Options::default());
    assert!(messages.is_empty(), "{messages:?}");
    // Both redirected: `arc` from the manifest's answer, `stackup` needing no line to redirect.
    let overrides: Vec<_> = p
        .overrides()
        .map(|(n, r)| (n.to_string(), r.origin, r.written.clone()))
        .collect();
    assert_eq!(
        overrides,
        [
            ("arc".to_string(), Origin::Local, "../elsewhere".to_string()),
            ("stackup".to_string(), Origin::Local, "core".to_string()),
        ]
    );
    let (lib, messages) = t.load("board.kdl", Options::default());
    assert!(messages.is_empty(), "{messages:?}");
    assert!(
        lib.files
            .iter()
            .any(|f| f.path.ends_with("elsewhere/lamp.kdl"))
    );

    // Locked, the local file is not read: `arc` is the manifest's and `stackup` is nobody's.
    let (p, messages) = t.resolve(
        "board.kdl",
        Options {
            locked: true,
            lib: None,
        },
    );
    assert!(messages.is_empty(), "{messages:?}");
    assert_eq!(p.overrides().count(), 0);
    assert!(p.dir("arc").unwrap().ends_with("lib"));
    assert_eq!(p.dir("stackup"), None);
    let _ = std::fs::remove_dir_all(t.dir.join("../elsewhere"));
}

#[test]
fn the_local_file_cannot_introduce_a_library() {
    let t = Tree::new(
        "introduce",
        &[
            ("manifest.kdl", "stackup \"0.1\"\n"),
            (
                "manifest.local.kdl",
                "stackup \"0.1\"\nlibrary ti path=\"ti\"\nlibrary stackup git=\"x\" rev=\"y\"\n",
            ),
            ("ti/x.kdl", ""),
            ("board.kdl", "design b {}\n"),
        ],
    );
    let (p, messages) = t.resolve("board.kdl", Options::default());
    assert_eq!(messages.len(), 3, "{messages:?}");
    assert!(
        messages[0].contains("`stackup` belongs in manifest.kdl"),
        "{}",
        messages[0]
    );
    assert!(
        messages[1].contains("`ti` is not a library manifest.kdl names"),
        "{}",
        messages[1]
    );
    assert!(
        messages[2].contains("a local override is a checkout"),
        "{}",
        messages[2]
    );
    assert_eq!(p.roots.len(), 0);
}

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn pinned_git_library_fetches_and_update_preserves_the_manifest() {
    let t = Tree::new(
        "git",
        &[("board.kdl", "use \"@stackup/passives\"\ndesign b {}\n")],
    );
    let remote = t.dir.join("remote");
    std::fs::create_dir_all(remote.join("lib")).unwrap();
    git(&remote, &["init", "-b", "main"]);
    std::fs::write(
        remote.join("lib/manifest.kdl"),
        "name stackup\nstackup \"0.1\"\n",
    )
    .unwrap();
    std::fs::write(remote.join("lib/passives.kdl"), CORE).unwrap();
    git(&remote, &["add", "."]);
    git(
        &remote,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-m",
            "first",
        ],
    );
    let first = git(&remote, &["rev-parse", "HEAD"]);
    let original = format!(
        "// keep this comment\nstackup \"0.1\"\nlibrary stackup git=\"{}\" rev=\"{first}\" dir=\"lib\"\n",
        remote.display()
    );
    std::fs::write(t.dir.join("manifest.kdl"), &original).unwrap();

    let (p, messages) = t.resolve(
        "board.kdl",
        Options {
            locked: true,
            lib: None,
        },
    );
    assert!(messages.is_empty(), "{messages:?}");
    assert!(p.dir("stackup").unwrap().ends_with("lib"));
    assert!(t.dir.join(".stackup/cache").is_dir());
    let (lib, messages) = t.load(
        "board.kdl",
        Options {
            locked: true,
            lib: None,
        },
    );
    assert!(messages.is_empty(), "{messages:?}");
    assert!(!lib.incomplete);

    std::fs::create_dir_all(t.dir.join("override")).unwrap();
    std::fs::write(t.dir.join("override/passives.kdl"), CORE).unwrap();
    std::fs::write(
        t.dir.join("manifest.local.kdl"),
        "library stackup path=\"override\"\n",
    )
    .unwrap();
    let (p, messages) = t.resolve("board.kdl", Options::default());
    assert!(messages.is_empty(), "{messages:?}");
    assert_eq!(p.overrides().next().unwrap().1.origin, Origin::Local);
    let (p, messages) = t.resolve(
        "board.kdl",
        Options {
            locked: true,
            lib: None,
        },
    );
    assert!(messages.is_empty(), "{messages:?}");
    assert_eq!(p.overrides().count(), 0);

    std::fs::write(
        remote.join("lib/passives.kdl"),
        format!("{CORE}\n// second\n"),
    )
    .unwrap();
    git(&remote, &["add", "."]);
    git(
        &remote,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-m",
            "second",
        ],
    );
    let second = git(&remote, &["rev-parse", "HEAD"]);
    std::fs::create_dir_all(t.dir.join("nested")).unwrap();
    let updated = Command::new(env!("CARGO_BIN_EXE_stackup"))
        .current_dir(t.dir.join("nested"))
        .args(["update", "stackup"])
        .output()
        .unwrap();
    assert!(
        updated.status.success(),
        "{}",
        String::from_utf8_lossy(&updated.stderr)
    );
    assert!(
        String::from_utf8_lossy(&updated.stderr)
            .contains(&format!("@stackup: {first} -> {second}"))
    );
    assert_eq!(
        std::fs::read_to_string(t.dir.join("manifest.kdl")).unwrap(),
        original.replace(&first, &second)
    );
    let (_, messages) = t.resolve(
        "board.kdl",
        Options {
            locked: true,
            lib: None,
        },
    );
    assert!(messages.is_empty(), "{messages:?}");
}

#[test]
fn a_local_override_does_not_fetch_an_unavailable_pin() {
    let t = Tree::new(
        "override-git",
        &[
            (
                "manifest.kdl",
                "library stackup git=\"https://invalid.example/lib\" rev=\"0000000000000000000000000000000000000000\"\n",
            ),
            ("manifest.local.kdl", "library stackup path=\"core\"\n"),
            ("core/manifest.kdl", "name stackup\nstackup \"0.1\"\n"),
            ("core/passives.kdl", CORE),
            ("board.kdl", "use \"@stackup/passives\"\ndesign b {}\n"),
        ],
    );
    let (lib, messages) = t.load("board.kdl", Options::default());
    assert!(messages.is_empty(), "{messages:?}");
    assert!(!lib.incomplete);
    assert!(!t.dir.join(".stackup/cache").exists());
}

#[test]
fn a_library_is_mounted_under_its_own_name() {
    let t = Tree::new(
        "name",
        &[
            (
                "manifest.kdl",
                "library parts path=\"lib\"\nlibrary stackup path=\"core\"\n",
            ),
            ("core/passives.kdl", CORE),
            (
                "lib/manifest.kdl",
                "name arc\nstackup \"0.1\"\nlibrary stackup path=\"../core\"\nlibrary ti path=\"x\"\n",
            ),
            ("lib/lamp.kdl", LAMP),
            ("board.kdl", "design b {}\n"),
        ],
    );
    let (_, messages) = t.resolve("board.kdl", Options::default());
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert!(
        messages[0].contains("calls itself `arc` and is mounted as `@parts`"),
        "{}",
        messages[0]
    );
    // Its own `library stackup` is fine, since the root names it; `ti` is not named.
    assert!(
        messages[1].contains("`@parts` needs `@ti`"),
        "{}",
        messages[1]
    );
}

#[test]
fn a_manifest_from_the_future_is_refused() {
    let t = Tree::new(
        "future",
        &[
            ("manifest.kdl", "stackup \"99.0\"\n"),
            ("board.kdl", "design b {}\n"),
        ],
    );
    let (_, messages) = t.resolve("board.kdl", Options::default());
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(
        messages[0].starts_with("written against stackup 99.0; this is stackup "),
        "{}",
        messages[0]
    );
}

#[test]
fn a_missing_directory_is_named() {
    let t = Tree::new(
        "missing",
        &[
            ("manifest.kdl", "library arc path=\"nowhere\"\n"),
            (
                "board.kdl",
                "use \"@arc/lamp\"\nuse \"@nobody/x\"\nuse \"@bare\"\ndesign b {}\n",
            ),
        ],
    );
    let (lib, messages) = t.load("board.kdl", Options::default());
    assert!(lib.incomplete);
    assert_eq!(messages.len(), 4, "{messages:?}");
    assert!(
        messages[0].starts_with("no such directory:"),
        "{}",
        messages[0]
    );
    assert!(
        messages[1].contains("`@arc` is not a library")
            && messages[1].contains("manifest.kdl names"),
        "{}",
        messages[1]
    );
    assert!(
        messages[2].contains("`@nobody` is not a library"),
        "{}",
        messages[2]
    );
    assert!(
        messages[3].contains("`@bare` is not an import"),
        "{}",
        messages[3]
    );
}
