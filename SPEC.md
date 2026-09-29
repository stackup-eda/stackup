# stackup design format — specification

*Draft. Describes the domain model as settled so far.*

## 1. Overview

A **design** is a deterministic, declarative description of an electronic circuit, written in
[KDL v2](https://kdl.dev). There is no control flow and no evaluation order: a design is a set of
statements, and every statement is a fact about the circuit.

The model has a small number of concepts:

- **Parts** describe physical components: their pins, their packages, the ports a contract can
  see, and — for a part with configurable I/O — which peripheral lines can appear on which pins.
- **Blocks** are parameterized sub-circuits. A design is the root block of a board.
- **Placements** instantiate a part or block at a path in the design's hierarchy.
- **Types** describe what a connection carries, as a set of named lines.
- **Facts** are properties of the circuit — a voltage, a draw, an I²C address — contributed by
  ports, each belonging to one **aspect**: a segment, a net or a signal.
- **`circuit`** joins terminals electrically.
- **`connect`** joins two ends of a typed link, and records which pins a configurable end uses.

stackup is a validator. It never chooses a pin, a peripheral or a value; it checks the choices a
design makes.

### Terms

| Term | Meaning |
|---|---|
| statement | A KDL node: one declaration or fact in a file. |
| part | A declared physical component. |
| block | A declared parameterized sub-circuit. |
| placement | An instance of a part or block, named by a path. |
| pin | A logical terminal of a part, named as its datasheet names it. |
| pad | A physical terminal of one of a part's packages. A pin has one or more pads. |
| port | A named, typed interface of a part or block. |
| line | A named member of a type, and of any port of that type. |
| type | A set of lines, with relations and checks between them. |
| segment | One connection between two endpoints, made by a statement. |
| net | Everything at one potential: the connected set of segments. Derived, never declared. |
| signal | One piece of information, across every net that carries it. |
| aspect | Segment, net or signal: what a fact belongs to. |
| fact | A property of a segment, net or signal. |
| side | One end of a `connect`. |
| finding | A reported violation. |

## 2. Files and imports

A file is a KDL v2 document. Each KDL node is a **statement**; a file's top-level statements
are declarations and imports:

| Statement | Declares |
|---|---|
| `part <name>` | a part (§5) |
| `block <name>` | a block (§6) |
| `design <name>` | a design (§6.1) |
| `type <name>` | a composed type (§8.3) |
| `use "<path>"` | an import |

`use` takes a path relative to the importing file (`"./ne555.kdl"`), or a core-library path
beginning `@stackup/` (`"@stackup/passives"`). An import brings every top-level name the imported
file declares into scope. Two declarations of the same name in scope are an error, reported at the
`use` that introduced the second; nothing shadows anything.

Order carries no meaning, within a file or across files. A statement may refer to a name
declared later.

## 3. Names and references

### 3.1 Statement names and arguments

**Statement names belong to the language.** A user-chosen name appears only in argument
position: in `place resistor Ra`, `place` is the statement, `resistor` refers to a declared part,
and `Ra` is the new name. A user name is never a statement name.

**Property keys are free.** A key may be a language keyword (`value=`, `intent=`), a block's
parameter or port (`freq=`, `vcc=`), or a type's line (`sda=`).

**A number is a name.** A plain header's pins are `1`, `2`, `3`, so `pin 1 passive` and `pin=1`
name the pin `1`, and `J.2` refers to it.

### 3.2 Paths

A placement's **path** is its name composed onto the path of the block it is placed in, with `/`
between levels. A 555 placed as `u` inside a block placed as `timer` is `timer/u`; a capacitor
placed as `Cvdd` inside the `supply` block of a part placed as `mcu` is `mcu/supply/Cvdd`.

References use three separators:

| Separator | Selects | Example |
|---|---|---|
| `/` | a level of hierarchy | `timer/u` |
| `.` | a pin, port or line of what precedes it | `u.OUT`, `reg.out.rail` |
| `@` | a pad of a pin, by its pad label | `VDD@4`, `mcu.VDD@A1` |

A pad label is a string, not a number, so a ball-grid label (`A1`) is a label like any other.

### 3.3 Implicit scopes

A placed name may itself contain `/`: `place decouple "Cvdd/1" …`. This creates `Cvdd` as a scope
with nothing placed at it, and `Cvdd/1` within it. KDL does not allow `/` in a bare identifier, so
such a name is quoted.

**A path segment is either a placed name or a scope, never both.** Placing `Cvdd` and
`"Cvdd/1"` in the same block is an error.

### 3.4 Resolution

Inside a block, a reference resolves against the block's parameters, derived values, ports and
placements. Inside a part's child block (§7), the part's pins, ports and parameters are also in
scope by bare name — `VDD` is the part's pin, `vdd` its port.

## 4. Values and expressions

### 4.1 Values

Every value is an **expression**; a literal is the simplest expression. In a property or argument:

- a **quoted string** is an expression — `"100nF"`, `"3.3V * 500mA / 0.85"`;
- a **bare identifier** is a name — a reference to a value in scope (`value=ra`), or a member of
  an enumerated parameter type (`add0=gnd`).

A KDL value may not begin with a digit unless it is a number, so a quantity with a unit is always
quoted: `"2Hz"`, not `2Hz`.

### 4.2 Quantities

A quantity is a number with a unit: `"100nF"`, `"4.7kΩ"`, `"3.3V"`, `"500mA"`, `"2Hz"`. SI
prefixes apply (`p n µ m k M G`). A ratio may be written as a percentage (`"85%"`). A quantity
may carry a range — `min`, `max`, or a nominal value with a `tolerance` — and a range's bounds are
readable as `.min` and `.max`.

### 4.3 Expressions

Expressions are pure: arithmetic over quantities with unit checking, comparisons, the boolean
operators `&&`, `||` and `!`, and functions provided by core:

| Function | Result |
|---|---|
| `clamp(x, lo, hi)` | `x` limited to `[lo, hi]` |
| `e3(x)`, `e6(x)`, `e12(x)`, `e24(x)`, `e96(x)` | the nearest value of that E-series, in log space |
| `min(a, b)`, `max(a, b)`, `abs(x)` | |
| `match(k, a: x, b: y, …)` | the value paired with `k`; no arm for `k` is an error |
| `ln2`, `pi` | constants |

The function set is core's; a design cannot add to it.

**Every quantity is a range**, of zero width when it was stated as one number, and arithmetic
is interval arithmetic: a lamp's current off a 12–18 V rail is a range, and `.min`/`.max` read
its ends. A comparison over ranges holds when it holds for every value — `a <= b` is the top of
`a` under the bottom of `b` — so an assertion is written against the end it means
(`current <= limit` with `current` derived from `rail.voltage.max`), and one that compares two
wide ranges is asking a stricter question than it may intend.

**Unknown is a value.** A fact nothing states (§9.7) evaluates to *unknown*, and anything
computed from it is unknown too, carrying the reason. `&&` and `||` decide without it where one
side settles the matter; everything else absorbs it. A name that resolves to nothing is a bare
word, for `match` and `==` against an enumerated member; used in arithmetic it is an error.

### 4.4 Declarations

| Statement | Declares |
|---|---|
| `param <name> <type> [default=<value>]` | a parameter, set by the placement |
| `derive <name> "<expr>"` | a value computed from params, facts and other derived values |
| `text <name> "<template>"` | a string; `{name}` substitutes a value, `{name:.0%}` formats it |
| `assert "<expr>" [message="<template>"]` | a test; failing it is a finding |

Facts and requirements name their aspect (§9.1): `set net.voltage`, `require signal.dma`.

A parameter's type is a quantity type (`resistance`, `capacitance`, `inductance`, `voltage`,
`current`, `frequency`, `ratio`), `part`, or an enumerated type. A parameter without a default
must be given by every placement.

