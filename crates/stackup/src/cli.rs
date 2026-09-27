//! `stackup <command> <file.kdl> [--design NAME] [--lib DIR] [--locked] [-o FILE]`
//! `stackup update [library]`
//!
//! - `check`: load and elaborate, and report everything found.
//! - `tree`: the instances and nets of a design, for reading.
//! - `netlist`: the KiCad netlist, to stdout or `-o`.
//! - `bom`: a purchasing CSV grouped by value, footprint and order fields.
//!
//! `@prefix/…` imports resolve through the `manifest.kdl` at or above the file, and this
//! machine's `manifest.local.kdl` beside it; every redirect the local file makes is printed, and
//! `--locked` ignores that file. `--lib DIR` is the core library from the command line, the same
//! statement as `library stackup path=DIR` in the local file.

use std::{io::IsTerminal, path::PathBuf, process::exit};

use crate::{
    bom,
    elaborate::elaborate,
    load::Library,
    manifest::{LOCAL, Options, Origin, Prefixes},
    model::{Kind, Model, Terminal},
    netlist, update,
};

struct Args {
    command: String,
    file: PathBuf,
    design: Option<String>,
    lib: Option<PathBuf>,
    locked: bool,
    out: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let command = args.next().ok_or(
        "usage: stackup <check|tree|netlist|bom> <file.kdl> [--design NAME] [--lib DIR] [--locked] [-o FILE]; stackup update [library]",
    )?;
    let mut file = None;
    let mut design = None;
    let mut lib = None;
    let mut locked = false;
    let mut out = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--design" => design = Some(args.next().ok_or("--design needs a name")?),
            "--lib" => lib = Some(PathBuf::from(args.next().ok_or("--lib needs a directory")?)),
            "--locked" => locked = true,
            "-o" | "--out" => out = Some(PathBuf::from(args.next().ok_or("-o needs a file")?)),
            other if other.starts_with('-') => return Err(format!("unknown option {other}")),
            other => {
                if file.is_some() {
                    return Err(format!("unexpected argument {other}"));
                }
                file = Some(PathBuf::from(other));
            }
        }
    }
    Ok(Args {
        command,
        file: file.ok_or("a design file is needed")?,
        design,
        lib,
        locked,
        out,
    })
}

