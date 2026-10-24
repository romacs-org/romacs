#!/usr/bin/env python3
"""Fresh-VM replay counts for an effect=global primitive.

primh.py's Python replay streams every case through ONE VM process, so a
global effect (defvaralias leaves symbols aliased) leaks into later rows. The
Rust replay (tests/primh_<name>.rs) uses a fresh Vm per row; this mirrors it in
Python with one --eval-stream process per row, so the counts it prints are the
ones the Rust test certifies. Verdicts and pins are primh.py's own.

  fresh_replay.py SPEC VM2-BINARY [--json OUT] [--pin]

--pin records every non-PASS fresh-VM verdict as the evidence's pin map
(primh.py replay --pin does the same from its single-VM stream, which leaks
global state between rows).
"""
import argparse, importlib.util, json, subprocess
from pathlib import Path
H = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("primh", H / "primh.py")
primh = importlib.util.module_from_spec(spec); spec.loader.exec_module(primh)
ap = argparse.ArgumentParser(); ap.add_argument("spec"); ap.add_argument("vm2"); ap.add_argument("--json"); ap.add_argument("--pin", action="store_true")
a = ap.parse_args()
s = primh.load_spec(a.spec); ev = primh.evidence(s)
actuals = []
for row in ev["cases"]:
    p = subprocess.run([a.vm2, "--gc-floor", "1", "--eval-stream"], input=json.dumps({"source": row["source"]}) + "\n",
                       capture_output=True, text=True, timeout=60)
    lines = p.stdout.splitlines()
    out = json.loads(lines[0]) if lines else {}
    res = out.get("result") or {}
    actuals.append(res["value"] if res.get("type") == "string" else f"VM-ERROR {out.get('error')}" if lines else None)
counts, bad = primh.ratchet(ev, actuals)
if a.pin:
    ev["pinned"] = {r["id"]: primh.verdict(r, x) for r, x in zip(ev["cases"], actuals)
                    if not primh.verdict(r, x).startswith(("PASS", "EXCLUDED"))}
    primh.atomic_json(primh.EVID / f"{s['name']}.json", ev)
    counts, bad = primh.ratchet(ev, actuals)
print(f"{s['name']}: {len(ev['cases'])} cases, fresh VM per row: {json.dumps(counts, sort_keys=True)}")
for b in bad[:10]:
    print("  " + b)
print(f"  {len(bad)} ratchet violations")
if a.json:
    primh.atomic_json(Path(a.json), {s["name"]: counts, "ratchet_violations": len(bad)})
raise SystemExit(1 if bad else 0)
