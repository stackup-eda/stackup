"""KiCad action plugins for syncing and arranging stackup KDL PCB layouts."""

import importlib
import os

import pcbnew

from . import anchor as _anchor
from . import apply as _apply
from . import board_config as _board_config
from . import fplib as _fplib
from . import netclass as _netclass
from . import netlist as _netlist
from . import pour as _pour
from . import via as _via
from . import repeat as _repeat

#: Reloaded on every press, in dependency order. KiCad caches imported modules in `sys.modules`,
#: and *Refresh Plugins* re-runs discovery without re-importing a package's submodules — so editing
#: this plugin normally needs the whole editor restarted.
#:
#: The CLI exports the design on every press, so the KDL side is always current while the Python side silently is
#: not. That asymmetry produced exactly one confusing bug (new net names appearing, new footprint
#: fields not), and the fix belongs here rather than in a habit of restarting.
_RELOADABLE = (
    _board_config,
    _fplib,
    _netlist,
    _anchor,
    _pour,
    _via,
    _repeat,
    _netclass,
    _apply,
)


class StackupSync(pcbnew.ActionPlugin):
    def defaults(self):
        self.name = "Sync PCB from stackup"
        self.category = "stackup"
        self.description = "Export the linked KDL design and update this board from it"
        self.show_toolbar_button = True
        _icons(self, "icon")

    def Run(self):
        board = pcbnew.GetBoard()
        try:
            for module in _RELOADABLE:
                importlib.reload(module)
            outcome = _sync(board)
        except _board_config.SyncError as error:
            _say(error.summary, error.detail, error=True)
            return
        except Exception as error:  # a plugin crash should still explain itself
            _say("Sync failed", f"{type(error).__name__}: {error}", error=True)
            return

        _say(
            "Synced from stackup",
            outcome.summary(),
            error=bool(outcome.problems),
        )
        pcbnew.Refresh()


class StackupPlaceAtAnchor(pcbnew.ActionPlugin):
    """Move the selected parts onto the pads they were placed for.

    The sync will not do this to a footprint that already exists, and should not: it cannot tell a
    position someone chose from one it dropped there itself, and quietly undoing an afternoon's
    placement is the one unrecoverable thing a sync could do. Selecting the parts and pressing a
    button settles that — an explicit ask outranks the person's own earlier placement in a way an
    automatic pass never does.

    It reads the anchor straight off the footprint, so there is no cargo run and no netlist: this
    works instantly, offline, and on a board whose design does not currently build.
    """

    def defaults(self):
        self.name = "Place at anchor pad"
        self.category = "stackup"
        self.description = (
            "Move parts with a Stackup Anchor field onto their host pads, and draw copper "
            "and vias declared on selected pads"
        )
        self.show_toolbar_button = True
        _icons(self, "icon-anchor")

    def Run(self):
        board = pcbnew.GetBoard()
        try:
            for module in _RELOADABLE:
                importlib.reload(module)
            # Two selections, one button. A selected *part* is asking to be moved onto its pad; a
            # selected *pad* is asking for the copper the design declared on it. They cannot be
            # confused for each other, so there is nothing to disambiguate and no second button to
            # explain — and selecting a chip and a pad of it does both, which is what you want.
            outcome = _anchor.snap(board)
            outcome.poured = _via.apply(board, _pour.apply(board, _pour.Poured()))
        except Exception as error:  # a plugin crash should still explain itself
            _say("Could not place", f"{type(error).__name__}: {error}", error=True)
            return

        pcbnew.Refresh()
        if outcome.quiet():
            # No dialog on a clean run. Placing parts is a loop and a modal in the middle of it
            # costs more than the report is worth — the parts moving *is* the report. The line
            # still goes to the scripting console for anyone who wants a trace.
            print(
                f"stackup: placed {len(outcome.moved)} part(s) at their anchor pads, "
                f"poured {len(outcome.poured.filled)} pad(s), "
                f"placed vias under {len(outcome.poured.vias)}"
            )
            return
        _say("Place at anchor pad", outcome.summary(), error=bool(outcome.problems))


