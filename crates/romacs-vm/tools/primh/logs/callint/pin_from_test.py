#!/usr/bin/env python3
"""Write each evidence file's `pinned` map from a `cargo test --test primh_cmdloop`
log: every NEW DIVERGENCE the Rust replay (fresh VM per row, collection floor 1)
reports becomes a pin of the class it reported. Run only after each pinned row
has been checked against a DIVERGENCES.md row; prints the non-PRINTER pins.

  python3 pin_from_test.py TEST.log EVIDENCE_DIR
"""
import collections, json, re, sys
from pathlib import Path

log, evid = Path(sys.argv[1]).read_text(), Path(sys.argv[2])
pins = collections.defaultdict(dict)
for cls, case in re.findall(r"NEW DIVERGENCE \(([A-Z-]+)\): ([^\s:]+/\d{4})", log):
    pins[case.rsplit("/", 1)[0]][case] = cls
for name, found in sorted(pins.items()):
    path = evid / f"{name}.json"
    ev = json.loads(path.read_text())
    ev["pinned"] = {**ev.get("pinned", {}), **found}
    ev["pinned"] = dict(sorted(ev["pinned"].items()))
    path.write_text(json.dumps(ev, ensure_ascii=False, indent=1) + "\n")
    print(name, len(ev["cases"]), "pins", len(ev["pinned"]),
          {k: v for k, v in ev["pinned"].items() if v != "PRINTER"})
