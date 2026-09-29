//! Elaboration, on designs written for the purpose and an ARC board when available.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use stackup::{
    bom,
    elaborate::elaborate,
    load::Library,
    manifest::{Options, Prefixes},
    model::{Model, Terminal},
    netlist,
};

#[test]
fn bom_groups_identical_orders_and_preserves_csv_fields() {
    let fx = Fixture::new(
        "bom-csv",
        &[(
            "board.kdl",
            r#"
part capacitor {
    symbol "Device:C"
    reference C
    manufacturer "Default Maker"
    order mpn="DEFAULT"
    pin A passive
    pin B passive
    package chip footprint="Capacitor_SMD:C_0402_1005Metric" { pad A 1; pad B 2 }
}

design demo {
    place capacitor C1 bom_value="100nF, tested" manufacturer="Maker \"One\"" mpn="ABC" lcsc="C123"
    place capacitor C2 bom_value="100nF, tested" manufacturer="Maker \"One\"" mpn="ABC" lcsc="C123"
    place capacitor C3 bom_value="100nF, tested" manufacturer="Maker \"One\"" mpn="ABC" lcsc="C123" hand=#true
}

"#,
        )],
    );
    let (lib, loaded) = Library::load(&fx.dir.join("board.kdl"), &Prefixes::default());
    assert!(loaded.is_empty(), "{}", loaded.render());
    let (file, design) = lib.designs()[0];
    let model = elaborate(&lib, file, design);
    assert!(model.report.is_empty(), "{}", model.report.render());
    let (csv, report) = bom::csv(&lib, &model);
    assert!(report.is_empty(), "{}", report.render());
    assert!(csv.starts_with("Refs,Quantity,Value,Footprint,MF,MPN,LCSC,Mouser,DigiKey,Hand,DNP\n"));
    assert!(csv.contains("\"C1,C2\",\"2\",\"100nF, tested\",\"Capacitor_SMD:C_0402_1005Metric\",\"Maker \"\"One\"\"\",\"ABC\",\"C123\",\"\",\"\",\"\",\"\""), "{csv}");
    assert!(csv.contains("\"C3\",\"1\",\"100nF, tested\",\"Capacitor_SMD:C_0402_1005Metric\",\"Maker \"\"One\"\"\",\"ABC\",\"C123\",\"\",\"\",\"Yes\",\"Yes\""), "{csv}");
}

#[test]
fn board_matches_library_children_to_validated_mpns() {
    let fx = Fixture::new(
        "matched-mpns",
        &[
            (
                "parts.kdl",
                r#"
part capacitor {
    reference C
    pin A passive
    pin B passive
    package chip footprint="Capacitor_SMD:C_0402_1005Metric" { pad A 1; pad B 2 }
}
block supply {
    place capacitor C value="100nF" required_voltage="12V" digikey="OLD"
}
"#,
            ),
            (
                "board.kdl",
                r#"
use "./parts.kdl"
mpn C16 manufacturer="Acme" kind=capacitor value="100nF" footprint="Capacitor_SMD:C_0402_1005Metric" rated_voltage="16V" {
    catalog lcsc="C123" mouser="123-C16"
}
design demo {
    place supply rail
    place capacitor high value="100nF" required_voltage="25V"
    match placement {
        when kind=capacitor
        when package.size="0402"
        when value="100nF"
        when required_voltage max="16V"
        set mpn=C16
    }
}
"#,
            ),
        ],
    );
    let (lib, loaded) = Library::load(&fx.dir.join("board.kdl"), &Prefixes::default());
    assert!(loaded.is_empty(), "{}", loaded.render());
    let (file, design) = lib.designs()[0];
    let model = elaborate(&lib, file, design);
    assert!(model.report.is_empty(), "{}", model.report.render());
    let cap = model.parts().find(|(_, i)| i.path == "rail/C").unwrap().1;
    assert_eq!(cap.fields.get("mpn").map(String::as_str), Some("C16"));
    assert_eq!(cap.fields.get("voltage").map(String::as_str), Some("16V"));
    assert_eq!(cap.fields.get("lcsc").map(String::as_str), Some("C123"));
    assert_eq!(cap.fields.get("digikey").map(String::as_str), Some(""));
    let (csv, report) = bom::csv(&lib, &model);
    assert!(
        report.render().contains("`high` has no MPN for the BOM"),
        "{}",
        report.render()
    );
    assert!(csv.contains("\"C16\",\"C123\",\"123-C16\""), "{csv}");
    let (netlist, report) = netlist::kicad(&lib, &model);
    assert!(report.is_empty(), "{}", report.render());
    assert!(
        netlist.contains("(name \"Manufacturer_Part_Number\")\n\t\t\t\t(value \"C16\")"),
        "{netlist}"
    );
    assert!(
        netlist.contains("(name \"Voltage\")\n\t\t\t\t(value \"16V\")"),
        "{netlist}"
    );
    assert!(!netlist.contains("OLD"), "{netlist}");
}

#[test]
fn placement_match_reads_a_rating_derived_from_a_net_fact() {
    let fx = Fixture::new(
        "derived-required-rating",
        &[
            ("passives.kdl", PASSIVES),
            ("connectors.kdl", CONNECTORS),
            (
                "board.kdl",
                r#"
use "./passives.kdl"
use "./connectors.kdl"
mpn C16 kind=capacitor value="100nF" footprint="Capacitor_SMD:C_0402_1005Metric" rated_voltage="16V"
block filter {
    port rail type=power
    derive needed "rail.voltage.max * 2"
    place capacitor C value="100nF" required_voltage=needed
    circuit C.a rail.rail
}
design demo {
    stock { packages imperial="0402" }
    place filter f rail=V5.out
    place power-jack V5 voltage="5V"
    match placement {
        when kind=capacitor
        when required_voltage max="16V"
        set mpn=C16
    }
}
"#,
            ),
        ],
    );
    let model = fx.model("board.kdl", "demo");
    assert!(model.report.is_empty(), "{}", model.report.render());
    let cap = model.parts().find(|(_, i)| i.path == "f/C").unwrap().1;
    assert_eq!(cap.required_voltage.unwrap().to_string(), "10V");
    assert_eq!(cap.fields.get("mpn").map(String::as_str), Some("C16"));
}

#[test]
fn chosen_mpn_must_satisfy_placement_rating() {
    let fx = Fixture::new(
        "mpn-rating",
        &[(
            "board.kdl",
            r#"
part capacitor { reference C; pin A passive; package chip footprint="C_0402" { pad A 1 } }
mpn C16 kind=capacitor value="10nF" footprint="C_0402" rated_voltage="16V"
design demo {
    place capacitor C value="100nF" required_voltage="25V"
    match placement { when kind=capacitor; set mpn=C16 }
}

"#,
        )],
    );
    let model = fx.model("board.kdl", "demo");
    assert!(
        model.report.render().contains("below `C`'s required 25V"),
        "{}",
        model.report.render()
    );
    assert!(model.report.render().contains("has value `10nF`"));
}

#[test]
fn conflicting_mpn_rules_and_missing_declarations_are_findings() {
    let fx = Fixture::new(
        "mpn-policy-errors",
        &[(
            "board.kdl",
            r#"
part capacitor { reference C; pin A passive; package chip footprint="C_0402" { pad A 1 } }
mpn A kind=capacitor
mpn B kind=capacitor
design demo {
    place capacitor C
    match placement { when kind=capacitor; set mpn=A }
    match placement { when kind=capacitor; set mpn=B }
    match placement { when kind=inductor; set mpn=missing }
}
"#,
        )],
    );
    let model = fx.model("board.kdl", "demo");
    let messages = model.report.render();
    assert!(
        messages.contains("MPN `missing` is not in scope"),
        "{messages}"
    );
    assert!(
        messages.contains("matches conflicting MPN choices"),
        "{messages}"
    );
}