A `part` parameter takes a bare name of a part in the caller's scope. Its default, if any, names
a part in the declaring file's scope. The binding carries that declaration into the block, so
`place fet Q` places the selected part even when the block's file does not import it. A block may
pass the binding to another block with `fet=chosen`. The selected part's pins and ports must fit
the block's wiring; an absent one is an error. A part parameter is a placement input, never a
choice made from propagated facts.

### 4.5 Values are stated, facts are derived

A part's value is chosen — a parameter with a default, or a literal at the placement — and never
computed from a fact. What a chosen value does in the circuit it is bound to is a **fact**,
derived from the value and the facts around it, and an **assertion** holds that fact to a bound:

```kdl
param ballast resistance default="4.3kΩ"
port cathode { set net.voltage "rail.voltage - forward" }
derive current "cathode.voltage.max / ballast"
assert "current <= limit" message="{ballast} lets {current} through at {rail.voltage.max}"
```

So a block bound to a rail it was not sized for fails an assertion that names the value, rather
than changing what the BOM buys. A placement's component value and a `when` condition may read
parameters, features, and derives computed only from them. Reading a propagated fact through a
derive in either context is an error. Assertions and fact contributions may read propagated facts.

## 5. Parts

A part describes one component: what it is, its pins, its packages, its ports, and — where it
has configurable I/O — its peripherals. It may also carry application circuits as child blocks
(§7).

### 5.1 Metadata

| Statement | Meaning |
|---|---|
| `symbol "<lib>:<name>"` | the KiCad symbol it corresponds to |
| `reference <prefix>` | the designator prefix (`U`, `R`, `J`) |
| `value "<text>"` | the exported value, where it differs from the symbol's name |
| `datasheet "<url>"` | |
| `description "<text>"` | |
| `manufacturer "<name>"` | |
| `order mpn=… <distributor>=…` | orderable identifiers |

`reference` and `manufacturer` are the part's alone. The rest may also be stated inside a
package (§5.3), for what a body changes.

### 5.2 Pins

```kdl
pin <name> <kind> [required] [once] [symbol="<name>"] [symbol-kind=<kind>] [{ … }]
```

A pin is a logical terminal, named as the datasheet names it. Its **kind** is its electrical
type, in KiCad's vocabulary: `input`, `output`, `bidirectional`, `passive`, `power_in`,
`power_out`, `open_collector`, and so on.

| Flag / property | Meaning |
|---|---|
| `required` | the pin must be connected — by a `circuit`, a `connect`, or a child block |
| `once` | the pin may be named by at most one link |
| `symbol=` | the symbol's name for the pin, where it differs |
| `symbol-kind=` | the symbol's kind for the pin, where the part corrects it |

A pin's children:

