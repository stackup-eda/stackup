"""Reading the netlist the stackup CLI writes.

The CLI emits KiCad's own netlist format, so this parses that rather than some private JSON — one
export format, and the same file a person can hand to `File > Import > Netlist` if they ever want
to bypass the button.

The parser is deliberately tiny. A KiCad netlist is a single s-expression with no dotted pairs, no
comments and no numbers-as-tokens, so a reader is a few dozen lines and needs no dependency — which
matters, because this runs inside KiCad's bundled Python where installing anything is not ours to
do.
"""

from __future__ import annotations

from dataclasses import dataclass, field


@dataclass
class Component:
    """One `(comp …)`: a part to place, and how to find it again next time."""

    ref: str
    value: str = ""
    footprint: str = ""
    #: The path-derived UUID from `(tstamps …)`. This — not the designator — is what a part is
    #: matched by, so a rebuild that renumbers does not detach the layout.
    uuid: str = ""
    #: What the part must survive, and what grade it is — the *specification*, as opposed to the
    #: provenance below. These are what make a footprint orderable: a 100 nF 0402 is sold at six
    #: working voltages in three dielectrics, and a board that does not carry which is a board whose
    #: BOM comes back as whatever was cheapest. Resolved values, so a rating the design's derating
    #: policy arrived at rides across beside one a placement stated. Empty where the design has
    #: nothing to say, which `_set_field` turns into an emptied, hidden field rather than a stale
    #: one.
    voltage: str = ""
    dissipation: str = ""
    current: str = ""
    tolerance: str = ""
    dielectric: str = ""
    #: The design path (`astable/timer/Ra`) — what `why` and `trace` address.
    path: str = ""
    #: The `file:line` that asked for this part — the board line to edit to change it.
    source: str = ""
    #: Why the design placed this part, as the intent that placed it recorded it.
    why: str = ""
    #: The pad this part belongs beside, as `designator.pad` (`U1.49`) — empty for a part that
    #: stands on its own. The one thing here that is not in the nets and cannot be: a decoupling
    #: capacitor and the pin it decouples are the same node, so connectivity has no way to say
    #: which pad of a rail this capacitor is for. Read it with `anchor.reference`, which is also
    #: what reads it back off a footprint — one spelling, one parser.
    anchor: str = ""
    #: An *exact* placement beside that pad, for the few parts a datasheet draws rather than
    #: describes: `dx dy rotation` in the host's own frame. Empty for everything else, which is
    #: nearly everything — near the pad is the whole requirement for a decoupling capacitor, and a
    #: design that pinned down what it does not care about would be one nobody could lay out.
    spot: str = ""
    #: The copper this part's pads carry, as `pad|side|kind|priority|points` joined by `;`. A zone
    #: is neither a component nor a net, so a netlist has no place of its own for one — it rides on
    #: the part its outline is measured from. Read it with `pour.parse`.
    pours: str = ""
    #: The vias this part's pads carry — an exposed pad's heat path — as
    #: `pad|drill|diameter|tenting|points` joined by `;`, on the part for the reason a pour is.
    #: Read it with `via.parse`.
    vias: str = ""
    #: The 3D bodies to draw this part with *instead of* whatever its footprint names, as
    #: `where:file|ox oy oz|rx ry rz` joined by `;` — empty for nearly every part, which keeps the
    #: land's own. Stated only where the land's body is a file KiCad does not ship: a name its 3D
    #: library changed, a drawing never made, a bundled land with no install to name one from. Read
    #: it with `parse_models`.
    models: str = ""
    #: pad number -> net name
    pads: dict[str, str] = field(default_factory=dict)


