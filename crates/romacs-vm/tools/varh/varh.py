#!/usr/bin/env python3
"""varh: differential harness for Emacs Lisp VARIABLE semantics in VM2.

  varh.py record [--gnu emacs] [--jobs N]     # live GNU -> evidence/varh.json (atomic)
  varh.py replay --fresh  --vm2 BIN [--json OUT] [--show N]   # one VM process per case
  varh.py replay --single --vm2 BIN [--json OUT] [--show N]   # ONE VM process for all
  varh.py pin --vm2 BIN                       # pins := non-PASS verdicts of the FRESH replay
  varh.py rust [--out ../../tests/varh.rs]    # emit the cargo replay test
  varh.py probe FORM [--setup S] [--gnu emacs] [--vm2 BIN]    # design aid; writes nothing

Variables are global state, so isolation is per CASE on both sides:
  GNU   a fresh `emacs -Q --batch` with a private temporary HOME per case per
        regime (two processes per case): byte-compiled via
        (funcall (byte-compile `(lambda () ,FORM))) and interpreted via
        (eval FORM t).  The byte-compiled outcome is `expected' (VM2 compiles);
        an interpreted outcome that differs is kept as `gnu_regime_variant'.
  VM2   a fresh VM per case (replay --fresh, the Rust test).  replay --single
        streams every case through one VM: global effects leak between rows,
        so its counts are reported separately and never set pins; rows whose
        single-VM verdict differs from the fresh verdict are PHANTOMS.

A case may carry SETUP (a top-level form evaluated first, on its own, in the
same fresh process / VM): defvars and helper defuns.  Expected values are
never hand-written: they are GNU's outcomes.
"""
import argparse, concurrent.futures, hashlib, json, os, shutil, subprocess, sys, tempfile, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
CASES = HERE / "cases.json"
DRIVER = HERE / "gnu_driver.el"
TEMPLATE = HERE / "replay.rs.in"
EVID = HERE / "evidence" / "varh.json"
PROTOCOL = "varh-v1"
WRAP = "(prin1-to-string (condition-case e (list 'ok {expr}) (error (list 'sig e))))"
REGIMES = ("compiled", "interpreted")


def sha(b):
    return hashlib.sha256(b).hexdigest()