- **`also <port-pin>`** — another name the same pin can be. A pad that carries `PA11` out of
  reset and `PA9` behind a remap is `pin PA11 … { also PA9 }`; peripheral tables (§5.5) name
  either, and both refer to the one pin. Naming both in one design is using the pin twice.
- **`role <role> …`** — what the pin is for (`role reset active=low pull=internal`). The
  vocabulary is open; two roles are established:
  - `role reset active=<low|high> pull=<internal|none>`;
  - `role strap sampled=reset pull=<internal-up|internal-down|none> note="…"` — a pin the die
    reads a boot fact off at reset, with the pull it has of its own and what the levels mean.
- **`require <aspect>.<fact> …`** — what the pin's own net must satisfy (§9.5), in the form a
  port's `require` takes. The one a strap wants is the net's rest level:

  ```kdl
  pin IO0 bidirectional once {
      role strap sampled=reset pull=internal-up note="high is SPI boot, low is download"
      require net.rest high
  }
  pin IO46 bidirectional once {
      role strap sampled=reset pull=internal-down note="with IO0 low, an invalid boot mode"
      require net.rest not=high
  }
  ```

  A role says why; a `require` is what is checked. A strap that is only a note — sampled only
  when an option bit firmware owns says so — carries the role and no requirement.

### 5.3 Packages

```kdl
package <name> footprint="<footprint>" {
    symbol "<lib>:<name>"          // what this body changes, if anything
    description "<text>"
    order mpn=…
    pad <pin> <label>
    …
}
```

A package maps pins to pads. A pin may have several pads in one package (a rail spread across
pads, a thermal pad). A part may list several packages; the first listed is the default, and a
placement chooses another with `package=<name>` (§6.2).

**The part is the die and the package is the body.** The pin table names every pin the part can
bring out, and each package maps the ones it bonds. A pin the chosen package does not bond is not
on that placement: naming it — in a `circuit`, an answer, an `nc`, a child block — is an error
that says which packages have it; a peripheral row (§5.5) that names it does not apply; a port
line (§5.4) mapped onto it is left out of the port, and a port over that pin alone is not a port
of the placement. So one table serves every body, and what a body adds is stated once.

A package may carry the part's metadata statements — `symbol`, `value`, `datasheet`,
`description` — and its `order`, for what is true of the part in that body and not in another.
The package's statement is read first and the part's answers for whatever it leaves unsaid;
`reference` and `manufacturer` cannot appear in a package, because a body changes neither.

Inside the part, `package` reads as a parameter holding the chosen package's name, so a child
block can be conditional on the body (`when "package == lqfp48"`, §7).

### 5.4 Ports

A port is what a contract sees of a part. It maps a type's lines onto pins:

```kdl
port a pin=A                        // a terminal, on one pin

port vdd type=power {               // a multi-line port
    require net.voltage min="2.0V" max="3.6V"
    line rail pin=VDD
    line gnd pin=VSS
}
```

A line mapped to a pin is on that pin's net.

A port may also carry facts (§9) and have lines with no pin, whose net the part's child blocks
decide (§7).

### 5.5 Peripherals

A part with configurable I/O declares its **peripherals**: which lines of which instance can
appear on which port pins, and what each can do.

```kdl
peripheral TIM1 timer {
    has break
    ch1 PA0 PA5 PA8 PA14 { has dma }
    ch1n PA3 PA7
    …
}
```

- The **kind** (`gpio`, `timer`, `i2c`, `spi`, `uart`, `adc`, `swd`) is core's. It names the
  lines an instance has, the relations between them (`chXn` is the complement of `chX`), and the
  types an instance provides (§8.2).
- Each row lists every port pin that line of that instance can appear on. A row names port pins,
  so a pin's `also` names are valid here.
- **`has`** marks a capability of the instance, or of one line. Capabilities become facts on the
  lines the peripheral drives (§9.4).

An instance capability can be conditional on the connection's complete pin assignment and
selected package:

```kdl
peripheral I2C2 i2c {
    scl PB10 PB13
    sda PB11 PB14
    has bootloader when="scl == PB10 && sda == PB11"
}
connect i2c {
    from mcu scl=PB10 sda=PB11
    to expansion
    require peripheral.bootloader
}
```

`when=` uses the expression language. Line names resolve to the chosen canonical pin names;
`package` resolves to the selected package name. The part's pin and package names are valid
bare enum literals. Unknown names, invalid expressions, and non-boolean results are errors.
Instance-level conditional capabilities are evaluated per connection and are never stamped
onto nets or signals. Multiple true declarations of the same capability are alternatives.

`require peripheral.<capability>` in a `connect` requires a flag on its `from` provider.
The provider must resolve to one part with a peripheral table; explicit `line=pin` answers
are the usual form. All requirements must be satisfied by one candidate instance that can
carry every answered line. Requirements may select among otherwise ambiguous candidates;
facts from different candidates or connections cannot be combined. Missing capabilities
are errors, including when the part declares no peripheral of the requested kind. These
requirements take no values or properties and are only supported inside `connect`.
Unconditional instance capabilities are also available in this scope. Existing signal
capabilities retain their behavior; line-level `has` does not declare an instance capability.

The vocabulary is open: `bootloader` is a library fact, with no special engine behavior.
A library using it for ROM interface compatibility does not thereby assert boot entry,
transceiver enable state, or a complete board programming procedure.

