//! Connection-local peripheral capabilities. Never publish these on electrical nets.
use super::*;
use crate::expr::{self, Eval, Scope};

struct BindingScope {
    values: HashMap<String, String>,
    names: HashSet<String>,
}

impl Scope for BindingScope {
    fn lookup(&mut self, path: &[String], _: Span) -> Result<Option<Eval>, String> {
        if let [name] = path {
            if let Some(value) = self.values.get(name) {
                return Ok(Some(Eval::text(value)));
            }
            if self.names.contains(name) {
                return Ok(Some(Eval::text(name)));
            }
        }
        Err(format!(
            "unknown peripheral condition name `{}`",
            path.join(".")
        ))
    }
}

impl Builder<'_> {
    pub(super) fn check_connection_peripherals(
        &mut self,
        file: FileId,
        connect: &stackup_kdl::ast::Connect,
        ty: &TypeInfo,
        from: &HashMap<String, Terminal>,
    ) {
        let providers: HashSet<usize> = from
            .values()
            .filter_map(|t| match t {
                Terminal::Pin { inst, .. }
                    if self
                        .part_of(*inst)
                        .is_some_and(|p| p.peripherals().next().is_some()) =>
                {
                    Some(*inst)
                }
                _ => None,
            })
            .collect();
        if !connect.requires.is_empty() && providers.len() != 1 {
            self.error(file, connect.span, "peripheral requirements need one MCU/peripheral provider on the `from` side with explicit pin bindings");
            return;
        }
        for inst in providers {
            let part = self.part_of(inst).unwrap().clone();
            let package = match self.instances[inst].kind {
                Kind::Part {
                    package: Some(i), ..
                } => part.packages().nth(i).map(|p| p.name.clone()),
                _ => None,
            };
            let mut values = HashMap::new();
            for (line, terminal) in from {
                if let Terminal::Pin { inst: owner, pin } = terminal {
                    if *owner == inst {
                        values.insert(line.clone(), pin.clone());
                    }
                }
            }
            values.insert("package".into(), package.unwrap_or_default());
            let mut names: HashSet<String> = part.pins().map(|p| p.name.clone()).collect();
            names.extend(part.packages().map(|p| p.name.clone()));
            let mut scope = BindingScope { values, names };
            let mut possibilities = Vec::new();
            let mut details = Vec::new();
            for peripheral in part.peripherals().filter(|p| p.kind == ty.name) {
                // Keep the whole binding together; requirements may narrow candidates, but
                // capabilities from different candidates cannot be combined.
                let matches = from.iter().all(|(line, terminal)| {
                    matches!(terminal, Terminal::Pin { inst: owner, pin }
                        if *owner == inst && peripheral.lines.iter().any(|l| &l.name == line && l.pins.contains(pin)))
                });
                let mut caps = HashSet::new();
                for has in &peripheral.has {
                    let when = has.props.iter().find(|p| p.key == "when");
                    let enabled = if let Some(when) = when {
                        let result = when
                            .value
                            .as_str()
                            .ok_or_else(|| "`when` needs an expression string".to_string())
                            .and_then(|text| expr::eval(text, &mut scope, when.span))
                            .and_then(|v| {
                                v.as_bool().ok_or_else(|| {
                                    "peripheral `when` must evaluate to a boolean".to_string()
                                })
                            });
                        match result {
                            Ok(on) => on,
                            Err(error) => {
                                // The declaration belongs to the part's file, not the board.
                                let source = match self.instances[inst].kind {
                                    Kind::Part { decl, .. } => decl.file,
                                    _ => file,
                                };
                                self.error(source, when.span, error);
                                false
                            }
                        }
                    } else {
                        true
                    };
                    if enabled {
                        caps.insert(has.capability.clone());
                    }
                    if let Some(when) = when {
                        details.push(format!(
                            "{}: {} when {}",
                            peripheral.name,
                            has.capability,
                            when.value.as_str().unwrap_or("<invalid>")
                        ));
                    }
                }
                if matches {
                    possibilities.push(caps);
                }
            }
            if !connect.requires.is_empty()
                && !possibilities
                    .iter()
                    .any(|caps| connect.requires.iter().all(|r| caps.contains(&r.fact)))
            {
                let mut bindings: Vec<_> = scope
                    .values
                    .iter()
                    .filter(|(k, _)| k.as_str() != "package")
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect();
                bindings.sort();
                let required = connect
                    .requires
                    .iter()
                    .map(|r| format!("peripheral.{}", r.fact))
                    .collect::<Vec<_>>()
                    .join(", ");
                self.error(file, connect.span, format!("`{}` connection from `{}` requires {required}, but no single peripheral provides them for {}; {}", ty.name, self.instances[inst].path, bindings.join(", "), details.join("; ")));
            }
        }
    }
}
