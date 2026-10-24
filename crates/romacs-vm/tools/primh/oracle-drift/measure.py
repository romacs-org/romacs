#!/usr/bin/env python3
"""Measure what changes when the recording oracle becomes the pin.

    measure.py --tree LABEL=DIR ... --pin EMACS --old EMACS --out DIR [--jobs N]
               [--only NAME ...] [--bulk core]

For every primh evidence file under each DIR (a `tools/primh` tree, e.g. a
`git archive` of a branch) whose spec can be found by name AND spec_sha256,
re-record the corpus twice with that tree's own unmodified primh.py:

  pin  -- the GNU built at the pin (refused unless it reports the pin)
  old  -- the binary the stored evidence names (control for nondeterminism)

and compare per case id the stored GNU answer (`expected`, and the
interpreted-regime `gnu_regime_variant`) against both re-recordings:

  stored != old   the corpus is not reproducible even on its own binary (noise)
  old != pin      the revision changed GNU's answer          (THE drift)
  stored != pin   what a re-record against the pin would change

Nothing in DIR is written; re-recordings go to OUT.
"""
import os as _portable_os
from pathlib import Path as _PortablePath
import argparse
import concurrent.futures as cf
import contextlib
import hashlib
import importlib.util
import io
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
sys.path.insert(0, str(HERE.parents[4] / "scripts"))
import gnu_oracle  # noqa: E402
import record as recorder  # noqa: E402


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def units(trees, only):
    """(label, tree, evidence path, spec path) for every bindable evidence file."""
    seen, out, unbound = set(), [], []
    for label, tree in trees:
        specs = {}
        for s in sorted(tree.glob("*specs*/*.json")):
            try:
                specs.setdefault(json.loads(s.read_text())["name"], []).append(s)
            except (KeyError, json.JSONDecodeError):
                continue
        for ev_path in sorted((tree / "evidence").glob("*.json")):
            ev = json.loads(ev_path.read_text())
            name = ev.get("primitive")
            if not name:   # another protocol (e.g. widen-gnu-offline-v1): say so, never skip silently
                unbound.append(f"{label}:{ev_path.name} (protocol {ev.get('protocol')!r}, no primitive)")
                continue
            if only and name not in only:
                continue
            key = (name, sha(ev_path))
            if key in seen:
                continue
            match = [s for s in specs.get(name, []) if sha(s) == ev.get("spec_sha256")]
            if not match:
                unbound.append(f"{label}:{ev_path.name}")
                continue
            seen.add(key)
            out.append((label, str(tree), str(ev_path), str(match[0])))
    return out, unbound


def rerecord(job):
    kind, label, tree, ev_path, spec_path, binary, out = job
    tree = Path(tree)
    dst = Path(out) / kind / label
    dst.mkdir(parents=True, exist_ok=True)
    log = io.StringIO()
    try:
        with contextlib.redirect_stdout(log), contextlib.redirect_stderr(log):
            primh = recorder.load("primh", tree)
            primh.EVID = recorder.FileNamedDir(dst)
            gnu_src = Path(str(_PortablePath(_portable_os.environ.get('ROMACS_GNU_SOURCE', _PortablePath(_portable_os.environ.get('GNU_ORACLE_ROOT', _PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'gnu-emacs-pin')) / 'src')) / 'src'))
            primh.record(argparse.Namespace(spec=[spec_path], gnu=binary, gnu_src=str(gnu_src)))
        name = json.loads(Path(spec_path).read_text())["name"]
        return kind, (label, ev_path), str(dst / (name.replace("/", "%2F") + ".json")), None
    except BaseException as error:  # a refusal is a result, not a crash
        return kind, (label, ev_path), None, f"{type(error).__name__}: {error} :: {log.getvalue()[-400:]}"


def bulk_rerecord(job):
    kind, tree, group, binary, out = job
    dst = Path(out) / kind / "bulk"
    log = io.StringIO()
    with contextlib.redirect_stdout(log), contextlib.redirect_stderr(log):
        bulk = recorder.load("bulk", tree)
        bulk.BULK = dst
        dst.mkdir(parents=True, exist_ok=True)
        spec = Path(tree) / "bulk" / f"{group}.spec.json"
        bulk.spec_path = lambda g, _s=spec: _s
        bulk.record(argparse.Namespace(group=group, gnu=binary))
    return kind, ("bulk", str(Path(tree) / "bulk" / f"{group}.corpus.json")), str(dst / f"{group}.corpus.json"), None


def ledger_key(label, stored_path):
    """The LEGACY-ORACLE.json key this measurement covers (paths under tools/primh)."""
    p = Path(stored_path)
    return f"{p.parent.name}/{p.name}"