Peripherals describe what is **legal**. They carry no configuration a firmware needs.

## 6. Blocks and placement

A block is a parameterized sub-circuit:

```kdl
block <name> {
    param …       // parameters
    port …        // its interface
    derive …      // computed values
    text …
    assert …
    place …       // what it contains
    circuit …     // electrical wiring
    connect …     // typed links
    set …         // a fact on a terminal — a net's name (§10.1)
}
```

### 6.1 Designs

`design <name> { … }` is a block with no ports: the root of a board's hierarchy. A file may
declare several designs.

### 6.2 Placement

```kdl
place <part-or-block> <name> [<arg>=<value> …] [{ with <feature> …; ignore <selector> reason="<why>" … }]
```

A placement instantiates a part or block at `<name>`, composed onto the enclosing path.

An `ignore` child acknowledges one check on that placement. Its selector is an exact check
identity, not a regular expression or a diagnostic message. For a port or pin requirement use
`<port-or-pin>.<aspect>.<fact>`, for example `ignore vdd.net.voltage reason="tested at 3.3 V"`.
For a body assertion use `assert:<expression>`; for a port assertion use
`<port>.assert:<expression>`. Quote selectors containing spaces. The assertion expression must
match the declaration exactly. `reason=` is required. A failing acknowledged check is reported
as a note with the reason and its supporting facts; it does not stop evaluation of other checks.
An ignore selector that matches no check is an error, so changed library checks cannot silently
leave stale acknowledgements. The selector is scoped to this placement's own checks, not those of
its child placements.

Its properties are **arguments**:

- a **parameter** of the placed part or block — `freq="2Hz"`, `add0=gnd` — or `package=soic8`,
  which chooses one of the part's packages (§5.3) and reads as a parameter inside it;
- a **port** of the placed part or block — `vcc=V5.out`, `node=i2c.sda`. A port argument stands
  for the link its type implies: a `circuit` joining it, for a `terminal` port; a `connect` of
  the port's type, for any other.

Wiring at placement is allowed and never required: every port argument can instead be written as
the `circuit` or `connect` it stands for.

Any part placement may state its selected value, footprint and purchasing details without a
new part declaration. `manufacturer=`, `mpn=`, `lcsc=`, `mouser=`, `digikey=`, and `series=`
override the library part's corresponding defaults for that instance. `voltage=`,
`dissipation=`, `current=`, `tolerance=`, and `dielectric=` record the selected rating or grade
on the exported PCB footprint. `value=` is the component's electrical value; `bom_value=` can
give KiCad's Value field a fuller assembly string when the fab does not export purchasing fields.
These choices do not change the library part's pins or behavior.

The language properties `intent=` and `note=` state what a part is placed to do
(`decouple`, `bypass`, `bulk`, `filter`, `pull-up`, `pull-down`, `timing`, `series`, `divider`)
and record a note. Any placement takes `note=`, and two
properties about what the silkscreen prints: `reference=<prefix>` (`LED`, numbered in placement
order — `LED1`, `LED2`) and `designator=<word>` (`PXL`, whole, for a part there is one of). A
designator is an export artifact; nothing in a design refers to a part by one.

A part placed for a particular pin can say `anchor=<placement.pin>`: for example, after
`place chip controller`, write `place capacitor bypass anchor=controller.VCC`. `controller` is the
KDL placement name in scope, not the printed reference `U1`. At PCB export the anchor becomes the
host's designator and physical pad number for **Stackup Anchor** (for example, `U1.2`). If the pin
has several distinct pads, select one explicitly with `@pad` (`anchor=controller.VCC@2`). The host
must be a different placed part with a package and that pad.
`spot="dx dy rotation"` optionally gives an exact position relative to the host pad in the host's
local frame, in millimetres and degrees; it requires `anchor=`. Without it, the KiCad plugin picks
a nearby position. These properties may pass through a block to its `as=self` part.
An application block can expose an optional anchor input with
`param anchor reference default=#null`, then forward it using
`place capacitor as=self anchor=anchor`. The reference is
resolved in the caller's scope, so a part's child block can pass its own pin as
`place decouple Cvdd anchor=VDD@4`. The null default leaves unanchored uses unchanged.

### 6.3 Root parts: `as=self`

```kdl
block decouple {
    port node
    port rail type=power
    place capacitor as=self intent=decouple
    circuit node self rail.gnd
}
```

A block may place at most one part `as=self`. That part takes the **block's own path** — a
placement `Cvdd` of `decouple` is the capacitor at `Cvdd`, not a block containing `Cvdd/C` — and
the block takes the part's **parameters**, so `place decouple Cvdd value="100nF"` sets the
capacitor's value. The block's interface is still its own ports; nothing else is inherited. Inside
the block, the part is `self`.

### 6.4 Ports face both ways

A block's port is one thing seen from two sides. From outside, `port rail type=power` is
something the block needs; from inside, it is what the block's contents are joined to. So a
block passes its own port to its children — `place pull-up sda node=i2c.sda rail=rail` — and the
children are powered from whatever the block's user joined `rail` to.

### 6.5 Unconnected pins

```kdl
nc mcu.PA5 mcu.PA12 mcu.PC14 note="spare — the oscillator pads, free because there is no crystal"
```

