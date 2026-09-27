//! Reads every file given and reports what it could not read.
//!
//!     cargo run -p stackup-kdl --example check -- path/to/design.kdl

use stackup_kdl::Document;

fn main() {
    let mut bad = false;
    for path in std::env::args().skip(1) {
        match Document::open(&path) {
            Err(e) => {
                bad = true;
                print!("{}", e.render());
            }
            Ok(doc) => {
                let (file, diags) = doc.file();
                if diags.is_empty() {
                    println!("{path}: ok ({} declarations)", file.items.len());
                } else {
                    bad = true;
                    print!("{}", diags.render());
                }
            }
        }
    }
    if bad {
        std::process::exit(1);
    }
}
