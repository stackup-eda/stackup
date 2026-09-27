//! The KiCad netlist: what Pcbnew imports, and the seam to a PCB.
//!
//! Three things in it are load-bearing: the `(footprint …)`, which is the entire payload of an
//! import; the `(tstamps …)` UUID, derived from the part's path so a re-import finds the footprint
//! it already placed; and the absence of a date, because a generated artifact that changes every
//! second is not diffable.

use std::collections::HashMap;

use stackup_kdl::ast::MetaKey;

use crate::{
    load::{Decl, Library},
    model::{Kind, Model, Terminal},
    report::Report,
    sexpr::List,
    uuid::part_uuid,
};

const VERSION: &str = "E";

/// Writes the model as a KiCad netlist, and reports what it could not export (a part with no
/// footprint, a pin with no pad).
pub fn kicad(lib: &Library, model: &Model) -> (String, Report) {
    let mut report = Report::default();
    let mut root = List::named("export");
    root.field("version", VERSION);

    let mut design = List::named("design");
    design.field("source", format!("stackup:{}", model.name));
    design.field("tool", concat!("stackup ", env!("CARGO_PKG_VERSION")));
    root.push(design);

    root.push(components(lib, model, &mut report));
    root.push(nets(lib, model, &mut report));
    (root.render(), report)
}

fn components(lib: &Library, model: &Model, report: &mut Report) -> List {
    let mut list = List::named("components");
    for (_, inst) in model.parts() {
        let Kind::Part { decl, package } = inst.kind else {
            continue;
        };
        let Decl::Part(part) = lib.decl(decl) else {
            continue;
        };
        let designator = inst.designator.clone().unwrap_or_default();
        // Every field is read for the body the placement chose, then for the part.
        let pkg = package.and_then(|i| part.packages().nth(i));
        let symbol = part
            .meta_in(pkg, MetaKey::Symbol)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let (lib_name, sym_name) = symbol.split_once(':').unwrap_or(("", symbol));

        let mut comp = List::named("comp");
        comp.field("ref", &designator);
        let value = inst
            .value
            .clone()
            .or_else(|| {
                part.meta_in(pkg, MetaKey::Value)
                    .and_then(|v| v.as_str().map(str::to_string))
            })
            .unwrap_or_else(|| {
                if sym_name.is_empty() {
                    part.name.clone()
                } else {
                    sym_name.to_string()
                }
            });
        comp.field("value", value);

        let footprint = inst.footprint.clone().or_else(|| {
            pkg.and_then(|p| p.footprint.clone()).or_else(|| {
                generic_footprint(
                    part.meta(MetaKey::Reference).and_then(|v| v.as_str()),
                    model.imperial.as_deref(),
                )
            })
        });
        match footprint {
            Some(f) => {
                comp.field("footprint", f);
            }
            None => report.error(
                lib.source(inst.file),
                inst.span,
                format!(
                    "`{}` has no footprint: its package names none, and no `stock` picks a body",
                    inst.path
                ),
            ),
        }
        if let Some(d) = part
            .meta_in(pkg, MetaKey::Datasheet)
            .and_then(|v| v.as_str())
        {
            comp.field("datasheet", d);
        }
        let mut libsource = List::named("libsource");
        libsource.field("lib", lib_name);
        libsource.field("part", sym_name);
        libsource.field(
            "description",
            part.meta_in(pkg, MetaKey::Description)
                .and_then(|v| v.as_str())
                .unwrap_or(""),
        );
        comp.push(libsource);

        comp.push(property("Stackup Path", &inst.path));
        let source = lib.source(inst.file);
        let (line, _) = source.line_col(inst.span.offset);
        comp.push(property(
            "Stackup Source",
            &format!("{}:{line}", source.name),
        ));
        if !inst.notes.is_empty() {
            comp.push(property("Stackup Why", &inst.notes.join("; ")));
        }
        if let Some(intent) = &inst.intent {
            comp.push(property("Stackup Intent", intent));
        }
        if let Some(anchor) = &inst.anchor {
            comp.push(property("Stackup Anchor", anchor));
            if let Some(spot) = &inst.spot {
                comp.push(property("Stackup Spot", spot));
            }
        }

        let mut sheetpath = List::named("sheetpath");
        sheetpath.field("names", "/");
        sheetpath.field("tstamps", "/");
        comp.push(sheetpath);
        comp.field("tstamps", part_uuid(&model.name, &inst.path));
        list.push(comp);
    }
    list
}

