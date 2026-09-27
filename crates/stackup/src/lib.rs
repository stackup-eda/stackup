//! The stackup engine: loads a design's files, elaborates a design into instances and nets, and
//! writes what a PCB tool imports.
//!
//! - [`manifest::Prefixes`] reads the manifest beside a design — and this machine's local
//!   redirects — and says where each `@prefix/…` import resolves.
//! - [`load::Library`] reads the root file and everything it `use`s, and says what each file can
//!   see.
//! - [`elaborate::elaborate`] walks a design: every `place` becomes an [`model::Instance`], a
//!   part's child blocks come with it as its features say, and `circuit` and `connect` join
//!   terminals into [`model::Net`]s. Every problem is a finding and the walk carries on.
//! - [`netlist::kicad`] writes the model as a KiCad netlist.
//!
//! - Elaboration ends by evaluating the facts, derived values, assertions and requirements over
//!   the finished nets ([`expr`], [`quantity`]), and reports what does not hold.

pub mod cli;
pub mod elaborate;
pub mod expr;
pub mod git_cache;
pub mod load;
pub mod manifest;
pub mod model;
pub mod netlist;
pub mod quantity;
pub mod report;
pub mod sexpr;
pub mod update;
pub mod uuid;
