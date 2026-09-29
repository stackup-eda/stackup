# stackup — open questions and working notes

`SPEC.md` describes the language as settled. This file holds everything that is not: questions
the language has not answered, places where the implementation is behind the specification, and
the notes from real boards that raised them. When a question is settled it moves into `SPEC.md`
and leaves here; when a gap is closed its bullet is deleted.

## Language: unsettled

- **How a signal crosses from one net to another.** Whether a series element preserves a signal
  depends on how it is used — a series termination does, a divider leg does not, an RC low-pass
  ends one signal and starts another — so a crossing is stated where the element is used. There
  is no statement for it yet.
- **Segment endpoints.** A segment's end may be a place (a pin, or a specific pad — a Kelvin
  connection attaches at one pad of a shunt) or a net (anywhere on it, like `vcc.rail`). How the
  two are written, and whether only the first constrains layout, is unsettled.
- **Segment topology.** Whether the order of a `circuit` and the `from` side of a `connect`
  declare which segments exist — a chain rather than a star — and so where current flows.
- **Segment facts.** `current`, derived from draws and topology rather than summed over a net;
  and sense, a segment known to carry none. The mechanism for both is open.
- **A separate return the board must join.** A converter with its own return pin that has to
  meet the input's somewhere, as opposed to sharing a pin with it or being isolated.
- **`terminal`'s line name.** It is `node`, which reads as neither a statement nor a net;
  `at` (`place pull-up … at=i2c.sda`) is a candidate.
- **Peak and duty.** `add net.draw` sums one number; a pulsed load wants a peak and a duty.
- **Naming links.** Links are unnamed; `current=` on a `circuit` is not yet specified.
- **Pin roles.** `role` exists; its vocabulary does not.
- **Two selects on one net.** Two chip-selects joined by a `circuit` are not seen by the bus.
- **Package selection by intent.** The generic parts take their body from their `intent` and a
  board's purchasing policy; that policy is outside the specification.
- **The expression function set** beyond the functions listed in SPEC §4.3.
- **Re-exporting a child's port.** A block that offers a child's port as its own — a node sheet
  whose `v3v3` is its SBC's `vout`, whose `link` is the SBC's `uart` — joins them line by line
  with `circuit` (`circuit v3v3.rail sbc.vout.rail`). It cannot be a `connect`, which would apply
  the type's crossover a second time. Correct, and verbose for a multi-line port.
- **One line of a multi-line peripheral.** A pixel stream is an SPI's MOSI alone, wanted for the
  DMA behind it. `nc` inside a `connect` (SPEC §11.5) covers the write-only display, where the
  other lines are still carried; a link that carries one line of a three-line kind is a `spi`
  with two `nc`s, which says it, and whether a type can take some of a kind's lines more directly
  is open.
- **References across a scope.** `expansion/ldo.en` from outside the scope carries a `/`, which
  KDL will not take bare; writing the statement inside the scope avoids the quoting. Whether a
  statement may reach into a placement's children at all (`lamps/l3.gate`) is unsettled; the
  port re-exports through the block's own ports instead.
- **A shunt's tolerance.** A placement can now carry `tolerance=` into the KiCad BOM fields.
  Deriving measurement accuracy from it and checking that against a circuit requirement are
  still open.
- **What a board says about a sheet's part.** A designator is a board's to choose and a sheet's
  part to carry, and a sheet placed on several boards has no word for it. There is no `designate`
  by path, so a sheet that belongs to one board states its designators itself, and one shared by
  several cannot.
- **Fitted or not.** A land kept for a module that is not yet chosen is placed and not soldered,
  and nothing in the format says so beyond a note; an assembly listing cannot read a note.