@dataclass
class NetClass:
    """One `(netclass …)` under `(design …)`: a class the design derived, and the patterns that
    assign nets to it — see `netclass.py` for what the board does with one.

    A rule class sets every parameter — width, clearance, via diameter and drill — so a net in
    it is held to the board's stated copper and to nothing left to `Default`; a kind class
    (`stackup:power`) sets none and is there for its name, what a custom DRC rule matches on. An
    empty string is *unset*, which is how KiCad composes a kind with the rule beside it.
    Millimetres, as the strings the netlist carries them as."""

    name: str
    description: str = ""
    track_width: str = ""
    clearance: str = ""
    via_diameter: str = ""
    via_drill: str = ""
    #: The net-name patterns assigned to this class, in KiCad's own spelling: `vlink/*` for
    #: everything under a path, or an exact name. Written straight into the project's pattern
    #: table, so Board Setup reads the way a person would have written it.
    patterns: list[str] = field(default_factory=list)


def parse(text: str) -> dict[str, Component]:
    """Parses a netlist into components keyed by reference designator."""
    tree = _sexpr(text)
    if not tree or tree[0] != "export":
        raise ValueError("not a KiCad netlist: expected a leading (export …)")

    components: dict[str, Component] = {}
    for comp in _children(_section(tree, "components"), "comp"):
        ref = _value(comp, "ref")
        if not ref:
            continue
        component = Component(
            ref=ref,
            value=_value(comp, "value"),
            footprint=_value(comp, "footprint"),
            uuid=_value(comp, "tstamps"),
        )
        for prop in _children(comp, "property"):
            name = _value(prop, "name")
            # The specification fields are plain-named, because they are facts about the component
            # rather than about the design — the same category as `value` and `datasheet`, neither
            # of which is namespaced. The `Stackup …` ones below are stackup's own account.
            if name == "Voltage":
                component.voltage = _value(prop, "value")
            elif name == "Dissipation":
                component.dissipation = _value(prop, "value")
            elif name == "Current":
                component.current = _value(prop, "value")
            elif name == "Tolerance":
                component.tolerance = _value(prop, "value")
            elif name == "Dielectric":
                component.dielectric = _value(prop, "value")
            elif name == "Stackup Path":
                component.path = _value(prop, "value")
            elif name == "Stackup Source":
                component.source = _value(prop, "value")
            elif name == "Stackup Why":
                component.why = _value(prop, "value")
            elif name == "Stackup Anchor":
                component.anchor = _value(prop, "value")
            elif name == "Stackup Spot":
                component.spot = _value(prop, "value")
            elif name == "Stackup Pours":
                component.pours = _value(prop, "value")
            elif name == "Stackup Vias":
                component.vias = _value(prop, "value")
            elif name == "Stackup Models":
                component.models = _value(prop, "value")
        components[ref] = component

    # Connectivity is stored per net; the board is updated per pad, so invert it here.
    for net in _children(_section(tree, "nets"), "net"):
        name = _value(net, "name")
        for node in _children(net, "node"):
            ref, pad = _value(node, "ref"), _value(node, "pin")
            if ref in components and pad:
                components[ref].pads[pad] = name

    return components


@dataclass
class Model3D:
    """One body off `Component.models`: a file and how it sits on the land."""

    #: `kicad` for a file under the install's `3dmodels/`, `project` for one beside the board.
    where: str
    #: The file, relative to that root.
    file: str
    offset: tuple[float, float, float] = (0.0, 0.0, 0.0)
    rotate: tuple[float, float, float] = (0.0, 0.0, 0.0)

    def path(self, kicad_var: str) -> str:
        """The path a `.kicad_pcb` spells this as. `kicad_var` is the `KICAD<N>_3DMODEL_DIR` of
        the KiCad applying it — the stock footprints beside it use that one, and a board that
        spells the same directory two ways is a board whose bodies half-load."""
        root = "${%s}" % kicad_var if self.where == "kicad" else "${KIPRJMOD}"
        return f"{root}/{self.file}"


#: The spec that means *draw nothing*: a bundled land with no body, which has to reach a footprint
#: placed when it still named one. Distinct from an empty spec, which says nothing at all.
NO_MODELS = "none"


