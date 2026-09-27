"""An arrangement made once, repeated onto every other instance of the same block.

A board with eight of one block on it — eight bridge drivers, four districts — is a board where
somebody arranges one instance's capacitors around its chip and then does it seven more times, and
the seven are the tedious copies of a decision already made. KiCad cannot help: its footprints are
a flat list, and the "replicate layout" plugins people reach for key on a schematic's sheet paths,
which a board with no schematic has none of.

What it has instead is `Stackup Path`. `machines/ch7/Cvm` *is* the sheet path, and it carries
exactly the structure the copying needs: the part the arrangement is around (`machines/ch7`), the
other instances of it (`machines/ch0` … `ch6`), and the counterpart of each arranged part on each
of them, by swapping the one segment that names the instance. The arranged parts need not sit
*under* the anchor — a district's range switch at `sense/range` and the legs beside it at
`sense/run/*` are one arrangement, and `district0` → `district1` carries all of them across —
only on the same instance of it. So the whole of this is path arithmetic and one rigid
transform, and the transform is `anchor.exact` — the one that already carries a datasheet's
regulator drawing round with a turned host — fed with an offset *measured* off the board rather than
stated by the design.

The button reads the selection: the anchor and the parts arranged about it. Only what is selected
is repeated, which is what lets an instance's other parts — a shunt's filter that lives by the MCU
rather than by the bridge — stay where they are on every copy. Nothing here judges the result any
more than `anchor` does: DRC is what says whether the copies collide with anything.

Routing follows the same rule, with one more condition that settles whose copper it is. A selected
track or via is carried when it runs **from a pad of a selected part, through selected copper** —
the walk is end-to-end contact on a shared layer, seeded from those pads — so a `GND` via that
happens to be in the box but touches nothing selected stays where it is. That seed pad is also what
names the copy's net: the counterpart footprint's same pad on the sibling, whatever net it carries.
Nothing here reads a net's name, which is why a track on the instance's own `sense/run/tap` and a
via on the board-wide `GND` are one case rather than two. And a copy whose geometry is already on
the sibling — the same ends on the same layer — is skipped rather than doubled, so pressing again
after moving one capacitor redraws what moved and leaves the rest alone.
"""

from __future__ import annotations

import pcbnew

from . import anchor

#: The footprint field the design's stable path arrives on.
FIELD = "Stackup Path"


def segments(path: str) -> list[str]:
    return [s for s in path.split("/") if s]


def is_within(path: str, ancestor: str) -> bool:
    """Whether `path` is `ancestor` or under it. `ch1` is not within `ch`: whole segments only."""
    inner, outer = segments(path), segments(ancestor)
    return inner[: len(outer)] == outer


def anchor_of(selected, present) -> str | None:
    """The path of the part the selected arrangement is around, or `None` if there is no such part.

    The explicit form first: if one selected part sits above every other, it was selected *as* the
    anchor — a chip with its capacitors — and that is the answer whether or not anything else would
    have qualified. Otherwise the parts' deepest common ancestor is walked upward until a path with
    a footprint behind it is found, so a selection of only the passives still finds its chip, and a
    selection of a block's sub-block finds the block's part above the sub-block that has none.

    When neither answers — the parts sit *beside* one another under a namespace with no part of
    its own, as a switch at `sense/range` does with the legs at `sense/run/*` and `sense/prog/*`
    — the one selected part nearest the top is what the rest are about, provided there is exactly
    one. Two at the same depth would be a guess, and a guess moves parts.

    `present` is every path with a footprint on the board. A selection spanning two instances
    (`ch7/Cvm` with `ch6/Cvm`) climbs to `machines`, which has no part, and so gets `None` — the
    right answer, since there is no one frame such a selection could be measured in.
    """
    selected = sorted(set(selected), key=lambda path: len(segments(path)))
    if not selected:
        return None
    if len(selected) > 1 and all(is_within(path, selected[0]) for path in selected):
        return selected[0]
    common = segments(selected[0])
    for path in selected[1:]:
        parts = segments(path)
        keep = 0
        while keep < min(len(common), len(parts)) and common[keep] == parts[keep]:
            keep += 1
        common = common[:keep]
    if len(selected) == 1:
        # One part on its own is the thing being arranged, never the thing it is arranged around.
        common.pop()
    present = set(present)
    while common:
        candidate = "/".join(common)
        if candidate in present:
            return candidate
        common.pop()
    if len(selected) > 1 and len(segments(selected[0])) < len(segments(selected[1])):
        return selected[0]
    return None


