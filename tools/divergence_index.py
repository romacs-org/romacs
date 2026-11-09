#!/usr/bin/env python3
"""Derive a machine-readable index of GNU divergences from the prose ledgers.

Stage 1 of R98/R99. The point is stated in R99:

    Every answer is derived from the artefact it describes, carries its
    provenance, and states when it was computed. A fact with no derivation is
    a claim and must be labelled one.

So this does NOT introduce a second hand-maintained list. `DIVERGENCES.md` and
the per-crate `GNU-DIVERGENCES.md` files stay canonical and human-written; this
reads them when asked. Nothing derived is committed, so nothing can drift.

Why an index at all: R90 and R93 license breaking Lisp-visible behaviour on one
condition -- it may never be silent. Today "not silent" means written in a file
that nothing queries, so an agent calling `maphash` indirectly does not read H1,
it takes the error. An index keyed by symbol is what makes the licence real.

Symbols are derived from backticked identifiers in each row, then filtered:
Lisp names and known primitives are kept, prose and Rust type names are not.
Where derivation is wrong or incomplete a row may carry an explicit override:

    | H1 | ... | ... | ... | ... |   <!-- symbols: maphash, apply, funcall -->

Usage:
    python3 tools/divergence_index.py SYMBOL...  # the rows naming SYMBOL, as JSON
    python3 tools/divergence_index.py            # counts
    python3 tools/divergence_index.py --check    # exit 1 if any row has no symbol
    python3 tools/divergence_index.py --names    # exit 1 if a file lacks the ID it is named by
"""
from __future__ import annotations
import argparse, json, pathlib, re, sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
DIR = pathlib.Path('ledger/divergences')
NOT_AN_ID = ('#', 'crate', 'mechanism', 'what', 'lane')
SEPARATOR = re.compile(r'^\|[\s:|-]+\|$')

# Backticked tokens that are structure, types or prose rather than Lisp symbols.
# Kept deliberately small: a false keep is visible in the output and easy to
# fix, a false drop silently hides a divergence, which is the failure that
# matters here.
NOT_A_SYMBOL = re.compile(
    r'''^( .*\.(c|rs|el|md|json)(:\d+([-:]\d+)?)?   # src/xfaces.c:1976-2000
        | romacs(-[a-z0-9]+)*                        # crate names
        | [A-Z][A-Za-z0-9_]*                         # Rust types: BoundedDatum, View
        | :[a-z-]+                                   # keywords: :inherit, :font
        | \{.*\}                                     # json fragments
        | [0-9^*>= ]+                                # numbers / operators
        | .*::.*                                     # Rust paths: CaseTable::translate
        | .*\(\)                                     # Rust calls: char_table_write_tick()
        | [-%].*                                     # suffixes/format specs: -values, %S
        | .*[A-Z].*                                  # anything capitalised: Foo-Bar
        )$''', re.X)

def symbols_in(cells: list[str], override: str | None) -> list[str]:
    if override:
        return [s.strip() for s in override.split(',') if s.strip()]
    out: list[str] = []
    for tok in re.findall(r'`([^`]+)`', ' '.join(cells)):
        tok = tok.strip()
        # `(require 'transient)` -> transient ; `#'maphash` -> maphash
        m = re.fullmatch(r"\(require '([a-z0-9-]+)\)", tok) or re.fullmatch(r"#'([a-z0-9-]+)", tok)
        if m:
            tok = m.group(1)
        if NOT_A_SYMBOL.match(tok) or ' ' in tok:
            continue
        if tok not in out:
            out.append(tok)
    return out

def parse(path: pathlib.Path, source: str) -> list[dict]:
    rows: list[dict] = []
    section = None
    lines = path.read_text().split('\n')
    for i, line in enumerate(lines):
        h = re.match(r'^##+\s+(.*)', line)
        if h:
            section = h.group(1).strip()
            continue
        if not line.startswith('| ') or SEPARATOR.match(line):
            continue
        if i + 1 < len(lines) and SEPARATOR.match(lines[i + 1].strip()):
            continue          # a table's header row
        override = None
        if re.search(r'<!--\s*not-a-divergence\s*-->', line):
            continue          # e.g. a table documenting where code moved
        mo = re.search(r'<!--\s*symbols:\s*([^>]*?)\s*-->', line)
        if mo:
            override = mo.group(1)
            line = line[:mo.start()]
        cells = [c.strip() for c in line.strip().strip('|').split('|')]
        if len(cells) < 4:                      # header rows, detail tables
            continue
        ident = cells[0]
        if ident.lower() in NOT_AN_ID:
            continue
        rows.append({
            'id': ident if ident not in ('—', '-', '') else None,
            'source': source,
            'section': section,
            'observable': bool(section and 'Observable by a package' in section),
            'planned': bool(section and 'Planned' in section),
            'mechanism': cells[1],
            'gnu': cells[2],
            'romacs': cells[3],
            'why': cells[4] if len(cells) > 4 else '',
            'symbols': symbols_in(cells[1:4], override),
            'symbols_derived': override is None,
        })
    return rows

