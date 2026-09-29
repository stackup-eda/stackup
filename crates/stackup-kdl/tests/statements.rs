//! Each statement of the format, read from a snippet; what a wrong one reports; and the edits.

use stackup_eda_parser::{
    Document, Value,
    ast::{BlockItem, BlockKind, MetaKey, PartItem, PortItem, Rule, SideRole},
    emit,
};

fn read(text: &str) -> stackup_eda_parser::ast::File {
    match Document::read("test.kdl", text) {
        Ok(f) => f,
        Err(e) => panic!("{}", e.render()),
    }
}

/// The messages a snippet produces.
fn errors(text: &str) -> Vec<String> {
    let doc = Document::parse("test.kdl", text).unwrap_or_else(|e| panic!("{}", e.render()));
    let (_, diags) = doc.file();
    diags.iter().map(|d| d.message.clone()).collect()
}

fn one_error(text: &str, expected: &str) {
    let errs = errors(text);
    assert_eq!(
        errs.len(),
        1,
        "expected one error containing {expected:?}, got {errs:#?}"
    );
    assert!(
        errs[0].contains(expected),
        "expected {expected:?} in {:?}",
        errs[0]
    );
}

#[test]
fn multiline_circuit_reads_and_writes() {
    let source = r#"design board {
    circuit {
        from supply.v
        to pullup decouple sense name="INPUT"
        to "mcu/gpio0" name="INPUT_SENSE"
    }
}
"#;
    let file = read(source);
    let circuit = file.designs().next().unwrap().circuits().next().unwrap();
    assert_eq!(circuit.from.as_ref().unwrap().to_string(), "supply.v");
    assert_eq!(circuit.legs.len(), 2);
    assert_eq!(circuit.legs[0].elements[2].to_string(), "sense");
    assert_eq!(
        circuit.legs[1].props[0].value,
        Value::String("INPUT_SENSE".into())
    );
    assert_eq!(emit(&file), source);
}

#[test]
fn multiline_circuit_requires_ordered_nonempty_lines() {
    let missing_from = errors("design d { circuit { to p } }");
    assert!(
        missing_from
            .iter()
            .any(|e| e.contains("`to` needs a preceding `from`"))
    );
    one_error(
        "design d { circuit { from p; to } }",
        "`to` needs at least one element",
    );
}

