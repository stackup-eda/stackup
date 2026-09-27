"""Copper a design asked for: the islands hung off a pin, and the holes cut under one.

The sibling of `anchor.py`, and the same argument one step further. An anchor says *which pad this
capacitor is for*, which a netlist cannot say because the capacitor and the pad are one node. A pour
says *what shape the copper on this node is around that pad*, which a netlist cannot say because it
has components and nets and no third thing at all — a zone is neither.

Both are placement, and both arrive as a field on the footprint the geometry is measured from:
`Stackup Pours`, one entry per pour, `pad|side|kind|priority|x,y x,y …`. Living on the footprint is
what lets this run with no cargo and no netlist, the same as the anchor button.

**It fires on a selected pad, not a selected part.** Pouring copper across a chip is not something
to do by accident, and the pin is the unit the design declared in anyway: select `U1` pad 47, press
*Place at anchor pad*, and the regulator's return island appears on that pad's net. Nothing happens
to a part you merely selected.

Two things this deliberately does not decide. It does not fill the zones — that is `B`, and doing
it here would mean a filler pass on every press. And it does not touch a zone it did not create:
every zone it makes is named `stackup:U1.47#0`, and a second press replaces exactly those, so
copper you drew yourself is yours.
"""

from __future__ import annotations

import math

import pcbnew

from .anchor import pad_origin

#: The footprint field the outlines ride on, written by the sync from `Design::pours`.
FIELD = "Stackup Pours"

#: How a zone this places is named, and what makes a second press an update rather than a
#: duplicate. The pad is in it because that is what the outline is measured from, and the index
#: because one pad can carry more than one pour (an island and the hole under it).
NAME = "stackup:{designator}.{pad}#{index}"

#: Fill settings for a placed island. A local island exists to keep a switching loop small, and a
#: thermal relief on the return of one is precisely the wrong thing — so pads connect solid, and the
#: clearance and minimum width are the reference design's rather than the board's defaults, which
#: are chosen for a plane rather than for a 0.4 mm pin pitch.
CLEARANCE = pcbnew.FromMM(0.15)
MIN_THICKNESS = pcbnew.FromMM(0.15)


def parse(text: str) -> list[dict]:
    """Reads the field into a list of pours, skipping anything that will not read.

    Malformed entries are dropped rather than raised on, for the reason `anchor.offset` gives: this
    is a placement preference, and the cost of one that cannot be read is that copper does not get
    drawn — not that a button stops working. What is *not* skipped is a pour that reads fine and
    resolves to no layer; that one is reported, because it is a real answer about the board.
    """
    pours = []
    for index, entry in enumerate((text or "").split(";")):
        entry = entry.strip()
        if not entry:
            continue
        parts = entry.split("|")
        if len(parts) != 5:
            continue
        pad, side, kind, priority, points = parts
        outline = _points(points)
        if not pad or kind not in ("copper", "keepout") or len(outline) < 3:
            continue
        try:
            rank = int(priority)
        except ValueError:
            continue
        pours.append(
            {
                "index": index,
                "pad": pad,
                "side": side,
                "kind": kind,
                "priority": rank,
                "outline": outline,
            }
        )
    return pours


def _points(text: str) -> list[tuple[float, float]]:
    out = []
    for pair in text.split():
        x, _, y = pair.partition(",")
        try:
            out.append((float(x), float(y)))
        except ValueError:
            return []
    return out


def copper_stack(board) -> list[int]:
    """The board's copper layers, front to back.

    By *name* rather than by layer id arithmetic. The ids are not contiguous and were renumbered
    between KiCad 8 and 9 — `B.Cu` moved from 31 to 2, and the inner layers from 1,2,3… to 4,6,8…
    — so anything that counts them is version-specific in a way that fails silently. The names have
    not changed since the format existed.
    """
    count = board.GetCopperLayerCount()
    names = ["F.Cu"] + [f"In{n}.Cu" for n in range(1, max(count - 1, 1))] + ["B.Cu"]
    return [board.GetLayerID(name) for name in names[:count]]


def layer_of(stack: list[int], on_back: bool, side: str) -> int | None:
    """Resolves a design's `Side` against a real stackup. `None` if the board has no such layer.

    Pure, so the one part of this that is arithmetic can be checked without a board.

    `own` and `opposite` exist on every board. `innerN` does not, and that asymmetry is carried
    rather than smoothed over: RP2350 §6.3.8.1 asks for its copper cut-out only "for a multi-layer
    board (4 or more layers)", so a two-layer jig should get *no* cut-out — not one on the back,
    which is where counting layers inward from the front would put it, and which on that board is
    the ground plane the datasheet wants left alone.
    """
    if not stack:
        return None
    if side == "own":
        return stack[-1] if on_back else stack[0]
    if side == "opposite":
        return stack[0] if on_back else stack[-1]
    if side.startswith("inner"):
        try:
            n = int(side[len("inner") :])
        except ValueError:
            return None
        if n < 1 or n > len(stack) - 2:
            return None
        return stack[-1 - n] if on_back else stack[n]
    return None


def outline_at(
    pad: tuple[int, int], host_rotation: float, points: list[tuple[float, float]]
) -> list[tuple[int, int]]:
    """The outline in board coordinates: each point turned by the host's rotation and offset.

    The same transform `anchor.exact` uses, applied to a polygon instead of a position — which is
    the whole reason a pour and a spot share a convention. Pure, and in internal units.
    """
    theta = math.radians(host_rotation)
    out = []
    for dx, dy in points:
        x = pad[0] + pcbnew.FromMM(dx) * math.cos(theta) + pcbnew.FromMM(dy) * math.sin(theta)
        y = pad[1] - pcbnew.FromMM(dx) * math.sin(theta) + pcbnew.FromMM(dy) * math.cos(theta)
        out.append((int(round(x)), int(round(y))))
    return out