def atomic_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, pending = tempfile.mkstemp(prefix=path.name + ".", suffix=".pending", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as stream:
            stream.write(json.dumps(value, ensure_ascii=False, indent=1) + "\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(pending, path)
    finally:
        if os.path.exists(pending):
            os.unlink(pending)


def hashes():
    return {"cases_sha256": sha(CASES.read_bytes()),
            "harness_sha256": sha(Path(__file__).read_bytes()),
            "driver_sha256": sha(DRIVER.read_bytes()),
            "replay_sha256": sha(TEMPLATE.read_bytes())}


def load_cases():
    rows = json.loads(CASES.read_text())
    ids = [r["id"] for r in rows]
    if len(set(ids)) != len(ids):
        raise ValueError("duplicate case ids in cases.json")
    out = []
    for r in rows:
        row = {k: r[k] for k in ("id", "family", "kind", "variable", "form")}
        row["setup"] = r.get("setup")
        row["source"] = WRAP.format(expr=r["form"])
        out.append(row)
    return out


def evidence():
    ev = json.loads(EVID.read_text())
    if ev.get("protocol") != PROTOCOL:
        raise ValueError("evidence protocol changed; re-record")
    for key, value in hashes().items():
        if ev.get(key) != value:
            raise ValueError(f"{key} changed since recording; re-record")
    keys = ("id", "family", "kind", "variable", "form", "setup", "source")
    if [{k: r.get(k) for k in keys} for r in ev["cases"]] != load_cases():
        raise ValueError("recorded cases differ from cases.json")
    return ev


# ---------------------------------------------------------------- GNU side

def gnu_one(gnu, row, regime, timeout):
    """One case, one regime, one fresh `emacs -Q --batch` with a private HOME."""
    with tempfile.TemporaryDirectory(prefix="varh-home.", dir=os.environ.get("TMPDIR")) as home:
        case = Path(home) / "case.json"
        case.write_text(json.dumps({"setup": row["setup"], "source": row["source"]}))
        env = dict(os.environ, HOME=home, VARH_CASE=str(case), VARH_REGIME=regime)
        try:
            p = subprocess.run([gnu, "-Q", "--batch", "-l", str(DRIVER)], env=env,
                               capture_output=True, text=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            raise ValueError(f"GNU timeout on {row['id']} ({regime}); refusing evidence")
    if p.returncode != 0:
        raise ValueError(f"GNU exited {p.returncode} on {row['id']} ({regime}): {p.stderr[-400:]}")
    lines = [l for l in p.stderr.splitlines() if l.startswith("{")]
    if len(lines) != 1:
        raise ValueError(f"GNU reply count {len(lines)} != 1 on {row['id']} ({regime}): {p.stderr[-400:]}")
    reply = json.loads(lines[0])
    if reply.get("regime") != regime or not isinstance(reply.get("outcome"), str):
        raise ValueError(f"GNU reply malformed on {row['id']} ({regime})")
    return reply


def record(args):
    gnu = shutil.which(args.gnu) or args.gnu
    rows = load_cases()
    t = time.monotonic()
    jobs = [(r, g) for r in rows for g in REGIMES]
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        replies = list(pool.map(lambda j: gnu_one(gnu, j[0], j[1], args.timeout), jobs))
    heads = {(x["version"], x["revision"]) for x in replies}
    if len(heads) != 1:
        raise ValueError(f"GNU identity changed during recording: {heads}")
    (version, revision), = heads
    by = {(r["id"], g): x["outcome"] for (r, g), x in zip(jobs, replies)}
    bad = [k for k, v in by.items() if v.startswith("HARNESS-ERROR")]
    if bad:
        for k in bad[:10]:
            print(f"  {k}: {by[k]}", file=sys.stderr)
        raise ValueError(f"{len(bad)} HARNESS-ERROR replies; refusing evidence")
    for r in rows:
        r["expected"] = by[(r["id"], "compiled")]
        interp = by[(r["id"], "interpreted")]
        if interp != r["expected"]:
            r["gnu_regime_variant"] = interp
    ev = dict(protocol=PROTOCOL, **hashes(),
              gnu=dict(version=version, revision=revision, binary=str(Path(gnu).resolve()),
                       binary_sha256=sha(Path(gnu).resolve().read_bytes())),
              regime=("GNU: fresh `emacs -Q --batch` (private HOME) per case per regime; expected = "
                      "(funcall (byte-compile `(lambda () ,FORM))), gnu_regime_variant = (eval FORM t) "
                      "when it differs. VM2: fresh Vm per case, compiled, collection floor 1."),
              cases=rows, pinned={})
    atomic_json(EVID, ev)
    variants = sum(1 for r in rows if "gnu_regime_variant" in r)
    print(f"recorded {len(rows)} cases x {len(REGIMES)} regimes ({len(jobs)} GNU processes) "
          f"in {time.monotonic() - t:.1f}s; GNU {version} {revision[:12]}; {variants} regime variants -> {EVID}")


# ---------------------------------------------------------------- VM2 side

def parse_reply(line):
    out = json.loads(line)
    result = out.get("result") or {}
    if result.get("type") == "string":
        return result["value"]
    return f"VM-ERROR {out.get('error')}"


def stream_input(row):
    lines = []
    if row["setup"] is not None:
        lines.append(json.dumps({"source": row["setup"]}))
    lines.append(json.dumps({"source": row["source"]}))
    return lines


def vm_fresh_one(vm2, row):
    lines = stream_input(row)
    try:
        p = subprocess.run([vm2, "--gc-floor", "1", "--eval-stream"], input="\n".join(lines) + "\n",
                           capture_output=True, text=True, timeout=120)
    except subprocess.TimeoutExpired:
        return None
    out = p.stdout.splitlines()
    if len(out) != len(lines):
        return None                                   # VM-CRASH
    if row["setup"] is not None:
        setup = json.loads(out[0])
        if setup.get("error") is not None:
            return f"VM-ERROR setup {setup.get('error')}"
    return parse_reply(out[-1])


def vm_fresh(vm2, rows, jobs):
    with concurrent.futures.ThreadPoolExecutor(jobs) as pool:
        return list(pool.map(lambda r: vm_fresh_one(vm2, r), rows))


def vm_single(vm2, rows):
    lines, where = [], []
    for r in rows:
        ls = stream_input(r)
        lines += ls
        where.append((len(lines) - len(ls), len(lines) - 1))
    p = subprocess.run([vm2, "--gc-floor", "1", "--eval-stream"], input="\n".join(lines) + "\n",
                       capture_output=True, text=True, timeout=1800)
    out = p.stdout.splitlines()
    actuals = []
    for r, (first, last) in zip(rows, where):
        if last >= len(out):
            actuals.append(None)
            continue
        if r["setup"] is not None and json.loads(out[first]).get("error") is not None:
            actuals.append(f"VM-ERROR setup {json.loads(out[first]).get('error')}")
            continue
        actuals.append(parse_reply(out[last]))
    return actuals


# ---------------------------------------------------------------- verdicts (primh's)

def classify(exp, act):
    if act is None:
        return "VM-CRASH"
    if exp == act:
        return "PASS"
    if act.startswith("VM-ERROR") and "print" in act:
        return "PRINTER"
    if act.startswith("VM-ERROR"):
        return "VM-INTERNAL"
    e_sig, a_sig = exp.startswith("(sig "), act.startswith("(sig ")
    if a_sig and act.startswith("(sig (void-function"):
        return "MISSING"
    if e_sig and a_sig:
        ec, ac = exp[6:].split(" ")[0].rstrip(")"), act[6:].split(" ")[0].rstrip(")")
        return "DATA" if ec == ac else "CONDITION"
    if e_sig != a_sig:
        return "SIGNAL-VS-VALUE"
    if "#<" in exp or "#[" in exp or "#<" in act:
        return "PRINTER"
    return "VALUE"


def verdict(row, actual):
    if actual is None:
        return "VM-CRASH"
    if actual == row["expected"]:
        return "PASS"
    if actual == row.get("gnu_regime_variant"):
        return "PASS-REGIME"
    return classify(row["expected"], actual)


def ratchet(ev, actuals):
    if len(actuals) != len(ev["cases"]):
        raise ValueError(f"reply count mismatch: {len(actuals)} != {len(ev['cases'])}")
    pins = ev.get("pinned", {})
    ids = {r["id"] for r in ev["cases"]}
    counts, bad = {}, [f"UNKNOWN PIN: {k}" for k in sorted(set(pins) - ids)]
    for row, actual in zip(ev["cases"], actuals):
        kind = verdict(row, actual)
        counts[kind] = counts.get(kind, 0) + 1
        pin = pins.get(row["id"])
        same = kind.startswith("PASS")
        if same and pin is not None:
            bad.append(f"STALE PIN ({pin}): {row['id']}")
        elif not same and pin is None:
            bad.append(f"NEW DIVERGENCE ({kind}): {row['id']}: {row['form']}\n    GNU {row['expected']}\n    VM2 {actual}")
        elif not same and pin != kind:
            bad.append(f"CHANGED PIN ({pin} -> {kind}): {row['id']}")
    return counts, bad


def by_family(ev, actuals):
    fam = {}
    for row, a in zip(ev["cases"], actuals):
        k = verdict(row, a)
        f = fam.setdefault(row["family"], {})
        f[k] = f.get(k, 0) + 1
    return fam


def replay(args):
    ev = evidence()
    rows = ev["cases"]
    t = time.monotonic()
    if args.single:
        actuals = vm_single(args.vm2, rows)
        mode = "single VM for all cases"
    else:
        actuals = vm_fresh(args.vm2, rows, args.jobs)
        mode = "fresh VM per case"
    counts, bad = ratchet(ev, actuals)
    print(f"varh replay ({mode}, {args.vm2}): {len(rows)} cases in {time.monotonic() - t:.1f}s  "
          f"{json.dumps(dict(sorted(counts.items())))}")
    for f, c in by_family(ev, actuals).items():
        print(f"  {f}: {json.dumps(dict(sorted(c.items())))}")
    report = dict(mode=mode, vm2=args.vm2, total=len(rows), counts=counts,
                  families=by_family(ev, actuals), ratchet_violations=bad,
                  verdicts={r["id"]: verdict(r, a) for r, a in zip(rows, actuals)},
                  actuals={r["id"]: a for r, a in zip(rows, actuals)})
    if args.single:
        fresh = vm_fresh(args.vm2, rows, args.jobs)
        phantoms = [(r["id"], verdict(r, f), verdict(r, a), r["form"], a)
                    for r, f, a in zip(rows, fresh, actuals) if verdict(r, f) != verdict(r, a)]
        report["phantoms"] = [dict(id=i, fresh=f, single=s, form=form, single_actual=a)
                              for i, f, s, form, a in phantoms]
        print(f"  {len(phantoms)} PHANTOM rows (single-VM verdict != fresh-VM verdict)")
        for i, f, s, form, a in phantoms[:args.show]:
            print(f"    {i}: fresh {f} / single {s}: {form}\n      single VM2 {a}")
    print(f"  {len(bad)} ratchet violations")
    for b in bad[:args.show]:
        print("  " + b)
    if args.json:
        atomic_json(Path(args.json), report)
    sys.exit(1 if bad else 0)


def pin(args):
    ev = evidence()
    actuals = vm_fresh(args.vm2, ev["cases"], args.jobs)
    ev["pinned"] = {r["id"]: verdict(r, a) for r, a in zip(ev["cases"], actuals)
                    if not verdict(r, a).startswith("PASS")}
    atomic_json(EVID, ev)
    counts, bad = ratchet(ev, actuals)
    print(f"pinned {len(ev['pinned'])} rows from the FRESH replay: {json.dumps(dict(sorted(counts.items())))}")
    for i, k in ev["pinned"].items():
        print(f"  {i} {k}")
    sys.exit(1 if bad else 0)


def rust(args):
    evidence()   # never emit a replay for stale evidence
    out = Path(args.out).resolve()
    rel = lambda p: os.path.relpath(p, out.parent)
    text = TEMPLATE.read_text()
    for key, path in (("@EVIDENCE@", EVID), ("@CASES@", CASES), ("@HARNESS@", Path(__file__).resolve()),
                      ("@DRIVER@", DRIVER), ("@TEMPLATE@", TEMPLATE)):
        text = text.replace(key, json.dumps(rel(path)))
    out.write_text(text)
    print(f"wrote {out}")


def probe(args):
    row = dict(id="probe", setup=args.setup, source=WRAP.format(expr=args.form))
    if args.gnu:
        for g in REGIMES:
            print(f"GNU {g:12s}", gnu_one(shutil.which(args.gnu) or args.gnu, row, g, 60)["outcome"])
    if args.vm2:
        print(f"VM2 {'fresh':12s}", vm_fresh_one(args.vm2, row))


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sp = ap.add_subparsers(dest="cmd", required=True)
    a = sp.add_parser("record"); a.add_argument("--gnu", default="emacs"); a.add_argument("--jobs", type=int, default=8)
    a.add_argument("--timeout", type=int, default=60); a.set_defaults(f=record)
    a = sp.add_parser("replay"); m = a.add_mutually_exclusive_group(required=True)
    m.add_argument("--fresh", action="store_true"); m.add_argument("--single", action="store_true")
    a.add_argument("--vm2", required=True); a.add_argument("--jobs", type=int, default=8)
    a.add_argument("--json"); a.add_argument("--show", type=int, default=20); a.set_defaults(f=replay)
    a = sp.add_parser("pin"); a.add_argument("--vm2", required=True); a.add_argument("--jobs", type=int, default=8); a.set_defaults(f=pin)
    a = sp.add_parser("rust"); a.add_argument("--out", default=str(HERE.parents[1] / "tests" / "varh.rs")); a.set_defaults(f=rust)
    a = sp.add_parser("probe"); a.add_argument("form"); a.add_argument("--setup"); a.add_argument("--gnu")
    a.add_argument("--vm2"); a.set_defaults(f=probe)
    ns = ap.parse_args(); ns.f(ns)