- **A placement is an aspect.** A segment, a net and a signal are what a fact about the
  *wiring* belongs to; a placement is what a fact about the *thing* belongs to, and the language
  properties a `place` takes today — `value`, `footprint`, `intent`, `note`, `reference`,
  `designator`, and purchasing and rating fields — are its facts, spelled as arguments because
  nothing else could state them. As
  an aspect they are stated where any fact is: `set R placement.footprint "R_0805"` in the block
  that knows the watts, the same statement from the board that knows the layout, and `derive`
  and `assert` read them (`R.footprint`, a dissipation against the body's rating). Several open
  questions above are then one mechanism: a shunt's tolerance, fitted-or-not, and what a board
  says about a sheet's part (`d.designate` by path) are placement facts a board states from
  outside. Two things it needs settled. **The combining rule is precedence, not conflict**: a
  block stating 0805 and its board stating 1206 is the normal case, so the outer statement wins
  — the Rust core's "outermost attribution wins" and "composed loses to stated" — and whether
  that rule stays the placement aspect's alone or also reaches `net.name` (today a conflict, by
  the rule that names are the board's) is a decision. And **a statement from outside reaches
  into a placement** (`link/link-led/R`), which is the cross-scope reference above.
  Where nothing states a footprint, it is resolved from `intent` and the stock's policy, as a
  derived fact is.
- **A net has a level, and a board has states.** `rest` (SPEC §9.6) is a direction — high, low,
  float — derived with every rail up, and the engine already sums the pull conductances on a net
  to get it and then discards the numbers. Keeping them gives a *level*: the divider of the
  pulls on the net between their rails and the return, plus the supply of any output pin on it,
  with a diode a drop from anode to cathode and a transistor a clamp from a small model on the
  part. A level is never unknown — a net nothing lifts is at the return — so a per-pin
  `require net.level max="v3v3.voltage.max + 0.3V"` on an MCU's IO table notes on nothing and
  catches a pull-up to the wrong rail. Then a **state** fixes only the externals — which
  controlled rails are up, which switches are closed, what an MCU pin drives — and the engine
  solves the network to a fixpoint over the device models, so a block writes what it *expects*
  (`state press { rail logic down; switch Sw closed; expect Qlatch on }`) and the levels are
  derived rather than hand-calculated. An `expect` is a demand for certainty, so a FET whose
  gate lands between its thresholds fails it with the window in the message. Two states cost
  nothing extra: a rail *down*, and a rail *coming up* with every capacitor pinned at its initial
  voltage, which is the t=0⁺ of a ramp without a time solver. The step after is not assuming the
  rails at all — a buck part already says what EN does — and searching for the board's stable
  states. Open: how a state is named, whether a block declares the rails it switches, how a
  device model is written and how far conduction is worth modelling, and whether a diode's drop
  is a range.

## Engine: behind the specification

Connection-local `require peripheral.<capability>` now checks instance capabilities, including
conditional `has ... when=...`, and requires one candidate to satisfy them together. The
older signal-capability gaps below remain specific to `require signal.*`.


- **A capability an instance lacks is absent, not unstated.** A peripheral table is exhaustive,
  so when the surviving instances of an answer do not `has` what a port `require`s, the answer
  should fail rather than be reported as unknown (SPEC §9.7). Today it is a note: the Snips
  board's VSYS sense on an ADC2 pin says "nothing states `wifi-safe`" where it should say ADC2
  does not have it.
- **A requirement could narrow an answer's instances, and does not.** With a `uart` answered on
  the G0B1's PA2/PA3, both USART2 and LPUART1 survive; USART2 `has lin` and LPUART1 does not, so
  the capability is not stamped and a port's `require signal.lin` is reported as unstated.
  Whether a port's requirement should prune the surviving instances to those that have it — the
  answer is legal, on USART2 — or fail, is part of the capability question above.
- **`require` inside a type's line** (SPEC §8.3) is read and not evaluated; a composed type that
  requires a capability on a line checks nothing until it is. Port-level requirements are.
- **Type-level assertions are not evaluated.** psu-01 clocks three timer channels — the DCC
  sync, the pixel bar's stream and a backlight — at three periods, and a link answered on a pin
  several counters reach cannot say "not the same counter as that one". A composed type with
  legs to the three consumers and an `assert` over their `counter`s is the shape (SPEC §8.3,
  §11.4), and it is unchecked until type-level assertions are; until then the design says it in
  a comment.
- **`once` is not checked.** Two links naming a `once` pin should be a finding (SPEC §12) and
  are not yet, as of the Snips port.
- **A part name that starts with a digit** (`2n7002`) is a KDL number, not a name, and a
  placement of it is a parse error. The library spells such parts with a prefix (`nfet-2n7002`);
  whether the reader should accept a quoted string in name position is open.

## Distribution: settled, partly built

Decided 2026-09-24. Tooling rather than language, so it lives here until it is implemented and
documented beside the CLI. The shape: every library, the core one included, is a named prefix
declared in a manifest and resolved from a path or an exact git commit, and a library is a
directory of KDL plus an optional `.pretty` of footprints. No registry, no lockfile, no semver.

**Built (2026-09-24):** the manifest reader (`stackup_kdl::manifest`, `Document::manifest`) and
the resolver (`stackup::manifest::Prefixes`) — discovery by walking up to the nearest
`manifest.kdl` or `manifest.local.kdl`, `path=` libraries, the local file's three rules (redirect
only, reported on every run as `@name: path (manifest.local.kdl)`, ignored under `--locked`), the
`stackup "<version>"` check, a library's own manifest held to the root's (`name` is the prefix,
its `library` lines must be ones the root names), and `--lib` as the flag form of redirecting
`@stackup`. `find_lib` is gone. A design
with an import missing is not elaborated, since findings over a partial design would be about
the gap. Tests: `stackup-kdl/tests/manifest.rs`, `stackup/tests/manifest.rs`, and
`the_trk_led_board_is_wired_as_arc_wired_it` resolving arc-kdl's real manifests when that tree is
beside this repository (or at `STACKUP_ARC_KDL`). **Built (2026-09-25):** `git=` libraries fetch
exact commits into project-local `.stackup/cache/`; `stackup update [library]` fetches the remote
default branch and edits manifest pins without reformatting. Tests exercise a local Git remote and
the CLI; all three ARC boards pass with local overrides and cached pinned commits. **Not built:**
`stackup pin`/`init` and the `.pretty` half. **To undo:** the resolver still treats
`@stackup` as named by every manifest without a line (`manifest::CORE`), and `--lib` redirects it
from the command line; under the decision below both go, and `@stackup` is a `library` line
like any other.