#[test]
fn hand_placement_requires_a_boolean() {
    let fx = Fixture::new(
        "hand-boolean",
        &[(
            "board.kdl",
            "part widget { reference U; pin P passive; package body footprint=\"Test:Widget\" { pad P 1 } }\ndesign demo { place widget U1 hand=\"yes\" }\n",
        )],
    );
    let (lib, loaded) = Library::load(&fx.dir.join("board.kdl"), &Prefixes::default());
    assert!(loaded.is_empty(), "{}", loaded.render());
    let (file, design) = lib.designs()[0];
    let model = elaborate(&lib, file, design);
    assert!(model.report.has_errors());
    assert!(
        model
            .report
            .render()
            .contains("`hand=` needs #true or #false")
    );
}

/// A scratch directory holding the given files, loaded as a library.
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str, files: &[(&str, &str)]) -> Fixture {
        let dir = std::env::temp_dir().join(format!("stackup-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, text) in files {
            let full = dir.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, text).unwrap();
        }
        Fixture { dir }
    }

    fn model(&self, root: &str, design: &str) -> Model {
        let (lib, report) = Library::load(
            &self.dir.join(root),
            &Prefixes::single("stackup", self.dir.join("stackup")),
        );
        assert!(report.is_empty(), "{}", report.render());
        let (file, block) = lib
            .designs()
            .into_iter()
            .find(|(_, d)| d.name == design)
            .unwrap_or_else(|| panic!("no design {design}"));
        elaborate(&lib, file, block)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn placement_selects_ordering_and_rating_without_a_part_variant() {
    let fx = Fixture::new(
        "placement-stock",
        &[(
            "board.kdl",
            r#"
part capacitor {
    symbol "Device:C"
    reference C
    manufacturer "Generic"
    order mpn="BASE"
    pin A passive
    pin B passive
    package chip footprint="Capacitor_SMD:C_0402_1005Metric" {
        pad A 1
        pad B 2
    }
}

design demo {
    place capacitor Caccel value="100nF" bom_value="100nF | CL05B104KO5NNNC | Samsung | 0402" \
        manufacturer="Samsung" mpn="CL05B104KO5NNNC" lcsc=C1525 \
        series="CL05" voltage="16V" dielectric="X7R"
}
"#,
        )],
    );
    let (lib, loaded) = Library::load(
        &fx.dir.join("board.kdl"),
        &Prefixes::single("stackup", fx.dir.join("stackup")),
    );
    assert!(loaded.is_empty(), "{}", loaded.render());
    let (file, design) = lib.designs()[0];
    let model = elaborate(&lib, file, design);
    assert!(model.report.is_empty(), "{}", model.report.render());
    let (text, report) = netlist::kicad(&lib, &model);
    assert!(report.is_empty(), "{}", report.render());
    for expected in [
        "(value \"100nF | CL05B104KO5NNNC | Samsung | 0402\")",
        "(name \"MF\")\n\t\t\t\t(value \"Samsung\")",
        "(name \"Manufacturer_Part_Number\")\n\t\t\t\t(value \"CL05B104KO5NNNC\")",
        "(name \"LCSC\")\n\t\t\t\t(value \"C1525\")",
        "(name \"Voltage\")\n\t\t\t\t(value \"16V\")",
        "(name \"Dielectric\")\n\t\t\t\t(value \"X7R\")",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
}

#[test]
fn netlist_source_labels_are_independent_of_library_checkout_paths() {
    let fx = Fixture::new(
        "portable-sources",
        &[
            (
                "stackup/parts.kdl",
                r#"
part resistor {
    symbol "Device:R"
    reference R
    pin A passive
    package chip footprint="Resistor_SMD:R_0402_1005Metric" { pad A 1 }
}
block wrapper { place resistor as=self }
"#,
            ),
            (
                "board.kdl",
                "use \"@stackup/parts\"\ndesign board { place wrapper R }\n",
            ),
        ],
    );
    let (lib, loaded) = Library::load(
        &fx.dir.join("board.kdl"),
        &Prefixes::single("stackup", fx.dir.join("stackup")),
    );
    assert!(loaded.is_empty(), "{}", loaded.render());
    assert_eq!(lib.source_label(0), "board.kdl");
    assert_eq!(lib.source_label(1), "@stackup/parts.kdl");
    let (file, design) = lib.designs()[0];
    let model = elaborate(&lib, file, design);
    let (netlist, report) = netlist::kicad(&lib, &model);
    assert!(report.is_empty(), "{}", report.render());
    assert!(
        netlist.contains("(value \"@stackup/parts.kdl:"),
        "{netlist}"
    );
}

/// Every net as `name: D.pin D.pin …`, pins sorted, for comparing.
fn nets(model: &Model) -> HashMap<String, Vec<String>> {
    model
        .nets
        .iter()
        .map(|n| {
            let mut pins: Vec<String> = n
                .members
                .iter()
                .filter_map(|m| match m {
                    Terminal::Pin { inst, pin } => Some(format!(
                        "{}.{pin}",
                        model.instances[*inst].designator.clone().unwrap()
                    )),
                    Terminal::Line { .. } => None,
                })
                .collect();
            pins.sort();
            (n.name.clone(), pins)
        })
        .filter(|(_, pins)| pins.len() >= 2)
        .collect()
}

fn pins(s: &str) -> Vec<String> {
    let mut v: Vec<String> = s.split_whitespace().map(str::to_string).collect();
    v.sort();
    v
}

#[test]
fn pin_associated_parts_export_physical_anchor_pads() {
    let fx = Fixture::new(
        "anchors",
        &[(
            "board.kdl",
            r#"
part chip {
    reference U
    pin VCC power_in { also SUPPLY }
    pin GND power_in
    package qfn footprint="Test:QFN" {
        pad VCC 1
        pad VCC 2
        pad GND 3
    }
}
part capacitor {
    reference C
    pin A passive
    pin B passive
    package smd footprint="Test:C" { pad A 1; pad B 2 }
}
block decoupler {
    place capacitor as=self
}
design board {
    place capacitor bypass anchor=controller.SUPPLY@2 spot="0.4 -1.16 -90"
    place capacitor bulk anchor=controller.GND
    place decoupler local anchor=controller.GND
    place chip controller
}
"#,
        )],
    );
    let (lib, loaded) = Library::load(&fx.dir.join("board.kdl"), &Prefixes::default());
    assert!(loaded.is_empty(), "{}", loaded.render());
    let (file, design) = lib.designs()[0];
    let model = elaborate(&lib, file, design);
    assert!(model.report.is_empty(), "{}", model.report.render());
    let parts: Vec<_> = model
        .parts()
        .map(|(_, i)| (i.path.as_str(), i.anchor.as_deref()))
        .collect();
    assert_eq!(
        parts,
        [
            ("bypass", Some("U1.2")),
            ("bulk", Some("U1.3")),
            ("local", Some("U1.3")),
            ("controller", None)
        ]
    );
    let (netlist, report) = netlist::kicad(&lib, &model);
    assert!(report.is_empty(), "{}", report.render());
    assert!(netlist.contains("(name \"Stackup Anchor\")\n\t\t\t\t(value \"U1.2\")"));
    assert!(netlist.contains("(name \"Stackup Spot\")\n\t\t\t\t(value \"0.4 -1.16 -90\")"));
}

#[test]
fn a_block_anchor_parameter_resolves_in_its_callers_scope() {
    let fx = Fixture::new(
        "anchor-parameter",
        &[(
            "board.kdl",
            r#"
part capacitor {
    reference C
    pin A passive
    package body footprint="Test:C" { pad A 1 }
}
block decouple {
    param anchor reference default=#null
    place capacitor as=self anchor=anchor
}
part controller {
    reference U
    pin VDD power_in
    package body footprint="Test:U" { pad VDD 7 }
    block supply {
        default
        place decouple Cvdd anchor=VDD
    }
}
design board {
    place controller mcu
    place decouple free
}
"#,
        )],
    );
    let model = fx.model("board.kdl", "board");
    assert!(model.report.is_empty(), "{}", model.report.render());
    let anchored = model
        .parts()
        .find(|(_, i)| i.path == "mcu/supply/Cvdd")
        .unwrap()
        .1;
    assert_eq!(anchored.anchor.as_deref(), Some("U1.7"));
    let free = model.parts().find(|(_, i)| i.path == "free").unwrap().1;
    assert_eq!(free.anchor, None);
}

#[test]
fn ambiguous_anchor_pin_needs_a_pad() {
    let fx = Fixture::new(
        "ambiguous-anchor",
        &[(
            "board.kdl",
            r#"
part chip {
    pin VCC power_in
    package body footprint="Test:Chip" { pad VCC 1; pad VCC 2 }
}

part capacitor {
    pin A passive
    package body footprint="Test:C" { pad A 1 }
}
design board {
    place chip controller
    place capacitor C anchor=controller.VCC
    place capacitor wrong anchor=U1.VCC@1
}
"#,
        )],
    );
    let model = fx.model("board.kdl", "board");
    assert!(model.report.render().contains("select one with `@pad`"));
    assert!(
        model
            .report
            .render()
            .contains("`U1` is not a name in scope")
    );
}

#[test]
fn multiline_circuit_names_each_equipotential_section() {
    let fx = Fixture::new(
        "multiline-circuit",
        &[(
            "board.kdl",
            r#"
part point {
    reference J
    pin N passive
    port node pin=N
    package body footprint="Test:Point" { pad N 1 }
}
part resistor {
    reference R
    pin A passive
    pin B passive
    port a pin=A
    port b pin=B
    package body footprint="Test:R" { pad A 1; pad B 2 }
}
design board {
    place point supply
    place point pullup
    place point decouple
    place resistor sense
    scope mcu { place point gpio0 }
    circuit {
        from supply
        to pullup decouple sense name="INPUT"
        to "mcu/gpio0" name="INPUT_SENSE"
    }
}
design invalid {
    place point supply
    place point load
    place resistor sense
    circuit {
        from supply
        to load
        to sense
    }
    circuit {
        from supply
        to sense load
    }
    circuit {
        from supply
        to sense
    }
    circuit {
        from sense
        to load
    }
}
"#,
        )],
    );
    let model = fx.model("board.kdl", "board");
    assert!(model.report.is_empty(), "{}", model.report.render());
    let n = nets(&model);
    assert_eq!(n["INPUT"], pins("J1.N J2.N J3.N R1.A"));
    assert_eq!(n["INPUT_SENSE"], pins("R1.B J4.N"));

    let invalid = fx.model("board.kdl", "invalid");
    let messages = invalid.report.render();
    assert!(
        messages.contains("a `to` ending at a terminal must be the final `to`"),
        "{messages}"
    );
    assert!(
        messages.contains("a series element must end its `to` line"),
        "{messages}"
    );
    assert!(
        messages.contains("the final `to` must end at a terminal"),
        "{messages}"
    );
    assert!(
        messages.contains("`from` must name a terminal"),
        "{messages}"
    );
}

#[test]
fn a_derived_quantity_becomes_the_placed_part_value() {
    let fx = Fixture::new(
        "derived-value",
        &[
            ("stackup/passives.kdl", PASSIVES),
            (
                "board.kdl",
                r#"
use "@stackup/passives"
block load {
    derive resistance "e96(937Ω)"
    place resistor R value=resistance
}
design board {
    stock { packages imperial="0603" }
    place load load
}
"#,
            ),
        ],
    );
    let model = fx.model("board.kdl", "board");
    assert!(model.report.is_empty(), "{}", model.report.render());
    let resistor = model.parts().find(|(_, i)| i.path == "load/R").unwrap().1;
    assert_eq!(resistor.value.as_deref(), Some("931Ω"));
}

#[test]
fn static_derives_can_choose_a_feature_and_a_part_value() {
    let fx = Fixture::new(
        "static-choice",
        &[
            ("stackup/passives.kdl", PASSIVES),
            (
                "board.kdl",
                r#"
use "@stackup/passives"
block bank {
    param rated current
    derive resistance "e96(680Ω * rated / 1A)"
    block extra {
        when "resistance > 1kΩ"
        place resistor R value=resistance
    }
}

design board {
    stock { packages imperial="0603" }
    place bank small rated="1A"
    place bank large rated="2A"
}
"#,
            ),
        ],
    );
    let model = fx.model("board.kdl", "board");
    assert!(model.report.is_empty(), "{}", model.report.render());
    let parts: Vec<_> = model
        .parts()
        .map(|(_, i)| (i.path.as_str(), i.value.as_deref()))
        .collect();
    assert_eq!(parts, [("large/extra/R", Some("1.37kΩ"))]);
}

#[test]
fn a_part_parameter_selects_a_declaration_from_the_callers_scope() {
    let fx = Fixture::new(
        "part-parameter",
        &[
            (
                "stackup/stage.kdl",
                r#"
part default-fet {
    symbol "Lib:DEFAULT"
    reference Q
    pin G input
    pin S passive
    pin D passive
    package sot23 footprint="DEFAULT" { pad G 1; pad S 2; pad D 3 }
}
block stage {
    param fet part default=default-fet
    port gate
    place fet Q
    circuit gate Q.G
}
block forwarded {
    param chosen part
    place stage inner fet=chosen
}
"#,
            ),
            (
                "stackup/alternate.kdl",
                r#"
part alternate-fet {
    symbol "Lib:ALTERNATE"
    reference Q
    pin G input
    pin S passive
    pin D passive
    package sot23 footprint="ALTERNATE" { pad G 1; pad S 2; pad D 3 }
}
part incompatible {
    symbol "Lib:INCOMPATIBLE"
    reference Q
    pin X passive
    package p footprint="INCOMPATIBLE" { pad X 1 }
}
"#,
            ),
            (
                "board.kdl",
                r#"
use "@stackup/stage"
use "@stackup/alternate"
design board {
    place stage normal
    place stage custom fet=alternate-fet
    place forwarded nested chosen=alternate-fet
}
design invalid {
    place stage wrong fet=forwarded
    place stage missing fet=unknown-fet
    place stage quoted fet="alternate-fet"
}
design wrong-interface {
    place stage wrong fet=incompatible
}
"#,
            ),
        ],
    );
    let model = fx.model("board.kdl", "board");
    assert!(model.report.is_empty(), "{}", model.report.render());
    let part = |path: &str| {
        let (_, instance) = model.parts().find(|(_, i)| i.path == path).unwrap();
        let stackup::model::Kind::Part { decl, .. } = instance.kind else {
            unreachable!()
        };
        decl.file
    };
    assert_ne!(part("normal/Q"), part("custom/Q"));
    assert_eq!(part("custom/Q"), part("nested/inner/Q"));

    let model = fx.model("board.kdl", "invalid");
    let messages = model.report.messages();
    assert!(
        messages.iter().any(|m| m == "`forwarded` is not a part"),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m == "`unknown-fet` is not a part in scope"),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m == "a `part` parameter takes a bare part name"),
        "{messages:?}"
    );

    let model = fx.model("board.kdl", "wrong-interface");
    assert!(
        model
            .report
            .messages()
            .iter()
            .any(|m| m.contains("wrong/Q has no pin or port `G`")),
        "{}",
        model.report.render()
    );
}

