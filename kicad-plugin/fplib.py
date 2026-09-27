"""Turning `Capacitor_SMD:C_0603_1608Metric` into a directory on disk.

`PCB_IO.FootprintLoad` wants a *library path* — a `.pretty` directory — not the nickname a netlist
carries. KiCad resolves nicknames through `fp-lib-table`, which the API does not expose to Python
(there is no `FP_LIB_TABLE` in the SWIG wrapper), so the table is read here.

The tables nest: the user's global table usually has one `(lib (type "Table"))` entry pointing at
the table KiCad ships, which holds the real entries. So resolution follows those references rather
than assuming a flat file.
"""

from __future__ import annotations

import os
import re
from pathlib import Path

#: Where KiCad keeps the per-user table, newest version first.
_USER_TABLES = [
    Path.home() / "Library/Preferences/kicad" / version / "fp-lib-table"  # macOS
    for version in ("10.0", "9.0")
] + [
    Path.home() / ".config/kicad" / version / "fp-lib-table"  # Linux
    for version in ("10.0", "9.0")
]


class LibraryError(Exception):
    pass


def resolve(nickname: str, project_dir: Path | None = None) -> Path:
    """The `.pretty` directory a library nickname names."""
    for table in _tables(project_dir):
        for name, uri in _entries(table):
            if name == nickname:
                path = Path(_expand(uri, project_dir))
                if path.is_dir():
                    return path
                raise LibraryError(f"`{nickname}` resolves to {path}, which is not a directory")
    raise LibraryError(
        f"no footprint library named `{nickname}`.\n\n"
        "Looked in:\n  " + "\n  ".join(str(t) for t in _tables(project_dir))
    )


def load_path(footprint_id: str, project_dir: Path | None = None) -> tuple[Path, str]:
    """Splits `Lib:Name` and resolves the library half."""
    if ":" not in footprint_id:
        raise LibraryError(f"`{footprint_id}` is not a `Library:Footprint` reference")
    nickname, name = footprint_id.split(":", 1)
    return resolve(nickname, project_dir), name


def _tables(project_dir: Path | None) -> list[Path]:
    """Every table to consult, project-local first — the order KiCad itself uses."""
    tables = []
    if project_dir:
        local = project_dir / "fp-lib-table"
        if local.is_file():
            tables.append(local)
    tables += [table for table in _USER_TABLES if table.is_file()]

    # Follow `(type "Table")` entries, which is how the user's table points at the one KiCad ships.
    seen, queue, out = set(), list(tables), []
    while queue:
        table = queue.pop(0)
        if table in seen or not table.is_file():
            continue
        seen.add(table)
        out.append(table)
        for _, uri in _entries(table, types={"Table"}):
            queue.append(Path(_expand(uri, project_dir)))
    return out


def _entries(table: Path, types: set[str] | None = None) -> list[tuple[str, str]]:
    """`(name, uri)` for each `(lib …)` in a table, optionally filtered by `type`."""
    try:
        text = table.read_text(errors="replace")
    except OSError:
        return []
    out = []
    for line in re.findall(r"\(lib\s+(.*)", text):
        name = re.search(r'\(name\s+"?([^")]+)"?\s*\)', line)
        uri = re.search(r'\(uri\s+"?([^")]+)"?\s*\)', line)
        kind = re.search(r'\(type\s+"?([^")]+)"?\s*\)', line)
        if not (name and uri):
            continue
        if types is not None and (not kind or kind.group(1) not in types):
            continue
        out.append((name.group(1), uri.group(1)))
    return out


def _expand(uri: str, project_dir: Path | None) -> str:
    """Expands `${VAR}` the way KiCad does: the project directory, then the environment, then the
    footprint directory of an installed KiCad."""
    def replace(match: re.Match) -> str:
        var = match.group(1)
        if var == "KIPRJMOD" and project_dir:
            return str(project_dir)
        if var in os.environ:
            return os.environ[var]
        if var.endswith("FOOTPRINT_DIR"):
            for candidate in _INSTALLED_FOOTPRINT_DIRS:
                if candidate.is_dir():
                    return str(candidate)
        return match.group(0)

    return re.sub(r"\$\{([^}]+)\}", replace, uri)


_INSTALLED_FOOTPRINT_DIRS = [
    Path("/Applications/KiCad/KiCad.app/Contents/SharedSupport/footprints"),
    Path("/usr/share/kicad/footprints"),
    Path("/usr/local/share/kicad/footprints"),
]