#[test]
fn a_part() {
    let file = read(
        r#"
part ne555 {
    symbol "Timer:LMC555xN"
    reference U
    manufacturer STMicroelectronics
    order mpn=STM32C011F6P6 mouser="511-STM32C011F6P6"
    param add0 strap default=gnd
    pin GND power_in
    pin RST input required symbol="~{RST}" {
        role reset active=low
        require net.rest not=low
    }
    pin PA11 bidirectional once { also PA9; require net.rest high }
    pin DISCH open_collector symbol-kind=input
    package dip8 footprint="DIP-8_W7.62mm" {
        pad GND 1
        pad VDD A1
    }
    package soic8 footprint="SOIC-8_3.9x4.9mm_P1.27mm" {
        symbol "Timer:LMC555xM"
        description "the same die, surface mount"
        order mpn=LMC555CMX
        pad GND 1
        pad VDD 8
    }
    port a pin=A
    port vcc type=power {
        require net.voltage min="1.5V" max="15V"
        add net.draw "100µA"
        set net.voltage voltage tolerance="2%"
        claim signal.address address
        assert "draw <= 500mA" message="too much"
        line rail pin=VCC
        line gnd pin=GND
    }
    port in type=pwm-pair require=signal.break
    peripheral TIM1 timer {
        has break
        ch1 PA0 PA5 { has dma }
        ch1n PA3 PA7
    }
    derive input "out.draw * out.voltage"
    text summary "555 astable {achieved}"
    block supply {
        default
        place decouple Cin node=VCC rail=vcc value="100nF"
    }
    block direct {
        when "!emc-class-b"
        circuit in.rail IN
    }
}
"#,
    );
    let part = file.parts().next().unwrap();
    assert_eq!(part.name, "ne555");
    assert_eq!(
        part.meta(MetaKey::Symbol),
        Some(&Value::String("Timer:LMC555xN".into()))
    );
    assert_eq!(
        part.meta(MetaKey::Reference),
        Some(&Value::Name("U".into()))
    );
    assert_eq!(
        part.meta(MetaKey::Manufacturer),
        Some(&Value::Name("STMicroelectronics".into()))
    );

    let param = part.params().next().unwrap();
    assert_eq!((param.name.as_str(), param.ty.as_str()), ("add0", "strap"));
    assert_eq!(param.default, Some(Value::Name("gnd".into())));

    let pins: Vec<_> = part.pins().collect();
    assert_eq!(pins.len(), 4);
    assert!(pins[1].required && !pins[1].once);
    assert_eq!(pins[1].symbol.as_deref(), Some("~{RST}"));
    assert_eq!(pins[1].roles[0].role, "reset");
    assert_eq!(pins[1].roles[0].props[0].value, Value::Name("low".into()));
    // What a pin asks of its own net: a level, or a level it must not be.
    let rest = &pins[1].requires[0];
    assert_eq!(
        (rest.aspect, rest.fact.as_str()),
        (stackup_eda_parser::ast::Aspect::Net, "rest")
    );
    assert_eq!(rest.value, None);
    assert_eq!(rest.props[0].key, "not");
    assert_eq!(rest.props[0].value, Value::Name("low".into()));
    assert!(pins[2].once);
    assert_eq!(pins[2].also[0].name, "PA9");
    assert_eq!(pins[2].requires[0].value, Some(Value::Name("high".into())));
    assert_eq!(pins[3].symbol_kind.as_deref(), Some("input"));

    let packages: Vec<_> = part.packages().collect();
    let package = packages[0];
    assert_eq!(package.footprint.as_deref(), Some("DIP-8_W7.62mm"));
    let pads: Vec<_> = package.pads().collect();
    assert_eq!(pads[0].label, "1");
    assert_eq!(pads[1].label, "A1");
    assert!(package.bonds("GND") && !package.bonds("PA11"));
    // What a body says for itself, and what it leaves to the part.
    let soic = packages[1];
    assert_eq!(
        soic.meta(MetaKey::Symbol),
        Some(&Value::String("Timer:LMC555xM".into()))
    );
    assert_eq!(soic.order().unwrap().props[0].key, "mpn");
    assert_eq!(
        part.meta_in(Some(soic), MetaKey::Symbol),
        Some(&Value::String("Timer:LMC555xM".into()))
    );
    assert_eq!(
        part.meta_in(Some(package), MetaKey::Symbol),
        Some(&Value::String("Timer:LMC555xN".into()))
    );
    assert_eq!(
        part.meta_in(Some(soic), MetaKey::Reference),
        Some(&Value::Name("U".into()))
    );
    assert_eq!(
        part.order_in(Some(soic)).unwrap().props[0].value,
        Value::Name("LMC555CMX".into())
    );
    assert_eq!(
        part.order_in(Some(package)).unwrap().props[0].value,
        Value::Name("STM32C011F6P6".into())
    );

    let ports: Vec<_> = part.ports().collect();
    assert_eq!(ports[0].pin.as_deref(), Some("A"));
    assert_eq!(ports[0].ty, None);
    let vcc = ports[1];
    assert_eq!(vcc.ty.as_deref(), Some("power"));
    assert_eq!(vcc.requires().next().unwrap().props.len(), 2);
    let facts: Vec<_> = vcc.facts().collect();
    assert_eq!(facts[0].rule, Rule::Add);
    assert_eq!(facts[0].value, Some(Value::String("100µA".into())));
    assert_eq!(facts[1].rule, Rule::Set);
    assert_eq!(facts[1].value, Some(Value::Name("voltage".into())));
    assert_eq!(facts[1].props[0].key, "tolerance");
    assert_eq!(facts[2].rule, Rule::Claim);
    assert!(
        matches!(&vcc.items[4], PortItem::Assert(a) if a.message.as_deref() == Some("too much"))
    );
    assert_eq!(
        vcc.lines()
            .map(|l| l.pin.clone().unwrap())
            .collect::<Vec<_>>(),
        ["VCC", "GND"]
    );
    // `require=break` on the port line is a `require` inside it.
    assert_eq!(ports[2].requires().next().unwrap().fact, "break");
    assert_eq!(
        ports[2].requires().next().unwrap().aspect,
        stackup_eda_parser::ast::Aspect::Signal
    );
    assert_eq!(facts[0].aspect, stackup_eda_parser::ast::Aspect::Net);

    let tim1 = part.peripherals().next().unwrap();
    assert_eq!(tim1.kind, "timer");
    assert_eq!(tim1.has[0].capability, "break");
    assert_eq!(tim1.lines[0].pins, ["PA0", "PA5"]);
    assert_eq!(tim1.lines[0].has[0].capability, "dma");
    assert!(tim1.lines[1].has.is_empty());

    assert!(matches!(&part.items[15], PartItem::Derive(d) if d.name == "input"));
    assert!(matches!(&part.items[16], PartItem::Text(t) if t.template.contains("{achieved}")));

    let blocks: Vec<_> = part.blocks().collect();
    assert!(blocks[0].is_default());
    assert_eq!(blocks[1].when().unwrap().expr, "!emc-class-b");
    let place = blocks[0].places().next().unwrap();
    assert_eq!(
        (place.what.as_str(), place.name.as_deref()),
        ("decouple", Some("Cin"))
    );
    assert_eq!(place.arg("node"), Some(&Value::Name("VCC".into())));
    assert_eq!(place.arg("value"), Some(&Value::String("100nF".into())));
    let circuit = blocks[1].circuits().next().unwrap();
    assert_eq!(circuit.elements[0].to_string(), "in.rail");
    assert_eq!(circuit.elements[0].members, ["rail"]);
}