`nc` declares pins deliberately joined to nothing. On a board a dropped join and a spare pin are
the same netlist entry, so the difference has to be stated to be checked: a pin declared `nc`
that is joined to anything is a finding. A pin nothing declares is only unconnected.

## 7. Features

A part may declare child blocks: its application circuits. **Each is a feature**, named for
itself. A block may declare them too — a sheet's options: the body its port lands on, whether a
flag is placed — and they read the same way; a design may not, since nothing places a design.

| Declaration | Placed when |
|---|---|
| `block <name> { default … }` | always, unless the placement turns it off |
| `block <name> { … }` | the placement asks for it: `place … { with <name> }` |
| `block <name> { when "<expr>" … }` | its condition holds; it cannot be asked for by name |

A `when` condition is an expression over the placement's features (a feature's name is true when
it is on), parameters, and derives computed only from them:

```kdl
block direct { when "!emc-class-b"; … }     // the input straight onto the pin, unless filtered
block add0-gnd { when "add0 == gnd"; … }     // one strap per address
block reference { when "package == lqfp48"; … }   // the circuit a pin only this body has
```

`package` is the parameter a multi-body part reads its body from (§5.3): the block that
decouples a pin one package bonds and another does not is `when` on the package rather than
`default`, since on the other body there is no pin and nothing for a placement to opt out of.

Features belong to one placement. Two placements of one part have independent feature sets, and
nothing unifies them.

A feature elaborates in a frame that opens onto its owner's: a part's child block sees the
part's pins, ports and parameters by bare name (§3.4), and a block's child block sees the block's
ports, parameters and placements the same way — so `circuit a.VLINK buck.in.rail` inside a
block's `jacks` feature reaches the `buck` the block placed. Its own placements are under the
feature's path (`link/jacks/a`), and nothing outside the owner can reach into them.

A pin marked `required` that a part also offers a feature for (`unused-cont`) is satisfied by the
placement turning that feature on, or by the circuit around it connecting the pin.

## 8. Types

### 8.1 Lines

A type is a set of named **lines**. A port of a type has those lines; a link of a type joins them.

`terminal` is the default type: one line, `node`. Wherever a terminal is used as a point — in a
`circuit`, as a `node=` argument — it stands for its `node` line, so `port node` is written `node`
and never `node.node`.

A line meets the **same-named line** on the other side of a link, unless its type says otherwise
with **`match`**:

```kdl
type uart {
    line tx { match uart.rx }
    line rx { match uart.tx }
}
```

So every port names its lines from its own side — a module's `tx` is its own transmit pin — and
the crossover is stated once, in the type.

### 8.2 Core types

| Type | Lines | Notes |
|---|---|---|
| `terminal` | `node` | the default |
| `output` | `node` | a driven signal; refines `terminal` |
| `pwm` | `node` | refines `output` |
| `analog` | `node` | a level a converter can sample; refines `terminal` |
| `pwm-pair` | `high`, `low` | `low` is the complement of `high` |
| `i2c` | `sda`, `scl` | `claim signal.address`; asserts each line has a pull-up |
| `spi` | `sck`, `mosi`, `miso` | like-to-like; a chip-select is not part of it |
| `uart` | `tx`, `rx`, `rts`, `cts` | `tx`↔`rx` and `rts`↔`cts` by `match`; flow control optional |
| `swd` | `swdio`, `swclk`, `nrst` | |
| `power` | `rail`, `gnd` | facts `voltage` (set) and `draw` (add) |

A type **refines** another when it has the same lines and more meaning: a `pwm` is usable
wherever an `output` or a `terminal` is expected.

Peripheral kinds provide core types: a `gpio` line provides `output`; a `timer` channel provides
`pwm`, and a channel with its complement a `pwm-pair`; an `adc` input provides `analog`; `i2c`,
`spi`, `uart` and `swd` instances provide the types of the same names.

### 8.3 Composed types

A board or library composes its own types from core's:

```kdl
type leds {
    line red type=pwm { require signal.dma }
    line green type=pwm { require signal.dma }
    line blue type=pwm { require signal.dma }
    assert "red.counter == green.counter && green.counter == blue.counter" \
        message="red, green and blue must share a counter to stay in phase"
}
```

A line may have a type, requirements on its facts (§9.3) and a `match`. A type may assert over its
lines' facts. Its assertions are checked for every link of the type.

## 9. Facts

### 9.1 What a fact is

A fact is a named property of the circuit: a rail's `voltage`, the `draw` on it, a timer channel's
`counter`, an I²C device's `address`. Every fact is defined by four things:

- a **name**;
- an **aspect** — segment, net or signal — which decides what the fact is shared across;
- a **combining rule** — `set`, `add` or `claim` — which decides what happens when several
  contributions meet within that scope;
- a **value type** — a quantity, an identifier, or a flag.

**A fact is always written with its aspect**: `net.voltage`, `net.draw`, `signal.counter`,
`signal.address`. The aspect decides how the fact moves, so a contribution that names it is
complete on its own; the same word can be a fact on more than one aspect (`net.name`,
`signal.name`); and a type introduces a fact by writing it, with nothing to declare elsewhere.

