//! Board purchasing policy over resolved placements.
//!
//! Every `when` reads the same placement before policy is applied. A selected MPN is a
//! declaration with actual ratings and catalog numbers; those are checked against what the
//! placement requires, then projected into the BOM and KiCad fields.

use std::collections::BTreeSet;

use stackup_kdl::{
    Property, Value,
    ast::{Block, BlockItem, MetaKey, Mpn, Part, Statement},
};

use crate::{
    load::{Decl, FileId, Library},
    model::{Instance, Kind, Model},
    netlist::generic_footprint,
    quantity::{Quantity, Unit},
};

fn value(v: &Value) -> String {
    match v {
        Value::String(s) | Value::Name(s) => s.clone(),
        Value::Integer(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
    }
}

fn prop<'a>(props: &'a [Property], key: &str) -> Option<&'a Property> {
    props.iter().find(|p| p.key == key)
}

fn selected_value(inst: &Instance, part: &Part, pkg: Option<&stackup_kdl::ast::Package>) -> String {
    inst.value
        .clone()
        .or_else(|| part.meta_in(pkg, MetaKey::Value).map(value))
        .unwrap_or_default()
}

fn footprint(
    inst: &Instance,
    part: &Part,
    pkg: Option<&stackup_kdl::ast::Package>,
    imperial: Option<&str>,
) -> Option<String> {
    inst.footprint
        .clone()
        .or_else(|| pkg.and_then(|p| p.footprint.clone()))
        .or_else(|| {
            generic_footprint(
                part.meta(MetaKey::Reference).and_then(Value::as_str),
                imperial,
            )
        })
}

fn fact(
    key: &str,
    inst: &Instance,
    part: &Part,
    pkg: Option<&stackup_kdl::ast::Package>,
    imperial: Option<&str>,
) -> Option<String> {
    match key {
        "kind" | "part" => Some(part.name.clone()),
        "value" => Some(selected_value(inst, part, pkg)),
        "footprint" => footprint(inst, part, pkg, imperial),
        "package.size" => {
            let fp = footprint(inst, part, pkg, imperial)?;
            [
                "01005", "0201", "0402", "0603", "0805", "1206", "1210", "1812",
            ]
            .into_iter()
            .find(|size| fp.contains(&format!("_{size}_")))
            .map(str::to_string)
        }
        "intent" => inst.intent.clone(),
        "path" => Some(inst.path.clone()),
        other => inst.fields.get(other).cloned(),
    }
}

fn equal(left: &str, right: &str) -> bool {
    match (Quantity::parse(left), Quantity::parse(right)) {
        (Ok(a), Ok(b)) => a.same(&b),
        _ => left == right,
    }
}

fn matches_when(
    when: &Statement,
    inst: &Instance,
    part: &Part,
    pkg: Option<&stackup_kdl::ast::Package>,
    imperial: Option<&str>,
) -> Result<bool, String> {
    let (key, operator, expected) = when_spec(when)?;
    matches_bound(&key, &operator, &expected, inst, part, pkg, imperial)
}

fn when_spec(when: &Statement) -> Result<(String, String, String), String> {
    if !when.children.is_empty() {
        return Err("`when` takes no children".into());
    }
    let spec = if when.args.is_empty() && when.props.len() == 1 {
        (
            when.props[0].key.clone(),
            "is".to_string(),
            value(&when.props[0].value),
        )
    } else if when.args.len() == 1 && when.props.len() == 1 {
        (
            value(&when.args[0]),
            when.props[0].key.clone(),
            value(&when.props[0].value),
        )
    } else {
        return Err("`when` needs `fact=value` or `fact min=…` / `fact max=…`".into());
    };
    let (key, operator, expected) = &spec;
    if !matches!(
        key.as_str(),
        "kind"
            | "part"
            | "value"
            | "footprint"
            | "package.size"
            | "intent"
            | "path"
            | "required_voltage"
    ) {
        return Err(format!("`{key}` is not a placement fact a match can read"));
    }
    if !matches!(operator.as_str(), "is" | "min" | "max") {
        return Err("`when` needs exactly one of `is=`, `min=` or `max=`".into());
    }
    if operator != "is" {
        Quantity::parse(expected).map_err(|e| format!("`{key}` bound: {e}"))?;
    }
    Ok(spec)
}

