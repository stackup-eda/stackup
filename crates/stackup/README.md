# stackup-eda

Stackup is a declarative electronics design tool. Its `stackup` CLI checks KDL board designs
and exports KiCad netlists and purchasing BOMs.

Install the command with `cargo install stackup-eda`, then run
`stackup check path/to/board.kdl`. Run `stackup bom path/to/board.kdl --locked` to print a CSV
BOM. Rows group placements with the same value, footprint, manufacturer, MPN and distributor
codes; `Refs` lists their designators. The Rust library also exposes the checker and CLI runner.

See the [project repository](https://github.com/stackup-eda/stackup) for the format specification,
parts library, and KiCad integration.
