"""Applying a parsed netlist to an open board.

KiCad's own netlist updater is not reachable from a plugin: `BOARD_NETLIST_UPDATER` is not in the
SWIG wrapper (the whole 24k-line wrapper mentions "netlist" only in Gerber plot options), and the
IPC API's `ImportNetlist` is a post-10.0 addition. So the update is done here, part by part.

This is a **sync**, not a merge: afterwards the board holds the design's parts and the design's
nets, and nothing else. What survives a sync is the *layout* — position, rotation, routing, copper
pours, silkscreen — because that is the hand work the design cannot express. What does not survive
is anything claiming to be a part or a net, because those the design does express, and a board that
kept a part the design has dropped would be a board disagreeing with its own schematic.

* a part is matched by **UUID**, held in the footprint's path field, not by its designator — so a
  rebuild that renumbers finds the footprint it placed last time and renames it, leaving position,
  rotation and routing alone;
* an electrical footprint the design no longer places is **deleted**. KiCad `board_only`
  footprints are layout objects and survive without a KDL placement;
* a part whose **footprint** changed swaps bodies in place: the new land arrives on the old one's
  position, rotation and side, and the old body goes. Without this, a design that moved a part to a
  different package kept the old land with the shared pad numbers remapped and the rest silently
  unmapped — a footprint that read as placed and was wrong in every dimension that matters. Routing
  stays where it was, so a track to a pad the new body does not have becomes unconnected, which DRC
  says out loud;
* nets are assigned per pad, and nets that already exist are reused rather than duplicated;
* a net the design has **renamed** takes its copper with it, and every net the design does not have
  is removed — see `_retire`, which is the half of a netlist import that is not about parts at all.

One thing here is not an import at all. A part the design placed *for* a pad — a decoupling
capacitor, a series resistor — arrives carrying `Stackup Anchor`, and a new one is dropped beside
that pad rather than in the pile beside the board. That association is invisible in a netlist by
construction: a decoupling capacitor and the pin it decouples are one node, so the ratline goes to
whichever pad of the rail happens to be nearest and there is nothing to read off it. The design is
the only thing that knows, and placement is the only place the knowledge fits. See `_anchor`.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

import pcbnew

from . import anchor, fplib, netclass, netlist, pour, via
from .netlist import Component, NetClass

#: How far apart to drop newly placed footprints, in KiCad internal units (nm). New parts land in a
#: row beside the board rather than stacked on the origin, so they can be picked up individually —
#: for the ones with nowhere better to be. A part that knows its pad goes there instead; see
#: `anchor.py`, which holds everything about that and is shared with the button that does it on
#: demand.
_SPACING = pcbnew.FromMM(5)


@dataclass
class Outcome:
    placed: list[str] = field(default_factory=list)
    updated: list[str] = field(default_factory=list)
    untouched: int = 0
    #: Footprints deleted because the design does not place them. Listed rather than counted: this
    #: is the one thing a sync does that a person cannot undo by pressing the button again.
    deleted: list[str] = field(default_factory=list)
    #: Parts whose body was swapped because the design chose a different footprint, as
    #: `ref: old → new`. Listed like `deleted`, and for the same reason — the old body and its
    #: field positions do not come back — though the part itself, its place and its nets all
    #: survive.
    rebodied: list[str] = field(default_factory=list)
    nets_created: list[str] = field(default_factory=list)
    #: Pads moved from one net to another. Counted separately from `updated`, which is about a
    #: part's identity: renaming every net on the board changes no reference and no value, and a
    #: report that called that "0 updated" would be lying about the biggest change it just made.
    rewired: int = 0
    #: `old -> new` for each net the design renamed, which is the one thing a person reading this
    #: report cannot check for themselves: the pads look right either way, and what they want to
    #: know is that the track they drew went with them.
    renamed: dict[str, str] = field(default_factory=dict)
    #: Tracks, vias and zones carried across by those renames.
    recoppered: int = 0
    #: Nets retired because the design does not have them.
    nets_removed: list[str] = field(default_factory=list)
    #: Newly placed parts that landed beside the pad they belong to rather than in the pile. Worth
    #: reporting because it is the one thing here a person will not otherwise notice happened: the
    #: parts are simply already where they go.
    anchored: int = 0
    #: Pads and copper left unconnected by those retirements — a track drawn on a net the design
    #: no longer has, or a pad the design stopped mentioning. Reported because it is the sync's one
    #: silent consequence: everything stays exactly where it was and only DRC will mention it.
    orphaned: int = 0
    #: What happened to the net classes — the rules, as against the parts and the nets. See
    #: `netclass.py`.
    classed: netclass.Classed = field(default_factory=netclass.Classed)
    problems: list[str] = field(default_factory=list)

    def summary(self) -> str:
        lines = [
            f"{len(self.placed)} placed, {len(self.updated)} updated, "
            f"{len(self.deleted)} deleted, {self.untouched} left as they were",
            f"{len(self.nets_created)} new net(s), {self.rewired} pad(s) rewired, "
            f"{len(self.nets_removed)} net(s) retired",
        ]
        if self.placed:
            lines.append("\nPlaced: " + ", ".join(sorted(self.placed)))
        if self.anchored:
            lines.append(
                f"{self.anchored} of them landed on the pad they decouple or feed "
                f"(labelled on the `{anchor.LAYER}` layer)."
            )
        if self.updated:
            lines.append("Updated: " + ", ".join(sorted(self.updated)))
        if self.rebodied:
            lines.append("Rebodied: " + ", ".join(sorted(self.rebodied)))
        if self.deleted:
            lines.append("Deleted: " + ", ".join(sorted(self.deleted)))
        if self.orphaned:
            lines.append(
                f"\n{self.orphaned} pad(s) and copper item(s) are now unconnected — they were on a "
                "net the design no longer has."
            )
        if self.renamed:
            lines.append(
                "\nRenamed: "
                + ", ".join(f"{old} → {new}" for old, new in sorted(self.renamed.items()))
                + f" ({self.recoppered} copper item(s) moved with them)"
            )
        if self.classed.changed():
            lines.append("")
            lines.extend(self.classed.summary())
        if self.problems:
            lines.append("\nProblems:\n  " + "\n  ".join(self.problems))
        return "\n".join(lines)


def apply(
    board, components: dict[str, Component], classes: dict[str, NetClass] | None = None
) -> Outcome:
    """Brings `board` in line with `components`, and its rules in line with `classes`."""
    outcome = Outcome()
    project_dir = Path(board.GetFileName()).parent if board.GetFileName() else None

    # Order matters here, and in two places: the parts the design has dropped go first, so the
    # pads read next are the pads of parts that are actually still on the board — and body swaps
    # go before the `was` snapshot below, so an old body's pads never testify in `_retire` about
    # nets they are no longer on. A swapped-in body's pads are on nothing until `_wire`, and
    # nothing is exactly what the rename detection ignores.
    _prune(board, components, outcome)
    existing = _by_uuid(board)
    swapped = _rebody(board, components, existing, project_dir, outcome)
    nets = _nets(board, components, outcome)
    cursor = _free_space(board)
    # Which pads sit on which net before anything moves. That is the only evidence there is that a
    # net was *renamed* rather than deleted and another one coined, and it is gone the moment the
    # first pad is reassigned — so it is taken here and read in `_retire` at the end.
    was = _pads_by_net(board)
    hints = anchor.layer(board)

    #: Everything on the board after this loop, by designator, so the anchoring pass below can find
    #: a part's anchor even when the two arrived in the same sync.
    on_board: dict[str, object] = {}
    fresh: list[tuple[object, Component]] = []

    for component in components.values():
        footprint = existing.get(component.uuid)
        try:
            if footprint is None:
                footprint = _place(board, component, project_dir, cursor, hints)
                cursor = (cursor[0], cursor[1] + _SPACING)
                outcome.placed.append(component.ref)
                fresh.append((footprint, component))
            elif _refresh(footprint, component, hints):
                # A swapped-in body refreshes by construction — its fields are all new — and is
                # already reported as rebodied, which is the stronger statement.
                if component.ref not in swapped:
                    outcome.updated.append(component.ref)
            else:
                outcome.untouched += 1
        except Exception as error:  # one bad part should not abandon the rest
            outcome.problems.append(f"{component.ref} ({component.path}): {error}")
            continue
        on_board[component.ref] = footprint
        _wire(footprint, component, nets, outcome)

    _anchor(fresh, on_board, anchor.slots(_anchors(components)), outcome)
    _retire(board, was, nets, outcome)
    # Last, once the nets are what the design says they are: a class is assigned by net name, so
    # a net renamed above is assigned under its new name and a retired one is not assigned at all.
    outcome.classed = netclass.apply(board, classes or {}, set(nets))
    return outcome


def _prune(board, components: dict[str, Component], outcome: Outcome) -> None:
    """Deletes electrical footprints the design does not place.

    KiCad `board_only` footprints are layout objects, such as mounting holes, fiducials, and
    artwork. Their position and presence are decided in the PCB editor, so sync leaves them alone.
    Other unclaimed footprints are stale electrical parts and are removed.

    This is the one destructive thing a sync does, so it is the one thing the report names outright
    rather than counting.
    """
    wanted = {component.uuid for component in components.values() if component.uuid}
    for footprint in list(board.GetFootprints()):
        if footprint.GetAttributes() & pcbnew.FP_BOARD_ONLY:
            continue
        if _uuid_of(footprint) in wanted:
            continue
        outcome.deleted.append(footprint.GetReference())
        _detach(board, footprint)


def _detach(board, item) -> None:
    """Takes an item off the board without handing Python the corpse.

    `BOARD.Remove` gives the wrapper ownership of the C++ item — but only when no action is
    running, so a plugin press and a headless script disagree — and SWIG has no destructor
    registered for a `FOOTPRINT` or a `NETINFO_ITEM`. Collecting one corrupts SWIG's type registry,
    after which every `GetFootprints()` entry comes back as an untyped `SwigPyObject` and the next
    attribute access raises. `BOARD.Delete` destroys the item outright instead, which is worse
    inside the editor, where the canvas is still holding a pointer to it.

    So: detach, and disown explicitly rather than relying on which of the two `Remove` is feeling
    like today. The item leaks for the rest of the session, which is already what a plugin press
    does — and a session is one button press long.
    """
    board.Remove(item)
    item.thisown = 0


def _by_uuid(board) -> dict[str, object]:
    """Footprints already on the board that stackup placed, keyed by the UUID it gave them."""
    return {_uuid_of(f): f for f in board.GetFootprints() if _uuid_of(f)}


def _uuid_of(footprint) -> str:
    """The UUID stackup gave a footprint, or `""` for one no design placed."""
    path = footprint.GetPath().AsString()
    # A path is `/<sheet>/<component>`; ours has a single element.
    return path.strip("/").split("/")[-1] if path else ""


def _nets(board, components: dict[str, Component], outcome: Outcome) -> dict[str, object]:
    """Every net the design needs, creating the ones the board does not have yet."""
    nets = {}
    for component in components.values():
        for name in component.pads.values():
            if name in nets:
                continue
            net = board.FindNet(name)
            if net is None:
                net = pcbnew.NETINFO_ITEM(board, name)
                board.Add(net)
                outcome.nets_created.append(name)
            nets[name] = net
    return nets


def _rebody(board, components: dict[str, Component], existing: dict[str, object],
            project_dir: Path | None, outcome: Outcome) -> set[str]:
    """Swaps the body of every matched footprint whose library id is not the design's choice.
    Returns the references it swapped.

    The UUID match answers *which part* a footprint is; this answers whether it is still the right
    *body*. A design that moved a part to a different package — a discrete regulator becoming a
    potted brick — used to sail through `_refresh`, which compares identity and fields but never
    the land: the shared pad numbers were remapped onto the old copper and the rest left silently
    unmapped, a footprint that read as synced and was wrong in every dimension that matters.

    What carries over is what the person made: position, rotation, side. What does not is the old
    body's field placement, which was laid out for a different outline and would be wrong beside
    the new one anyway — `_refresh` writes the fields back fresh. Routing is left exactly where it
    is, the same bargain `_retire` strikes: a track to a pad the new body does not have becomes
    unconnected and DRC says so, which beats copper quietly mapped onto the wrong land.
    """
    swapped: set[str] = set()
    for component in components.values():
        old = existing.get(component.uuid)
        if old is None or not component.footprint:
            continue
        # The *item name*, not the full `Library:Name` id: a footprint loaded from a `.pretty`
        # directory — which is how `_load` gets every one, the SWIG wrapper exposing no library
        # table — carries no nickname in its FPID, so comparing full ids would re-body every part
        # on every sync over a prefix neither side means anything by.
        current = str(old.GetFPID().GetLibItemName())
        wanted = component.footprint.rsplit(":", 1)[-1]
        if current == wanted:
            continue
        try:
            fresh = _load(component, project_dir)
        except Exception as error:
            outcome.problems.append(f"{component.ref} ({component.path}): {error}")
            continue
        board.Add(fresh)
        fresh.SetPosition(old.GetPosition())
        if old.IsFlipped():
            fresh.Flip(fresh.GetPosition(), False)
        # After the flip, so a back-side part keeps its angle rather than having it mirrored.
        fresh.SetOrientation(old.GetOrientation())
        _detach(board, old)
        existing[component.uuid] = fresh
        swapped.add(component.ref)
        outcome.rebodied.append(f"{component.ref}: {current} → {wanted}")
    return swapped


def _load(component: Component, project_dir: Path | None):
    """Loads the footprint the design chose for a part from its library."""
    if not component.footprint:
        raise fplib.LibraryError("no footprint — give the board `set_packages` or the part one")
    library, name = fplib.load_path(component.footprint, project_dir)
    footprint = pcbnew.PCB_IO_KICAD_SEXPR().FootprintLoad(str(library), name)
    if footprint is None:
        raise fplib.LibraryError(f"`{name}` is not in {library}")
    return footprint


def _place(board, component: Component, project_dir: Path | None, at: tuple[int, int], hints):
    """Loads a footprint from its library and adds it to the board."""
    footprint = _load(component, project_dir)
    board.Add(footprint)
    footprint.SetPosition(pcbnew.VECTOR2I(at[0], at[1]))
    _refresh(footprint, component, hints)
    return footprint


def _refresh(footprint, component: Component, hints) -> bool:
    """Brings an existing footprint's identity in line with the design. Returns whether anything
    changed — position and rotation are never among them."""
    changed = False
    if footprint.GetReference() != component.ref:
        footprint.SetReference(component.ref)
        changed = True
    if footprint.GetValue() != component.value:
        footprint.SetValue(component.value)
        changed = True
    # The UUID link, so the next sync finds this footprint however the designators have moved.
    path = pcbnew.KIID_PATH()
    path.push_back(pcbnew.KIID(component.uuid))
    if footprint.GetPath().AsString() != path.AsString():
        footprint.SetPath(path)
        changed = True

    # What the design knows about this part, carried onto the footprint so it is there when someone
    # clicks it during layout. `Stackup Path` is the id every CLI command takes, `Stackup Why` is
    # the provenance the intent left — the answer to "what is this capacitor even for", at the
    # moment the question actually gets asked, which is while laying the board out — and
    # `Stackup Source` is the board line that asked for the part, so the answer to "then change it"
    # is a file and a line rather than a hunt through the design.
    # The part's specification first: what to buy. Plain names, so the columns read as themselves
    # in the Symbol Fields Table and in `kicad-cli sch export bom --fields Voltage,Dielectric,…`.
    # Hidden like everything else here — a rating belongs in the properties dialog, not silkscreened
    # across the copper — and emptied by `_set_field` if the design stops having an answer, so a
    # re-rated part cannot leave last time's number on the board.
    changed |= _set_field(footprint, "Voltage", component.voltage)
    changed |= _set_field(footprint, "MF", component.manufacturer)
    changed |= _set_field(footprint, "Manufacturer_Part_Number", component.mpn)
    changed |= _set_field(footprint, "LCSC", component.lcsc)
    changed |= _set_field(footprint, "Mouser", component.mouser)
    changed |= _set_field(footprint, "DigiKey", component.digikey)
    changed |= _set_field(footprint, "Series", component.series)
    changed |= _set_field(footprint, "Dissipation", component.dissipation)
    changed |= _set_field(footprint, "Current", component.current)
    changed |= _set_field(footprint, "Tolerance", component.tolerance)
    changed |= _set_field(footprint, "Dielectric", component.dielectric)
    changed |= _set_field(footprint, "Stackup Path", component.path)
    changed |= _set_field(footprint, "Stackup Source", component.source)
    changed |= _set_field(footprint, "Stackup Why", component.why)
    # The anchor is the one field that is worth *seeing*, so it goes on the hint layer visibly
    # rather than into the properties dialog with the rest. Which pad a capacitor belongs to is a
    # question asked while dragging it, and an answer you have to click twice for is not an answer
    # at that moment. The layer is documentation, never fabricated, and toggling it off is one
    # click once the placement is done.
    changed |= _set_field(footprint, anchor.FIELD, component.anchor, layer=hints)
    # The exact placement rides along hidden. It is written even though the sync has just used it,
    # because the *Place at anchor pad* button reads everything it needs off the footprint and runs
    # with no cargo and no netlist — so a spot that lived only in the netlist would be a spot that
    # button could not honour.
    changed |= _set_field(footprint, anchor.SPOT_FIELD, component.spot)
    # The copper hung off this part's pads, hidden like the spot and for the same reason: it is data
    # for the button rather than a label, and it is long. Written here so *Place at anchor pad* can
    # draw a zone with no cargo run behind it.
    changed |= _set_field(footprint, pour.FIELD, component.pours)
    # The vias under this part's pads, for the same button and the same reason.
    changed |= _set_field(footprint, via.FIELD, component.vias)
    # The bodies, where the design has something to say. It says nothing for nearly every part,
    # and nothing means the land's own `(model …)` stays — so this never strips a body a person
    # added by hand to a part the design has no opinion about. A bundled land always says, since
    # it is stackup's and a body it dropped has to leave the board too.
    changed |= _set_models(footprint, component.models)
    return changed


def _set_models(footprint, spec: str) -> bool:
    """Draws the footprint with the bodies the design named, replacing the land's own. Returns
    whether anything changed. An empty spec leaves the footprint alone."""
    if not spec:
        return False
    wanted = netlist.parse_models(spec)
    if not wanted:
        if not footprint.Models():
            return False
        footprint.Models().clear()
        return True
    # The variable the stock footprints on this board use: the running KiCad's own. Every
    # `KICAD<N>_3DMODEL_DIR` resolves in the editor, but a board is read by people too, and one
    # spelling per board is the courtesy.
    major = pcbnew.GetMajorMinorVersion().split(".")[0]
    kicad_var = f"KICAD{major}_3DMODEL_DIR"
    current = [
        (m.m_Filename, _xyz(m.m_Offset), _xyz(m.m_Rotation)) for m in footprint.Models()
    ]
    target = [(m.path(kicad_var), m.offset, m.rotate) for m in wanted]
    if current == target:
        return False
    models = footprint.Models()
    models.clear()
    for m in wanted:
        model = pcbnew.FP_3DMODEL()
        model.m_Filename = m.path(kicad_var)
        model.m_Offset = pcbnew.VECTOR3D(*m.offset)
        model.m_Rotation = pcbnew.VECTOR3D(*m.rotate)
        model.m_Scale = pcbnew.VECTOR3D(1.0, 1.0, 1.0)
        model.m_Show = True
        models.push_back(model)
    return True


def _xyz(v) -> tuple[float, float, float]:
    return (float(v.x), float(v.y), float(v.z))


def _set_field(footprint, name: str, value: str, layer=None) -> bool:
    """Sets a footprint field. Returns whether it changed.

    Hidden by default, and that matters: a visible field is drawn on the board, and provenance
    belongs in the properties dialog rather than silkscreened across the copper. Passing `layer`
    is the exception — it makes the field visible on that layer, for the one field whose whole
    value is being readable on the canvas.

    A newly created field is dropped just below the footprint so it does not land on top of the
    reference; an existing one is left exactly where it is, because by then it may have been moved
    by hand and that is layout like any other.

    A field the design no longer has anything to say for is **emptied and hidden** rather than left
    reading whatever it said last time. Deleting it outright is not on offer — `RemoveField` is one
    more thing missing from the SWIG wrapper — and a stale value is worse here than anywhere else in
    this file, because a stale anchor is *drawn on the board*, pointing at the pad a part used to be
    for.
    """
    had = footprint.HasField(name)
    if not value:
        if not had or not footprint.GetFieldText(name):
            return False
        footprint.SetField(name, "")
        footprint.GetField(name).SetVisible(False)
        return True
    if had and footprint.GetFieldText(name) == value:
        return False
    fresh = not had
    footprint.SetField(name, value)
    field = footprint.GetField(name)
    field.SetVisible(layer is not None)
    if layer is not None:
        field.SetLayer(layer)
        field.SetTextSize(pcbnew.VECTOR2I(anchor.TEXT_HEIGHT, anchor.TEXT_HEIGHT))
        field.SetTextThickness(int(anchor.TEXT_HEIGHT / 6))
    if fresh:
        at = footprint.GetPosition()
        field.SetPosition(pcbnew.VECTOR2I(at.x, at.y + anchor.GAP))
    return True


def _anchor(fresh, on_board: dict[str, object], slots: dict[str, int], outcome: Outcome) -> None:
    """Lands each newly placed part beside the pad it belongs to.

    The whole answer to *which pin was this capacitor for* — see `anchor.py`, which holds the
    reasoning and the geometry, since the *Place at anchor pad* button wants both as well.

    **Only newly placed parts here.** A footprint already on the board has been positioned, and
    position is hand work that outlives any regeneration — the same rule the whole sync runs on.
    This is the starting point a part gets instead of the pile beside the board, never an opinion
    about where it should stay. Moving one that already exists is the button's business, because
    there a person asked for it.
    """
    for footprint, component in fresh:
        target = anchor.reference(component.anchor)
        if target is None:
            continue
        host = on_board.get(target[0])
        # A part can be anchored on something that failed to place, and a pad number is only true
        # of the body the design chose — if the board is on an older footprint it may not have it.
        if host is None or host is footprint:
            continue
        if anchor.place(
            footprint,
            host,
            target[1],
            slots.get(component.ref, 0),
            anchor.offset(component.spot),
        ):
            outcome.anchored += 1


def _anchors(components: dict[str, Component]):
    """`(designator, anchor)` for every part in the netlist, in the order the design placed them.

    Every part, not merely the ones about to be placed: the slot a capacitor takes on its pad is
    taken over all of them, so a part added in a later sync lands beside its neighbour rather than
    on top of it.
    """
    return [(component.ref, component.anchor) for component in components.values()]


def _wire(footprint, component: Component, nets: dict[str, object], outcome: Outcome) -> None:
    """Assigns each pad the net the design puts it on."""
    for pad in footprint.Pads():
        name = component.pads.get(str(pad.GetNumber()))
        if name is None:
            # A pad the design says nothing about (a mounting pad, a shield tab) keeps whatever it
            # has rather than being cleared.
            continue
        net = nets.get(name)
        if net is not None and pad.GetNetname() != name:
            pad.SetNet(net)
            outcome.rewired += 1


def _retire(board, was: dict[str, list], nets: dict[str, object], outcome: Outcome) -> None:
    """Follows the design's renames with the copper, and drops every net the design does not have.

    Without this a sync propagates a rename only as far as the pads. The nets themselves are
    create-only, so `GND` survives as an empty `NETINFO_ITEM` — cluttering every net dropdown until
    the board is closed and reopened — and, on a routed board, every track and zone drawn on it
    stays behind while its pads leave, quietly taking the layout apart.

    KiCad offers no net rename a plugin can use. `NETINFO_ITEM.SetNetname` does carry the copper,
    because copper holds the net rather than its name, but it renames the item behind the list's
    back: `FindNet` keeps answering to the old name and never learns the new one. Removing the item
    and re-adding it under the new name keeps the list honest and drops every item on it to netcode
    0. So the rename is done the long way round — each track, via and zone moved across by hand,
    and the emptied net then removed, which the wrapper does support.
    """
    landed = {name: {str(pad.GetNetname()) for pad in pads} for name, pads in was.items()}

    # Snapshot before moving anything, so every rename reads the board as the pads left it rather
    # than as an earlier rename in the same pass rearranged it.
    on_net: dict[str, list] = {}
    for item in _connected(board):
        on_net.setdefault(str(item.GetNetname()), []).append(item)

    for old, new in _renames(landed, nets).items():
        for item in on_net.pop(old, []):
            item.SetNet(nets[new])
            outcome.recoppered += 1
        outcome.renamed[old] = new

    # Then the nets themselves. The design's list is the whole list, so anything not on it goes,
    # copper or no copper — the same rule `_prune` applies to footprints. `BOARD.Remove` drops
    # whatever was still on the net to netcode 0, which is the honest outcome: a track drawn on a
    # net the design has dropped is unconnected, and unconnected is a thing DRC says out loud.
    for net in list(board.GetNetInfo().NetsByNetcode().values()):
        name = str(net.GetNetname())
        if not name or name in nets:
            continue  # the unconnected net, and everything the design still wants
        outcome.orphaned += len(on_net.get(name, []))
        _detach(board, net)
        outcome.nets_removed.append(name)


def _renames(was: dict[str, set], wanted) -> dict[str, str]:
    """`old -> new` for every net whose pads moved, as a body, to one net the design does want.

    Pure so it can be reasoned about without a board. A net is renamed only when the pads are
    unanimous: one that split across two nets was not renamed but rewired, and one still holding a
    pad the design says nothing about (`_wire` leaves those alone) has not moved as a body. Neither
    is renamed, because in neither case is there a single answer to move the copper to — and the
    net goes anyway, so the choice is between copper that is *unconnected* and copper that is on
    the *wrong net*. Unconnected loses a route and says so; wrong is a short nobody is told about.

    The nameless net is not a net. Pads that were on nothing are on nothing severally, so their all
    landing on one rail says only that the design wired them — not that the board's unconnected
    items should be dragged onto it.
    """
    renames = {}
    for old, landed in was.items():
        if not old or old in wanted or len(landed) != 1:
            continue
        new = next(iter(landed))
        if new and new != old and new in wanted:
            renames[old] = new
    return renames


def _pads_by_net(board) -> dict[str, list]:
    """Every pad on the board, grouped by the net it is on right now."""
    grouped: dict[str, list] = {}
    for footprint in board.GetFootprints():
        for pad in footprint.Pads():
            grouped.setdefault(str(pad.GetNetname()), []).append(pad)
    return grouped


def _connected(board):
    """Everything on the board that carries a net: pads, tracks, vias, zones."""
    for footprint in board.GetFootprints():
        yield from footprint.Pads()
        yield from footprint.Zones()
    yield from board.Tracks()  # vias and arcs are tracks
    yield from board.Zones()


def _free_space(board) -> tuple[int, int]:
    """A spot clear of whatever is already on the board, for new parts to land."""
    box = board.GetBoardEdgesBoundingBox()
    if box.GetWidth() == 0 and box.GetHeight() == 0:
        box = board.GetBoundingBox()
    return (box.GetRight() + _SPACING * 2, box.GetTop())