fn matches_bound(
    key: &str,
    operator: &str,
    expected: &str,
    inst: &Instance,
    part: &Part,
    pkg: Option<&stackup_kdl::ast::Package>,
    imperial: Option<&str>,
) -> Result<bool, String> {
    let Some(actual) = fact(key, inst, part, pkg, imperial) else {
        return Ok(false);
    };
    match operator {
        "is" if key == "required_voltage" => {
            let b = Quantity::parse(expected).map_err(|e| format!("`{key}`: {e}"))?;
            Ok(inst.required_voltage.is_some_and(|a| a.same(&b)))
        }
        "is" => Ok(equal(&actual, expected)),
        bound => {
            let a = if key == "required_voltage" {
                inst.required_voltage.unwrap()
            } else {
                Quantity::parse(&actual).map_err(|e| format!("`{key}`: {e}"))?
            };
            let b = Quantity::parse(expected).map_err(|e| format!("`{key}` bound: {e}"))?;
            if a.unit != b.unit {
                return Err(format!("`{key}` and its bound have different units"));
            }
            Ok(if bound == "min" {
                a.lo >= b.hi
            } else {
                a.hi <= b.lo
            })
        }
    }
}

fn rule_mpn(rule: &Statement) -> Result<String, String> {
    if rule.args != [Value::Name("placement".into())]
        && rule.args != [Value::String("placement".into())]
    {
        return Err("`match` needs `placement`".into());
    }
    if !rule.props.is_empty() {
        return Err("`match placement` takes no properties".into());
    }
    let mut chosen = None;
    for item in &rule.children {
        match item.name.as_str() {
            "when" => {
                when_spec(item)?;
            }
            "set"
                if item.args.is_empty()
                    && item.children.is_empty()
                    && item.props.len() == 1
                    && item.props[0].key == "mpn" =>
            {
                if chosen.replace(value(&item.props[0].value)).is_some() {
                    return Err("`match placement` sets `mpn=` twice".into());
                }
            }
            _ => return Err("`match placement` contains only `when` and `set mpn=…`".into()),
        }
    }
    chosen.ok_or_else(|| "`match placement` needs `set mpn=…`".into())
}

