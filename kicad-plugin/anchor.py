"""Where a part belongs on the board: the pad it was placed to serve.

This is the one thing about a stackup part that is neither its identity nor its connectivity, and
the one thing a board cannot work out for itself. A decoupling capacitor and the pin it decouples
are the *same node*, so a ratline runs to whichever pad of the rail happens to be nearest and says
nothing at all about which of six IOVDD pads this capacitor is for. KiCad has nowhere to put the
association either — a netlist carries no relation between two components, nothing hints the
ratsnest, and giving each capacitor its own netcode would take a physical net tie apiece.

The design knows, because `decouple` is what fanned one capacitor across each pad. It arrives as
`Stackup Anchor` on the netlist and is spent as *placement*, which is what the fact actually is.

Two things use what is in here and they want it at different moments:

* the sync (`apply.py`) drops a **newly created** footprint on its pad instead of in the pile
  beside the board, and never touches one that already exists — position is the hand work the whole
  sync is built to preserve;
* the *Place at anchor pad* button moves whatever is **selected**, because there the human is
  asking for it, which is the one thing that outranks their own earlier placement.

Nothing here judges anything. There is no rule that a part must be near its pad, no DRC assertion
for *within* (KiCad has minimum clearances, not maximum ones), and moving a capacitor somewhere
better afterwards is not a violation of anything. It is a starting point and a label.

A few parts want more than a starting point. Where a datasheet *draws* a layout rather than
describing one — a switching regulator's hot loop, where the distances are the specification and
getting them wrong is a functional failure — the design states the offset as well, and it arrives
as `Stackup Spot`. That is the difference between `spot`, which finds a reasonable place beside a
pad, and `exact`, which puts the part where it was told. A part with no spot takes the guess, which
is nearly all of them.
"""

from __future__ import annotations

import math

import pcbnew

#: How far past the edge of a part to drop something anchored on one of its pads, and how much
#: further out each additional part on the same pad goes. Small on purpose: a decoupling capacitor
#: that lands two millimetres off the chip is in roughly the right place, and a hint that overshoots
#: is no better than the pile beside the board.
GAP = pcbnew.FromMM(2)
STEP = pcbnew.FromMM(2.5)

#: The footprint field the association rides on, the user layer its label goes on, and the height of
#: that label. The layer name is what the layer is *found* by on the next sync, so renaming it in the
#: editor makes a fresh one — which is the right failure: the labels are ours to manage and a layer
#: someone has repurposed is not.
FIELD = "Stackup Anchor"
LAYER = "stackup"
TEXT_HEIGHT = pcbnew.FromMM(0.6)

#: The companion field, carrying an *exact* placement for the few parts a datasheet draws rather
#: than describes: `dx dy rotation`, millimetres and degrees, measured from the anchor pad in the
#: host part's own frame. Hidden, unlike `FIELD` — the anchor answers a question asked while
#: dragging a part, and a part that has one of these is not being dragged.
SPOT_FIELD = "Stackup Spot"


def reference(text: str) -> tuple[str, str] | None:
    """`U1.49` split into `("U1", "49")`, or `None` for a part that stands on its own.

    Split on the *first* dot: a designator is a prefix and digits and never contains one, while a
    pad number is a free string that might (`A.1` is legal, if unusual).
    """
    designator, dot, pad = text.partition(".")
    return (designator, pad) if dot and designator and pad else None


def slots(anchored) -> dict[str, int]:
    """Which slot each part takes among everything sharing its anchor pad, keyed by designator.

    Two parts can want the same pad — a bypass and a bulk capacitor on one supply pin, a divider's
    two legs on one tap — and stacking them at the same coordinates would hide one under the other.

    `anchored` is an ordered iterable of `(designator, anchor text)`, and the order is what decides
    who sits nearest the pad. Both callers pass every anchored part they know of rather than only
    the ones they are about to move, so a part gets the same slot whether its neighbours are being
    placed alongside it or were placed three syncs ago.
    """
    taken: dict[tuple[str, str], int] = {}
    assigned: dict[str, int] = {}
    for designator, text in anchored:
        pad = reference(text or "")
        if pad is None:
            continue
        assigned[designator] = taken.get(pad, 0)
        taken[pad] = assigned[designator] + 1
    return assigned