def rows(root: pathlib.Path = ROOT) -> list[dict]:
    """Every divergence row, read from one file per divergence."""
    return [row for path in sorted((root / DIR).glob('*.md')) if path.name != 'README.md'
            for row in parse(path, (DIR / path.name).as_posix())]


ID_SHAPED = re.compile(r'^[A-Za-z][A-Za-z0-9]*(?:-[A-Za-z0-9]+)*$')
# Row labels of comparison tables (`| GNU | ... |`, `| VM2 | ... |`), not IDs.
LABELS = {'GNU', 'VM2', 'Romacs'}


def owned_ids(path: pathlib.Path) -> set[str]:
    """The IDs PATH gives: each divergence row's ID-shaped first cell, and
    each heading that is an ID (`## ID`, `## ID — title`, `## ID / ...`)."""
    rows = {r['id'] for r in parse(path, '') if r['id'] and ID_SHAPED.match(r['id']) and r['id'] not in LABELS}
    heads = {m[1] for m in re.finditer(r'^## ([A-Z][A-Z0-9]*(?:-[A-Z0-9]+)*)(?=\s*(?:—|/|$))', path.read_text(), re.M)}
    return rows | heads


def misnamed(root: pathlib.Path = ROOT) -> list[str]:
    """Why the files break the rule of one file per divergence: an ID held
    by two files, a file named by an ID that does not hold it, or conflict
    markers. A `U-` file holds an entry that never had an ID."""
    found, owners = [], {}
    paths = [p for p in sorted((root / DIR).glob('*.md')) if p.name != 'README.md']
    for path in paths:
        if re.search(r'^(<{7}|={7}|>{7})( |$)', path.read_text(), re.M):
            found.append(f'{DIR / path.name}: unresolved conflict markers')
        for ident in owned_ids(path):
            owners.setdefault(ident, []).append(path.name)
    for ident, holders in sorted(owners.items()):
        if len(holders) > 1:
            found.append(f'{ident}: held by ' + ', '.join(f'{DIR / h}' for h in holders))
    for path in paths:
        if not path.stem.startswith('U-') and path.name not in owners.get(path.stem, []):
            found.append(f'{DIR / path.name}: holds no row or section named {path.stem}')
    return found


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument('symbols', nargs='*', help='print the rows that name these symbols, as JSON')
    ap.add_argument('--check', action='store_true',
                    help='exit 1 if a row yields no symbol (R99: unattached rows are invisible)')
    ap.add_argument('--names', action='store_true', help='exit 1 if a file does not hold the ID it is named by')
    args = ap.parse_args()
    if args.symbols and (args.check or args.names):
        ap.error('a lookup takes no --check or --names; run them on their own')
    if args.names:
        problems = misnamed()
        for problem in problems:
            print(f'divergences: {problem}', file=sys.stderr)
        return 1 if problems else 0

    found = rows()
    for symbol in args.symbols:
        print(json.dumps([r for r in found if symbol in r['symbols']], ensure_ascii=False))
    if args.symbols:
        return 0

    symbols = {s for r in found for s in r['symbols']}
    unattached = [r for r in found if not r['symbols']]
    print(f"  {len(found)} rows, {len(symbols)} symbols, {len(unattached)} unattached")
    if args.check and unattached:
        print("  FAIL: a divergence with no symbol attached is invisible to an agent;")
        print("  add a `<!-- symbols: a, b -->` comment to each row below (R98/R99).")
        for r in unattached:
            print(f"    [{r['source']}] {r['mechanism'][:72]}")
        return 1
    return 0

if __name__ == '__main__':
    sys.exit(main())