- **The core library is a library like any other — never embedded.** (User, 2026-09-24,
  reversing the first draft here.) It lives in its own repository (`stackup-eda/library`, beside
  the engine at `stackup-eda/stackup`) and a design pins it in its manifest as
  `library stackup git="…" rev="…"`, exactly as it pins a third-party one; the engine gives the
  prefix no special standing and ships no parts. The import system is the whole distribution
  mechanism, which is why it was designed. What embedding was for — one version of engine and
  library by construction — is answered instead by the manifest's `stackup "<version>"` line
  (KiCad stamps a format version, Cargo an edition) and the library's own manifest stating the
  version its files are written against: a design newer than the engine is refused, and a
  library newer than the engine is reported at its manifest. The nearest-`stackup/`-directory
  walk in `find_lib` went with it — the walk-up catch-all the old `stackup.toml [board]` section
  was killed for. A user fixing a core part still copies the file and imports the copy; nothing
  shadows anything, so both in scope is a loud error at the `use`.
- **Other libraries are prefixes in a manifest.** `manifest.kdl` at the project root:

  ```kdl
  stackup "0.3"
  library arc   path="./lib"
  library ti    git="https://github.com/someone/stackup-ti" rev="a1b2c3d…"
  library stm32 git="https://github.com/alxhub/stackup" dir="library/mcu/stm32" rev="…"
  ```

  Then `use "@arc/lamp"`, resolved by the one rule core uses — prefix, path, `.kdl`. `dir=` is
  a subdirectory of the repository (git only; a `path` already says where), so a repository
  that holds more than a library — stackup's own, with its Rust beside `library/` — serves a
  prefix from a slice of itself, and one repository can serve several prefixes, each at its
  own `dir`. The fetch is one checkout per `(git, rev)` shared by every prefix on it. `@stackup`
  is reserved and needs no line; a single-file design needs no manifest (opt-in, as the sourcing
  check is). Found by walking up to the nearest `manifest.kdl`, which is safe where the board
  section was not: a missing prefix is a loud error naming the manifest used, never a silently
  wrong design. Three constraints: **`rev` is a commit, never a branch or tag**, so the manifest
  is the lockfile (nothing is resolved, so nothing needs locking) and `stackup update @ti` moves
  the pin by editing the manifest in place with the span-preserving editor; **one definition
  per name across the whole load** (a part is an identity, so two revisions of `ina226` would be
  two parts under one name — the existing collision rule, extended across libraries); **the
  root manifest is authoritative for every prefix** — a library's own manifest names what it
  needs so a missing or mismatched dependency is reported, but never introduces a second source.
  Path libraries first (arc is that today, with `./../connectors.kdl`); git later, same manifest.
  Not crates.io: the pivot took designs out of cargo, and fetching data files should not need a
  Rust toolchain.