#[test]
fn a_design() {
    let file = read(
        r#"
use "@stackup/passives"
use "./ne555.kdl"

type leds {
    line red type=pwm { require signal.dma }
    line rts optional { match uart.cts }
    claim signal.address
    assert "red.counter == green.counter" message="in phase"
}

block decouple {
    port node
    port rail type=power
    place capacitor as=self intent=decouple
    circuit node self rail.gnd
}

design blinky {
    stock {
        packages imperial="0603"
    }
    place power-jack V5 voltage="5V"
    place ne555-astable timer freq="2Hz" vcc=V5.out {
        with unused-rst
        without supply
    }
    place decouple "Cvdd/1" node=VDD@1 rail=vdd value="100nF"
    scope indicator {
        place led D
        circuit timer.out Rled D V5.out.gnd name="indicator" current="9mA"
    }
    connect uart {
        from mcu tx=PA2 rx=PA3
        to console
    }
    connect i2c from=inside to=outside
    connect leds {
        from mcu red=PA6
        to red red=in
        to green green=in
    }
}
"#,
    );
    assert_eq!(
        file.uses().map(|u| u.path.as_str()).collect::<Vec<_>>(),
        ["@stackup/passives", "./ne555.kdl"]
    );

    let leds = file.types().next().unwrap();
    let lines: Vec<_> = leds.lines().collect();
    assert_eq!(lines[0].ty.as_deref(), Some("pwm"));
    assert!(lines[1].optional);
    assert!(
        matches!(&lines[1].items[0], stackup_eda_parser::ast::LineItem::Match(m) if m.target.to_string() == "uart.cts")
    );

    let decouple = file.blocks().next().unwrap();
    assert_eq!(decouple.ports().count(), 2);
    let place = decouple.places().next().unwrap();
    assert!(place.is_self() && place.name.is_none());

    let design = file.designs().next().unwrap();
    assert_eq!(design.kind, BlockKind::Design);
    assert!(matches!(&design.items[0], BlockItem::Stock(s) if s.items[0].name == "packages"));

    let places: Vec<_> = design.places().collect();
    assert_eq!(
        places[1]
            .features
            .iter()
            .map(|f| (f.on, f.name.as_str()))
            .collect::<Vec<_>>(),
        [(true, "unused-rst"), (false, "supply")]
    );
    assert_eq!(places[2].name.as_deref(), Some("Cvdd/1"));
    assert_eq!(
        places[2]
            .arg("node")
            .unwrap()
            .as_reference()
            .unwrap()
            .pad
            .as_deref(),
        Some("1")
    );

    let scope = design.items.iter().find_map(|i| match i {
        BlockItem::Scope(s) => Some(s),
        _ => None,
    });
    let scope = scope.unwrap();
    assert_eq!(scope.name, "indicator");
    let circuit = match &scope.items[1] {
        BlockItem::Circuit(c) => c,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        circuit
            .elements
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>(),
        ["timer.out", "Rled", "D", "V5.out.gnd"]
    );
    assert_eq!(circuit.props.len(), 2);

    let connects: Vec<_> = design.connects().collect();
    assert_eq!(connects[0].ty, "uart");
    let from = connects[0].from().next().unwrap();
    assert_eq!(from.target.to_string(), "mcu");
    assert_eq!(
        from.answers
            .iter()
            .map(|a| (a.key.as_str(), a.value.as_str().unwrap()))
            .collect::<Vec<_>>(),
        [("tx", "PA2"), ("rx", "PA3")]
    );
    assert!(connects[0].to().next().unwrap().answers.is_empty());
    // The one-line form is two sides with no answers.
    assert_eq!(
        connects[1]
            .sides
            .iter()
            .map(|s| (s.role, s.target.to_string()))
            .collect::<Vec<_>>(),
        [
            (SideRole::From, "inside".into()),
            (SideRole::To, "outside".into())
        ]
    );
    assert_eq!(connects[2].to().count(), 2);
}