#[test]
fn propagated_facts_cannot_choose_components() {
    let fx = Fixture::new(
        "dynamic-choice",
        &[
            ("stackup/passives.kdl", PASSIVES),
            ("stackup/connectors.kdl", CONNECTORS),
            (
                "board.kdl",
                r#"
use "@stackup/passives"
use "@stackup/connectors"
block bad {
    port rail type=power
    derive resistance "rail.voltage / 1mA"
    place resistor R value=resistance
    block extra {
        when "rail.voltage > 3V"
        place capacitor C value="100nF"
    }
}
design board {
    stock { packages imperial="0603" }
    place power-jack supply voltage="5V"
    place bad load rail=supply.out
}
"#,
            ),
        ],
    );
    let model = fx.model("board.kdl", "board");
    let messages = model.report.messages();
    assert_eq!(
        messages
            .iter()
            .filter(|m| m.contains("reads a propagated fact"))
            .count(),
        2,
        "{messages:?}"
    );
}

#[test]
fn a_feature_does_not_repeat_its_parent_assertions() {
    let fx = Fixture::new(
        "feature-assertions",
        &[(
            "board.kdl",
            r#"
block checked {
    param rating current
    assert "rating <= 1A" message="rating exceeds 1A"
    block extra {
        when "rating > 1A"
    }
}

design board {
    place checked c rating="2A"
}
"#,
        )],
    );
    let model = fx.model("board.kdl", "board");
    assert_eq!(
        model
            .report
            .messages()
            .iter()
            .filter(|m| m.contains("rating exceeds 1A"))
            .count(),
        1
    );
}