/// Run the `stackup` command with this process's arguments.
pub fn run() {
    if std::env::args().nth(1).as_deref() == Some("update") {
        let mut args = std::env::args().skip(2);
        let name = args.next();
        if args.next().is_some() {
            eprintln!("usage: stackup update [library]");
            exit(2);
        }
        let cwd = std::env::current_dir().unwrap_or_else(|e| {
            eprintln!("cannot read current directory: {e}");
            exit(1)
        });
        let project = update::project_from_cwd(&cwd).unwrap_or_else(|| {
            eprintln!("no manifest.kdl found from {}", cwd.display());
            exit(1)
        });
        match update::update(&project, name.as_deref()) {
            Ok(changes) => {
                let overrides = update::local_overrides(&project);
                for (lib, old, next) in changes {
                    if old == next {
                        eprintln!("@{lib}: already at {next}");
                    } else {
                        eprintln!("@{lib}: {old} -> {next}");
                    }
                    if overrides.contains(&lib) {
                        eprintln!(
                            "note: manifest.local.kdl overrides @{lib} during ordinary checks"
                        );
                    }
                }
            }
            Err(e) => {
                eprintln!("{e}");
                exit(1);
            }
        }
        return;
    }
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            exit(2);
        }
    };
    let (prefixes, mut report) = Prefixes::resolve(
        &args.file,
        Options {
            locked: args.locked,
            lib: args.lib.as_deref(),
        },
    );
    // A redirect is said on every run: a design elaborated against a library nobody else has
    // should never look like one that was not.
    for (name, root) in prefixes.overrides() {
        let from = match root.origin {
            Origin::Local => LOCAL,
            Origin::Flag => "--lib",
            Origin::Manifest => unreachable!("an override is never the manifest's"),
        };
        eprintln!("@{name}: {} ({from})", root.written);
    }
    let (lib, loaded) = Library::load(&args.file, &prefixes);
    report.findings.extend(loaded.findings);
    // A design with a file missing is not the design that was written, so it is not judged: the
    // findings would be about the gap, not the board.
    if lib.incomplete {
        eprint!("{}", report.render());
        eprintln!("not elaborated: an import could not be loaded");
        exit(1);
    }

    let designs = lib.designs();
    let chosen: Vec<_> = match &args.design {
        Some(name) => designs
            .iter()
            .filter(|(_, d)| &d.name == name)
            .cloned()
            .collect(),
        None => designs.clone(),
    };
    if chosen.is_empty() {
        eprint!("{}", report.render());
        match &args.design {
            Some(name) => eprintln!("no design `{name}` in {}", args.file.display()),
            None => eprintln!("no design in {}", args.file.display()),
        }
        exit(1);
    }
    if args.command != "check" && chosen.len() > 1 {
        eprintln!(
            "{} has several designs ({}); pick one with --design",
            args.file.display(),
            chosen
                .iter()
                .map(|(_, d)| d.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        exit(2);
    }

    let mut failed = report.has_errors();
    for (file, design) in chosen {
        let model = elaborate(&lib, file, design);
        report
            .findings
            .extend(model.report.findings.iter().cloned());
        failed |= model.report.has_errors();
        match args.command.as_str() {
            "check" => {
                let parts = model.parts().count();
                let (mut errors, mut warnings, mut notes) = (0, 0, 0);
                for f in &model.report.findings {
                    match f.diagnostic.severity {
                        stackup_kdl::Severity::Error => errors += 1,
                        stackup_kdl::Severity::Warning => warnings += 1,
                        stackup_kdl::Severity::Note => notes += 1,
                    }
                }
                eprintln!(
                    "{}: {parts} parts, {} nets; {errors} errors, {warnings} warnings, {notes} notes",
                    design.name,
                    model.nets.len(),
                );
            }
            "tree" => print!("{}", tree(&model)),
            "netlist" => {
                let (text, export) = netlist::kicad(&lib, &model);
                failed |= export.has_errors();
                report.findings.extend(export.findings);
                match &args.out {
                    Some(path) => {
                        if let Err(e) = std::fs::write(path, &text) {
                            eprintln!("can't write {}: {e}", path.display());
                            exit(1);
                        }
                    }
                    None => print!("{text}"),
                }
            }
            "bom" => {
                let (text, export) = bom::csv(&lib, &model);
                failed |= export.has_errors();
                report.findings.extend(export.findings);
                match &args.out {
                    Some(path) => {
                        if let Err(e) = std::fs::write(path, &text) {
                            eprintln!("can't write {}: {e}", path.display());
                            exit(1);
                        }
                    }
                    None => print!("{text}"),
                }
            }
            other => {
                eprintln!("unknown command `{other}`; one of check, tree, netlist, bom");
                exit(2);
            }
        }
    }
    // A terminal gets miette's graphical report; a pipe gets one line per finding, the same
    // text a test compares against. `STACKUP_FANCY=1` asks for the graphical one anywhere.
    let terminal = std::io::stderr().is_terminal();
    let fancy = terminal || std::env::var_os("STACKUP_FANCY").is_some();
    let colour = terminal && std::env::var_os("NO_COLOR").is_none();
    if fancy {
        eprint!("{}", report.render_fancy(colour));
    } else {
        eprint!("{}", report.render());
    }
    if failed {
        exit(1);
    }
}

fn tree(model: &Model) -> String {
    let mut out = String::new();
    // A block with an `as=self` part shares its path with the part: one line, the part's.
    let part_paths: std::collections::HashSet<&str> = model
        .instances
        .iter()
        .filter(|i| matches!(i.kind, Kind::Part { .. }))
        .map(|i| i.path.as_str())
        .collect();
    for inst in &model.instances {
        if inst.path.is_empty() {
            continue;
        }
        if matches!(inst.kind, Kind::Block { .. }) && part_paths.contains(inst.path.as_str()) {
            continue;
        }
        let depth = inst.path.matches('/').count();
        let name = inst.path.rsplit('/').next().unwrap_or(&inst.path);
        let what = match &inst.kind {
            Kind::Part { .. } => format!(
                "{}{}",
                inst.designator.clone().unwrap_or_default(),
                inst.value
                    .as_ref()
                    .map(|v| format!(" {v}"))
                    .unwrap_or_default()
            ),
            Kind::Block { .. } => "block".into(),
            Kind::Scope => "scope".into(),
        };
        out.push_str(&format!("{}{name}  {what}\n", "  ".repeat(depth)));
    }
    out.push('\n');
    for net in &model.nets {
        let pins: Vec<String> = net
            .members
            .iter()
            .filter_map(|m| match m {
                Terminal::Pin { inst, pin } => {
                    let i = &model.instances[*inst];
                    Some(format!(
                        "{}.{pin}",
                        i.designator.clone().unwrap_or_else(|| i.path.clone())
                    ))
                }
                Terminal::Line { .. } => None,
            })
            .collect();
        if pins.len() >= 2 {
            out.push_str(&format!("{}: {}\n", net.name, pins.join(" ")));
        }
    }
    out
}
