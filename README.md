# stackup

stackup is a declarative electronics design tool. A design written in [KDL v2](https://kdl.dev) states the parts, reusable circuits, connections, and constraints of a board. The CLI checks those statements together and exports the result for KiCad. It validates choices made by the design; it does not choose components or pins on the designer's behalf.

This repository is the KDL engine and CLI. Reusable parts live in the separate [stackup library](https://github.com/stackup-eda/library). The [ARC KDL designs](https://github.com/alxhub/arc) are one set of boards built with it.

## Start here

- [SPEC.md](SPEC.md) defines the design format and its intended behavior.
- [OPEN.md](OPEN.md) tracks unresolved questions and implementation gaps.
- [kicad-plugin/README.md](kicad-plugin/README.md) explains PCB synchronization and the KiCad editor integration.

## Build and check a design

The Rust workspace contains the `stackup` CLI and `stackup-kdl` parser. Build and test it with:

```sh
cargo test --workspace
cargo install --path crates/stackup
```

Run `stackup check path/to/board.kdl` to validate a design. A project can import a pinned parts library through its `manifest.kdl`; see the library README for an example. Use `stackup --help` for the available commands.