def siblings(anchor_path: str, kinds: dict) -> list[str]:
    """The other instances of the anchor's block: every path in `kinds` at the anchor's depth that
    differs from it in exactly one segment and carries the same footprint.

    `kinds` maps each path on the board to what is placed there — the footprint's library name —
    and matching on it is what keeps `machines/vref` from counting as an eighth bridge. One segment
    rather than a fixed position, so `bankA/ch0` and `bankB/ch0` are siblings too, and so are two
    `dcc/district<n>/bridge` under different districts.
    """
    here = segments(anchor_path)
    kind = kinds.get(anchor_path)
    found = []
    for path, what in kinds.items():
        if path == anchor_path or what != kind:
            continue
        there = segments(path)
        if len(there) != len(here):
            continue
        if sum(1 for a, b in zip(here, there) if a != b) == 1:
            found.append(path)
    return sorted(found, key=_natural_path)


def counterpart(path: str, anchor_path: str, sibling_path: str) -> str | None:
    """`machines/ch7/Cvm` on `machines/ch3` is `machines/ch3/Cvm`, and so is `machines/ch7/sense/tap`
    `machines/ch3/sense/tap` — the one segment the anchor and its sibling differ in is swapped,
    wherever under the instance the part sits. `None` if the part is not on the anchor's instance
    at all: `machines/vref` has no `ch7` to swap, and there is nothing it could stand in for.
    """
    here, there, mine = segments(anchor_path), segments(sibling_path), segments(path)
    varies = next(i for i, (a, b) in enumerate(zip(here, there)) if a != b)
    if mine[: varies + 1] != here[: varies + 1]:
        return None
    return "/".join(there[: varies + 1] + mine[varies + 1 :])


def carried(copper, pads, hits, touches) -> dict:
    """Which of `copper` runs from one of `pads` through the rest of `copper`, and from which pad.

    `hits(pad, item)` says whether an item ends on a pad and `touches(a, b)` whether two items
    meet end to end; both are handed in so that the walk is plain graph search with nothing of
    KiCad's in it. The result maps each reached item's *index* in `copper` to the pad its island
    was entered from — the pad that will name its net on the copy — and leaves out everything the
    walk never reaches, which is what makes a stray via in the selection box a stray rather than a
    copy. Indices because a `PCB_TRACK` is not hashable.
    """
    reached = {}
    for pad in pads:
        frontier = [i for i, item in enumerate(copper) if i not in reached and hits(pad, item)]
        while frontier:
            i = frontier.pop()
            if i in reached:
                continue
            reached[i] = pad
            frontier.extend(
                j
                for j, other in enumerate(copper)
                if j not in reached and touches(copper[i], other)
            )
    return reached


def carry(point: tuple[int, int], host, sibling) -> tuple[int, int]:
    """Where `point`, seen from `host`, lands when seen from `sibling` instead.

    `host` and `sibling` are `(position, rotation in degrees)`. The same measurement `_place_like`
    makes for a footprint, applied to a bare point — a track's end, a via's centre — so the copper
    turns with the part it was drawn around.
    """
    (at, rotation), (to, turned) = host, sibling
    dx, dy, _ = anchor.local(at, rotation, point, 0)
    x, y, _ = anchor.exact(to, turned, (dx, dy, 0))
    return (x, y)