#[test]
fn placement_acknowledges_only_the_selected_requirement() {
    let fx = Fixture::new(
        "ignore-requirement",
        &[(
            "board.kdl",
            r#"
part led {
    pin V power_in
    port vdd type=terminal pin=V {
        require net.voltage min="3.7V"
    }
}
design board {
    place led status { ignore "vdd.net.voltage" reason="bench verified" }
    place led other
    circuit "status.V" "other.V"
    set "status.V" net.voltage "3.3V"
}
"#,
        )],
    );
    let model = fx.model("board.kdl", "board");
    assert!(
        model
            .report
            .messages()
            .iter()
            .any(|m| m.contains("acknowledged: bench verified")),
        "{}",
        model.report.render()
    );
    assert!(
        model
            .report
            .messages()
            .iter()
            .any(|m| m.contains("other") && m.contains("requires")),
        "{}",
        model.report.render()
    );
    assert!(model.report.has_errors());
}

#[test]
fn placement_ignore_matches_assertion_and_rejects_stale_selector() {
    let fx = Fixture::new(
        "ignore-assertion",
        &[(
            "board.kdl",
            r#"
block checked {
    param rating current
    assert "rating <= 1A" message="over rating"
    assert "rating <= 0.5A" message="other failure"
}
design board {
    place checked c rating="2A" {
        ignore "assert:rating <= 1A" reason="tested"
        ignore "assert:old expression" reason="stale"
    }
}
"#,
        )],
    );
    let model = fx.model("board.kdl", "board");
    let messages = model.report.messages();
    assert!(
        messages
            .iter()
            .any(|m| m == "over rating (acknowledged: tested)"),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m == "other failure"),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("matches no check")),
        "{messages:?}"
    );
    assert!(model.report.has_errors());
}

const PASSIVES: &str = r#"
part resistor {
    symbol "Device:R"
    reference R
    pin A passive
    pin B passive
    port a pin=A
    port b pin=B
    package chip { pad A 1; pad B 2 }
}
part capacitor {
    symbol "Device:C"
    reference C
    pin A passive
    pin B passive
    port a pin=A
    port b pin=B
    package chip { pad A 1; pad B 2 }
}
block decouple {
    port node
    port rail type=power
    place capacitor as=self intent=decouple
    circuit node self rail.gnd
}
block pull-up {
    port node {
        add net.pull "1 / value"
    }
    port rail type=power
    place resistor as=self intent=pull-up
    circuit node self rail.rail
}
"#;

const CONNECTORS: &str = r#"
part power-jack {
    symbol "Connector:Conn_01x02_Pin"
    reference J
    param voltage voltage
    pin P1 passive
    pin P2 passive
    package header footprint="PinHeader_1x02_P2.54mm_Vertical" { pad P1 1; pad P2 2 }
    port out type=power {
        set net.voltage voltage
        line rail pin=P1
        line gnd pin=P2
    }
}
"#;

#[test]
fn placement_port_arguments_resolve_later_placements() {
    let fx = Fixture::new(
        "forward-port-arguments",
        &[
            ("stackup/passives.kdl", PASSIVES),
            ("stackup/connectors.kdl", CONNECTORS),
            (
                "board.kdl",
                r#"
use "@stackup/passives"
use "@stackup/connectors"

part load {
    pin V power_in
    pin G power_in
    port supply type=power { line rail pin=V; line gnd pin=G }
}

design demo {
    stock { packages imperial="0603" }
    place pull-up Rpd node=Rs.b rail=V5.out value="10kΩ"
    place load U supply=V5.out
    place resistor Rs value="1kΩ"
    place power-jack V5 voltage="5V"
}
"#,
            ),
        ],
    );
    let model = fx.model("board.kdl", "demo");
    assert!(model.report.is_empty(), "{}", model.report.render());

    let pin = |path: &str, name: &str| {
        let inst = model.parts().find(|(_, i)| i.path == path).unwrap().0;
        Terminal::Pin {
            inst,
            pin: name.into(),
        }
    };
    assert!(model.nets.iter().any(|net| {
        [pin("Rpd", "A"), pin("Rs", "B")]
            .iter()
            .all(|terminal| net.members.contains(terminal))
    }));
    assert!(model.nets.iter().any(|net| {
        [pin("Rpd", "B"), pin("U", "V"), pin("V5", "P1")]
            .iter()
            .all(|terminal| net.members.contains(terminal))
    }));
}

#[test]
fn a_part_with_features_and_a_self_block() {
    let fx = Fixture::new(
        "features",
        &[
            ("stackup/passives.kdl", PASSIVES),
            ("stackup/connectors.kdl", CONNECTORS),
            (
                "board.kdl",
                r#"
use "@stackup/passives"
use "@stackup/connectors"

part chip {
    symbol "Lib:CHIP"
    reference U
    param mode strap default=a
    pin VCC power_in
    pin GND power_in
    pin EN input required
    pin OUT output
    pin SEL input
    package dip footprint="DIP-4" { pad VCC 1; pad GND 2; pad EN 3; pad OUT 4; pad SEL 5 }
    port vcc type=power {
        line rail pin=VCC
        line gnd pin=GND
    }
    port out type=output pin=OUT
    block supply {
        default
        place decouple Cvcc node=VCC rail=vcc value="100nF"
    }
    block always-on {
        default
        circuit EN vcc.rail
    }
    block strap-a {
        when "mode == a"
        circuit SEL vcc.gnd
    }
    block strap-b {
        when "mode == b"
        circuit SEL vcc.rail
    }
}

design demo {
    stock {
        packages imperial="0603"
    }
    place power-jack V5 voltage="5V"
    place chip u1 vcc=V5.out
    place chip u2 vcc=V5.out mode=b {
        without always-on
    }
    place pull-up Ren node=u2.EN rail=V5.out value="10kΩ"
    connect output from=u1.out to=u2.EN
}
"#,
            ),
        ],
    );
    let model = fx.model("board.kdl", "demo");
    assert!(model.report.is_empty(), "{}", model.report.render());

    let designators: Vec<(String, String)> = model
        .parts()
        .map(|(_, i)| (i.path.clone(), i.designator.clone().unwrap()))
        .collect();
    assert_eq!(
        designators,
        [
            ("V5".to_string(), "J1".to_string()),
            ("u1".to_string(), "U1".to_string()),
            ("u1/supply/Cvcc".to_string(), "C1".to_string()),
            ("u2".to_string(), "U2".to_string()),
            ("u2/supply/Cvcc".to_string(), "C2".to_string()),
            ("Ren".to_string(), "R1".to_string()),
        ]
    );
    // The self capacitor took the block's path and its value.
    let cvcc = model
        .parts()
        .find(|(_, i)| i.path == "u1/supply/Cvcc")
        .unwrap()
        .1;
    assert_eq!(cvcc.value.as_deref(), Some("100nF"));
    assert_eq!(cvcc.intent.as_deref(), Some("decouple"));

    let n = nets(&model);
    // u1: default strap on, SEL to ground by `when`. u2: strap off, SEL to the rail, EN pulled up.
    assert_eq!(
        n["V5/out.rail"],
        pins("J1.P1 U1.VCC C1.A U1.EN U2.VCC C2.A U2.SEL R1.B")
    );
    assert_eq!(
        n["V5/out.gnd"],
        pins("J1.P2 U1.GND C1.B U1.SEL U2.GND C2.B")
    );
    assert_eq!(n["Net-(U1-OUT)"], pins("U1.OUT U2.EN R1.A"));

    let (text, report) = netlist::kicad(
        &Library::load(
            &fx.dir.join("board.kdl"),
            &Prefixes::single("stackup", fx.dir.join("stackup")),
        )
        .0,
        &model,
    );
    assert!(report.is_empty(), "{}", report.render());
    assert!(
        text.contains("(footprint \"Capacitor_SMD:C_0603_1608Metric\")"),
        "{text}"
    );
    assert!(text.contains("(footprint \"DIP-4\")"));
    assert!(
        text.contains(
            "(property\n\t\t\t\t(name \"Stackup Path\")\n\t\t\t\t(value \"u1/supply/Cvcc\")"
        ),
        "{text}"
    );
}