#[test]
fn numeric_names_and_nc() {
    let file = read(
        "part j {\n    pin 1 passive\n    port 1 pin=1\n}\ndesign d {\n    nc mcu.PA5 mcu.PC14 note=\"spare\"\n}\n",
    );
    let j = file.parts().next().unwrap();
    assert_eq!(j.pins().next().unwrap().name, "1");
    assert_eq!(j.ports().next().unwrap().pin.as_deref(), Some("1"));
    let d = file.designs().next().unwrap();
    let nc = match &d.items[0] {
        BlockItem::Nc(nc) => nc,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        nc.pins.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
        ["mcu.PA5", "mcu.PC14"]
    );
    assert_eq!(nc.note.as_deref(), Some("spare"));
    // A numeric name writes back as a number, and a key never does.
    assert_eq!(
        emit(&file),
        "part j {\n    pin 1 passive\n    port 1 pin=1\n}\n\ndesign d {\n    nc mcu.PA5 mcu.PC14 note=\"spare\"\n}\n"
    );
    one_error("design d { nc }", "`nc` needs at least one pin");
}

#[test]
fn what_is_reported() {
    one_error("part { pin A passive }", "`part` needs a name");
    one_error(
        "supply x {}",
        "`supply` is not a statement a file can contain",
    );
    one_error(
        "part r { resistor A passive }",
        "`resistor` is not a statement a part can contain",
    );
    one_error(
        "part r { pin A passive sometimes }",
        "unexpected `sometimes` on `pin`; only `required` or `once` can go here",
    );
    one_error(
        "part r { pin A passive required required }",
        "`required` is given twice",
    );
    one_error(
        "part r { pin A passive color=red }",
        "`pin` has no `color=` property; it takes `symbol=`, `symbol-kind=`",
    );
    one_error(
        "part r { derive x 1 }",
        "the expression of `derive` must be a quoted string, not a number",
    );
    one_error(
        "part r { derive x ra }",
        "the expression of `derive` must be quoted",
    );
    one_error(
        "block b { circuit \"a.b/c\" d }",
        "`a.b/c` is not a reference: `/` can't follow `.`",
    );
    one_error(
        "block b { circuit }",
        "`circuit` needs at least one element",
    );
    one_error(
        "block b { connect i2c { to x } }",
        "`connect` needs a `from` side",
    );
    one_error(
        "design d { default }",
        "`default` belongs to a part's child block, not a design",
    );
    one_error("use x.kdl", "the path of `use` must be quoted");
    one_error("part r { pin A passive { also } }", "`also` needs a name");
    one_error(
        "part r { package p { pad A } }",
        "`pad` needs a pin and a label",
    );
    one_error(
        "part r { pin A passive { require rest high } }",
        "`require` names a fact by its aspect",
    );
    one_error(
        "part r { package p { reference R } }",
        "`reference` is the part's, not a package's: a body changes neither",
    );
    one_error(
        "part r { package p { pin A passive } }",
        "`pin` is not a statement a package can contain",
    );
    one_error("block b { param x }", "`param` needs a type");
    one_error(
        "part r { port p { set voltage \"3V\" } }",
        "`set` names a fact by its aspect: `net.voltage`, `signal.voltage` or `segment.voltage`",
    );
    one_error(
        "part r { port p { add wire.draw \"3A\" } }",
        "`wire` is not an aspect",
    );

    // Everything is reported, and everything readable is kept.
    let doc = Document::parse(
        "t.kdl",
        "part a { pin X }\npart b { pin Y passive }\nbogus\n",
    )
    .unwrap();
    let (file, diags) = doc.file();
    assert_eq!(diags.items.len(), 2);
    assert_eq!(file.parts().count(), 2);
    assert_eq!(file.parts().nth(1).unwrap().pins().count(), 1);
    let rendered = diags.render();
    assert!(
        rendered.starts_with(
            "t.kdl:1:10: error: `pin` needs a kind\n    part a { pin X }\n             ^^^^^"
        ),
        "{rendered}"
    );
}

