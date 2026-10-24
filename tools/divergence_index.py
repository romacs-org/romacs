#!/usr/bin/env python3
"""Derive a machine-readable index of GNU divergences from the prose ledgers.

Stage 1 of R98/R99. The point is stated in R99:

    Every answer is derived from the artefact it describes, carries its
    provenance, and states when it was computed. A fact with no derivation is
    a claim and must be labelled one.

So this does NOT introduce a second hand-maintained list. `DIVERGENCES.md` and
the per-crate `GNU-DIVERGENCES.md` files stay canonical and human-written; this
reads them and emits `DIVERGENCES.json` for agents. They cannot drift, because
the JSON has no independent content.

Why an index at all: R90 and R93 license breaking Lisp-visible behaviour on one
condition -- it may never be silent. Today "not silent" means written in a file
that nothing queries, so an agent calling `maphash` indirectly does not read H1,
it takes the error. An index keyed by symbol is what makes the licence real.

Symbols are derived from backticked identifiers in each row, then filtered:
Lisp names and known primitives are kept, prose and Rust type names are not.
Where derivation is wrong or incomplete a row may carry an explicit override:

    | H1 | ... | ... | ... | ... |   <!-- symbols: maphash, apply, funcall -->

Usage:
    python3 tools/divergence_index.py            # write DIVERGENCES.json
    python3 tools/divergence_index.py --check    # exit 1 if any row has no symbol
"""
from __future__ import annotations
import argparse, hashlib, json, pathlib, re, sys

ROOT = pathlib.Path(__file__).resolve().parents[1]

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

def source_fingerprint(paths: list[str]) -> str:
    """SHA256 of sorted relative paths and exact bytes, including this generator.

    Frame each UTF-8 path with NUL and its ASCII byte length with NUL before
    the content. The length makes boundaries unambiguous even for NUL bytes.
    History, HEAD, timestamps and the generated output are not inputs: committing
    a dirty ledger and its index together must not invalidate that index.
    """
    digest = hashlib.sha256()
    for path in sorted(paths):
        content = (ROOT / path).read_bytes()
        digest.update(path.encode('utf-8') + b'\0')
        digest.update(str(len(content)).encode('ascii') + b'\0')
        digest.update(content)
    return digest.hexdigest()

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
    for line in path.read_text().split('\n'):
        h = re.match(r'^##+\s+(.*)', line)
        if h:
            section = h.group(1).strip()
            continue
        if not line.startswith('| ') or re.match(r'^\|[\s:|-]+\|$', line):
            continue
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
        if ident.lower() in ('#', 'crate', 'mechanism', 'what', 'lane'):
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

def write_if_changed(out: pathlib.Path, index: dict) -> str:
    """Compare complete deterministic bytes, repairing stale provenance too."""
    content = (json.dumps(index, indent=1, ensure_ascii=False) + '\n').encode('utf-8')
    if out.exists() and out.read_bytes() == content:
        return 'unchanged'
    out.write_bytes(content)
    return 'written'


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument('--check', action='store_true',
                    help='exit 1 if a row yields no symbol (R99: unattached rows are invisible)')
    args = ap.parse_args()

    ledgers = ['DIVERGENCES.md'] + [p.relative_to(ROOT).as_posix()
                                     for p in sorted(ROOT.glob('crates/*/GNU-DIVERGENCES.md'))]
    inputs = sorted([*ledgers, 'tools/divergence_index.py'])
    rows = []
    for source in ledgers:
        rows += parse(ROOT / source, source)

    by_symbol: dict[str, list[int]] = {}
    for i, r in enumerate(rows):
        for s in r['symbols']:
            by_symbol.setdefault(s, []).append(i)

    index = {
        'provenance': {
            'generated_by': 'tools/divergence_index.py',
            'input_hash': source_fingerprint(inputs),
            'inputs': inputs,
            'note': 'Derived from the prose ledgers. Do not edit; edit those and regenerate.',
        },
        'counts': {'rows': len(rows), 'symbols': len(by_symbol),
                   'rows_without_symbols': sum(1 for r in rows if not r['symbols'])},
        'by_symbol': {k: sorted(v) for k, v in sorted(by_symbol.items())},
        'rows': rows,
    }

    unattached = [r for r in rows if not r['symbols']]
    if args.check:
        print(f"  {len(rows)} rows, {len(by_symbol)} symbols, {len(unattached)} unattached")
        if unattached:
            print("  FAIL: a divergence with no symbol attached is invisible to an agent;")
            print("  add a `<!-- symbols: a, b -->` comment to each row below (R98/R99).")
            for r in unattached:
                print(f"    [{r['source']}] {r['mechanism'][:72]}")
            return 1
        return 0

    out = ROOT / 'DIVERGENCES.json'
    status = write_if_changed(out, index)
    print(f"  {status} {out.relative_to(ROOT)}: {len(rows)} rows, {len(by_symbol)} symbols, "
          f"{len(unattached)} unattached")
    return 0

if __name__ == '__main__':
    sys.exit(main())
