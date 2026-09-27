"""Net classes the design derived, written into the board's rules.

A netlist says which pads are one node and nothing about what the node is for; a track width is
the difference between a rail carrying six amps and a chip-select, and the only place KiCad has
for that difference is a **net class**. The design says what each net *is* (`vlink/*` is power,
because the supply that named those nets said so) and infers what each net *needs* from the loads
on it, resolved against the board's copper into one rung of the board's own ladder — and this is
where both reach the board: the one part of a sync that is about rules rather than parts or nets.

The assignments are **pattern assignments**, KiCad's own model — the table in Board Setup that
reads `vlink/*` → `stackup:power`, exactly as a person would have written it — and a net matching
several patterns is in several classes, which KiCad composes (`stackup:track-2mm,stackup:power,
Default`, each parameter from the first class that sets it). Two things about how, both settled by
probing KiCad 10's wrapper:

* Net classes live in **`NET_SETTINGS`**, which the board's design settings share with the
  project, so a class set here is what Board Setup shows and what DRC uses. The editor writes it
  to the `.kicad_pro` when the board is saved, and the headless `SaveBoard` does too unless told
  to skip settings.
* The wrapper can add a pattern and **clear them all**, and hands the list back opaque — so a
  person's own patterns are read from the `.kicad_pro` beside the board and put back after the
  clear. Their patterns survive a sync exactly as their footprint positions do; the cost is that
  a pattern typed into Board Setup and not yet saved to the project is one this cannot see, and
  the summary says so when the project file is missing. Removing a class needs `SetNetclasses`
  with the class left out: the map `GetNetclasses` hands back is a copy.

The sync owns every class named `stackup:…` and every pattern assigned to one, the way it owns
parts and nets: created, brought in line, and removed when the design stops deriving them. A class
or a pattern without the prefix is somebody's and is never touched.

**What a net answers is not what the router uses, and only the first is reachable from here.**
A net carries its class on its `NETINFO_ITEM`, set by `BOARD.SynchronizeNetsAndNetClasses` — the
call the editor makes on load, on save and when Board Setup is OK'd — so that is what this makes
after rewriting the table, and a net moved between classes answers with its new one at once.
(Clearing `NET_SETTINGS`' caches does *not* do that: the nets keep pointing at the classes they
had, and `RecomputeEffectiveNetclasses` over a just-cleared cache recomputes nothing.) But the
router and DRC read neither; they read the DRC engine's *implicit rules*, one per class name
compiled when the board was opened, and a rule set is rebuilt only by `InitEngine`, which nothing
in the wrapper reaches — not `Refresh`, not `UpdateUserInterface`, not running DRC. A net whose
class combination the engine has not seen matches no rule at all, and the router falls back to the
board's minimums: track at `min_track_width`, via at `min_via_diameter`/`min_through_hole` (the
0.5/0.3 that looks like a class nobody wrote), clearance at `min_clearance`. So the summary says
to OK Board Setup or reopen the board whenever a class or a pattern changed, because until then
the board is routing to rules this sync has already replaced.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path

import pcbnew

from .netlist import NetClass

#: What marks a class as stackup's. The same string the design writes, so the two cannot drift.
PREFIX = "stackup:"


@dataclass
class Classed:
    created: list[str] = field(default_factory=list)
    updated: list[str] = field(default_factory=list)
    removed: list[str] = field(default_factory=list)
    #: Pattern assignments added or dropped, either way.
    assigned: int = 0
    #: Whether a person's own patterns could be read back, or the project file was missing and
    #: nothing of theirs was there to keep.
    kept: int = 0
    problems: list[str] = field(default_factory=list)

    def changed(self) -> bool:
        return bool(self.created or self.updated or self.removed or self.assigned)

    def summary(self) -> list[str]:
        lines = []
        if self.created:
            lines.append("Net classes created: " + ", ".join(sorted(self.created)))
        if self.updated:
            lines.append("Net classes updated: " + ", ".join(sorted(self.updated)))
        if self.removed:
            lines.append("Net classes removed: " + ", ".join(sorted(self.removed)))
        if self.assigned:
            lines.append(
                f"{self.assigned} net class pattern(s) changed"
                + (f" ({self.kept} of yours kept)" if self.kept else "")
            )
        if self.changed():
            lines.append(STALE_RULES)
        lines.extend(self.problems)
        return lines


#: Said whenever a class or a pattern changed, because the consequence is invisible: the router
#: keeps offering sizes, they are just the board's minimums rather than the class's.
STALE_RULES = (
    "KiCad compiles net class rules only when a board is opened or Board Setup is closed with OK, "
    "so the router and DRC are still on the rules from before this sync — a net whose class "
    "changed here is routed at the board's minimum track, via and clearance until then. Open "
    "Board Setup and press OK."
)


def plan(existing: dict[str, dict], wanted: dict[str, NetClass]) -> tuple[list[str], list[str], list[str]]:
    """`(create, update, remove)` for stackup's classes, given what the board holds as
    `name -> {"track_width": mm|None, "clearance": mm|None, "description": str}`.

    Pure, so the decision can be checked without a board. Only names under the prefix are ever in
    `remove`; a class the design wants that a person happened to name `stackup:…` themselves is
    brought in line rather than duplicated, which is the cost of the prefix meaning something.
    `existing` carries the via too (`via_diameter`, `via_drill`), and a class is up to date only
    when all four agree: a rule class is complete or it is wrong.
    """
    create, update, remove = [], [], []
    for name, klass in wanted.items():
        held = existing.get(name)
        if held is None:
            create.append(name)
        elif held != _rules(klass):
            update.append(name)
    for name in existing:
        if name.startswith(PREFIX) and name not in wanted:
            remove.append(name)
    return sorted(create), sorted(update), sorted(remove)


def patterns(current: list[tuple[str, str]], wanted: dict[str, NetClass]) -> tuple[list[tuple[str, str]], int]:
    """The pattern table after a sync, as `(pattern, class)` pairs, and how many of ours changed.

    Pure. Everything assigned to a class without the prefix is a person's and is kept in its
    order; everything assigned to a stackup class is replaced by what the design derives now, in
    class order — so a pattern the design stopped deriving goes, and one it never wrote is never
    ours to touch. The count is against `current`, which `apply` reads from the project file as
    last *saved*: two presses without a save in between report the same change twice, and the
    table is the same both times.
    """
    theirs = [(p, c) for p, c in current if not c.startswith(PREFIX)]
    ours_before = {(p, c) for p, c in current if c.startswith(PREFIX)}
    ours = [(p, name) for name, klass in sorted(wanted.items()) for p in klass.patterns]
    changed = len(ours_before.symmetric_difference(set(ours)))
    return theirs + ours, changed


#: The parameters a class carries, as `(netlist field, NETCLASS getter suffix)` — the wrapper
#: spells each `HasX`/`GetX`/`SetX`.
PARAMETERS = (
    ("track_width", "TrackWidth"),
    ("clearance", "Clearance"),
    ("via_diameter", "ViaDiameter"),
    ("via_drill", "ViaDrill"),
)


def _rules(klass: NetClass) -> dict:
    rules = {name: _mm(getattr(klass, name)) for name, _ in PARAMETERS}
    rules["description"] = klass.description
    return rules


def _mm(text: str) -> float | None:
    return float(text) if text else None


def apply(board, wanted: dict[str, NetClass], on_board: set[str]) -> Classed:
    """Brings the board's net classes and pattern assignments in line with the design's.

    `on_board` is every net name the sync left on the board, used only to retire the per-net
    label assignments an earlier sync wrote before the assignments became patterns.
    """
    outcome = Classed()
    settings = board.GetDesignSettings().m_NetSettings

    existing = {}
    for name, held in settings.GetNetclasses().items():
        rules = {
            field: _held(getattr(held, f"Has{getter}")(), getattr(held, f"Get{getter}")())
            for field, getter in PARAMETERS
        }
        rules["description"] = str(held.GetDescription())
        existing[str(name)] = rules
    create, update, remove = plan(existing, wanted)

    for name in create + update:
        klass = wanted[name]
        # Built without defaults, so a parameter the design did not set stays unset — a kind
        # class sets none, and the rule class beside it or the board's default fills them in,
        # which is what KiCad does with a composite anyway. A rule class sets all of them.
        fresh = pcbnew.NETCLASS(name, False)
        fresh.SetDescription(klass.description)
        for field, getter in PARAMETERS:
            value = getattr(klass, field)
            if value:
                getattr(fresh, f"Set{getter}")(pcbnew.FromMM(float(value)))
        settings.SetNetclass(name, fresh)
    outcome.created, outcome.updated = create, update

    if remove:
        kept = pcbnew.netclasses_map()
        for name, held in settings.GetNetclasses().items():
            if str(name) not in remove:
                kept[name] = held
        settings.SetNetclasses(kept)
        outcome.removed = remove

    # The pattern table: a person's entries read back from the project file, ours regenerated.
    current, readable = _project_patterns(board)
    if not readable:
        outcome.problems.append(
            "no project file beside the board, so no net class patterns of yours could be read \
back before the table was rewritten — save the project first if you had any"
        )
    table, changed = patterns(current, wanted)
    settings.ClearNetclassPatternAssignments()
    for pattern, name in table:
        settings.SetNetclassPatternAssignment(pattern, name)
    outcome.assigned = changed
    outcome.kept = sum(1 for _, c in table if not c.startswith(PREFIX))

    # Label assignments were how an earlier sync assigned nets, one at a time; a board synced
    # then still carries them, and they would outlive the classes they point at.
    for net in sorted(on_board):
        if _assigned(settings, net) is not None:
            settings.ClearNetclassLabelAssignment(net)
            outcome.assigned += 1

    # What the editor itself does on save and on Board Setup's OK: clears the caches and hands
    # every net its class under the new table. The DRC engine's rules are the half this cannot
    # reach — see the module note — which is what `STALE_RULES` is for.
    board.SynchronizeNetsAndNetClasses(False)
    return outcome


def _project_patterns(board) -> tuple[list[tuple[str, str]], bool]:
    """The pattern table as the project file on disk has it, and whether there was one to read."""
    path = board.GetFileName()
    if not path:
        return [], False
    project = Path(path).with_suffix(".kicad_pro")
    if not project.is_file():
        return [], False
    try:
        settings = json.loads(project.read_text()).get("net_settings", {}) or {}
        entries = settings.get("netclass_patterns") or []
        return [(str(e.get("pattern", "")), str(e.get("netclass", ""))) for e in entries], True
    except (OSError, ValueError):
        return [], False


def _held(has: bool, value: int) -> float | None:
    return round(pcbnew.ToMM(value), 3) if has else None


def _assigned(settings, net: str) -> str | None:
    """The stackup class a net is label-assigned to, if any — the earlier sync's mechanism.

    The assignment map itself comes back from the wrapper as an opaque object, so the answer is
    read off the *effective* class instead — a composite whose constituents are what the label
    assignment contributed plus the rest. A stackup name in there with a label assignment present
    can only have come from that assignment.
    """
    if not settings.HasNetclassLabelAssignment(net):
        return None
    effective = settings.GetEffectiveNetClass(net)
    for part in str(effective.GetName()).split(","):
        if part.startswith(PREFIX):
            return part
    return None
