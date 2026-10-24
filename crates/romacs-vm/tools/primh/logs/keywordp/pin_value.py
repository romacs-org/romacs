#!/usr/bin/env python3
"""Pin keywordp/0071 and keywordp/0079, the two VALUE rows of the keywordp evidence.

Rows: the printer family of the domain values (intern "") and (make-symbol "").
GNU prints a symbol with an empty name as `##' (print.c:2498, print_object,
Lisp_Symbol arm), so both answer (ok ##). VM2's printer emits nothing for an
empty name, so both answer (ok ). Neither row calls keywordp: the printer
family only prints the catalog value. keywordp of both values agrees with GNU
(rows keywordp/0026 and /0027, nil). The difference predates this port (the
23deaa2a binary gives the same two VALUE rows, fresh-red.json).
DIVERGENCES.md VM2-EMPTYSYM1.
Only the pinned map is edited; no hash or recorded expectation is touched.
"""
import json, sys
from pathlib import Path
HERE = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(HERE))
import primh

ROWS = {"keywordp/0071": "(intern \"\")", "keywordp/0079": "(make-symbol \"\")"}
spec = primh.load_spec(HERE / "specs" / "keywordp.json")
ev = primh.evidence(spec)  # refuses stale evidence
for rid, value in ROWS.items():
    row = next(r for r in ev["cases"] if r["id"] == rid)
    assert row["family"] == "printer", row
    assert row["expected"] == "(ok ##)", row["expected"]
    assert f"(list 'ok {value})" in row["source"], row["source"]
    ev.setdefault("pinned", {})[rid] = "VALUE"
primh.atomic_json(primh.EVID / "keywordp.json", ev)
print(f"pinned {sorted(ROWS)} VALUE; {len(ev['pinned'])} pins total")
