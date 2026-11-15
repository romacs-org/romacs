"""The Lisp tree's layout for the Python tools, from lisp/bundled.json, which Rust and
romacs-bundled-autoloads.el read too: Romacs's subsystem directories, the packages' directory,
the directories on no load-path, and `files' the bundled-autoloads scan holds besides.
crates/romacs-vm-gui/src/startup.rs runtime_load_path follows the same rules.
"""
import json
from pathlib import Path


def layout(root: Path) -> dict:
    return json.loads((root / 'lisp/bundled.json').read_text())


def own_directories(root: Path) -> set[str]:
    """The names under lisp/ that are not GNU's directories."""
    data = layout(root)
    return {*data['directories'], *data['off_path'], data['packages']}


def is_gnu_directory(root: Path, directory: Path) -> bool:
    return directory.parent == root / 'lisp' and directory.name not in own_directories(root)


def romacs_load_path(root: Path) -> list[Path]:
    """Romacs's subsystem directories in their load-path order, then each package."""
    data = layout(root)
    lisp = root / 'lisp'
    path = [lisp / name for name in data['directories'] if (lisp / name).is_dir()]
    packages = lisp / data['packages']
    if packages.is_dir():
        path += sorted(p for p in packages.iterdir() if p.is_dir())
    return path


def census_files(root: Path) -> list[Path]:
    """The files the census loads: every .el under lisp/ (ignored build output too) that the
    bundled-autoloads scan does not hold and that is outside the VM's halves. Romacs's runtime files
    are load scripts or have suites of their own; the rest are the libraries the census has always
    measured, GNU's among them, and the generated files (generated_files) as support."""
    lisp = root / 'lisp'
    off = set(layout(root)['off_path'])
    return sorted(p for p in lisp.rglob('*.el')
                  if p.relative_to(lisp).parts[0] not in off and not is_scanned(root, p))


def generated_files(root: Path) -> set[str]:
    """The generated files lisp/ keeps, relative to ROOT: lisp/GENERATED's and the loaddefs files."""
    listed = {line.strip() for line in (root / 'lisp/GENERATED').read_text().splitlines()
              if line.strip() and not line.startswith('#')}
    return listed | {str(p.relative_to(root)) for p in (root / 'lisp').rglob('*loaddefs.el')}


def loaddefs_directories(root: Path) -> list[Path]:
    """The directories tools/lisp-loaddefs generates the loaddefs files from: GNU's build rule
    (every directory of lisp/ but obsolete/ and term/, lisp/Makefile.in SUBDIRS_ALMOST), which takes
    Romacs's subsystem directories in too, less the bundled packages and the VM's halves."""
    lisp = root / 'lisp'
    data = layout(root)
    out = (set(data['off_path']) | {data['packages'], 'obsolete', 'term'})
    return sorted(d for d in [lisp, *lisp.rglob('*')] if d.is_dir()
                  and (d == lisp or d.relative_to(lisp).parts[0] not in out))


def is_scanned(root: Path, path: Path) -> bool:
    """Whether the bundled-autoloads scan, not the loaddefs files, holds PATH's autoloads: Romacs's
    runtime files (its subsystem directories and `romacs-*' files) and the packages, whose
    cookies run after loadup; lisp/bundled.json `files' names any other."""
    lisp = root / 'lisp'
    data = layout(root)
    rel = path.relative_to(lisp)
    return (rel.parts[0] in {*data['directories'], data['packages']} or path.name.startswith('romacs-')
            or str(path.relative_to(root)) in data.get('files', []))


def scanned_files(root: Path) -> list[Path]:
    """The .el files is_scanned holds, outside the VM's halves."""
    lisp = root / 'lisp'
    off = set(layout(root)['off_path'])
    return sorted(p for p in lisp.rglob('*.el') if p.relative_to(lisp).parts[0] not in off and is_scanned(root, p))
