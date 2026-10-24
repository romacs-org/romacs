#!/usr/bin/env python3
"""Pin delete/0680, the one semantic (VALUE) row of the delete evidence.

Row: (let ((a0 (copy-sequence "")) (a1 (copy-sequence ""))) (let ((r (delete a0 a1))) (list (eq r a0) (eq r a1))))
GNU (t t), VM2 (nil t). `delete' returns SEQ itself here in both (ELT is not a
character), so (eq r a1) agrees; (eq r a0) differs only because GNU's empty
strings are ONE shared object (alloc.c empty_unibyte_string /
empty_multibyte_string; `copy-sequence' of "" returns it), while VM2 allocates
a fresh empty string each time: (eq (copy-sequence "") (copy-sequence "")) is t
in GNU, nil in VM2, before this port. DIVERGENCES.md VM2-EMPTYSTR1.
Only the pinned map is edited; no hash or recorded expectation is touched.
"""
import json, sys
from pathlib import Path
HERE = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(HERE))
import primh

ROW = "delete/0680"
spec = primh.load_spec(HERE / "specs" / "delete.json")
ev = primh.evidence(spec)  # refuses stale evidence
row = next(r for r in ev["cases"] if r["id"] == ROW)
assert row["expected"] == "(ok (t t))", row["expected"]
assert "(copy-sequence \"\")) (a1 (copy-sequence \"\"))" in row["source"], row["source"]
ev.setdefault("pinned", {})[ROW] = "VALUE"
primh.atomic_json(primh.EVID / "delete.json", ev)
print(f"pinned {ROW} VALUE; {len(ev['pinned'])} pins total")