def shape(kind: str, layer: int, ends: list[tuple[int, int]], mid=None):
    """What makes two pieces of copper the same piece: the class, the layer, and where it is.

    Ends are unordered — a track drawn from A to B is the track drawn from B to A — and the width
    and net are left out on purpose: a track already where the copy would go *is* that track, and
    a second one on top of it is a duplicate whatever it is called or however wide it is.
    """
    return (kind, layer, frozenset(ends), mid)


def _natural_path(path: str):
    """`ch2` before `ch10`, so the report reads in the order a person expects."""
    key = []
    for segment in segments(path):
        digits = "".join(c for c in segment if c.isdigit())
        key.append((segment.rstrip("0123456789"), int(digits) if digits else -1))
    return key


class Repeated:
    """What the *Repeat arrangement* button did, in the terms its report needs."""

    def __init__(self):
        self.selected = 0
        #: The anchor's path and designator, once one was found.
        self.anchor: tuple[str, str] | None = None
        #: How many parts around the anchor were selected to be repeated.
        self.members = 0
        #: Designators of the siblings' anchors the arrangement was copied onto.
        self.onto: list[str] = []
        #: Designators moved, across every sibling.
        self.moved: list[str] = []
        #: Selected parts with no `Stackup Path` — nothing the sync placed, so nothing to find a
        #: counterpart of. Left alone and mentioned.
        self.unplaced: list[str] = []
        #: Siblings on the other side of the board from the anchor, which are not repeated onto.
        self.other_side: list[str] = []
        #: Tracks and vias selected along with the parts, and how many of them ran from a
        #: selected part's pad — the rest are `loose`, and left alone.
        self.copper = 0
        self.loose = 0
        #: Pieces of copper drawn across every sibling, and pieces skipped because the sibling
        #: already had the same piece in the same place.
        self.drawn = 0
        self.already = 0
        self.problems: list[str] = []

    def quiet(self) -> bool:
        """A run where something moved and nothing needs saying — the same bargain the anchor
        button makes, because this too is a loop: arrange, select, press, look."""
        return bool(self.moved or self.drawn) and not (self.problems or self.other_side)

    def summary(self) -> str:
        if not self.selected:
            return (
                "Nothing is selected.\n\nSelect the part an arrangement is around and the parts "
                "arranged about it — a bridge and its capacitors — and press the button again. "
                "The same arrangement is made around every other part like it."
            )
        if self.anchor is None:
            return (
                f"The {self.selected} selected part(s) have no one part above them to repeat "
                "around.\n\nAn arrangement is measured from one part — a chip, a bridge — and "
                "copied onto the other parts like it. Select that part and the parts around it, "
                "all from one instance."
            )
        path, designator = self.anchor
        if not self.members and not self.copper:
            return (
                f"Only {designator} ({path}) is selected, and there is nothing arranged about it "
                "to repeat.\n\nSelect the parts around it as well."
            )
        if not self.members and self.loose == self.copper:
            return (
                f"Only {designator} ({path}) and {self.copper} piece(s) of copper are selected, "
                "and none of the copper runs from one of its pads.\n\nCopper is repeated when it "
                "runs from a pad of a selected part through other selected copper."
            )
        if not self.onto and not self.other_side:
            return (
                f"{designator} ({path}) is the only one of its kind on the board, so there is "
                "nothing to repeat its arrangement onto."
            )
        lines = [
            f"Repeated {self.members} part(s) arranged around {designator} ({path}) onto "
            f"{len(self.onto)} other(s): {', '.join(self.onto)}."
        ]
        if self.copper:
            lines.append("\n" + self.copper_line())
        if self.unplaced:
            lines.append(
                "\nNot placed by the design, so left alone: "
                + ", ".join(sorted(self.unplaced, key=anchor._natural))
            )
        if self.other_side:
            lines.append(
                "\nOn the other side of the board, so not repeated onto: "
                + ", ".join(self.other_side)
            )
        if self.problems:
            lines.append("\nProblems:\n  " + "\n  ".join(self.problems))
        return "\n".join(lines)

    def copper_line(self) -> str:
        """One line on the routing, for the dialog and the console alike."""
        parts = [f"{self.drawn} piece(s) of copper drawn"]
        if self.already:
            parts.append(f"{self.already} already there")
        if self.loose:
            parts.append(f"{self.loose} selected but joined to no selected part's pad, so left alone")
        return f"Copper: {', '.join(parts)}."


