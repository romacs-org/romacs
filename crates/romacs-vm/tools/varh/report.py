#!/usr/bin/env python3
"""Render README score/pin/phantom tables from saved replay outputs.

  report.py RUST_LOG FRESH_JSON SINGLE_JSON RED_JSON > tables.md

Reads only saved evidence and replay logs; runs nothing.
"""
import ast, json, re, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ORDER = ["PASS", "PASS-REGIME", "VALUE", "DATA", "CONDITION", "SIGNAL-VS-VALUE", "MISSING",
         "PRINTER", "VM-INTERNAL", "VM-CRASH"]

# One-line explanation per pinned row (hand-written; the class is the oracle's).
PIN_WHY = {
    "per-buffer/043": "make-local-variable of a bool built-in: VM2 signals, GNU localizes",
    "per-buffer/049": "set-default of a localized bool built-in to a non-Boolean (71): VM2 refuses (VM2-CVAR2), GNU stores it",
    "local-variable-if-set-p/012": "VM2 BUG: local-variable-if-set-p has arity 1; GNU (subr-arity) is (1 . 2)",
    "local-variable-if-set-p/013": "same arity bug (per-buffer variable)",
    "local-variable-if-set-p/014": "same arity bug (control: ordinary defvar'd variable; fails at 6eb76ddc too)",
    "local-variable-if-set-p/015": "same arity bug (control: make-variable-buffer-local'd variable)",
}


def table(rows, cols):
    out = ["| " + " | ".join(cols) + " |", "|" + "---|" * len(cols)]
    out += ["| " + " | ".join(str(c) for c in r) + " |" for r in rows]
    return "\n".join(out)


def esc(s, n=110):
    s = (s or "").replace("|", "\\|")
    return s if len(s) <= n else s[:n] + "…"


def cause(row, single):
    v, kind = row["variable"], row["kind"]
    if single and "buffer-local variable an alias" in single:
        return f"{v} was left buffer-local (LOCALIZED) by earlier make-local-variable rows"
    if row["id"].startswith("defvaralias/017"):
        return f"{v} left buffer-local by earlier rows, so this row's defvaralias fails silently"
    if row["family"] == "makunbound" and "varh-l" == v and "(nil nil)" in (single or ""):
        return f"{v}'s default left void by an earlier makunbound row"
    if "varh-alias" in row["form"] and row["family"] == "defvaralias":
        return f"varh-alias/{v} values left by earlier defvaralias rows (alias + setq 85 persist)"
    local = kind in ("per-buffer", "control-local")
    if local and ("get-buffer-create" in row["form"]):
        return f"{v} local values left in the reused buffers varh-a/varh-b and default changed by earlier rows"
    if local:
        return f"{v} local value left in the shared current buffer / default changed by earlier rows"
    return f"global value of {v} left by earlier set/setq/set-default rows"


def counts_line(c):
    return [c.get(k, 0) for k in ORDER]


def main(rust_log, fresh_json, single_json, red_json):
    ev = json.loads((HERE / "evidence" / "varh.json").read_text())
    rows = {r["id"]: r for r in ev["cases"]}
    m = re.search(r"varh Rust replay \(fresh Vm per case, collection floor 1\): (\d+) cases (\{.*\})", Path(rust_log).read_text())
    rust = {k: int(v) for k, v in re.findall(r'"([A-Z-]+)": (\d+)', m.group(2))}
    fresh, single, red = (json.loads(Path(p).read_text()) for p in (fresh_json, single_json, red_json))
    print("### Scores (GNU unavailable: PATH=/nonexistent GNU=/no/such/emacs EMACS=/no/such/emacs)\n")
    print(table([
        ["(a) Rust fresh-VM replay `cargo test --test varh` (AUTHORITATIVE)", m.group(1), *counts_line(rust)],
        ["(b) Python fresh replay `varh.py replay --fresh`", fresh["total"], *counts_line(fresh["counts"])],
        ["(c) Python single-VM replay `varh.py replay --single` (never pins)", single["total"], *counts_line(single["counts"])],
        ["red-first: Python fresh replay, VM2 at 6eb76ddc", red["total"], *counts_line(red["counts"])],
    ], ["replay", "total", *ORDER]))
    print("\n### Per family (b: Python fresh replay at 52fde6b2 | red-first: Python fresh replay at 6eb76ddc)\n")
    fams = list(dict.fromkeys(r["family"] for r in ev["cases"]))
    t = []
    for f in fams:
        n = sum(1 for r in ev["cases"] if r["family"] == f)
        ok = lambda rep: sum(v for k, v in rep["families"].get(f, {}).items() if k.startswith("PASS"))
        t.append([f, n, ok(fresh), n - ok(fresh), ok(red), n - ok(red)])
    print(table(t, ["family", "cases", "pass (b)", "fail (b)", "pass @6eb76ddc", "fail @6eb76ddc"]))
    kinds = list(dict.fromkeys(r["kind"] for r in ev["cases"]))
    print("\n### Red-first by kind (Python fresh replay, VM2 at 6eb76ddc)\n")
    t = []
    for k in kinds:
        ids = [r["id"] for r in ev["cases"] if r["kind"] == k]
        bad = sum(1 for i in ids if not red["verdicts"][i].startswith("PASS"))
        t.append([k, len(ids), len(ids) - bad, bad])
    print(table(t, ["kind", "cases", "pass", "fail"]))
    print(f"\n### Pinned rows ({len(ev['pinned'])}, set from the fresh replay)\n")
    print(table([[i, c, esc(rows[i]["form"], 90), PIN_WHY.get(i, "UNEXPLAINED")] for i, c in ev["pinned"].items()],
                ["id", "class", "form", "why"]))
    ph = single["phantoms"]
    print(f"\n### Phantom rows of the single-VM replay ({len(ph)}; all PASS/PASS-REGIME in a fresh VM)\n")
    print(table([[p["id"], f"{p['fresh']} → {p['single']}", cause(rows[p["id"]], p["single_actual"]),
                  esc(rows[p["id"]]["expected"], 50), esc(p["single_actual"], 50)] for p in ph],
                ["id", "fresh → single", "cause (state leaked from earlier rows in the one VM)", "GNU", "single-VM VM2"]))


if __name__ == "__main__":
    main(*sys.argv[1:5])