#[test]
fn spans_are_byte_offsets_and_survive_unicode() {
    let text = "// 4.7kΩ, 100µF\npart r {\n    pin A passive\n}\n";
    let doc = Document::parse("t.kdl", text).unwrap();
    let (file, _) = doc.file();
    let part = file.parts().next().unwrap();
    assert_eq!(
        &text[part.span.offset..part.span.end()],
        "part r {\n    pin A passive\n}"
    );
    let pin = part.pins().next().unwrap();
    assert_eq!(&text[pin.span.offset..pin.span.end()], "pin A passive");
    assert_eq!(doc.source().line_col(pin.span.offset), (3, 5));
}

#[test]
fn quoted_and_bare_are_kept_apart() {
    let file = read("block b {\n    place resistor R value=ra note=\"ra\"\n}\n");
    let place = file.blocks().next().unwrap().places().next().unwrap();
    assert_eq!(place.arg("value"), Some(&Value::Name("ra".into())));
    assert_eq!(place.arg("note"), Some(&Value::String("ra".into())));
    assert_eq!(
        emit(&file),
        "block b {\n    place resistor R value=ra note=\"ra\"\n}\n"
    );
}

#[test]
fn placement_ignore_round_trips_and_requires_reason() {
    let source = "design board {\n    place led status {\n        ignore vdd.net.voltage reason=\"bench verified\"\n    }\n}\n";
    let file = read(source);
    assert_eq!(
        file.designs()
            .next()
            .unwrap()
            .places()
            .next()
            .unwrap()
            .ignores[0]
            .target,
        "vdd.net.voltage"
    );
    assert_eq!(emit(&file), source);
    one_error(
        "design board { place led status { ignore \"vdd.net.voltage\" } }",
        "needs a nonempty `reason=`",
    );
}

#[test]
fn emitting() {
    let file = read(
        r#"
use "@stackup/passives"
part led {
    symbol "Device:LED"
    pin A passive
    pin K passive { role strap sampled=reset; require net.rest not=high }
    port a pin=A
    port b pin=K
    package p0805 footprint="LED_0805" {
        pad K 1
        pad A 2
    }
    package p0603 footprint="LED_0603" { symbol "Device:LED_Small"; order mpn=LTST-C190KRKT; pad K 1; pad A 2 }
}
design blinky {
    place decouple "Cvdd/1" node=VDD@1 rail=vdd value="100nF"
    connect i2c from=inside to=outside
    connect output {
        from mcu
        to status
    }
    connect uart {
        from mcu tx=PA2 rx=PA3
        to console
    }
}
"#,
    );
    assert_eq!(
        emit(&file),
        r#"use "@stackup/passives"

part led {
    symbol "Device:LED"
    pin A passive
    pin K passive {
        role strap sampled=reset
        require net.rest not=high
    }
    port a pin=A
    port b pin=K
    package p0805 footprint="LED_0805" {
        pad K 1
        pad A 2
    }
    package p0603 footprint="LED_0603" {
        symbol "Device:LED_Small"
        order mpn=LTST-C190KRKT
        pad K 1
        pad A 2
    }
}

design blinky {
    place decouple "Cvdd/1" node=VDD@1 rail=vdd value="100nF"
    connect i2c from=inside to=outside
    connect output from=mcu to=status
    connect uart {
        from mcu tx=PA2 rx=PA3
        to console
    }
}
"#
    );
}

