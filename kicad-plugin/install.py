#!/usr/bin/env python3
"""Install the stackup action plugins into a user's KiCad scripting directory."""

from __future__ import annotations

import argparse
import os
import platform
import re
import shutil
import sys
import tempfile
from pathlib import Path


def kicad_data_root() -> Path:
    system = platform.system()
    if system == "Darwin":
        return Path.home() / "Documents" / "KiCad"
    if system == "Windows":
        return Path(os.environ["APPDATA"]) / "kicad"
    return Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local" / "share")) / "kicad"


def plugin_parent(version: str | None) -> Path:
    root = kicad_data_root()
    if version is None:
        versions = [
            path.name for path in root.iterdir()
            if path.is_dir() and re.fullmatch(r"\d+\.\d+", path.name)
        ] if root.is_dir() else []
        if not versions:
            raise ValueError(f"no KiCad user directory found in {root}; pass --version or --target")
        version = max(versions, key=lambda value: tuple(map(int, value.split("."))))
    if not re.fullmatch(r"\d+\.\d+", version):
        raise ValueError("--version must be MAJOR.MINOR, for example 10.0")
    installation = root / version
    if not installation.is_dir():
        raise ValueError(f"KiCad {version} user directory not found in {root}")
    return installation / "scripting" / "plugins"


def cli_path(given: str | None) -> Path:
    value = given or shutil.which("stackup")
    if not value:
        raise ValueError("stackup CLI not found; install it on PATH or pass --cli /path/to/stackup")
    path = Path(value).expanduser().resolve()
    if not path.is_file() or not os.access(path, os.X_OK):
        raise ValueError(f"stackup CLI is not executable: {path}")
    return path


def install(parent: Path, cli: Path, replace: bool = False) -> Path:
    source = Path(__file__).resolve().parent
    destination = parent / "stackup"
    parent.mkdir(parents=True, exist_ok=True)
    if destination.exists() or destination.is_symlink():
        if not replace:
            raise FileExistsError(f"{destination} already exists; pass --replace to update it")
    with tempfile.TemporaryDirectory(prefix="stackup-install-", dir=parent) as temporary:
        staged = Path(temporary) / "stackup"
        shutil.copytree(
            source, staged,
            ignore=shutil.ignore_patterns("__pycache__", "*.pyc", "stackup-cli.txt"),
        )
        (staged / "stackup-cli.txt").write_text(f"{cli}\n")
        backup = Path(temporary) / "previous"
        if destination.exists() or destination.is_symlink():
            destination.rename(backup)
        try:
            staged.rename(destination)
        except OSError:
            if backup.exists() or backup.is_symlink():
                backup.rename(destination)
            raise
    return destination


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", help="KiCad version, such as 10.0; defaults to the newest installed")
    parser.add_argument("--target", type=Path, help="plugin directory; useful for custom KiCad installs")
    parser.add_argument("--cli", help="absolute path to the stackup KDL executable")
    parser.add_argument("--replace", action="store_true", help="replace an existing stackup plugin")
    args = parser.parse_args()
    try:
        parent = args.target.expanduser().resolve() if args.target else plugin_parent(args.version)
        destination = install(parent, cli_path(args.cli), args.replace)
    except (OSError, ValueError) as error:
        parser.exit(1, f"{error}\n")
    print(f"Installed stackup plugin in {destination}")
    print("Refresh Plugins in KiCad's PCB editor, or restart the editor.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