- **A library is a directory** with a manifest naming it, its stackup version and its
  dependencies, plus `<path>.kdl` reached as `@name/<path>`. The core library is one of these, so
  there is one resolver. The footprint is the part not yet true: the netlist writes footprints
  bare (`SOT-23`, not `Package_TO_SOT_SMD:SOT-23`) and KiCad resolves through a library
  nickname. A library shipping its own land pattern (the XBee, the WROOM) carries a `.pretty`
  beside its KDL, the netlist writes `<nickname>:<name>`, and `pcb`/the plugin registers the
  `.pretty` in the board's `fp-lib-table` under the library's name — KiCad's own way of
  distributing footprints, so no new format. Core parts keep the stock nicknames, a dependency
  on a KiCad version the correspondence tests police at dev time. A standalone library check
  (parses clean, symbols match stock when KiCad is present) lets a library be tested without a
  board.
- **Linking a checkout: two files with different lifetimes** (`go.work`, not `npm link`). The
  committed `manifest.kdl` says the version. A gitignored `manifest.local.kdl` beside it says
  "use my checkout instead": `library ti path="../stackup-ti"`, and `library stackup
  path="~/dev/stackup/lib"` is the same statement aimed at the embedded core, which is what
  replaces `--lib`. Three rules: **an override can only redirect a prefix the manifest already
  names**, never introduce one (Cargo's `[patch]` property — otherwise the design loads for one
  person); **an active override is always visible**, in every `check` summary; **`--locked`
  ignores the local file**, for CI and release. While an override is active the engine compares
  the checkout's HEAD to the pin: matching and clean says nothing; differing or dirty is a
  *note* ("elaborating `@ti` from ../stackup-ti at f9e8d7 (dirty); manifest.kdl pins a1b2c3d").
  `stackup pin @ti` writes HEAD into the manifest, refusing a dirty tree, a commit the remote
  does not have (a pin nobody can fetch is worse than a stale one) and a checkout whose origin
  is not the manifest's URL (the wrong fork). `stackup pin --check` exits nonzero on drift for a
  pre-commit hook or CI. The publish sequence is edit, push, `pin`, commit, so the committed
  manifest never names anything unpushed. A library in the design's own repository is a `path`
  entry and needs no link. A user-level link file keyed by URL (npm's global link) is worth
  adding after the project-local one.

## Notes from boards

- **Snips (thePunderWoman/SnipsControllers).** The first board not written
  here, ported across four latch revisions (PRs #56, #58, #59). The engine's own findings were
  the buck's EN floating (rev1–2) and the pixel on 3.3 V (all); the latch's levels are a hand
  model in the block, and the assertions that earned their keep are the ones written against one
  revision that fired on the next (the press that could not latch, the button node above the
  MCU's input ceiling). One hand-written off-state assertion was wrong: it took a dead 3V3 as a
  hard ground on the button node and reported a self-starting board, where the node is a 100 kΩ
  Thevenin into the rail, the Schottky sinks microamps through it and the inverter's base stays
  at Vbe. The other side's DC solve had it right, which is the case for the level-and-state
  proposal above. Her SPICE then found what neither model looked for: at cell insertion the
  100 nF debounce capacitor holds the button node at 0 V for a millisecond or two — the wrong
  model's condition, true transiently — and the latch gate spikes to VSYS. A "rail coming up,
  capacitors at initial voltage" state reaches that with a DC solve.
