"""Vias a design asked for: the arrays hung off a pin, under an exposed pad.

The pour's sibling and the same argument one step further still. A pour says *what shape the copper
on this node is around that pad*; a via array says *where this node goes through the board* — a
PowerPAD's heat path into the plane beneath it, which a netlist cannot carry because a via is
neither a component nor a net.

It arrives the way a pour does, as a field on the footprint the positions are measured from:
`Stackup Vias`, one entry per array, `pad|drill|diameter|tenting|x,y x,y …`, with a trailing
`|filled` or `|capped` when the barrels are not left open. And it fires the same
way — on a selected pad, from the *Place at anchor pad* button — so a chip's thermal vias appear
with its copper and nothing happens to a part you merely selected.

Every via this places goes into a group named `stackup:U1.17#vias`, and a second press replaces
exactly that group's members: vias you drew yourself are yours.
"""

from __future__ import annotations

import pcbnew

from .anchor import pad_origin
from .pour import outline_at, selected_pads

#: The footprint field the arrays ride on, written by the sync from `Design::via_arrays`.
FIELD = "Stackup Vias"

#: How the group holding one array is named — what makes a second press an update.
NAME = "stackup:{designator}.{pad}#vias"

TENTINGS = ("opposite", "own", "both", "none")

#: What the barrels carry. `open` is what an entry with no sixth field means.
FILLS = ("open", "filled", "capped")


def parse(text: str) -> list[dict]:
    """Reads the field into a list of arrays, skipping anything that will not read.

    Dropped rather than raised on, as `pour.parse` does: this is placement, and the cost of an
    entry that cannot be read is that vias do not get placed — not that a button stops working.
    """
    arrays = []
    for entry in (text or "").split(";"):
        entry = entry.strip()
        if not entry:
            continue
        parts = entry.split("|")
        if len(parts) == 5:
            parts.append("open")
        if len(parts) != 6:
            continue
        pad, drill, diameter, tenting, points, fill = parts
        try:
            hole, ring = float(drill), float(diameter)
        except ValueError:
            continue
        at = _points(points)
        if not pad or tenting not in TENTINGS or fill not in FILLS:
            continue
        if not at or not (0 < hole < ring):
            continue
        arrays.append(
            {
                "pad": pad,
                "drill": hole,
                "diameter": ring,
                "tenting": tenting,
                "fill": fill,
                "at": at,
            }
        )
    return arrays


def _points(text: str) -> list[tuple[float, float]]:
    out = []
    for pair in text.split():
        x, _, y = pair.partition(",")
        try:
            out.append((float(x), float(y)))
        except ValueError:
            return []
    return out


def masks(tenting: str, on_back: bool) -> tuple[bool, bool]:
    """`(front, back)`: which faces of the board the mask covers, resolved against which way up
    the part is. Pure, so the one part of this that can be silently wrong is checkable without a
    board — `opposite` on a part on the back is the *front* of the board."""
    own, other = (not on_back), on_back  # the face the part is on, as "is it the front"
    if tenting == "both":
        return True, True
    if tenting == "none":
        return False, False
    if tenting == "own":
        return own, other
    return other, own  # opposite


def apply(board, outcome):
    """Places the vias the design declared for every selected pad. Reports into a `Poured`."""
    for footprint, pad in selected_pads(board):
        designator = footprint.GetReference()
        number = pad.GetNumber()
        text = footprint.GetFieldText(FIELD) if footprint.HasField(FIELD) else ""
        wanted = [a for a in parse(text) if a["pad"] == number]
        if not wanted:
            continue
        outcome.pads += 1
        at = pad_origin(footprint, number)
        if at is None:  # unreachable — the pad is one this footprint just yielded
            continue
        name = NAME.format(designator=designator, pad=number)
        outcome.replaced += _remove(board, name)
        placed = 0
        try:
            group = pcbnew.PCB_GROUP(board)
            group.SetName(name)
            board.Add(group)
            for spec in wanted:
                front, back = masks(spec["tenting"], footprint.IsFlipped())
                for x, y in outline_at(at, footprint.GetOrientationDegrees(), spec["at"]):
                    via = _via(board, pad, (x, y), spec, front, back)
                    board.Add(via)
                    group.AddItem(via)
                    placed += 1
        except Exception as error:  # a via that will not build should not take the rest down
            outcome.problems.append(f"{designator}.{number}: {type(error).__name__}: {error}")
            continue
        outcome.vias.append(f"{designator}.{number} ({placed})")
    return outcome


def _via(board, pad, at, spec, front: bool, back: bool):
    via = pcbnew.PCB_VIA(board)
    via.SetViaType(pcbnew.VIATYPE_THROUGH)
    via.SetPosition(pcbnew.VECTOR2I(at[0], at[1]))
    via.SetDrill(pcbnew.FromMM(spec["drill"]))
    via.SetWidth(pcbnew.FromMM(spec["diameter"]))
    via.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu)
    via.SetNetCode(pad.GetNetCode())
    tented, open_ = pcbnew.TENTING_MODE_TENTED, pcbnew.TENTING_MODE_NOT_TENTED
    via.SetFrontTentingMode(tented if front else open_)
    via.SetBackTentingMode(tented if back else open_)
    # IPC-4761: a filled barrel is Type V, filled and plated over is Type VII. Stated per via
    # rather than left to the board's default, because the design asked for these and not others.
    fill = spec.get("fill", "open")
    via.SetFillingMode(
        pcbnew.FILLING_MODE_FILLED if fill != "open" else pcbnew.FILLING_MODE_NOT_FILLED
    )
    via.SetCappingMode(
        pcbnew.CAPPING_MODE_CAPPED if fill == "capped" else pcbnew.CAPPING_MODE_NOT_CAPPED
    )
    if fill != "open":
        # A filled barrel is not also plugged: inheriting a board's back-plug would hand the fab
        # two instructions for one hole.
        via.SetFrontPluggingMode(pcbnew.PLUGGING_MODE_NOT_PLUGGED)
        via.SetBackPluggingMode(pcbnew.PLUGGING_MODE_NOT_PLUGGED)
    return via


def _remove(board, name: str) -> int:
    """Deletes the vias this placed under `name`, group and all, so a second press updates."""
    doomed = [g for g in board.Groups() if str(g.GetName()) == name]
    removed = 0
    for group in doomed:
        for item in list(group.GetItems()):
            board.Remove(item)
            removed += 1
        board.Remove(group)
    return removed