def on_board(board) -> list[tuple[str, str]]:
    """Every footprint on `board` that records an anchor, in designator order.

    Designator order rather than the netlist's placement order, because a board has no placement
    order to read — and the two agree anyway in the case that matters, since stackup assigns
    designators in placement order. What is actually needed is only that the answer does not depend
    on which parts happened to be selected.
    """
    anchored = []
    for footprint in board.GetFootprints():
        if footprint.HasField(FIELD) and footprint.GetFieldText(FIELD):
            anchored.append((footprint.GetReference(), footprint.GetFieldText(FIELD)))
    return sorted(anchored, key=lambda pair: _natural(pair[0]))


def _natural(designator: str) -> tuple[str, int, str]:
    """`R10` after `R9`, not before it. Plain string order would put `C10` between `C1` and `C2`
    and quietly swap which of two capacitors on one pad sits nearer to it."""
    digits = "".join(c for c in designator if c.isdigit())
    prefix = designator[: len(designator) - len(digits)] if digits else designator
    return (prefix, int(digits) if digits else 0, designator)


def pad_envelope(footprint) -> tuple[int, int, int, int]:
    """`(left, top, right, bottom)` over a footprint's pad centres, in board coordinates.

    The pads rather than `GetBoundingBox`, which takes in silkscreen and the reference text and so
    reports a QFN as several times its own size — the question here is where the *copper* stops.
    """
    positions = [pad.GetPosition() for pad in footprint.Pads()]
    if not positions:
        at = footprint.GetPosition()
        return (at.x, at.y, at.x, at.y)
    xs = [at.x for at in positions]
    ys = [at.y for at in positions]
    return (min(xs), min(ys), max(xs), max(ys))