#[test]
fn a_part_in_two_packages() {
    let fx = Fixture::new(
        "packages",
        &[
            ("stackup/passives.kdl", PASSIVES),
            ("stackup/connectors.kdl", CONNECTORS),
            (
                "board.kdl",
                r#"
use "@stackup/passives"
use "@stackup/connectors"

part chip {
    symbol "Lib:CHIP-S"
    reference U
    description "a chip"
    pin VCC power_in
    pin GND power_in
    pin OUT output
    pin VREF input
    package small footprint="SOT-23-5" { pad VCC 1; pad GND 2; pad OUT 3 }
    package big footprint="SOIC-8" {
        symbol "Lib:CHIP-B"
        description "the chip with its reference brought out"
        order mpn=CHIP-B
        pad VCC 1
        pad GND 4
        pad OUT 5
        pad VREF 8
    }
    port vcc type=power {
        line rail pin=VCC
        line gnd pin=GND
    }
    port out type=output pin=OUT
    port vref pin=VREF
    block supply {
        default
        place decouple Cvcc node=VCC rail=vcc value="100nF"
    }
    block reference {
        when "package == big"
        circuit VREF VCC
        place decouple Cref node=VREF@8 rail=vcc value="1µF"
    }
}

design demo {
    stock {
        packages imperial="0603"
    }
    place power-jack V5 voltage="5V"
    place chip u1 vcc=V5.out
    place chip u2 vcc=V5.out package=big
    place resistor R value="1k" intent=series
    circuit u2.vref R V5.out.gnd
}

design wrong {
    place power-jack V5 voltage="5V"
    place chip u1 vcc=V5.out
    place chip u2 vcc=V5.out package=big
    place chip u3 vcc=V5.out package=huge
    circuit u1.VREF V5.out.rail
    circuit u2.VREF@9 V5.out.rail
    nc u1.vref
}
"#,
            ),
        ],
    );
    let model = fx.model("board.kdl", "demo");
    assert!(model.report.is_empty(), "{}", model.report.render());

    // The body decides the pins, so the `when` block lands on one placement only.
    let paths: Vec<String> = model.parts().map(|(_, i)| i.path.clone()).collect();
    assert_eq!(
        paths,
        [
            "V5",
            "u1",
            "u1/supply/Cvcc",
            "u2",
            "u2/supply/Cvcc",
            "u2/reference/Cref",
            "R",
        ]
    );
    let n = nets(&model);
    assert_eq!(
        n["V5/out.rail"],
        pins("J1.P1 U1.VCC C1.A U2.VCC C2.A U2.VREF C3.A R1.A")
    );

    // The netlist reads every field for the body the placement chose.
    let (text, report) = netlist::kicad(
        &Library::load(
            &fx.dir.join("board.kdl"),
            &Prefixes::single("stackup", fx.dir.join("stackup")),
        )
        .0,
        &model,
    );
    assert!(report.is_empty(), "{}", report.render());
    let u1 = text.find("(ref \"U1\")").unwrap();
    let u2 = text.find("(ref \"U2\")").unwrap();
    let u1 = &text[u1..u2];
    let u2 = &text[u2..];
    assert!(
        u1.contains("(value \"CHIP-S\")") && u1.contains("(footprint \"SOT-23-5\")"),
        "{u1}"
    );
    assert!(u1.contains("(part \"CHIP-S\")") && u1.contains("(description \"a chip\")"));
    assert!(
        u2.contains("(value \"CHIP-B\")") && u2.contains("(footprint \"SOIC-8\")"),
        "{u2}"
    );
    assert!(u2.contains("(part \"CHIP-B\")"));
    assert!(u2.contains("(description \"the chip with its reference brought out\")"));

    // What a wrong body, an unbonded pin and a wrong pad are told.
    let model = fx.model("board.kdl", "wrong");
    let messages: Vec<String> = model
        .report
        .findings
        .iter()
        .map(|f| f.diagnostic.message.clone())
        .collect();
    let expect = [
        "`chip` has no package `huge`; it comes in `small`, `big`",
        "`u1.VREF`: `VREF` is not bonded in `small`, the package `u1` is placed in; it is in `big`",
        "`u2.VREF@9`: pin `VREF` has no pad `9` in `big`; its pads are 8",
        "u1 has no pin or port `vref`",
    ];
    for e in expect {
        assert!(
            messages.iter().any(|m| m.contains(e)),
            "missing {e:?} in {messages:#?}"
        );
    }
    assert_eq!(messages.len(), expect.len(), "{messages:#?}");
}

#[test]
fn a_uart_crosses_over_and_a_bus_chains() {
    let fx = Fixture::new(
        "links",
        &[
            ("stackup/passives.kdl", PASSIVES),
            (
                "board.kdl",
                r#"
use "@stackup/passives"

part mcu {
    symbol "Lib:MCU"
    reference U
    pin PA0 bidirectional once
    pin PA1 bidirectional once
    pin PA2 bidirectional once
    pin PB6 bidirectional once
    pin PB7 bidirectional once
    package p footprint="QFN" { pad PA0 1; pad PA1 2; pad PA2 3; pad PB6 4; pad PB7 5 }
    peripheral USART1 uart {
        has lin
        tx PA0 PB6
        rx PA1 PB7
    }
    peripheral I2C1 i2c {
        sda PB7
        scl PB6
    }
    peripheral GPIO gpio {
        io PA0 PA1 PA2 PB6 PB7
    }
}

part xcvr {
    symbol "Lib:XCVR"
    reference U
    pin RXD output
    pin TXD input
    package p footprint="SOIC" { pad RXD 1; pad TXD 2 }
    port uart type=uart {
        line tx pin=RXD
        line rx pin=TXD
    }
}

part sensor {
    symbol "Lib:SENSOR"
    reference U
    pin SDA bidirectional
    pin SCL input
    package p footprint="SOT" { pad SDA 1; pad SCL 2 }
    port i2c type=i2c {
        line sda pin=SDA
        line scl pin=SCL
    }
}

block node {
    port link type=uart
    place xcvr sbc
    circuit link.tx sbc.uart.tx
    circuit link.rx sbc.uart.rx
}

design demo {
    place mcu mcu
    place node node
    place sensor s1
    place sensor s2
    connect uart {
        from mcu tx=PA0 rx=PA1
        to node
    }
    connect i2c {
        from mcu sda=PB7 scl=PB6
        to s1
    }
    connect i2c from=s1 to=s2
    connect output {
        from mcu.PA2
        to s2.SCL
    }
}
"#,
            ),
        ],
    );
    let model = fx.model("board.kdl", "demo");
    assert!(model.report.is_empty(), "{}", model.report.render());
    let n = nets(&model);
    // The MCU's tx lands on the transceiver's TXD, through the block's re-exported port.
    assert_eq!(n["node/link.rx"], pins("U1.PA0 U2.TXD"));
    assert_eq!(n["node/link.tx"], pins("U1.PA1 U2.RXD"));
    // A bus chained from a device already on it.
    assert_eq!(n["s1/i2c.sda"], pins("U1.PB7 U3.SDA U4.SDA"));
    assert_eq!(n["s1/i2c.scl"], pins("U1.PB6 U3.SCL U4.SCL U1.PA2"));
}