Facts are contributed by ports, read by expressions as `<port>.<fact>` (`in.voltage.min`,
`out.draw`, `red.counter`), and checked by requirements and assertions. A read names the aspect
only when it has to — `in.net.voltage` — which is when a port carries the same fact name on two
aspects.

### 9.2 Aspects

| Aspect | What it is | A fact on it is shared by |
|---|---|---|
| **segment** | one connection between two endpoints | that connection alone |
| **net** | everything at one potential | every end on the net |
| **signal** | one piece of information | every net the signal spans |

A **segment** is made by a statement: each step of a `circuit` (§10), each line of a `connect`
(§11). Different segments of one net carry different things — the segment from a supply to a
motor driver carries the motor's current, and a sense connection to the same net carries none.

A **net** is derived: the connected set of segments. Nothing declares a net.

A **signal** is what a link's line carries. A net carries at most one signal; a signal may span
several nets — a PWM line through a series resistor is one signal on two nets — where the design
says it crosses from one to the next (an open question; see `OPEN.md`).

**A fact moves only within its aspect.** A net fact never crosses to another net, even one carrying
the same signal; a signal fact is shared by every net the signal spans, whatever their net facts
are. A fact on a multi-line port belongs to the signals or nets of all its lines.

### 9.3 Core facts

| Fact | Aspect | Rule | Value | Contributed by |
|---|---|---|---|---|
| `voltage` | net | `set` | voltage, with a range | a converter's output, a power input connector |
| `draw` | net | `add` | current | a load's power port |
| `pull` | net | `add` | conductance | a pull-up |
| `rest` | net | derived | `high`, `low`, `float` or unknown | what the net sits at with nothing driving it (§9.6) |
| `counter` | signal | `set` | identifier | a timer channel |
| `dma` | signal | `set` | flag | a peripheral line that `has dma` |
| `address` | signal | `claim` | integer | an I²C device's port |
| `name` | net | `set` | text | a port line (`line rail { set net.name "VPWR" }`), or a `circuit`'s `name=` |

### 9.4 Combining rules

A port contributes a fact with the statement for its rule. Written on the port, the fact goes
to every line's net; written inside a `line`, to that line's alone:

| Statement | Rule | Several contributions in one scope are | Example |
|---|---|---|---|
| `set <aspect>.<fact> <value>` | one writer | a conflict | `set net.voltage "3.3V" tolerance="2%"` |
| `add <aspect>.<fact> <value>` | accumulates | summed | `add net.draw "100µA"` |
| `claim <aspect>.<fact> <value>` | distinct | a conflict if any two are equal | `claim signal.address address` |

The scope is the fact's aspect: two ports setting `voltage` on one net conflict, while two nets
joined by a signal — the two sides of a level shifter — each have a `voltage` of their own.
There is no port direction: the port that `set`s a net's voltage is its supply, and a port that
`add`s a draw is a load on it.

**A `power` port's `gnd` line is a return.** A `voltage` set on the port is the rail's and does
not reach the return; a `draw` does, since a return carries every rail on it; and a return rests
low (§9.6). A fact read through a multi-line port (`out.draw`, `rail.voltage`) is read on the
first line whose net states it, returns last; a line can be named (`out.rail.draw`), and an
aspect where the name is on two (`in.net.voltage`).

A contribution's value is an expression in the frame of the port's owner, so it can read the
part's parameters and derived values, and the facts on its other ports — the converter's
`add net.draw input` above. On a port, a bare name is looked up as a parameter, a derived value
or a text first, and as one of the port's own facts after, so `set net.voltage voltage` sets the
rail from the parameter rather than from itself. A read that comes back to the fact being
computed is a loop and a finding.

### 9.5 Requirements

`require` constrains the facts of whatever a port is joined to:

```kdl
require net.voltage min="4.5V" max="36V"
require signal.dma
```

A requirement is part of the port's interface and holds between ports. An **assertion** (§4.4) is
a test over values and holds wherever it is written. Failing either is a finding.

### 9.6 Computed contributions

A contribution may be an expression, including over other facts. A converter adds to its input
the power its output delivers, over its efficiency:

```kdl
derive input "out.draw * out.voltage / (0.85 * in.voltage.min)"
port in type=power {
    require net.voltage min="4.5V" max="36V"
    add net.draw input
}
```

Facts that depend on one another are evaluated in dependency order; a cycle is a finding.

**`rest` is derived from what is on the net**, and is the fact a strapping pin's `require`
reads (§5.2). A rail on the net gives its level (`high` above the return, `low` for a return);
a pull-up gives `high` and a pull-down `low`; a net with none of those is `float`, which a pin's
own internal pull then resolves for that pin. Another part's pin of an output kind on the net
(`output`, `bidirectional`, `open_collector`) makes it **unknown**: its idle level is that part's,
and a requirement against an unknown rest is reported as holding only for what is stated, the
way a draw sum is. The pin asking is not counted against itself. So `require net.rest high` is met by a pull-up or a rail and not by a wire to a
push-pull output, and `require net.rest not=high` fails on a pull-up whatever else the net does.

### 9.7 Unknown is not zero

A fact no port contributes is unknown. A sum with an unknown contribution is a lower bound, and a
check over a lower bound reports that it holds only for what is stated, naming what states
nothing.