def parse_models(text: str) -> list[Model3D]:
    """Reads `Component.models`. Empty text is no bodies, which means *leave the land's own*;
    `NO_MODELS` is also no bodies, and means *strip them* — `apply` tells the two apart."""
    out: list[Model3D] = []
    if text == NO_MODELS:
        return out
    for entry in filter(None, text.split(";")):
        spec, _, rest = entry.partition("|")
        where, _, file = spec.partition(":")
        if where not in ("kicad", "project") or not file:
            raise ValueError(f"not a 3D body spec: {entry!r}")
        offset, _, rotate = rest.partition("|")
        triple = lambda s: tuple(float(v) for v in s.split()) if s.strip() else (0.0, 0.0, 0.0)
        ox, rx = triple(offset), triple(rotate)
        if len(ox) != 3 or len(rx) != 3:
            raise ValueError(f"not a 3D body spec: {entry!r}")
        out.append(Model3D(where, file, ox, rx))
    return out


def classes(text: str) -> dict[str, NetClass]:
    """Parses the net classes a netlist carries, keyed by name, each with its patterns.

    The definitions live under `(design …)` because KiCad keeps net classes in the project rather
    than the netlist, so the format has no slot of its own for one — and `design` is the section
    KiCad's own reader skips whole, so ours costs an import nothing. The assignments are the
    patterns on each class, which is KiCad's own model; the `(class …)` on each net is written for
    a reader and not read back here.
    """
    tree = _sexpr(text)
    if not tree or tree[0] != "export":
        raise ValueError("not a KiCad netlist: expected a leading (export …)")
    found: dict[str, NetClass] = {}
    for entry in _children(_section(tree, "design"), "netclass"):
        name = _value(entry, "name")
        if name:
            found[name] = NetClass(
                name=name,
                description=_value(entry, "description"),
                track_width=_value(entry, "track_width"),
                clearance=_value(entry, "clearance"),
                via_diameter=_value(entry, "via_diameter"),
                via_drill=_value(entry, "via_drill"),
                patterns=[
                    child[1]
                    for child in _children(entry, "pattern")
                    if len(child) > 1 and isinstance(child[1], str)
                ],
            )
    return found


def nets_of(components: dict[str, Component]) -> set[str]:
    """Every net name mentioned, so they can be created before anything is assigned to them."""
    return {net for component in components.values() for net in component.pads.values()}


# --- a very small s-expression reader ---------------------------------------
#
# A list becomes a Python list, an atom a string. That is the whole model: the netlist has no other
# kinds of value in it.


def _sexpr(text: str):
    tokens = _tokenize(text)
    position = 0

    def parse_at(index: int):
        nonlocal position
        position = index
        if tokens[position] != "(":
            raise ValueError(f"expected ( at token {position}")
        position += 1
        out = []
        while position < len(tokens):
            token = tokens[position]
            if token == ")":
                position += 1
                return out
            if token == "(":
                out.append(parse_at(position))
            else:
                out.append(token)
                position += 1
        raise ValueError("unbalanced parentheses")

    return parse_at(0)


def _tokenize(text: str) -> list[str]:
    tokens: list[str] = []
    index = 0
    while index < len(text):
        char = text[index]
        if char in "()":
            tokens.append(char)
            index += 1
        elif char.isspace():
            index += 1
        elif char == '"':
            index += 1
            out = []
            while index < len(text) and text[index] != '"':
                if text[index] == "\\" and index + 1 < len(text):
                    index += 1
                    out.append({"n": "\n", "r": "\r", "t": "\t"}.get(text[index], text[index]))
                else:
                    out.append(text[index])
                index += 1
            index += 1
            tokens.append("".join(out))
        else:
            start = index
            while index < len(text) and not text[index].isspace() and text[index] not in '()"':
                index += 1
            tokens.append(text[start:index])
    return tokens


def _children(node, head: str) -> list:
    """Direct child lists whose head is `head`."""
    if not node:
        return []
    return [item for item in node if isinstance(item, list) and item and item[0] == head]


def _section(tree, head: str):
    found = _children(tree, head)
    return found[0] if found else []


def _value(node, head: str) -> str:
    """The first value of the first `(head value)` child — `""` when absent, since every caller
    treats a missing field and an empty one the same way."""
    for child in _children(node, head):
        if len(child) > 1 and isinstance(child[1], str):
            return child[1]
    return ""