#[test]
fn editing_keeps_the_rest_of_the_file() {
    let text = r#"// The board.
design demo {
    place stm32c011f6 mcu vdd=reg.out   // the MCU

    // Not yet routed.
    connect output from=mcu to=status
    connect uart {
        from mcu   // answer me
        to console
    }
}
"#;
    let mut doc = Document::parse("demo.kdl", text).unwrap();
    let (file, _) = doc.file();
    let design = file.designs().next().unwrap();
    let uart = design.connects().nth(1).unwrap();
    let from = uart.from().next().unwrap();

    // Answer the MCU's side: two properties on the `from`, nothing else moves.
    doc.set_property(from.span, "tx", &Value::Name("PA2".into()))
        .unwrap();
    doc.set_property(from.span, "rx", &Value::Name("PA3".into()))
        .unwrap();
    // Change a placement argument in place.
    let mcu = design.places().next().unwrap();
    doc.set_property(mcu.span, "vdd", &Value::Name("V3V3.out".into()))
        .unwrap();
    // Rewrite the one-line connect as the block form, and add a statement to the design.
    let output = design.connects().next().unwrap();
    doc.replace(
        output.span,
        "connect output {\n    from mcu node=PA1\n    to status\n}",
    )
    .unwrap();
    doc.insert(
        Some(design.span),
        None,
        "place indicator status vcc=reg.out",
    )
    .unwrap();

    assert_eq!(
        doc.to_string(),
        r#"// The board.
design demo {
    place stm32c011f6 mcu vdd=V3V3.out   // the MCU

    // Not yet routed.
    connect output {
        from mcu node=PA1
        to status
    }
    connect uart {
        from mcu tx=PA2 rx=PA3   // answer me
        to console
    }
    place indicator status vcc=reg.out
}
"#
    );

    // The edited text reads back, and the answers are there.
    let doc = doc.reparse().unwrap();
    let (file, diags) = doc.file();
    assert!(diags.is_empty(), "{}", diags.render());
    let design = file.designs().next().unwrap();
    let uart = design.connects().nth(1).unwrap();
    assert_eq!(uart.from().next().unwrap().answers.len(), 2);
    assert_eq!(
        design
            .connects()
            .next()
            .unwrap()
            .from()
            .next()
            .unwrap()
            .answers[0]
            .value,
        Value::Name("PA1".into())
    );

    // Stale spans are refused, not misapplied.
    let mut stale = Document::parse("demo.kdl", text).unwrap();
    let err = stale
        .set_property(stackup_eda_parser::Span::new(3, 0), "x", &Value::Null)
        .unwrap_err();
    assert!(err.render().contains("no statement starts here"));
    // Removing takes the comment above along with the statement.
    let mut doc = Document::parse("demo.kdl", text).unwrap();
    let (file, _) = doc.file();
    let output = file.designs().next().unwrap().connects().next().unwrap();
    doc.remove(output.span).unwrap();
    assert!(!doc.to_string().contains("Not yet routed"));
    assert!(doc.to_string().contains("connect uart {"));
}

#[test]
fn a_new_file_from_the_model() {
    use stackup_eda_parser::ast::*;
    use stackup_eda_parser::{Property, Reference, Span};
    let s = Span::default();
    let file = File {
        items: vec![
            Item::Use(Use {
                path: "@stackup/passives".into(),
                span: s,
            }),
            Item::Block(Block {
                kind: BlockKind::Design,
                name: "board".into(),
                items: vec![
                    BlockItem::Place(Place {
                        what: "power-jack".into(),
                        name: Some("V5".into()),
                        args: vec![Property::new("voltage", "5V")],
                        features: vec![],
                        ignores: vec![],
                        span: s,
                    }),
                    BlockItem::Place(Place {
                        what: "decouple".into(),
                        name: Some("Cvdd/1".into()),
                        args: vec![
                            Property::name("node", "VDD@1"),
                            Property::name("rail", "vdd"),
                            Property::new("value", "100nF"),
                        ],
                        features: vec![],
                        ignores: vec![],
                        span: s,
                    }),
                    BlockItem::Circuit(Circuit {
                        elements: vec![
                            Reference::parse("V5.out.rail").unwrap(),
                            Reference::name("R"),
                            Reference::parse("V5.out.gnd").unwrap(),
                        ],
                        props: vec![],
                        from: None,
                        legs: vec![],
                        span: s,
                    }),
                ],
                span: s,
            }),
        ],
    };
    let text = emit(&file);
    assert_eq!(
        text,
        "use \"@stackup/passives\"\n\ndesign board {\n    place power-jack V5 voltage=\"5V\"\n    place decouple \"Cvdd/1\" node=VDD@1 rail=vdd value=\"100nF\"\n    circuit V5.out.rail R V5.out.gnd\n}\n"
    );
    let again = read(&text);
    assert_eq!(emit(&again), text);
}

#[test]
fn peripheral_connection_requirements_round_trip() {
    let source = r#"part chip {
    peripheral I2C2 i2c {
        has bootloader when="scl == PB10 && sda == PB11"
        scl PB10 PB13
        sda PB11 PB14
    }
}

design demo {
    connect i2c {
        from mcu scl=PB10 sda=PB11
        to expansion
        require peripheral.bootloader
    }
}
"#;
    assert_eq!(emit(&read(source)), source);
    assert!(!errors("design demo { connect i2c from=mcu to=port { require peripheral.bootloader #true; }; }\n").is_empty());
    assert!(!errors("part chip { port x { require peripheral.bootloader; }; }\n").is_empty());
    assert!(
        !errors("design demo { connect i2c from=mcu to=port { require signal.bootloader; }; }\n")
            .is_empty()
    );
}