def _path(footprint) -> str:
    return footprint.GetFieldText(FIELD) if footprint.HasField(FIELD) else ""


def _kind(footprint) -> str:
    return str(footprint.GetFPID().GetLibItemName())


def _place_like(source, target, host, sibling) -> None:
    """Puts `target` where `source` is, as seen from `host`, but seen from `sibling` instead.

    Position and rotation through the footprint, so the pads follow; then each text field the two
    have in common, in the footprint's own frame — where a reference was dragged to, whether it is
    shown at all — because a hidden designator is as much a part of an arrangement as a position
    is, and a copy with seven labels to hide by hand is a copy of most of it.
    """
    offset = anchor.local(
        _at(host), host.GetOrientationDegrees(), _at(source), source.GetOrientationDegrees()
    )
    x, y, rotation = anchor.exact(_at(sibling), sibling.GetOrientationDegrees(), offset)
    target.SetPosition(_vector(x, y))
    target.SetOrientationDegrees(rotation)
    for field in source.GetFields():
        name = field.GetName()
        if not target.HasField(name):
            continue
        twin = target.GetField(name)
        twin.SetFPRelativePosition(field.GetFPRelativePosition())
        turn = field.GetTextAngleDegrees() - source.GetOrientationDegrees()
        twin.SetTextAngleDegrees((target.GetOrientationDegrees() + turn) % 360)
        twin.SetVisible(field.IsVisible())


def _at(footprint) -> tuple[int, int]:
    position = footprint.GetPosition()
    return (position.x, position.y)


def _vector(x: int, y: int):
    return pcbnew.VECTOR2I(x, y)


def _frame(footprint) -> tuple[tuple[int, int], float]:
    return (_at(footprint), footprint.GetOrientationDegrees())


def _is_via(item) -> bool:
    return item.GetClass() == "PCB_VIA"


def _ends(item) -> list[tuple[int, int]]:
    """Where a piece of copper can meet another: a via's centre, a track's or arc's two ends."""
    if _is_via(item):
        position = item.GetPosition()
        return [(position.x, position.y)]
    start, end = item.GetStart(), item.GetEnd()
    return [(start.x, start.y), (end.x, end.y)]


def _touches(a, b) -> bool:
    """Whether two pieces of copper meet: an end in common, on a layer both are on. A via is on
    every layer it spans, so it is the other item's layer that is asked about."""
    if not any(p == q for p in _ends(a) for q in _ends(b)):
        return False
    if not _is_via(b):
        return a.IsOnLayer(b.GetLayer())
    if not _is_via(a):
        return b.IsOnLayer(a.GetLayer())
    return True


def _hits(pad, item) -> bool:
    """Whether a piece of copper ends on `pad`, on a layer the pad has copper on."""
    layer = pad.GetLayer() if _is_via(item) else item.GetLayer()
    if not (pad.IsOnLayer(layer) and item.IsOnLayer(layer)):
        return False
    return any(pad.HitTest(_vector(x, y)) for x, y in _ends(item))


def _shape_of(item, ends: list[tuple[int, int]], mid=None):
    layer = None if _is_via(item) else item.GetLayer()
    return shape(item.GetClass(), layer, ends, mid)


def _draw(board, item, host, sibling, net: int, present: set) -> bool:
    """Puts a copy of `item` on the board where it lands seen from `sibling`, on `net`. False if
    the sibling already has that copper, in which case nothing is added."""
    ends = [carry(end, host, sibling) for end in _ends(item)]
    mid = None
    if item.GetClass() == "PCB_ARC":
        centre = item.GetMid()
        mid = carry((centre.x, centre.y), host, sibling)
    key = _shape_of(item, ends, mid)
    if key in present:
        return False
    copy = item.Duplicate()
    if _is_via(item):
        copy.SetPosition(_vector(*ends[0]))
    else:
        copy.SetStart(_vector(*ends[0]))
        copy.SetEnd(_vector(*ends[1]))
        if mid is not None:
            copy.SetMid(_vector(*mid))
    copy.SetNetCode(net)
    copy.ClearSelected()
    board.Add(copy)
    present.add(key)
    return True