class Poured:
    """What pouring did, in the terms its report needs."""

    def __init__(self):
        self.pads = 0
        #: `U1.47` for each pad whose copper was drawn.
        self.filled: list[str] = []
        #: Zones — and vias, which `via.apply` reports into the same outcome — removed because
        #: they were replaced: counted, not listed, since the replacement is the interesting half.
        self.replaced = 0
        #: `U1.17 (15)` for each pad whose vias were placed, with how many.
        self.vias: list[str] = []
        #: Pours that read correctly and had nowhere to go: an inner layer on a two-layer board.
        #: Not a failure, and worth saying rather than swallowing — it is the board answering.
        self.skipped: list[str] = []
        self.problems: list[str] = []

    def touched(self) -> bool:
        return bool(self.filled or self.vias or self.skipped or self.problems)


def selected_pads(board):
    """Every selected pad, with the footprint it belongs to."""
    found = []
    for footprint in board.GetFootprints():
        for pad in footprint.Pads():
            if pad.IsSelected():
                found.append((footprint, pad))
    return found


def apply(board, outcome: Poured) -> Poured:
    """Draws the copper the design declared for every selected pad."""
    for footprint, pad in selected_pads(board):
        outcome.pads += 1
        designator = footprint.GetReference()
        number = pad.GetNumber()
        text = footprint.GetFieldText(FIELD) if footprint.HasField(FIELD) else ""
        wanted = [p for p in parse(text) if p["pad"] == number]
        if not wanted:
            continue
        stack = copper_stack(board)
        on_back = footprint.IsFlipped()
        # The number's position, not the clicked pad's. They differ on an exposed thermal pad, whose
        # one number is several copper pads: selecting the land and selecting a via barrel are the
        # same request, and an outline that moved 1.3 mm depending on which of eight overlapping
        # items KiCad's disambiguation menu handed back would be the one kind of wrong this whole
        # mechanism exists to rule out. The selection says *which* pour to place; the number says
        # what it is measured from.
        at = pad_origin(footprint, number)
        if at is None:  # unreachable — the pad is one this footprint just yielded
            continue
        drew = False
        for spec in wanted:
            name = NAME.format(designator=designator, pad=number, index=spec["index"])
            outcome.replaced += _remove(board, name)
            layer = layer_of(stack, on_back, spec["side"])
            if layer is None:
                outcome.skipped.append(
                    f"{designator}.{number} wants {spec['kind']} on `{spec['side']}`, "
                    f"which a {len(stack)}-layer board does not have"
                )
                continue
            try:
                _zone(board, name, layer, pad, at, footprint, spec)
            except Exception as error:  # a zone that will not build should not take the rest down
                outcome.problems.append(
                    f"{designator}.{number}: {type(error).__name__}: {error}"
                )
                continue
            drew = True
        if drew:
            outcome.filled.append(f"{designator}.{number}")
    return outcome


def _remove(board, name: str) -> int:
    """Deletes the zones this placed under `name`, so a second press updates rather than doubles."""
    doomed = [z for z in board.Zones() if _name_of(z) == name]
    for zone in doomed:
        board.Remove(zone)
    return len(doomed)


def _name_of(zone) -> str:
    try:
        return str(zone.GetZoneName())
    except Exception:
        return ""


def _zone(board, name: str, layer: int, pad, at, footprint, spec) -> None:
    zone = pcbnew.ZONE(board)
    zone.SetLayer(layer)
    if spec["kind"] == "keepout":
        # A rule area: the content is that nothing is here, so it takes no net and forbids the fill.
        # Only copper is forbidden — a cut-out under `VREG_LX` is about the plane beneath the node,
        # not about whether a via may pass through the board somewhere in that rectangle.
        zone.SetIsRuleArea(True)
        _call(zone, ("SetDoNotAllowCopperPour", "SetDoNotAllowZoneFills"), True)
        for forbid in ("SetDoNotAllowTracks", "SetDoNotAllowVias", "SetDoNotAllowPads"):
            _call(zone, (forbid,), False)
    else:
        zone.SetNetCode(pad.GetNetCode())
        _call(zone, ("SetAssignedPriority", "SetPriority"), spec["priority"])
        _call(zone, ("SetLocalClearance",), CLEARANCE)
        _call(zone, ("SetMinThickness",), MIN_THICKNESS)
        _call(zone, ("SetPadConnection",), pcbnew.ZONE_CONNECTION_FULL)
    shape = zone.Outline()
    shape.NewOutline()
    for x, y in outline_at(at, footprint.GetOrientationDegrees(), spec["outline"]):
        shape.Append(x, y)
    try:
        zone.SetZoneName(name)
    except Exception:
        # Without a name this zone cannot be found again, so a later press would double it rather
        # than replace it. Better to say so than to leave a board accumulating copper.
        raise RuntimeError("this KiCad will not name a zone, so a pour cannot be re-placed")
    board.Add(zone)


def _call(obj, names, *args):
    """Calls the first of `names` that exists, and shrugs if none does.

    The zone API has been renamed more than once and these are all *preferences* — a priority, a
    clearance, a pad-connection mode. A build of KiCad that spells one differently should give a
    zone with a default instead of a button that does not work.
    """
    for name in names:
        method = getattr(obj, name, None)
        if method is not None:
            method(*args)
            return True
    return False
