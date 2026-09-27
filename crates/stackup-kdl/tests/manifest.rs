//! Reading a `manifest.kdl`.

use stackup_eda_parser::{
    Document,
    manifest::{LibrarySource, Manifest},
};

fn read(text: &str) -> (Manifest, Vec<String>) {
    let doc = Document::parse("manifest.kdl", text).unwrap();
    let (m, diags) = doc.manifest();
    (m, diags.iter().map(|d| d.message.clone()).collect())
}

#[test]
fn every_statement() {
    let (m, messages) = read(
        r#"
// A project's manifest.
stackup "0.1"
name arc
library ti    path="../ti"
library stm32 git="https://github.com/alxhub/stackup" dir="library/mcu/stm32" rev="a1b2c3"
"#,
    );
    assert!(messages.is_empty(), "{messages:?}");
    let v = m.stackup.as_ref().unwrap();
    assert_eq!((v.major, v.minor, v.text.as_str()), (0, 1, "0.1"));
    assert_eq!(m.name.as_ref().unwrap().name, "arc");
    assert_eq!(m.libraries.len(), 2);
    assert_eq!(m.library("ti").unwrap().path(), Some("../ti"));
    assert_eq!(
        m.library("stm32").unwrap().source,
        LibrarySource::Git {
            url: "https://github.com/alxhub/stackup".into(),
            rev: "a1b2c3".into(),
            dir: Some("library/mcu/stm32".into()),
        }
    );
    assert_eq!(m.library("nobody"), None);
}

#[test]
fn what_is_refused() {
    let (m, messages) = read(
        r#"
stackup "0.1"
stackup "0.2"
stackup "two"
name a
name b
library one
library two path="x" git="y" rev="z"
library three git="y"
library four path="x" rev="z"
library five path="x" size=big
library five path="y"
part resistor {}
"#,
    );
    assert_eq!(
        messages,
        [
            "`stackup` is stated twice",
            "`stackup` takes a version like \"0.1\", not \"two\"",
            "`name` is stated twice",
            "`library` needs `path=` or `git=`",
            "`library` takes `path=` or `git=`, not both",
            "a `git=` library needs `rev=`, the commit it is pinned to",
            "`rev=` and `dir=` go with `git=`; a `path=` library is a directory as it is",
            "`library` has no `size=` property; it takes `path=`, `git=`, `rev=`, `dir=`",
            "library `five` is declared twice",
            "`part` is not a statement a manifest can contain",
        ]
    );
    // What could be read was kept.
    assert_eq!(m.stackup.unwrap().text, "0.1");
    assert_eq!(m.name.unwrap().name, "a");
    assert_eq!(m.libraries.len(), 1);
    assert_eq!(m.libraries[0].name, "five");
}

#[test]
fn a_design_file_is_not_a_manifest_and_the_reverse() {
    let doc = Document::parse("manifest.kdl", "name arc\nstackup \"0.1\"\n").unwrap();
    let (_, diags) = doc.file();
    assert_eq!(diags.items.len(), 2);
    assert_eq!(
        diags.items[0].message,
        "`name` is not a statement a file can contain"
    );
}