class StackupRepeat(pcbnew.ActionPlugin):
    """Copy the selected arrangement onto every other instance of its block.

    The selection is the whole of the instruction: the part the arrangement is around and the
    parts arranged about it. What is *not* selected is not touched on any copy, which is how an
    instance's other parts — the ones that belong somewhere else, beside the MCU that reads them —
    keep their own places.
    """

    def defaults(self):
        self.name = "Repeat arrangement"
        self.category = "stackup"
        self.description = (
            "Copy how the selected parts sit around one part onto every other part like it — "
            "arrange one bridge's capacitors and give the other bridges the same"
        )
        self.show_toolbar_button = True
        _icons(self, "icon-repeat")

    def Run(self):
        board = pcbnew.GetBoard()
        try:
            for module in _RELOADABLE:
                importlib.reload(module)
            outcome = _repeat.repeat(board)
        except Exception as error:  # a plugin crash should still explain itself
            _say("Could not repeat", f"{type(error).__name__}: {error}", error=True)
            return

        pcbnew.Refresh()
        if outcome.quiet():
            path, designator = outcome.anchor
            copper = f" — {outcome.copper_line()}" if outcome.copper else ""
            print(
                f"stackup: repeated {outcome.members} part(s) around {designator} ({path}) "
                f"onto {len(outcome.onto)} other(s){copper}"
            )
            return
        _say("Repeat arrangement", outcome.summary(), error=bool(outcome.problems))


def _sync(board):
    """Everything between the button and the board.

    Reaches through the module objects rather than importing the names directly, so that a reload
    above is actually seen here — a `from .apply import apply` would keep pointing at the function
    that existed when this module was first imported, which is the very staleness being fixed.
    """
    from pathlib import Path

    path = board.GetFileName()
    if not path:
        raise _board_config.SyncError(
            "this board has never been saved",
            "The design this board comes from is named by a file beside it. Save the board "
            "then press the button again.",
        )

    # Read from the path rather than the open board: the link is in the sidecar, not on the board.
    link = _board_config.parse_link(_board_config.link_of(Path(path)), Path(path))
    command = _board_config.find_cli(link)
    text = _board_config.build_netlist(link, command)
    components = _netlist.parse(text)
    classes = _netlist.classes(text)
    if not components:
        raise _board_config.SyncError("the design has no parts in it", str(link.file))
    return _apply.apply(board, components, classes)


def _icons(plugin, stem: str) -> None:
    """Points a plugin at `<stem>.png` and `<stem>-dark.png`, when they are there.

    Both, because `ActionPlugin.GetIconFileName(dark)` only reaches for the dark one if
    `dark_icon_file_name` has been set — a file sitting beside the light one is not enough, and a
    dark-mode toolbar showing a near-black glyph is the result.
    """
    here = os.path.dirname(__file__)
    light = os.path.join(here, f"{stem}.png")
    dark = os.path.join(here, f"{stem}-dark.png")
    plugin.icon_file_name = light if os.path.isfile(light) else ""
    plugin.dark_icon_file_name = dark if os.path.isfile(dark) else ""


def _say(title: str, message: str, error: bool = False) -> None:
    """Reports the outcome. Runs inside KiCad, so wx is really there — but a plugin that dies in
    its own error reporting is worse than one that prints."""
    try:
        import wx

        style = wx.ICON_ERROR if error else wx.ICON_INFORMATION
        wx.MessageBox(message, title, style=style | wx.OK)
    except Exception:
        print(f"{title}\n{message}")


for _plugin in (StackupSync, StackupPlaceAtAnchor, StackupRepeat):
    # One at a time, so a button that cannot register does not take the other down with it. That is
    # the same failure this whole block exists to prevent, one level up: two buttons registered in
    # one `try` are two buttons that vanish together.
    try:
        _plugin().register()
    except Exception as _error:
        # Registration needs a running wxApp, so it only succeeds inside the editor. Importing this
        # module anywhere else — a test, a headless `pcbnew` script — should still work, since
        # everything below `Run` is ordinary code worth exercising without the GUI.
        #
        # It is reported rather than swallowed: "the button is not there and nothing said why" is
        # the single worst failure this plugin has, and silence is what causes it. Inside KiCad
        # this line lands in the scripting console.
        print(
            f"stackup: could not register {_plugin.__name__}: "
            f"{type(_error).__name__}: {_error}"
        )
