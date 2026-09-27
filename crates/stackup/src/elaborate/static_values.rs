//! Values that choose components are evaluated without access to propagated net facts.

use stackup_kdl::{
    Span,
    ast::{Block, BlockItem, Derive, Part, PartItem},
};

use super::Builder;
use crate::{
    expr::{self, Eval, Scope},
    load::Decl,
    model::Kind,
};

pub(super) fn eval(b: &Builder<'_>, frame: usize, src: &str, span: Span) -> Result<Eval, String> {
    let mut scope = StaticScope {
        b,
        frame,
        stack: Vec::new(),
    };
    expr::eval(src, &mut scope, span)
}

pub(super) fn when(b: &Builder<'_>, frame: usize, src: &str) -> Result<bool, String> {
    let mut scope = StaticScope {
        b,
        frame,
        stack: Vec::new(),
    };
    expr::when(src, &mut scope)
}

struct StaticScope<'b, 'a> {
    b: &'b Builder<'a>,
    frame: usize,
    stack: Vec<(usize, String)>,
}

impl StaticScope<'_, '_> {
    fn declared_derive(&self, frame: usize, name: &str) -> Option<&Derive> {
        let inst = &self.b.instances[self.b.frames[frame].inst];
        match inst.kind {
            Kind::Part { decl, .. } => match self.b.lib.decl(decl) {
                Decl::Part(part) => part_derive(part, name),
                _ => None,
            },
            Kind::Block { decl } => match self.b.lib.decl(decl) {
                Decl::Block(block) if self.b.frames[frame].parent.is_some() => {
                    let block_name = inst.path.rsplit('/').next()?;
                    block
                        .blocks()
                        .find(|b| b.name == block_name)
                        .and_then(|b| block_derive(b, name))
                }
                Decl::Block(block) => block_derive(block, name),
                Decl::Part(part) => {
                    let block_name = inst.path.rsplit('/').next()?;
                    part.blocks()
                        .find(|b| b.name == block_name)
                        .and_then(|b| block_derive(b, name))
                }
                _ => None,
            },
            Kind::Scope => None,
        }
    }
}

impl Scope for StaticScope<'_, '_> {
    fn feature(&mut self, name: &str) -> Option<bool> {
        let mut frame = Some(self.frame);
        while let Some(f) = frame {
            if let Some(value) = self.b.frames[f].features.get(name) {
                return Some(*value);
            }
            frame = self.b.frames[f].parent;
        }
        None
    }

    fn lookup(&mut self, path: &[String], span: Span) -> Result<Option<Eval>, String> {
        if path.len() != 1 {
            return Err(format!(
                "`{}` reads a propagated fact; component selection and `when` may use only parameters and static derives",
                path.join(".")
            ));
        }
        let name = &path[0];
        let mut frame = Some(self.frame);
        while let Some(f) = frame {
            if let Some(value) = self.b.frames[f].params.get(name) {
                return Ok(Some(Eval::from_value(value)));
            }
            if let Some(expr) = self.declared_derive(f, name).map(|d| d.expr.clone()) {
                let key = (f, name.clone());
                if self.stack.contains(&key) {
                    return Err(format!("static derive `{name}` depends on itself"));
                }
                let saved = self.frame;
                self.frame = f;
                self.stack.push(key);
                let result = expr::eval(&expr, self, span);
                self.stack.pop();
                self.frame = saved;
                return result.map(Some);
            }
            frame = self.b.frames[f].parent;
        }
        Ok(None)
    }
}

fn part_derive<'a>(part: &'a Part, name: &str) -> Option<&'a Derive> {
    part.items.iter().find_map(|item| match item {
        PartItem::Derive(derive) if derive.name == name => Some(derive),
        _ => None,
    })
}

fn block_derive<'a>(block: &'a Block, name: &str) -> Option<&'a Derive> {
    block.items.iter().find_map(|item| match item {
        BlockItem::Derive(derive) if derive.name == name => Some(derive),
        _ => None,
    })
}