def _present(board) -> set:
    """Every piece of copper on the board, by shape, so a copy can tell whether it is already there."""
    found = set()
    for item in board.GetTracks():
        mid = None
        if item.GetClass() == "PCB_ARC":
            centre = item.GetMid()
            mid = (centre.x, centre.y)
        found.add(_shape_of(item, _ends(item), mid))
    return found


def repeat(board) -> Repeated:
    """Copies the selected arrangement onto every other instance of the block it belongs to."""
    outcome = Repeated()
    selected = [f for f in board.GetFootprints() if f.IsSelected()]
    outcome.selected = len(selected)
    if not selected:
        return outcome

    by_path = {}
    for footprint in board.GetFootprints():
        path = _path(footprint)
        if path:
            by_path[path] = footprint

    chosen = {}
    for footprint in selected:
        path = _path(footprint)
        if path:
            chosen[path] = footprint
        else:
            outcome.unplaced.append(footprint.GetReference())
    if not chosen:
        return outcome

    anchor_path = anchor_of(chosen, by_path)
    if anchor_path is None:
        return outcome
    host = by_path[anchor_path]
    outcome.anchor = (anchor_path, host.GetReference())
    members = [(path, f) for path, f in chosen.items() if path != anchor_path]
    outcome.members = len(members)

    # The copper: what was selected, narrowed to what runs from a selected part's pad. The anchor's
    # own pads seed the walk whether or not it was selected, because a track from the bridge to a
    # selected capacitor is the capacitor's arrangement even when the bridge stays put.
    copper = [t for t in board.GetTracks() if t.IsSelected()]
    outcome.copper = len(copper)
    seeds = [pad for f in [host, *(f for _, f in members)] for pad in f.Pads()]
    routed = carried(copper, seeds, _hits, _touches)
    outcome.loose = len(copper) - len(routed)
    if not members and not routed:
        return outcome
    present = _present(board)

    for sibling_path in siblings(anchor_path, {p: _kind(f) for p, f in by_path.items()}):
        sibling = by_path[sibling_path]
        if sibling.IsFlipped() != host.IsFlipped():
            outcome.other_side.append(sibling.GetReference())
            continue
        outcome.onto.append(sibling.GetReference())
        for i, pad in routed.items():
            item, owner = copper[i], pad.GetParentFootprint()
            wanted = counterpart(_path(owner), anchor_path, sibling_path)
            twin = by_path.get(wanted) if wanted else None
            twin_pad = twin.FindPadByNumber(pad.GetNumber()) if twin else None
            if twin_pad is None:
                outcome.problems.append(
                    f"{sibling.GetReference()} ({sibling_path}) has no {wanted or '?'} pad "
                    f"{pad.GetNumber()} to name the net of copper off "
                    f"{owner.GetReference()} pad {pad.GetNumber()}"
                )
                continue
            if _draw(board, item, _frame(host), _frame(sibling), twin_pad.GetNetCode(), present):
                outcome.drawn += 1
            else:
                outcome.already += 1
        for path, source in members:
            wanted = counterpart(path, anchor_path, sibling_path)
            if wanted is None:
                outcome.problems.append(
                    f"{source.GetReference()} ({path}) is not on {anchor_path}'s instance, so "
                    f"nothing on {sibling.GetReference()} ({sibling_path}) stands in for it"
                )
                continue
            target = by_path.get(wanted)
            if target is None:
                outcome.problems.append(
                    f"{sibling.GetReference()} ({sibling_path}) has no {wanted} to stand in "
                    f"for {source.GetReference()}"
                )
                continue
            _place_like(source, target, host, sibling)
            outcome.moved.append(target.GetReference())
    return outcome