#[test]
fn what_elaboration_reports() {
    let fx = Fixture::new(
        "findings",
        &[
            ("stackup/passives.kdl", PASSIVES),
            (
                "board.kdl",
                r#"
use "@stackup/passives"

part mcu {
    symbol "Lib:MCU"
    reference U
    pin PA0 bidirectional once
    pin PA1 bidirectional once
    package p footprint="QFN" { pad PA0 1; pad PA1 2 }
    peripheral I2C1 i2c {
        sda PA0
        scl PA1
    }
}

design demo {
    place mcu mcu
    place mcu other
    place resistor R value="1k"
    place nothing x
    place resistor R2 colour=red
    circuit mcu.PA9 R
    connect i2c {
        from mcu sda=PA1 scl=PA0
        to other
    }
    connect i2c from=mcu to=other
    nc mcu.PA1
    circuit mcu.PA1 R.a
}
"#,
            ),
        ],
    );
    let model = fx.model("board.kdl", "demo");
    let messages: Vec<String> = model
        .report
        .findings
        .iter()
        .map(|f| f.diagnostic.message.clone())
        .collect();
    let expect = [
        "`nothing` is not a part or block in scope",
        "`resistor` has no parameter or port `colour`",
        "`mcu.PA9`: mcu has no pin or port `PA9`",
        "`PA1` cannot carry `sda` of `i2c`: no i2c instance of `mcu` puts that line on it",
        "`PA0` cannot carry `scl` of `i2c`: no i2c instance of `mcu` puts that line on it",
        "`other` has no port of type `i2c`; its ports are none",
        "`mcu` has no port of type `i2c`; its ports are none",
        "`mcu.PA1` is declared no-connect but is joined to something",
    ];
    for e in expect {
        assert!(
            messages.iter().any(|m| m.contains(e)),
            "missing {e:?} in {messages:#?}"
        );
    }
}

#[test]
fn a_name_is_a_net_fact() {
    let fx = Fixture::new(
        "names",
        &[
            ("stackup/passives.kdl", PASSIVES),
            (
                "board.kdl",
                r#"
use "@stackup/passives"

part jack {
    symbol "Lib:JACK"
    reference J
    pin P1 passive
    pin P2 passive
    package p footprint="H" { pad P1 1; pad P2 2 }
    port out type=power {
        set net.voltage "5V"
        line rail pin=P1 { set net.name "VIN" }
        line gnd pin=P2 { set net.name "GND" }
    }
}

block sheet {
    port rail type=power {
        line gnd { set net.name "RETURN" }
    }
    place resistor R value="1k"
    circuit rail.rail R rail.gnd
}

design demo {
    place jack J
    place sheet s rail=J.out
    place resistor R2 value="2k"
    place resistor R3 value="2k"
    circuit J.out.rail R2 R3 J.out.gnd
    set R2.b net.name "tap"
    circuit J.out.rail R2 R3 J.out.gnd name="feed"
}
"#,
            ),
        ],
    );
    let model = fx.model("board.kdl", "demo");
    let messages: Vec<String> = model
        .report
        .findings
        .iter()
        .map(|f| f.diagnostic.message.clone())
        .collect();
    // The jack's return and the sheet's are one net with two stated names.
    assert!(
        messages
            .iter()
            .any(|m| m == "one net is named `J/GND` and `s/RETURN`; a name is set once"),
        "{messages:#?}"
    );
    // A circuit through a series element names the net its path starts on — here the jack's
    // rail, which the jack already named, so the second name is a conflict.
    assert!(
        messages.iter().any(|m| m.starts_with("one net is named")
            && m.contains("`J/VIN`")
            && m.contains("`feed`")),
        "{messages:#?}"
    );
    let n = nets(&model);
    assert_eq!(n["J/VIN"], pins("J1.P1 R1.A R2.A"));
    assert_eq!(n["tap"], pins("R2.B R3.A"));
    // The tap between R2 and the return — a `set` on a terminal named it.
    assert!(n.contains_key("tap"), "{:?}", n.keys());
}

// --- The sketches -------------------------------------------------------------------------------

const CHECKED: &str = r#"
use "@stackup/passives"
use "@stackup/connectors"

part reg {
    symbol "Lib:REG"
    reference U
    pin IN power_in
    pin GND power_in
    pin OUT power_out
    package p footprint="SOT-23-3" { pad IN 1; pad GND 2; pad OUT 3 }
    port in type=power {
        require net.voltage min="4.5V" max="6V"
        add net.draw "out.draw + 1mA"
        line rail pin=IN
        line gnd pin=GND
    }
    port out type=power {
        set net.voltage "3.3V" tolerance="2%"
        assert "draw <= 150mA" message="the reg gives 150mA; its loads draw {draw}"
        line rail pin=OUT
        line gnd pin=GND
    }
}

part sensor {
    symbol "Lib:SENSOR"
    reference U
    param draw current default="60mA"
    param add0 strap default=gnd
    pin VCC power_in
    pin GND power_in
    pin SDA bidirectional
    pin SCL input
    pin BOOT bidirectional once {
        role strap sampled=reset pull=internal-down note="high is a test mode"
        require net.rest not=high
    }
    package p footprint="SOT-23-6" { pad VCC 1; pad GND 2; pad SDA 3; pad SCL 4; pad BOOT 5 }
    derive address "match(add0, gnd: 0x48, vplus: 0x49)"
    port vcc type=power {
        require net.voltage min="2V" max="3.6V"
        add net.draw draw
        line rail pin=VCC
        line gnd pin=GND
    }
    port i2c type=i2c {
        claim signal.address address
        line sda pin=SDA
        line scl pin=SCL
    }
}

// A part that draws but does not say so.
part mcu {
    symbol "Lib:MCU"
    reference U
    pin VDD power_in
    pin VSS power_in
    package p footprint="QFN" { pad VDD 1; pad VSS 2 }
    port vdd type=power {
        require net.voltage min="2V" max="3.6V"
        line rail pin=VDD
        line gnd pin=VSS
    }
}

// Two ports that each draw what the other does.
part knot {
    symbol "Lib:KNOT"
    reference U
    pin A power_in
    pin B power_in
    pin G power_in
    package p footprint="SOT" { pad A 1; pad B 2; pad G 3 }
    port a type=power {
        add net.draw "b.draw"
        line rail pin=A
        line gnd pin=G
    }
    port b type=power {
        add net.draw "a.draw"
        line rail pin=B
        line gnd pin=G
    }
}

block indicator {
    param r resistance default="680Ω"
    param forward voltage default="2.0V"
    port rail type=power {
        add net.draw current
    }
    derive current "(rail.voltage.max - forward) / r"
    assert "current <= 3mA" message="{r} lets {current} through at {rail.voltage.max}"
    place resistor R value=r intent=series note="LED limit — {current} at {rail.voltage.max}"
    circuit rail.rail R rail.gnd
}

design fine {
    stock {
        packages imperial="0603"
    }
    place power-jack V5 voltage="5V"
    place reg reg in=V5.out
    place sensor a vcc=reg.out
    place sensor b vcc=reg.out add0=vplus
    connect i2c from=a to=b
    text budget "the pilot off {reg.out.voltage.max}, with {reg.out.draw} already drawn"
    place indicator led rail=reg.out note=budget
}

design wrong {
    stock {
        packages imperial="0603"
    }
    place power-jack V12 voltage="12V"
    place power-jack V3 voltage="3.0V"
    place reg reg in=V12.out
    circuit V3.out.rail reg.out.rail
    circuit V3.out.gnd reg.out.gnd
    place sensor a vcc=reg.out
    place sensor b vcc=reg.out
    place sensor c vcc=reg.out draw="70mA"
    connect i2c from=a to=b
    connect i2c from=b to=c
    place pull-up Rboot node=c.BOOT rail=reg.out value="10kΩ"
    place indicator led rail=V12.out r="1kΩ"
    place knot k a=V12.out b=V12.out
}

design partial {
    stock {
        packages imperial="0603"
    }
    place power-jack V5 voltage="5V"
    place reg reg in=V5.out
    place sensor a vcc=reg.out
    place mcu u vdd=reg.out
}
"#;