def pad_origin(footprint, number: str) -> tuple[int, int] | None:
    """Where a *numbered pad* is, in board coordinates. `None` if the footprint has no such number.

    Not `FindPadByNumber().GetPosition()`, and the difference is the whole reason this exists: a
    number can name several pads, and then the first one in the file is not the answer. An exposed
    thermal pad is the case — `Texas_HTSOP-8-1EP…ThermalVias` spells pad 9 as eight copper pads, two
    lands on the origin and six via barrels at ±0.65, ±1.3 mm — and the design's model of it is one
    pin with one number, which is right, because the six vias are *how* that pad is built and not
    six terminals. So the position it means is the middle of them, which is where the land is, rather
    than whichever barrel got written down first.

    The middle of the pad *centres*, not of their copper: for one pad it is exactly `GetPosition()`,
    so the ordinary case is unchanged even for a custom-shaped pad whose outline is off-centre.
    """
    positions = [
        pad.GetPosition() for pad in footprint.Pads() if str(pad.GetNumber()) == str(number)
    ]
    if not positions:
        return None
    xs = [at.x for at in positions]
    ys = [at.y for at in positions]
    return ((min(xs) + max(xs)) // 2, (min(ys) + max(ys)) // 2)


def spot(
    envelope: tuple[int, int, int, int], pad: tuple[int, int], slot: int
) -> tuple[int, int, int]:
    """Where the `slot`-th part anchored on `pad` lands: `(x, y, rotation in degrees)`.

    Straight out from the anchor pad, on whichever side of the part that pad is on, clear of the
    part's own copper. The direction is snapped to an axis rather than taken as the true radial —
    a pad near a corner would otherwise send its capacitor diagonally across two rows of neighbours,
    where what a person actually wants is *out through the side the pad is on*. That also makes the
    rotation one of two values, so the capacitor's long axis points away from the part and its near
    plate faces the pad, which is the placement it wants for its own sake.

    Pure, and in board coordinates (KiCad's Y grows downward), so it is the whole of the geometry
    and can be checked without a board.
    """
    left, top, right, bottom = envelope
    dx = pad[0] - (left + right) / 2
    dy = pad[1] - (top + bottom) / 2
    out = GAP + slot * STEP
    # A tie goes to x: it means a pad dead centre (a lone pad, a thermal pad), where there is no
    # side to be on and any deterministic answer will do.
    if abs(dx) >= abs(dy):
        return (int(right + out if dx >= 0 else left - out), int(pad[1]), 0)
    return (int(pad[0]), int(bottom + out if dy >= 0 else top - out), 90)


def offset(text: str) -> tuple[float, float, float] | None:
    """`"0.4 -1.16 -90"` read as `(dx mm, dy mm, degrees)`, or `None` if there is nothing to read.

    Anything malformed is `None` rather than an error: this is a *preference* about a placement, so
    a field that cannot be read costs the exact position and nothing else — the part still lands on
    its pad by the ordinary route. Silence would be wrong for connectivity and is right here.
    """
    parts = (text or "").split()
    if len(parts) != 3:
        return None
    try:
        return (float(parts[0]), float(parts[1]), float(parts[2]))
    except ValueError:
        return None


def exact(
    pad: tuple[int, int], host_rotation: float, offset_mm: tuple[float, float, float]
) -> tuple[int, int, int]:
    """Where a part offset from `pad` lands: `(x, y, rotation in degrees)`, in board coordinates.

    The offset is given in the **host's own frame**, so it has to be turned by however the host is
    turned before it means anything on the board — which is the whole reason a design can state one
    at all. Rotate the MCU and its regulator rotates with it; nothing here is a board coordinate,
    and no re-sync is needed after moving the chip.

    The rotation matches KiCad's own local-to-board transform (`RotatePoint` with `+orientation`,
    on a `y`-down board), so a number read off a reference layout goes in unchanged.

    Pure, like `spot`, and for the same reason: this is the whole of the geometry, and it should be
    checkable without a board in front of it. What it does *not* do is mirror for a host on the back
    of the board — the plugin places every footprint on the front and always has, so a back-side
    host is outside what this file can currently be right about.
    """
    dx, dy, rotation = offset_mm
    theta = math.radians(host_rotation)
    x = pad[0] + pcbnew.FromMM(dx) * math.cos(theta) + pcbnew.FromMM(dy) * math.sin(theta)
    y = pad[1] - pcbnew.FromMM(dx) * math.sin(theta) + pcbnew.FromMM(dy) * math.cos(theta)
    return (int(round(x)), int(round(y)), int(round(host_rotation + rotation)) % 360)


def local(
    host: tuple[int, int], host_rotation: float, part: tuple[int, int], part_rotation: float
) -> tuple[float, float, float]:
    """Where `part` is *as seen from* `host`: `(dx mm, dy mm, degrees)` in the host's own frame.

    The inverse of `exact`, and the measurement that makes an arrangement on the board something
    that can be said in the host's terms — which is the only form it can be carried in to another
    host that is turned differently. `exact(host, rotation, local(host, rotation, part))` gives the
    part back, to the nanometre a rounding allows, and the test holds it to that.
    """
    theta = math.radians(host_rotation)
    ex = part[0] - host[0]
    ey = part[1] - host[1]
    # Rounded to whole nanometres before the millimetre conversion: `sin(π)` is not zero but
    # 1e-16, and left in, that is what `FromMM` truncates to a coordinate one nanometre off grid.
    dx = round(ex * math.cos(theta) - ey * math.sin(theta))
    dy = round(ex * math.sin(theta) + ey * math.cos(theta))
    return (pcbnew.ToMM(dx), pcbnew.ToMM(dy), (part_rotation - host_rotation) % 360)


def place(
    footprint,
    host,
    pad_number: str,
    slot: int,
    offset_mm: tuple[float, float, float] | None = None,
) -> bool:
    """Moves `footprint` onto `host`'s pad. False if the host has no such pad.

    A pad number is only true of the body the design chose, so a board still carrying an older
    footprint for the host really can be missing it — which is a thing to report, not to guess at.

    With an `offset_mm` the part goes exactly there and the slot is not consulted: two parts sharing
    a pad no longer need queueing apart, because the design has already said where each of them
    goes. Without one it lands by `spot`, which is the hint the great majority of parts want.
    """
    at = pad_origin(host, pad_number)
    if at is None:
        return False
    if offset_mm is None:
        x, y, rotation = spot(pad_envelope(host), at, slot)
    else:
        x, y, rotation = exact(at, host.GetOrientationDegrees(), offset_mm)
    footprint.SetPosition(pcbnew.VECTOR2I(x, y))
    footprint.SetOrientationDegrees(rotation)
    return True


def layer(board):
    """The user layer the anchor labels live on, enabling and naming one if the board has none.

    A user layer rather than `Cmts.User` because the labels are stackup's and a board's comment
    layer is the person's — and because a layer of its own is a layer you can switch off in one
    click when the placement is done, without losing whatever else you had written down.

    A free layer is one still wearing its default name, not one that happens to be disabled: KiCad
    enables `User.1` and `User.2` on a board it creates, so "disabled" would step over two perfectly
    empty layers to claim the third. A *renamed* layer is one someone has spoken for, and that is
    what this leaves alone.

    Every step here is best-effort: a board with all nine user layers spoken for, or a KiCad that
    will not let a script touch the layer set, falls back to `Cmts.User` rather than failing over
    where a label goes.
    """
    users = [
        (pcbnew.User_1, "User.1"),
        (pcbnew.User_2, "User.2"),
        (pcbnew.User_3, "User.3"),
        (pcbnew.User_4, "User.4"),
        (pcbnew.User_5, "User.5"),
        (pcbnew.User_6, "User.6"),
        (pcbnew.User_7, "User.7"),
        (pcbnew.User_8, "User.8"),
        (pcbnew.User_9, "User.9"),
    ]
    try:
        for identifier, _ in users:
            if board.GetLayerName(identifier) == LAYER:
                return identifier
        for identifier, default in users:
            if board.GetLayerName(identifier) != default:
                continue
            if not board.IsLayerEnabled(identifier):
                enabled = board.GetEnabledLayers()
                enabled.AddLayer(identifier)
                board.SetEnabledLayers(enabled)
            visible = board.GetVisibleLayers()
            visible.AddLayer(identifier)
            board.SetVisibleLayers(visible)
            board.SetLayerName(identifier, LAYER)
            return identifier
    except Exception:
        pass
    return pcbnew.Cmts_User


class Snapped:
    """What the *Place at anchor pad* button did, in the terms its report needs."""

    def __init__(self):
        self.selected = 0
        #: Designators moved onto their pad.
        self.moved: list[str] = []
        #: Selected parts the design did not place for a pad — a controller, a connector, a
        #: capacitor written as a bare `capacitor(…)` with no rail verb behind it. Not a failure:
        #: most parts on a board stand on their own, and this button simply has nothing to say
        #: about them.
        self.standalone: list[str] = []
        self.problems: list[str] = []
        #: What pouring did, when the same press had a pad selected as well. Set by the caller
        #: rather than produced here, because copper is `pour.py`'s business and this class is only
        #: the thing that gets reported.
        self.poured = None

    def quiet(self) -> bool:
        """Whether this outcome speaks for itself and needs no dialog.

        Placing parts is a *loop* — pick a capacitor, press, pick the next — and a modal in the
        middle of it costs more than the report is worth. When something moved and nothing went
        wrong there is nothing to read anyway: the parts are visibly on their pads, which is both
        the result and the proof of it.

        Selecting a part this button has nothing to do with does not break the silence. Selecting a
        chip along with its capacitors is the normal way to use it, and the chip staying put is as
        visible as the capacitors moving. What does break it is **nothing having moved** — silence
        then is indistinguishable from a button that did not fire — and anything going wrong.
        """
        if self.poured is not None and (self.poured.problems or self.poured.skipped):
            return False
        drew = self.poured and (self.poured.filled or self.poured.vias)
        return bool(self.moved or drew) and not self.problems

    def summary(self) -> str:
        poured = self._poured_lines()
        if not self.selected and not (self.poured and self.poured.pads):
            return (
                "Nothing is selected.\n\nSelect the parts to place — a capacitor, a whole block, "
                "everything on the board — and press the button again. It moves what you picked "
                "and nothing else."
            )
        if poured and not self.moved and not self.problems:
            return "\n".join(poured)
        if not self.moved and not self.problems:
            # Every selected part stands on its own. Said in full, because the button appearing to
            # do nothing is exactly the case that needs explaining.
            return (
                f"None of the {self.selected} selected part(s) was placed for a pad, so there is "
                "nowhere to put them.\n\nThis button moves the parts a design placed *for* "
                "something — a capacitor decoupling a supply pin, a resistor in series with an "
                "output. A part that stands on its own has no anchor, and where it goes is yours "
                "to decide."
            )
        lines = [f"{len(self.moved)} of {self.selected} selected part(s) placed on their pad."]
        if self.moved:
            lines.append("\nPlaced: " + ", ".join(sorted(self.moved, key=_natural)))
        if self.standalone:
            lines.append(
                "\nNot placed for a pad, so left alone: "
                + ", ".join(sorted(self.standalone, key=_natural))
            )
        if self.problems:
            lines.append("\nProblems:\n  " + "\n  ".join(self.problems))
        lines.extend(poured)
        return "\n".join(lines)

    def _poured_lines(self) -> list[str]:
        """What to say about the copper, if any was asked for."""
        if self.poured is None or not self.poured.touched():
            return []
        lines = []
        if self.poured.filled:
            drawn = ", ".join(self.poured.filled)
            replaced = f", replacing {self.poured.replaced}" if self.poured.replaced else ""
            lines.append(f"\nCopper drawn on: {drawn}{replaced}.")
            lines.append("Zones are placed unfilled — press B to fill them.")
        if self.poured.vias:
            lines.append("\nVias placed under: " + ", ".join(self.poured.vias) + ".")
        if self.poured.skipped:
            lines.append("\nNot poured:\n  " + "\n  ".join(self.poured.skipped))
        if self.poured.problems:
            lines.append("\nCopper problems:\n  " + "\n  ".join(self.poured.problems))
        return lines


def snap(board) -> Snapped:
    """Moves every selected part onto the pad it was placed for.

    The counterpart to the sync's own anchoring, and deliberately the opposite policy about
    existing layout: the sync will not move a footprint that is already placed, because it cannot
    tell a considered position from a default one. Here the human has selected the part and pressed
    the button, which settles that question — an explicit ask outranks their own earlier placement
    in a way an automatic pass never should.

    It needs no netlist and no cargo run: the anchor is already on the footprint, put there by the
    last sync. So this is instant, works offline, and works on a board whose design will not build.
    """
    outcome = Snapped()
    selected = [f for f in board.GetFootprints() if f.IsSelected()]
    outcome.selected = len(selected)
    if not selected:
        return outcome

    # Over every anchored part on the board, not merely the selected ones — so that placing one
    # capacitor of a pair puts it beside its sibling rather than on top of it, and so the result
    # does not depend on what happened to be selected.
    assigned = slots(on_board(board))

    for footprint in selected:
        designator = footprint.GetReference()
        text = footprint.GetFieldText(FIELD) if footprint.HasField(FIELD) else ""
        target = reference(text or "")
        if target is None:
            outcome.standalone.append(designator)
            continue
        host = board.FindFootprintByReference(target[0])
        if host is None:
            outcome.problems.append(f"{designator} belongs beside {target[0]}, which is not here")
            continue
        if host is footprint:
            outcome.problems.append(f"{designator} is anchored on itself")
            continue
        spot_text = footprint.GetFieldText(SPOT_FIELD) if footprint.HasField(SPOT_FIELD) else ""
        if not place(
            footprint, host, target[1], assigned.get(designator, 0), offset(spot_text)
        ):
            outcome.problems.append(
                f"{designator} belongs on {target[0]} pad {target[1]}, which that footprint "
                "does not have"
            )
            continue
        outcome.moved.append(designator)
    return outcome
