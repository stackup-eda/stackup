# Working in stackup KDL

This repository contains the stackup KDL language, CLI, and KiCad integration. Use this file for collaboration and workflow. Keep language rules and implementation details in the project documentation and source.

## Read first

- `SPEC.md` is the language specification. Check it before changing behavior; update it when the intended behavior changes.
- `OPEN.md` records unsettled questions and known gaps. Consult it before treating a missing feature as an oversight, and update it when a question is resolved or a new one appears.
- `kicad-plugin/README.md` describes the plugin workflow. Read it for changes that cross the CLI and PCB editor boundary.

## Working together

- Check `git status` and the relevant files before editing. Other agents may be working in the same checkout.
- Agree on bounded files or tasks when work is parallel. Coordinate before touching a file another agent owns; do not reset or overwrite someone else's work.
- Keep a behavior change, its documentation, and its validation together. Put rationale in the relevant specification or code comment, rather than expanding this file with design notes.
- Run focused tests for changed code. For changes across crates or public behavior, run `cargo test --workspace` as well. Report what ran and any remaining gap.

The sibling `stackup-parts` and `arc-kdl` repositories consume this tool. Coordinate changes that alter their imports, validation results, or KiCad workflow across the affected repositories.