#[test]
fn the_checks_judge_facts_assertions_and_requirements() {
    let fx = Fixture::new(
        "checks",
        &[
            ("stackup/passives.kdl", PASSIVES),
            ("stackup/connectors.kdl", CONNECTORS),
            ("board.kdl", CHECKED),
        ],
    );

    // Everything holds, and the notes carry the values that were computed.
    let model = fx.model("board.kdl", "fine");
    assert!(model.report.is_empty(), "{}", model.report.render());
    let r = model.parts().find(|(_, i)| i.path == "led/R").unwrap().1;
    assert_eq!(r.value.as_deref(), Some("680Ω"));
    assert_eq!(r.notes, ["LED limit — 2.01mA at 3.37V"]);
    let led = model.instances.iter().find(|i| i.path == "led").unwrap();
    assert_eq!(led.notes, ["the pilot off 3.37V, with 122mA already drawn"]);

    // Every kind of failure, each pointing at what it read.
    let model = fx.model("board.kdl", "wrong");
    let messages = model.report.messages();
    let expect = [
        "`reg`.in requires `voltage` min=4.5V max=6V, and its net is 12V",
        "`voltage` is set to 3V by `V3`.out and to 3.23V–3.37V by `reg`.out on one net; a fact is set once",
        "the reg gives 150mA; its loads draw 190mA",
        "`address` 72 is claimed by `a`.i2c and by `b`.i2c on one net; a claim is distinct",
        "`address` 72 is claimed by `b`.i2c and by `c`.i2c on one net; a claim is distinct",
        "`c`.BOOT requires `rest` not=high, and its net is high",
        "1kΩ lets 10mA through at 12V",
        "facts depend on one another in a loop: `draw` on",
    ];
    for e in expect {
        assert!(
            messages.iter().any(|m| m.contains(e)),
            "missing {e:?} in:\n{}",
            model.report.render()
        );
    }
    // The assertion's failure names the three loads it summed.
    let f = model
        .report
        .findings
        .iter()
        .find(|f| f.diagnostic.message.starts_with("the reg gives"))
        .unwrap();
    let related: Vec<&str> = f
        .related
        .iter()
        .map(|r| r.diagnostic.message.as_str())
        .collect();
    assert_eq!(
        related,
        [
            "`a`.vcc adds `draw` by 60mA",
            "`b`.vcc adds `draw` by 60mA",
            "`c`.vcc adds `draw` by 70mA",
        ]
    );
    // The requirement's failure points at the supply that set the voltage.
    let f = model
        .report
        .findings
        .iter()
        .find(|f| f.diagnostic.message.starts_with("`reg`.in requires"))
        .unwrap();
    assert_eq!(
        f.related[0].diagnostic.message,
        "`V12`.out sets `voltage` to 12V"
    );
    assert!(f.related[0].source.name.ends_with("connectors.kdl"));

    // A load that states nothing makes the sum a lower bound: a note, not a pass.
    let model = fx.model("board.kdl", "partial");
    assert!(!model.report.has_errors(), "{}", model.report.render());
    let messages = model.report.messages();
    assert!(
        messages
            .iter()
            .any(|m| m == "holds for what is stated on `reg`.out: `u`.vdd states no draw"),
        "{messages:#?}"
    );
    assert_eq!(model.report.findings.len(), 1, "{}", model.report.render());
}

#[test]
fn an_isolated_converter_transfers_draw_without_joining_returns() {
    let fx = Fixture::new(
        "isolated-transfer",
        &[
            ("stackup/connectors.kdl", CONNECTORS),
            (
                "board.kdl",
                r#"
use "@stackup/connectors"
part converter {
    symbol "Lib:CONVERTER"
    reference U
    pin PVIN power_in
    pin NVIN power_in
    pin PVOUT power_out
    pin NVOUT power_out
    package p footprint="DIP-4" { pad PVIN 1; pad NVIN 2; pad PVOUT 3; pad NVOUT 4 }
    derive input "out.draw * out.voltage / (0.88 * in.voltage.min)"
    port in type=power {
        require net.voltage min="18V" max="36V"
        add net.draw input
        line rail pin=PVIN
        line gnd pin=NVIN
    }
    port out type=power {
        set net.voltage "24V"
        assert "draw <= 840mA" message="output draws {draw}; limit is 840mA"
        line rail pin=PVOUT
        line gnd pin=NVOUT
    }
}
block external-load {
    param current current
    port supply type=power { add net.draw current }
}
design normal {
    place power-jack source voltage="24V"
    place converter iso in=source.out
    place external-load load supply=iso.out current="500mA"
    assert "iso.in.draw > 568mA && iso.in.draw < 569mA"
}
design overloaded {
    place power-jack source voltage="24V"
    place converter iso in=source.out
    place external-load load supply=iso.out current="900mA"
}
"#,
            ),
        ],
    );

    let model = fx.model("board.kdl", "normal");
    assert!(model.report.is_empty(), "{}", model.report.render());
    let converter = model
        .instances
        .iter()
        .position(|i| i.path == "iso")
        .unwrap();
    let net_for = |pin: &str| {
        model
            .nets
            .iter()
            .position(|n| {
                n.members.contains(&Terminal::Pin {
                    inst: converter,
                    pin: pin.to_string(),
                })
            })
            .unwrap()
    };
    let four_nets = ["PVIN", "NVIN", "PVOUT", "NVOUT"].map(net_for);
    assert_eq!(
        four_nets
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        4
    );

    let model = fx.model("board.kdl", "overloaded");
    assert!(
        model
            .report
            .messages()
            .iter()
            .any(|m| m == "output draws 900mA; limit is 840mA"),
        "{}",
        model.report.render()
    );
}

/// The arc-kdl tree — arc's boards in this format — when it is beside this repository or named
/// by `STACKUP_ARC_KDL`. The one real project this suite can hold the loader to end to end: its
/// `manifest.kdl` and `manifest.local.kdl` are read for real, not built in a fixture.
fn arc_kdl() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("STACKUP_ARC_KDL") {
        return Some(PathBuf::from(dir));
    }
    let mut dir = Path::new(env!("CARGO_MANIFEST_DIR")).parent();
    while let Some(d) = dir {
        let candidate = d.join("arc-kdl");
        if candidate.join("manifest.kdl").is_file() {
            return Some(candidate);
        }
        dir = d.parent();
    }
    None
}

#[test]
fn the_trk_led_board_is_wired_as_arc_wired_it() {
    let Some(dir) = arc_kdl() else {
        eprintln!("no arc-kdl tree; skipping");
        return;
    };
    let board = dir.join("trackside/trk-led-01/board.kdl");
    let (prefixes, report) = Prefixes::resolve(&board, Options::default());
    assert!(report.is_empty(), "{}", report.render());
    let (lib, report) = Library::load(&board, &prefixes);
    assert!(report.is_empty(), "{}", report.render());
    let (f, block) = lib
        .designs()
        .into_iter()
        .find(|(_, d)| d.name == "trk-led-01")
        .unwrap();
    let model = elaborate(&lib, f, block);
    let n = nets(&model);
    // The LIN link crosses over into the SBC.
    assert_eq!(n["node/link.rx"], pins("STM32.PB6 XLIN.TxD"));
    assert_eq!(n["node/link.tx"], pins("STM32.PB7 XLIN.RxD"));
    // Stated names: the node's rails and the bus conductor.
    assert!(n["node/VPWR"].contains(&"XLIN.VS".to_string()));
    assert!(n["node/V5"].contains(&"PXL.1".to_string()));
    assert_eq!(n["node/LIN"], pins("C3.A XLIN.LIN TRK1.LIN TRK2.LIN LIN.1"));
    // One return for every rail on the board.
    let gnd = &n["node/GND"];
    for pin in [
        "TRK1.GND",
        "XLIN.GND",
        "STM32.VSS",
        "SWD.GND",
        "PXL.3",
        "U2.GND",
        "I2C.1",
        "I2C.MP",
        "GND.1",
    ] {
        assert!(
            gnd.contains(&pin.to_string()),
            "{pin} not on the return: {gnd:?}"
        );
    }
    // Reset: the SBC's flag, the filter, PF2 and the header.
    assert_eq!(n["nrst"], pins("R1.A XLIN.RSTN C9.A STM32.PF2 SWD.RESET"));
    // A head's aspect: gate, pull-down, switch; drain to ballast; ballast to the header.
    assert_eq!(n["head3/gates.l2"], pins("R17.A Q8.G STM32.PA6"));
    assert_eq!(n["head3/K2"], pins("R18.B LED3.3"));
    // The pixel header's data line is named where the lead lands.
    assert_eq!(n["pixels/DOUT"], pins("R2.B PXL.2"));
}