fn property(name: &str, value: &str) -> List {
    let mut p = List::named("property");
    p.field("name", name);
    p.field("value", value);
    p
}

/// A generic passive's body, from the design's stock and what the part is.
fn generic_footprint(reference: Option<&str>, imperial: Option<&str>) -> Option<String> {
    let imperial = imperial?;
    let metric = match imperial {
        "0201" => "0603",
        "0402" => "1005",
        "0603" => "1608",
        "0805" => "2012",
        "1206" => "3216",
        "1210" => "3225",
        "2512" => "6332",
        _ => return None,
    };
    let prefix = match reference? {
        "R" => "R",
        "C" => "C",
        "L" => "L",
        _ => return None,
    };
    let library = match prefix {
        "R" => "Resistor_SMD",
        "C" => "Capacitor_SMD",
        "L" => "Inductor_SMD",
        _ => unreachable!(),
    };
    Some(format!("{library}:{prefix}_{imperial}_{metric}Metric"))
}

fn nets(lib: &Library, model: &Model, report: &mut Report) -> List {
    // Each part's pads per pin, and each pin's kind, for the package the placement chose.
    let mut pads: HashMap<usize, HashMap<String, Vec<String>>> = HashMap::new();
    let mut kinds: HashMap<usize, HashMap<String, String>> = HashMap::new();
    for (i, inst) in model.parts() {
        let Kind::Part { decl, package } = inst.kind else {
            continue;
        };
        let Decl::Part(part) = lib.decl(decl) else {
            continue;
        };
        let mut by_pin: HashMap<String, Vec<String>> = HashMap::new();
        if let Some(pkg) = package.and_then(|p| part.packages().nth(p)) {
            for pad in pkg.pads() {
                by_pin
                    .entry(pad.pin.clone())
                    .or_default()
                    .push(pad.label.clone());
            }
        }
        pads.insert(i, by_pin);
        kinds.insert(
            i,
            part.pins()
                .map(|p| (p.name.clone(), p.kind.clone()))
                .collect(),
        );
    }

    let mut list = List::named("nets");
    let mut code = 0;
    for net in &model.nets {
        let mut nodes = Vec::new();
        for m in &net.members {
            let Terminal::Pin { inst, pin } = m else {
                continue;
            };
            let Some(by_pin) = pads.get(inst) else {
                continue;
            };
            match by_pin.get(pin) {
                Some(labels) => {
                    for label in labels {
                        nodes.push((*inst, pin.clone(), label.clone()));
                    }
                }
                None => {
                    let i = &model.instances[*inst];
                    report.error(
                        lib.source(i.file),
                        i.span,
                        format!("`{}` has no pad for pin `{pin}` in its package", i.path),
                    );
                }
            }
        }
        if nodes.len() < 2 {
            continue;
        }
        code += 1;
        let mut entry = List::named("net");
        entry.field("code", code.to_string());
        entry.field("name", &net.name);
        entry.field("class", "Default");
        for (inst, pin, label) in nodes {
            let mut node = List::named("node");
            node.field(
                "ref",
                model.instances[inst].designator.clone().unwrap_or_default(),
            );
            node.field("pin", label);
            node.field("pinfunction", &pin);
            if let Some(kind) = kinds.get(&inst).and_then(|k| k.get(&pin)) {
                node.bare("pintype", kind.clone());
            }
            entry.push(node);
        }
        list.push(entry);
    }
    list
}
