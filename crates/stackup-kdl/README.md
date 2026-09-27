# stackup-eda-parser

The KDL v2 syntax layer for [Stackup](https://github.com/stackup-eda/stackup). It reads designs
and manifests into a typed AST, reports source-spanned diagnostics, and writes the AST back to KDL.
The `stackup-eda` crate builds on this parser to elaborate and check boards.
