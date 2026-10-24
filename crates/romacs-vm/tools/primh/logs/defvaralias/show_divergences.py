#!/usr/bin/env python3
"""Show non-PASS defvaralias rows: id, verdict, source, GNU, VM2 (stream replay)."""
import json, subprocess, sys, importlib.util
from pathlib import Path
H = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("primh", H / "primh.py"); primh = importlib.util.module_from_spec(spec); spec.loader.exec_module(primh)
vm2 = sys.argv[1]; skip = set(sys.argv[2:])
s = primh.load_spec(H / "vars-specs/defvaralias.json"); ev = primh.evidence(s)
stdin = "".join(json.dumps({"source": r["source"]}) + "\n" for r in ev["cases"])
p = subprocess.run([vm2, "--gc-floor", "1", "--eval-stream"], input=stdin, capture_output=True, text=True, timeout=600)
for row, line in zip(ev["cases"], p.stdout.splitlines()):
    out = json.loads(line); res = out.get("result") or {}
    act = res["value"] if res.get("type") == "string" else f"VM-ERROR {out.get('error')}"
    v = primh.verdict(row, act)
    if v.startswith("PASS") or v in skip: continue
    print(row["id"], v, row["family"]); print("  src", row["source"][60:-36][:300]); print("  gnu", row["expected"][:200]); print("  vm2", act[:200])
