#!/usr/bin/env python3
"""Synchronize a KiCad PCB from the KDL design named in its .stackup_sch sidecar.

Run with KiCad's bundled Python, which provides pcbnew. Close the PCB editor first: this
command edits the board file on disk and refuses KiCad's open-board lock file.
"""

from __future__ import annotations

import argparse
import importlib
import sys
import types
from pathlib import Path

#: This file lives *inside* the plugin directory, so the modules it needs are its own siblings.
#: Derived rather than configured: a copy of this script somewhere else would otherwise sync a
#: board using whichever plugin a path constant happened to name, which is the one bug a
#: single-file tool like this can have.
PLUGIN_DIR = Path(__file__).resolve().parent


def plugin_modules():
    """Import the sync modules without registering KiCad toolbar actions.

    `apply` imports its
    neighbours relatively, so it needs *a* parent package; and the real `__init__.py` registers
    toolbar buttons, which needs a live wxApp and would fail here. A bare namespace pointed at this
    directory gives the relative imports something to resolve against without executing the package
    body — so what gets imported is exactly the code the button runs, and no more.
    """
    package = types.ModuleType("stackup_plugin")
    package.__path__ = [str(PLUGIN_DIR)]
    sys.modules["stackup_plugin"] = package
    return tuple(
        importlib.import_module(f"stackup_plugin.{name}")
        for name in ("board_config", "netlist", "apply")
    )


def lockfile_of(board: Path) -> Path:
    """The lock pcbnew drops beside a board it has open: `~<name>.lck`."""
    return board.with_name(f"~{board.name}.lck")


def sync(board_path: Path, force: bool = False) -> str:
    """Bring `board_path` in line with the design its sidecar names. Returns the summary line."""
    import pcbnew

    board_config, netlist, apply_module = plugin_modules()

    board_path = board_path.resolve()
    if not board_path.is_file():
        raise SystemExit(f"no such board: {board_path}")

    lock = lockfile_of(board_path)
    if lock.exists() and not force:
        raise SystemExit(
            f"{board_path.name} is open in pcbnew ({lock.name} is beside it).\n"
            "This writes the board file directly, and the editor would save over it. Close the "
            "board and run again — or press *Sync PCB from stackup* in the editor instead, which "
            "is the same update applied to the copy the editor is holding."
        )

    # Read the link from the path rather than from the loaded board: it lives in the sidecar.
    link = board_config.parse_link(board_config.link_of(board_path), board_path)
    command = board_config.find_cli(link)
    text = board_config.build_netlist(link, command)
    components = netlist.parse(text)
    classes = netlist.classes(text)
    if not components:
        raise SystemExit(f"the design has no parts in it: {link.file}")

    # `LoadBoard` reads the project beside the board too, which is where the net classes live —
    # and `SaveBoard` writes it back unless told to skip settings, so a class set here persists
    # the way one set in the editor does.
    board = pcbnew.LoadBoard(str(board_path))
    outcome = apply_module.apply(board, components, classes)

    pcbnew.SaveBoard(str(board_path), board)
    return outcome.summary()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Sync a stackup board from its design, without the editor."
    )
    parser.add_argument("board", type=Path, help="the .kicad_pcb to bring in line")
    parser.add_argument(
        "--force",
        action="store_true",
        help="sync even though a lock file suggests pcbnew has the board open",
    )
    args = parser.parse_args(argv)

    print(sync(args.board, force=args.force))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
