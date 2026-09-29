//! A purchasing BOM derived from the same elaborated parts as the KiCad netlist.

use std::collections::BTreeMap;

use stackup_kdl::ast::MetaKey;

use crate::{
    load::{Decl, Library},
    model::{Kind, Model},
    netlist::generic_footprint,
    report::Report,
};

/// One row per unique purchasing choice. `Refs` contains its placed designators.
pub fn csv(lib: &Library, model: &Model) -> (String, Report) {
    type Choice = (String, String, String, String, String, String, String, bool);
    let mut rows: BTreeMap<Choice, Vec<String>> = BTreeMap::new();
    let mut report = Report::default();

    for (_, inst) in model.parts() {
        let Kind::Part { decl, package } = inst.kind else {
            continue;
        };
        let Decl::Part(part) = lib.decl(decl) else {
            continue;
        };
        let pkg = package.and_then(|i| part.packages().nth(i));
        let symbol = part
            .meta_in(pkg, MetaKey::Symbol)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let symbol_name = symbol.split_once(':').map_or(symbol, |(_, name)| name);
        let value = inst
            .fields
            .get("bom_value")
            .cloned()
            .or_else(|| inst.value.clone())
            .or_else(|| {
                part.meta_in(pkg, MetaKey::Value)
                    .and_then(|v| v.as_str().map(str::to_string))
            })
            .unwrap_or_else(|| {
                if symbol_name.is_empty() {
                    part.name.clone()
                } else {
                    symbol_name.to_string()
                }
            });
        let order = part.order_in(pkg);
        let mpn = inst
            .fields
            .get("mpn")
            .map(String::as_str)
            .or_else(|| {
                order.and_then(|o| {
                    o.props
                        .iter()
                        .find(|p| p.key == "mpn")
                        .and_then(|p| p.value.as_str())
                })
            })
            .unwrap_or("")
            .to_string();
        if mpn.is_empty() {
            report.error(
                lib.source(inst.file),
                inst.span,
                format!("`{}` has no MPN for the BOM", inst.path),
            );
        }
        let footprint = inst.footprint.clone().or_else(|| {
            pkg.and_then(|p| p.footprint.clone()).or_else(|| {
                generic_footprint(
                    part.meta(MetaKey::Reference).and_then(|v| v.as_str()),
                    model.imperial.as_deref(),
                )
            })
        });
        let Some(footprint) = footprint else {
            report.error(
                lib.source(inst.file),
                inst.span,
                format!("`{}` has no footprint for the BOM", inst.path),
            );
            continue;
        };
        let field = |key: &str| {
            let default = if key == "manufacturer" {
                part.meta(MetaKey::Manufacturer).and_then(|v| v.as_str())
            } else {
                order.and_then(|o| {
                    o.props
                        .iter()
                        .find(|p| p.key == key)
                        .and_then(|p| p.value.as_str())
                })
            };
            inst.fields
                .get(key)
                .map(String::as_str)
                .or(default)
                .unwrap_or("")
                .to_string()
        };
        let choice = (
            value,
            footprint,
            field("manufacturer"),
            mpn,
            field("lcsc"),
            field("mouser"),
            field("digikey"),
            inst.hand,
        );
        rows.entry(choice)
            .or_default()
            .push(inst.designator.clone().unwrap_or_else(|| inst.path.clone()));
    }

    let mut output =
        String::from("Refs,Quantity,Value,Footprint,MF,MPN,LCSC,Mouser,DigiKey,Hand,DNP\n");
    for ((value, footprint, mf, mpn, lcsc, mouser, digikey, hand), mut refs) in rows {
        refs.sort();
        let fields = [
            refs.join(","),
            refs.len().to_string(),
            value,
            footprint,
            mf,
            mpn,
            lcsc,
            mouser,
            digikey,
            if hand { "Yes" } else { "" }.to_string(),
            if hand { "Yes" } else { "" }.to_string(),
        ];
        output.push_str(&fields.map(|field| quote(&field)).join(","));
        output.push('\n');
    }
    (output, report)
}

fn quote(field: &str) -> String {
    format!("\"{}\"", field.replace('"', "\"\""))
}