pub fn apply(lib: &Library, file: FileId, design: &Block, model: &mut Model) {
    let rules: Vec<_> = design
        .items
        .iter()
        .filter_map(|item| match item {
            BlockItem::Match(rule) => Some(rule),
            _ => None,
        })
        .collect();
    let mut parsed = Vec::new();
    for rule in rules {
        match rule_mpn(rule) {
            Ok(id) => match lib.lookup(file, &id) {
                Some(r) if matches!(lib.decl(r), Decl::Mpn(_)) => parsed.push((rule, id)),
                Some(_) => model.report.error(
                    lib.source(file),
                    rule.span,
                    format!("`{id}` is not an MPN declaration"),
                ),
                None => model.report.error(
                    lib.source(file),
                    rule.span,
                    format!("MPN `{id}` is not in scope"),
                ),
            },
            Err(e) => model.report.error(lib.source(file), rule.span, e),
        }
    }
    for index in 0..model.instances.len() {
        let Kind::Part { decl, package } = model.instances[index].kind else {
            continue;
        };
        let Decl::Part(part) = lib.decl(decl) else {
            continue;
        };
        let pkg = package.and_then(|p| part.packages().nth(p));
        let inst = &model.instances[index];
        let mut choices = Vec::new();
        for (rule, id) in &parsed {
            let mut yes = true;
            for when in &rule.children {
                if when.name != "when" {
                    continue;
                }
                match matches_when(when, inst, part, pkg, model.imperial.as_deref()) {
                    Ok(matches) => yes &= matches,
                    Err(e) => {
                        model.report.error(lib.source(file), when.span, e);
                        yes = false;
                    }
                }
            }
            if yes {
                choices.push((rule.span, id.as_str()));
            }
        }
        let distinct: BTreeSet<_> = choices.iter().map(|(_, id)| *id).collect();
        if distinct.len() > 1 {
            model.report.error(
                lib.source(inst.file),
                inst.span,
                format!(
                    "`{}` matches conflicting MPN choices: {}",
                    inst.path,
                    distinct.into_iter().collect::<Vec<_>>().join(", ")
                ),
            );
            continue;
        }
        let mut source_file = inst.file;
        let chosen = if let Some((span, id)) = choices.first() {
            let Some(r) = lib.lookup(file, id) else {
                model.report.error(
                    lib.source(file),
                    *span,
                    format!("MPN `{id}` is not in scope"),
                );
                continue;
            };
            if !matches!(lib.decl(r), Decl::Mpn(_)) {
                model.report.error(
                    lib.source(file),
                    *span,
                    format!("`{id}` is not an MPN declaration"),
                );
                continue;
            }
            source_file = file;
            Some((*id).to_string())
        } else {
            inst.fields.get("mpn").cloned().or_else(|| {
                part.order_in(pkg)
                    .and_then(|o| prop(&o.props, "mpn").map(|p| value(&p.value)))
            })
        };
        let Some(id) = chosen else {
            continue;
        };
        let Some(r) = lib
            .lookup(source_file, &id)
            .or_else(|| lib.lookup(decl.file, &id))
        else {
            // Existing part and placement `mpn=` strings remain literal choices.
            continue;
        };
        let Decl::Mpn(mpn) = lib.decl(r) else {
            continue;
        };
        validate(lib, model, index, part, pkg, mpn);
        let inst = &mut model.instances[index];
        // A declared MPN is the whole purchasing choice. Empty fields shadow old `order`
        // defaults, so an unlisted supplier cannot be exported for a different component.
        for key in [
            "manufacturer",
            "lcsc",
            "mouser",
            "digikey",
            "series",
            "voltage",
            "current",
            "dissipation",
            "tolerance",
            "dielectric",
        ] {
            inst.fields.insert(key.into(), String::new());
        }
        inst.fields.insert("mpn".into(), id);
        for key in ["manufacturer", "rated_voltage"] {
            if let Some(p) = prop(&mpn.props, key) {
                inst.fields.insert(
                    if key == "rated_voltage" {
                        "voltage"
                    } else {
                        key
                    }
                    .into(),
                    value(&p.value),
                );
            }
        }
        for p in &mpn.catalog {
            inst.fields.insert(p.key.clone(), value(&p.value));
        }
    }
}

fn validate(
    lib: &Library,
    model: &mut Model,
    index: usize,
    part: &Part,
    pkg: Option<&stackup_kdl::ast::Package>,
    mpn: &Mpn,
) {
    let inst = &model.instances[index];
    let mut errors = Vec::new();
    for key in ["kind", "value", "footprint"] {
        if let Some(p) = prop(&mpn.props, key) {
            let actual = fact(key, inst, part, pkg, model.imperial.as_deref()).unwrap_or_default();
            if !equal(&actual, &value(&p.value)) {
                errors.push(format!(
                    "MPN `{}` has {key} `{}`, but `{}` needs `{actual}`",
                    mpn.name,
                    value(&p.value),
                    inst.path
                ));
            }
        }
    }
    if let Some(required) = inst.required_voltage {
        match prop(&mpn.props, "rated_voltage") {
            None => errors.push(format!(
                "MPN `{}` has no `rated_voltage` for `{}`",
                mpn.name, inst.path
            )),
            Some(rated) => match Quantity::parse(&value(&rated.value)) {
                Ok(rating) if rating.unit == Unit::VOLT && rating.lo >= required.hi => {}
                Ok(rating) if rating.unit == Unit::VOLT => errors.push(format!(
                    "MPN `{}` is rated {}, below `{}`'s required {}",
                    mpn.name,
                    value(&rated.value),
                    inst.path,
                    required
                )),
                _ => errors.push(format!(
                    "`{}` and MPN `{}` need voltage quantities",
                    inst.path, mpn.name
                )),
            },
        }
    }
    for message in errors {
        model
            .report
            .error(lib.source(inst.file), inst.span, message);
    }
}