def answers(path):
    ev = json.loads(Path(path).read_text())
    return ev, {r["id"]: (r["expected"], r.get("gnu_regime_variant")) for r in ev["cases"]}


def compare(label, stored_path, pin_path, old_path):
    ev, stored = answers(stored_path)
    _, pin = answers(pin_path)
    _, old = answers(old_path)
    if not (stored.keys() == pin.keys() == old.keys()):
        raise ValueError(f"{stored_path}: case ids differ between recordings")
    by_source = {r["id"]: r["source"] for r in ev["cases"]}
    by_prim = {r["id"]: r.get("p", ev.get("primitive")) for r in ev["cases"]}
    rows = []
    for cid in stored:
        s, p, o = stored[cid], pin[cid], old[cid]
        if s == p == o:
            continue
        rows.append(dict(id=cid, primitive=by_prim[cid], source=by_source[cid],
                         stored=s, old=o, pin=p,
                         timeout=any("GNU-TIMEOUT" in str(x) for x in (*s, *p, *o))))
    return dict(tree=label, evidence=stored_path, name=Path(stored_path).name,
                ledger_key=ledger_key(label, stored_path), stored_sha256=sha(stored_path),
                primitives=sorted({by_prim[c] for c in stored}),
                revision=ev["gnu"].get("revision"),
                cases=len(stored),
                stored_ne_old=sum(r["stored"] != r["old"] for r in rows),
                old_ne_pin=sum(r["old"] != r["pin"] for r in rows),
                stored_ne_pin=sum(r["stored"] != r["pin"] for r in rows),
                # reproducible on the old binary, different on the pin: revision drift
                drift=sum(r["stored"] == r["old"] != r["pin"] for r in rows),
                rows=rows)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--tree", action="append", required=True, help="LABEL=DIR (a tools/primh tree)")
    ap.add_argument("--pin", required=True)
    ap.add_argument("--old", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--jobs", type=int, default=12)
    ap.add_argument("--only", action="append")
    ap.add_argument("--bulk", action="append", default=[], help="bulk group in the FIRST tree")
    a = ap.parse_args()
    pin_ident = gnu_oracle.require(Path(a.pin))   # the pin side must be the pin
    old_ident = gnu_oracle.identity(Path(a.old))
    trees = [(t.split("=", 1)[0], Path(t.split("=", 1)[1]).resolve()) for t in a.tree]
    todo, unbound = units(trees, set(a.only or []))
    jobs = [(k, *u, b, a.out) for u in todo for k, b in (("pin", a.pin), ("old", a.old))]
    bulk_jobs = [(k, str(trees[0][1]), g, b, a.out) for g in a.bulk for k, b in (("pin", a.pin), ("old", a.old))]
    got, errors = {}, []
    recorder.detach_stdin()   # before the pool, so every worker and GNU child inherits EOF
    with cf.ProcessPoolExecutor(a.jobs, initializer=recorder.detach_stdin) as pool:
        futs = [pool.submit(rerecord, j) for j in jobs] + [pool.submit(bulk_rerecord, j) for j in bulk_jobs]
        for f in cf.as_completed(futs):
            kind, key, new, err = f.result()
            if err:
                errors.append(dict(kind=kind, tree=key[0], evidence=key[1], error=err))
            else:
                got.setdefault(key, {})[kind] = new
            print(f"{len(got)} done, {len(errors)} errors: {kind} {Path(key[1]).name}", file=sys.stderr, flush=True)
    results = [compare(k[0], k[1], d["pin"], d["old"]) for k, d in sorted(got.items()) if set(d) == {"pin", "old"}]
    report = dict(pin=pin_ident, old=old_ident, unbound=unbound, errors=errors, results=results,
                  totals=dict(files=len(results), cases=sum(r["cases"] for r in results),
                              stored_ne_old=sum(r["stored_ne_old"] for r in results),
                              old_ne_pin=sum(r["old_ne_pin"] for r in results),
                              stored_ne_pin=sum(r["stored_ne_pin"] for r in results),
                              drift=sum(r["drift"] for r in results)))
    Path(a.out, "drift.json").write_text(json.dumps(report, indent=1, ensure_ascii=False) + "\n")
    print(json.dumps(report["totals"]))
    for r in results:
        print(f"{r['tree']:>10} {r['name']:44} {r['cases']:6} stored!=old {r['stored_ne_old']:4}  "
              f"old!=pin {r['old_ne_pin']:4}  stored!=pin {r['stored_ne_pin']:4}  drift {r['drift']:4}")
    if unbound:
        print("UNBOUND (no spec with matching sha):", unbound)
    for e in errors:
        print("ERROR", e["kind"], e["evidence"], e["error"][:300])


if __name__ == "__main__":
    main()