What states nothing, for a `draw`, is a **part's power port** on the net that neither adds a
draw nor sets a voltage — an MCU whose current is firmware's. A block's port passes through and
is not a load; a supply is not a load. So a rail feeding a decoupled MCU is a lower bound with
the MCU named, and one feeding only blocks whose parts all speak is exact.

A check therefore has three outcomes: it **holds**, it **fails**, or it **holds for what is
stated**. The third is a note, never a pass and never a finding, and it says what was not stated.
A check over a fact that is wholly unknown cannot be run, and says so the same way.

### 9.8 Names

A net's name is a fact like any other: on the net aspect, with the `set` rule, so two stated
names on one net conflict. It is stated where facts are stated — on a port line, beside the
voltage the same port sets — by `set <terminal> net.name` in a block (§10.1), or by `name=` on a
`circuit`, which contributes it to the net at the circuit's head (§10). Like every name, it is
composed onto the path of the block it is written in: `VPWR` in a sheet placed as `node` is
`node/VPWR`.

A net nobody names gets a name at export, projected from a port line or a pin on it. A projected
name claims nothing, so two of them on one net are not a conflict.

## 10. Wiring: `circuit`

```kdl
circuit <element> <element> …

circuit {
    from <terminal>
    to <terminal> … <series-element> [name=<net-name>]
    to <terminal> … [name=<net-name>]
}
```

A `circuit` is a directed path of elements. The one-line form remains available. The block form
breaks the path into named equipotential sections. Each element is matched to a **shape** by its lines:

| Shape | Has | In a path |
|---|---|---|
| terminal | a `node` line — a pin, a pad, a terminal port, a line of a port, a block with a `node` port | a point |
| series element | `a` and `b` lines | entered at `a`, left at `b` |

- Each step between two adjacent elements is a **segment** (§9.2).
- Consecutive terminals are joined: they are on one net.
- A series element's `a` is joined to the net before it and its `b` to the net after it.
  Two adjacent series elements are joined through a net of their own.
- A path starts and ends with a terminal.
- The order written is the direction of the path, so a series element with a polarity — an LED
  with `a` on its anode — is checkable against it.

In the block form, `from` names one terminal. Each `to` line adds one or more elements on **one
net**. Every element but the last must be a terminal. A nonfinal `to` must end with a series
element: its `a` side is on that line's net, and the next `to` begins at its `b` side. The final
`to` must end with a terminal. A `to` that ends at a terminal ends the path; no further `to` is
allowed. There is exactly one `from` and at least one `to`.

`name=` on a `to` names that line's net, so names on either side of a series element can be stated
in one circuit:

```kdl
circuit {
    from vcc.v
    to pullup decouple sense name="INPUT"
    to "mcu/gpio0" name="INPUT_SENSE"
}
```

Here `sense` is a series element; its `a` is on `INPUT` and its `b` is on `INPUT_SENSE`.
The hierarchical path `mcu/gpio0` is quoted because KDL does not permit `/` in a bare value.

`name=` on a circuit sets the `name` fact (§9.8) of **the net the path starts on**. A circuit
with no series element is one net, so the name is the net's; a circuit through a series element
has several, and the name goes to the head — the source end, when the path is written in the
order the signal flows: `circuit mcu.TXD0 Rdata led.din name="DIN"` names the MCU's side of the
resistor. A net is named without joining anything by `set` on a terminal (§10.1); a `circuit`
is for joins.

### 10.1 Facts on a terminal

```kdl
set <terminal> <aspect>.<fact> [<value>] [<prop>=… ]
add <terminal> <aspect>.<fact> <value>
claim <terminal> <aspect>.<fact> <value>
```

The statement a port line uses (§9.4), written in a block and aimed at a terminal in scope — a
pin, a port, a line of a port, a block with a `node` port. It contributes the fact to that
terminal's net (or signal) under the rule the statement names, exactly as a port line would,
and its value is an expression in the block's frame. The common case is a name:

```kdl
set charger.vsys.rail net.name "VSYS"
set latch-sense net.name "PWR_BTN_SENSE"
```

**Names are the board's.** A sheet's port is a net the board joins, so the board names it,
toward the source; a sheet names only what is internal to it. A net no statement names is
anonymous.

```kdl
circuit vcc.rail Ra u.DISCH Rb u.THRES u.TRIG C vcc.gnd
```

joins `vcc.rail` to `Ra.a`, `Ra.b` to `u.DISCH` and `Rb.a`, `Rb.b` to `u.THRES` and `u.TRIG` and
`C.a`, and `C.b` to `vcc.gnd`.

A block with a `node` port is a terminal wherever it appears, so a shunt block (a pull-up, a
decoupling capacitor) can also sit mid-path: `circuit src r p dest` joins `p` to the net between
`r` and `dest`.

## 11. Contracts: `connect`

### 11.1 Form

```kdl
connect <type> {
    from <side> [<line>=<value> …]
    to <side> [<line>=<value> …]
}

connect <type> from=<side> to=<side>        // when each side satisfies the type by itself
```

A `connect` is **one link** of one type. `from` and `to` are a logical ordering, not a
direction; the type's `match` does any crossover.

### 11.2 Sides

A side is **anything that satisfies the link's type**, by duck typing:

- a port of that type, or a placement with exactly one port of that type (`to console`);
- a line, for a one-line type (`to sense`, `from gates.l1`);
- a pin, for a one-line type — `mcu.PB2` satisfies `output` on its own;
- a placement with **answers**, which build the type's lines out of the placement's pins where
  the placement cannot satisfy the type by itself (§11.3).

A placement with none of the type's ports, or several, must say which.

### 11.3 Answers

A part with peripherals can put a type's lines on many pins, so it does not satisfy a type until
its side says where the lines land:

```kdl
connect i2c {
    from mcu sda=PB9 scl=PB8     // the MCU serves i2c on these pins
    to port                      // the port has one i2c port; nothing to say
}

connect output {
    from mcu.PB2                 // one pin satisfies a one-line type outright
    to ldo.en
}
connect output from=mcu.PB2 to=ldo.en   // the same, on one line
```

Keys are the type's lines; values are the side's port pins. A side's answer is from its own
point of view: `tx=PA2` is the pin the MCU transmits on. The consumer's side names its port
(`ldo.en`, which says what it wants driven by) rather than the pin behind it.

A side that does not yet satisfy the type — `from mcu` with nothing after it — is **unanswered**,
and the link is incomplete (§12).

### 11.4 Legs

A link may be split across several `from`s or several `to`s. **Across the `from`s, every line of
the type is supplied exactly once; across the `to`s, every line is received exactly once.** On a
leg, `<line>=<value>` names which of the side's lines or pins carries that line of the type:

```kdl
connect leds {
    from mcu red=PA6 green=PC14 blue=PC15
    to lamp-r red=in
    to lamp-g green=in
    to lamp-b blue=in
}
```

### 11.5 Unbound lines

```kdl
connect spi {
    from mcu sck=PA5 mosi=PA7 miso=PA6
    to display
    nc miso note="a display answers nothing over the bus"
}
```

`nc <line> …` inside a `connect` declares lines of the type this link deliberately leaves
unbound: nothing on the `to` side receives them, and a `to` that does is a finding. The `from`
side may still answer such a line — here PA6 is answered so SPI1 is pinned by all three of its
lines — and the pin it answers with is then a declared no-connect, checked exactly as `nc` on a
pin is (§6.5): joined to anything, it is a finding. A line left both unanswered and unbound is
simply absent from the link. A write-only bus (a display, a pixel string on MOSI alone) is the
case.

### 11.6 Buses

A bus with several devices is **several links**. The configurable side's answers are written
once, and further devices join from a device already on it:

```kdl
connect i2c {
    from mcu sda=PB7 scl=PB6
    to inside
}
connect i2c from=inside to=outside
connect i2c from=inside to=pulls
```

Each line of a bus is one signal, and the bus is the ends that signal joins. A type's
signal facts and its assertions — `i2c`'s distinct addresses, its pull-up check — are
evaluated across it.

### 11.7 Legality

An answer on a side with peripherals is legal if some assignment of peripheral instances satisfies
every link on that part at once:

- each answered pin can carry the line it is given, on the chosen instance, through the provided
  type's mapping (§8.2);
- the lines of one kind come from one instance — a `pwm-pair`'s two lines, an `i2c`'s, a
  `spi`'s — while the lines of a composed type (§8.3) each answer for themselves, so three
  `pwm` lines in one link may sit on three counters unless the type asserts otherwise;
- the kind's relations hold (a `pwm-pair`'s `low` is the complement of its `high`);
- each port pin is used once, and each peripheral line once per instance;
- the type's requirements and assertions hold over the facts the chosen instances stamp.

The instance is never named in the design. It is whatever the pins leave.

What the pins leave is also what they **stamp** (§9.3): a line's `counter` where the surviving
instances are one timer, and every capability (`has dma`, `has lin`, `has break`) the surviving
instances all have. A port's `require signal.lin` is checked against that, and a composed
type's assertion over `counter` (§8.3) reads it.

## 12. Validation

stackup validates a design; it never chooses a pin, a peripheral or a value. Checking a design
produces **findings**, all at once. A finding is one of:

| Finding | Raised when |
|---|---|
| requirement not met | a `require` does not hold for what its port is joined to |
| assertion failed | an `assert` — in a block, part or type — does not hold |
| conflicting `set` | two ports `set` one fact within its aspect's scope (two names on one net) |
| duplicate `claim` | two ports `claim` one value of one fact within its aspect's scope |
| unconnected pin | a `required` pin is joined to nothing |
| connected `nc` | a pin declared `nc` is joined to something |
| pin used twice | a `once` pin, or one pin under two `also` names, is named by two links |
| illegal answer | an answer admits no assignment of peripheral instances (§11.7) |
| lines apart | the answered lines of one kind sit on no one instance (§11.7) |
| unbound line received | a `connect` declares a line `nc` and a `to` side receives it (§11.5) |
| incomplete link | a `connect` side is unanswered |
| unmatched line | a line in a link has no counterpart on the other side |
| cycle | facts depend on one another in a loop |

Findings are errors; an incomplete link is a warning. A check that holds only for what is
stated (§9.7), or cannot be run because its fact is unknown, is a **note**: reported, never a
pass, never a failure. A failed assertion or requirement points at the statements whose values
it read — the loads it summed, the supply that set the voltage — wherever they live.

What the language leaves unsettled, and where the implementation is behind this document, is
kept in `OPEN.md`, with the notes from the boards that raised each question.
