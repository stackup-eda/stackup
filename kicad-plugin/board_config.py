"""Resolve a PCB's KDL sidecar and run the stackup netlist exporter."""

from __future__ import annotations

import os
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path

SIDECAR_EXTENSION = ".stackup_sch"


class SyncError(Exception):
    def __init__(self, summary: str, detail: str = ""):
        super().__init__(summary)
        self.summary = summary
        self.detail = detail


@dataclass
class BoardLink:
    file: Path
    directory: Path


def sidecar_of(board_path: Path) -> Path:
    return board_path.with_suffix(SIDECAR_EXTENSION)


def link_of(board_path: Path) -> str | None:
    try:
        lines = sidecar_of(board_path).read_text().splitlines()
    except OSError:
        return None
    return next((line.strip() for line in lines if line.strip() and not line.lstrip().startswith("#")), None)


def parse_link(value: str | None, path: Path) -> BoardLink:
    if not value:
        raise SyncError("board has no stackup KDL link", f"Write a path to board.kdl in {sidecar_of(path)}")
    file = (path.parent / value).resolve()
    if file.suffix != ".kdl" or not file.is_file():
        raise SyncError("stackup KDL design not found", str(file))
    return BoardLink(file=file, directory=path.parent)


def find_cli(link: BoardLink) -> list[str]:
    """Find the KDL CLI from an override, the installed plugin, PATH, or this checkout."""
    explicit = os.environ.get("STACKUP_CLI")
    if explicit:
        binary = Path(explicit).expanduser()
        if binary.is_file() and os.access(binary, os.X_OK):
            return [str(binary)]
        raise SyncError("STACKUP_CLI is not executable", str(binary))
    configured = Path(__file__).with_name("stackup-cli.txt")
    if configured.is_file():
        binary = Path(configured.read_text().strip()).expanduser()
        if binary.is_file() and os.access(binary, os.X_OK):
            return [str(binary)]
        raise SyncError("configured stackup CLI is not executable", str(binary))
    on_path = shutil.which("stackup")
    if on_path:
        return [on_path]
    repo = Path(__file__).resolve().parent.parent
    manifest = repo / "Cargo.toml"
    cargo = shutil.which("cargo") or str(Path.home() / ".cargo" / "bin" / "cargo")
    if manifest.is_file() and Path(cargo).is_file():
        return [cargo, "run", "--quiet", "--manifest-path", str(manifest), "-p", "stackup", "--"]
    raise SyncError("stackup CLI not found", "Set STACKUP_CLI to the stackup executable or put it on PATH")


def build_netlist(link: BoardLink, command: list[str]) -> str:
    argv = command + ["netlist", str(link.file)]
    try:
        result = subprocess.run(argv, cwd=link.directory, capture_output=True, text=True)
    except OSError as error:
        raise SyncError("could not run stackup", str(error)) from error
    if result.returncode:
        raise SyncError("stackup netlist failed", result.stderr.strip() or result.stdout.strip())
    if not result.stdout.strip():
        raise SyncError("stackup produced an empty netlist", " ".join(argv))
    return result.stdout