#[test]
fn a_block_with_features() {
    // A block's child blocks are its features, exactly as a part's are: on by default, asked
    // for with `with`, switched off with `without`, or following a `when` over the block's
    // parameters and features. A child sees the block's ports and placements by bare name.
    let fx = Fixture::new(
        "block-features",
        &[
            ("stackup/passives.kdl", PASSIVES),
            ("stackup/connectors.kdl", CONNECTORS),
            (
                "board.kdl",
                r#"
use "@stackup/passives"
use "@stackup/connectors"

block feed {
    param mode strap default=plain
    port rail type=power
    port out
    place resistor Rs value="10Ω" intent=series
    circuit rail.rail Rs out
    block bleed {
        default
        place pull-up Rb node=out rail=rail value="100kΩ"
    }
    block pilot {
        place decouple Cp node=out rail=rail value="100nF"
    }
    block clamp {
        when "mode == a && !pilot"
        place pull-up Ra node=Rs.b rail=rail value="4.7kΩ"
    }
    block clamp-lit {
        when "mode == a && pilot"
        place decouple Ca node=Rs.b rail=rail value="47nF"
    }
}

design demo {
    stock {
        packages imperial="0603"
    }
    place power-jack V5 voltage="5V"
    place feed f1 rail=V5.out
    place feed f2 rail=V5.out mode=a {
        without bleed
        with pilot
    }
}
"#,
            ),
        ],
    );
    let model = fx.model("board.kdl", "demo");
    assert!(model.report.is_empty(), "{}", model.report.render());

    let designators: Vec<(String, String)> = model
        .parts()
        .map(|(_, i)| (i.path.clone(), i.designator.clone().unwrap()))
        .collect();
    assert_eq!(
        designators,
        [
            ("V5".to_string(), "J1".to_string()),
            ("f1/Rs".to_string(), "R1".to_string()),
            ("f1/bleed/Rb".to_string(), "R2".to_string()),
            ("f2/Rs".to_string(), "R3".to_string()),
            ("f2/pilot/Cp".to_string(), "C1".to_string()),
            ("f2/clamp-lit/Ca".to_string(), "C2".to_string()),
        ]
    );
    // Looked up by a member rather than by name: a net with a block's port line on it is
    // projected under that line, and which line is not what this test is about.
    let n = nets(&model);
    let with = |pin: &str| n.values().find(|m| m.iter().any(|p| p == pin)).unwrap();
    assert_eq!(with("J1.P1"), &pins("J1.P1 R1.A R2.B R3.A"));
    assert_eq!(with("J1.P2"), &pins("J1.P2 C1.B C2.B"));

    // A feature a block does not have, and a `when` block asked for by name, are both errors.
    let fx = Fixture::new(
        "block-features-errors",
        &[
            ("stackup/passives.kdl", PASSIVES),
            ("stackup/connectors.kdl", CONNECTORS),
            (
                "board.kdl",
                r#"
use "@stackup/passives"
use "@stackup/connectors"

block feed {
    port rail type=power
    block bleed {
        when "true"
        place pull-up Rb node=rail.rail rail=rail value="100kΩ"
    }
}

design demo {
    place power-jack V5 voltage="5V"
    place feed f1 rail=V5.out {
        with sparkle
        without bleed
    }
}
"#,
            ),
        ],
    );
    let model = fx.model("board.kdl", "demo");
    let text = model.report.render();
    assert!(text.contains("`feed` has no feature `sparkle`"), "{text}");
    assert!(
        text.contains("`bleed` is placed by its `when` condition, not by name"),
        "{text}"
    );
}

#[test]
fn peripheral_requirements_are_conditional_and_connection_local() {
    let base = r#"
type i2c { line scl; line sda }
part mcu {
    reference U
    pin PB6 bidirectional
    pin PB7 bidirectional
    pin PB10 bidirectional
    pin PB11 bidirectional
    pin PB13 bidirectional
    pin PB14 bidirectional
    package large footprint="Test:MCU" { pad PB6 1; pad PB7 2; pad PB10 3; pad PB11 4; pad PB13 5; pad PB14 6 }
    package small footprint="Test:MCU" { pad PB6 1; pad PB7 2; pad PB10 3; pad PB11 4; pad PB13 5; pad PB14 6 }
    peripheral I2C1 i2c {
        scl PB6
        sda PB7
        has bootloader when="scl == PB6 && sda == PB7"
    }
    peripheral I2C2 i2c {
        scl PB10 PB13
        sda PB11 PB14
        has bootloader when="package == large && scl == PB10 && sda == PB11"
        has dma
    }
    peripheral ALT i2c {
        scl PB10 PB13
        sda PB11 PB14
        has special
    }
}
part connector {
    reference J
    pin C passive
    pin D passive
    package header footprint="Test:Header" { pad C 1; pad D 2 }
    port bus type=i2c { line scl pin=C; line sda pin=D }
}
design demo {
    place mcu cpu package=PACKAGE
    place connector monitor
    place connector expansion
    connect i2c { from cpu scl=PB6 sda=PB7; to monitor; require peripheral.bootloader }
    connect i2c { from cpu scl=CLOCK sda=DATA; to expansion; REQUIRE }
}
"#;
    for (label, package, clock, data, requirement, error) in [
        (
            "match",
            "large",
            "PB10",
            "PB11",
            "require peripheral.bootloader",
            false,
        ),
        (
            "qwiic",
            "large",
            "PB13",
            "PB14",
            "require peripheral.bootloader",
            true,
        ),
        (
            "package",
            "small",
            "PB10",
            "PB11",
            "require peripheral.bootloader",
            true,
        ),
        (
            "mixed",
            "large",
            "PB10",
            "PB14",
            "require peripheral.bootloader",
            true,
        ),
        (
            "unconditional",
            "small",
            "PB13",
            "PB14",
            "require peripheral.dma",
            false,
        ),
        (
            "same-instance",
            "large",
            "PB10",
            "PB11",
            "require peripheral.bootloader; require peripheral.special",
            true,
        ),
        (
            "missing",
            "large",
            "PB10",
            "PB11",
            "require peripheral.missing",
            true,
        ),
        ("ordinary", "large", "PB13", "PB14", "", false),
    ] {
        let source = base
            .replace("PACKAGE", package)
            .replace("CLOCK", clock)
            .replace("DATA", data)
            .replace("REQUIRE", requirement);
        let fx = Fixture::new(&format!("peripheral-{label}"), &[("board.kdl", &source)]);
        let (lib, loaded) = Library::load(&fx.dir.join("board.kdl"), &Prefixes::default());
        assert!(loaded.is_empty(), "{}", loaded.render());
        let (file, design) = lib.designs()[0];
        let model = elaborate(&lib, file, design);
        assert_eq!(
            model.report.has_errors(),
            error,
            "{label}: {}",
            model.report.render()
        );
        if error {
            assert!(
                model.report.render().contains("no single peripheral"),
                "{}",
                model.report.render()
            );
        }
    }
    for (label, condition) in [
        ("typo", "scll == PB10"),
        ("pin-typo", "scl == PB99"),
        ("nonboolean", "42"),
        ("syntax", "scl =="),
    ] {
        let source = base
            .replace("PACKAGE", "large")
            .replace("CLOCK", "PB10")
            .replace("DATA", "PB11")
            .replace("REQUIRE", "require peripheral.bootloader")
            .replace("package == large && scl == PB10 && sda == PB11", condition);
        let fx = Fixture::new(&format!("peripheral-{label}"), &[("board.kdl", &source)]);
        let (lib, loaded) = Library::load(&fx.dir.join("board.kdl"), &Prefixes::default());
        assert!(loaded.is_empty(), "{}", loaded.render());
        let (file, design) = lib.designs()[0];
        assert!(elaborate(&lib, file, design).report.has_errors(), "{label}");
    }
}
