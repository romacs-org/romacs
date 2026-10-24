#!/usr/bin/env python3
"""primh: declaration-driven differential harness for VM2 primitives.

  primh.py record  SPEC... --gnu emacs      # live GNU -> evidence/<name>.json (atomic)
  primh.py replay  SPEC... --vm2 BINARY     # offline: evidence only, no GNU needed
  primh.py infer   SPEC...                  # derive arity + per-argument CHECK table from evidence
  primh.py rust    SPEC... --out FILE.rs    # emit a cargo replay test (include_str of evidence)

A SPEC is derived from the production build's primitives.json declaration.
Everything else -- cases, arity probes, GC-pressure
variant, condition/data comparison, classification -- is generated.
"""
import argparse, hashlib, itertools, json, os, shutil, subprocess, sys, tempfile, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
EVID = HERE / "evidence"
GNU_SRC = Path(os.environ.get("ROMACS_GNU_SOURCE", Path(os.environ.get("GNU_ORACLE_ROOT", Path.home() / ".cache/gnu-emacs-pin")) / "src")) / "src"
WRAP = "(prin1-to-string (condition-case e (list 'ok {expr}) (error (list 'sig e))))"


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


def evidence(spec):
    ev = json.loads((EVID / f"{spec['name']}.json").read_text())
    hashes = {"spec_sha256": spec['_sha256'],
              "generator_sha256": sha(Path(__file__).read_bytes()),
              "driver_sha256": sha((HERE / 'gnu_driver.el').read_bytes()),
              "domains_sha256": sha((HERE / 'domains.json').read_bytes()),
              "replay_sha256": sha((HERE / 'replay.rs.in').read_bytes())}
    for key, expected in hashes.items():
        if ev.get(key) != expected:
            raise ValueError(f"{spec['name']}: {key} changed since recording; re-record")
    expected_cases = [{k: r[k] for k in ('id', 'family', 'source')} for r in ev['cases']]
    if expected_cases != cases(spec):
        raise ValueError("recorded cases differ from deterministic generation")
    return ev


def load_spec(path):
    spec = json.loads(Path(path).read_text())
    spec["_sha256"] = sha(Path(path).read_bytes())
    return spec


def domain(spec, pos):
    cat = json.loads((HERE / "domains.json").read_text())["values"]
    want = spec.get("args", [])[pos] if pos < len(spec.get("args", [])) else "any"
    vals = [v["src"] for v in cat if want == "any" or want in v["tags"]]
    return vals + spec.get("extra", {}).get(str(pos), [])


def cases(spec):
    """Generate (id, family, source).  Deterministic order; ids are stable."""
    name, lo, hi = spec["name"], spec["arity"][0], spec["arity"][1]
    n = hi if isinstance(hi, int) else lo + 1
    out = []
    # 1. typed cartesian product over the declared positions
    for combo in itertools.product(*[domain(spec, i) for i in range(n)]):
        out.append(("dom", f"({name} {' '.join(combo)})"))
    # 2. arity boundaries through APPLY so the byte compiler cannot replace the
    #    primitive's own wrong-number-of-arguments signal with a diagnostic
    for k in sorted({max(lo - 1, 0), (hi + 1) if isinstance(hi, int) else lo + 3}):
        if lo <= k and (hi == "many" or k <= hi):
            continue
        out.append(("arity", f"(apply #'{name} '({' '.join(['1'] * k)}))"))
    # 3. GC pressure: allocate between argument construction and the call
    for combo in itertools.islice(itertools.product(*[domain(spec, i) for i in range(n)]), 0, None, 37):
        binds = " ".join(f"(a{i} {s})" for i, s in enumerate(combo))
        args = " ".join(f"a{i}" for i in range(n))
        out.append(("gc", f"(let ({binds}) (let ((i 0)) (while (< i 3000) (list i i) (setq i (1+ i)))) ({name} {args}))"))
    # 4a. aliasing contract, generated: is the result EQ to an argument? GNU
    #     returns its argument in places (editfns.c:3733 `(format "%s" s)`),
    #     and a mutation of the result is then a mutation of the argument.
    for combo in itertools.islice(itertools.product(*[domain(spec, i) for i in range(n)]), 0, None, 3):
        binds = " ".join(f"(a{i} {s})" for i, s in enumerate(combo))
        args = " ".join(f"a{i}" for i in range(n))
        eqs = " ".join(f"(eq r a{i})" for i in range(n))
        out.append(("alias", f"(let ({binds}) (let ((r ({name} {args}))) (list {eqs})))"))
    # 4. aliasing witness for declared-destructive or sharing primitives
    for probe in spec.get("probes", []):
        out.append(("probe", probe))
    # 5. the catalog prints itself: separates printer divergence from primitive
    #    divergence and gives `infer` the printed form of every argument value
    for v in sorted({v for i in range(n) for v in domain(spec, i)}):
        out.append(("printer", v))
    return [dict(id=f"{name}/{i:04d}", family=f, source=WRAP.format(expr=e)) for i, (f, e) in enumerate(out)]


def gnu_rows(gnu, spec, rows, names):
    with tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR")) as home:
        env = dict(os.environ, HOME=home, PRIMH_NAMES=",".join(names))
        head, got = None, []

        def run(batch, limit=600):
            nonlocal head
            p = Path(home) / "cases.json"
            p.write_text(json.dumps([{"id": r["id"], "source": r["source"]} for r in batch]))
            env["PRIMH_CASES"] = str(p)
            timed_out = False
            try:
                r = subprocess.run([gnu, "-Q", "--batch", "-l", str(HERE / "gnu_driver.el")],
                                   env=env, capture_output=True, text=True, timeout=limit, check=True)
                out = r.stderr
            except subprocess.TimeoutExpired as t:
                timed_out = True
                out = t.stderr or ""
                if isinstance(out, bytes):
                    out = out.decode()
            lines = out.splitlines()
            if timed_out and lines and lines[-1].startswith("{") and not lines[-1].endswith("}"):
                lines.pop()
            try:
                replies = [json.loads(line) for line in lines if line.startswith("{")]
            except json.JSONDecodeError as error:
                raise ValueError("GNU reply malformed JSON") from error
            if not replies or not isinstance(replies[0], dict):
                raise ValueError("GNU reply missing header; no case exclusion is sound")
            h, done = replies[0], replies[1:]
            if h.get('subrs') != [[spec['name'], 'subr', *spec['arity']]] or (head is not None and h != head):
                raise ValueError("GNU reply header identity/arity changed")
            if (len(done) > len(batch) or (not timed_out and len(done) != len(batch))
                    or any(not isinstance(g, dict) for g in done)
                    or [g.get('id') for g in done] != [r['id'] for r in batch[:len(done)]]):
                raise ValueError("GNU reply count/order mismatch")
            if any(not isinstance(g.get(key), str) for g in done for key in ('expected', 'interp')):
                raise ValueError("GNU reply missing regime pair")
            head = h
            return done, timed_out

        if spec.get("effect", "pure") == "pure":
            todo = list(rows)
            while todo:
                done, timed_out = run(todo, 10)
                got += done
                todo = todo[len(done):]
                if timed_out and todo:
                    retry, retry_timed_out = run(todo[:1], 10)
                    got += retry
                    if not retry and retry_timed_out:
                        got.append(dict(id=todo[0]["id"], expected="GNU-TIMEOUT", interp="GNU-TIMEOUT"))
                    todo = todo[1:]
        else:
            for row in rows:
                done, timed_out = run([row])
                if timed_out:
                    raise ValueError("GNU reply timeout for effectful case; refusing evidence")
                got += done
    if [g["id"] for g in got] != [r["id"] for r in rows]:
        raise ValueError("GNU reply count/order mismatch")
    return head, {g["id"]: (g["expected"], g["interp"]) for g in got}


def record(args):
    for path in args.spec:
        spec = load_spec(path)
        rows = cases(spec)
        t = time.monotonic()
        head, exp = gnu_rows(args.gnu, spec, rows, [spec["name"]])
        for r in rows:
            r["expected"], interp = exp[r["id"]]
            if interp != r["expected"]:
                r["gnu_regime_variant"] = interp   # GNU disagrees with itself
        if head is None or head.get('subrs', []) != [[spec['name'], 'subr', *spec['arity']]]:
            raise ValueError(f"{spec['name']}: GNU arity/identity disagrees with declaration")
        if any(str(value).startswith('HARNESS-ERROR') for values in exp.values() for value in values):
            raise ValueError('GNU driver failed; refusing evidence')
        gnu_c = {f: sha((Path(args.gnu_src) / f).read_bytes()) for f in spec.get("gnu_files", [])}
        ev = dict(protocol="primh-v1", primitive=spec["name"], spec_sha256=spec["_sha256"],
                  generator_sha256=sha(Path(__file__).read_bytes()),
                  domains_sha256=sha((HERE / "domains.json").read_bytes()),
                  replay_sha256=sha((HERE / "replay.rs.in").read_bytes()),
                  driver_sha256=sha((HERE / "gnu_driver.el").read_bytes()),
                  gnu_binary_sha256=sha(Path(shutil.which(args.gnu) or args.gnu).resolve().read_bytes()),
                  gnu=head, gnu_sources=gnu_c,
                  regime="GNU byte-compiled, one process per primitive iff effect=pure; VM2 compiled",
                  gc="VM2 replay --gc-floor 1 (collection at every safepoint); no timing claim",
                  cases=rows)
        EVID.mkdir(exist_ok=True)
        dst = EVID / f"{spec['name']}.json"
        atomic_json(dst, ev)   # persist reference before any candidate runs
        print(f"recorded {len(rows)} cases for {spec['name']} in {time.monotonic()-t:.2f}s -> {dst}")


def classify(exp, act):
    if act is None:
        return "VM-CRASH"
    if exp == act:
        return "PASS"
    if act.startswith("VM-ERROR") and "print" in act:
        return "PRINTER"              # the measuring instrument, not the primitive
    if act.startswith("VM-ERROR"):
        return "VM-INTERNAL"          # a Rust string escaped without a condition
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
        return 'VM-CRASH'
    if row['expected'] == 'GNU-TIMEOUT':
        return 'EXCLUDED-GNU-DIVERGES'
    if actual == row['expected']:
        return 'PASS'
    if actual == row.get('gnu_regime_variant'):
        return 'PASS-REGIME'
    return classify(row['expected'], actual)


def ratchet(ev, actuals):
    if len(actuals) != len(ev['cases']):
        raise ValueError(f"reply count mismatch: {len(actuals)} != {len(ev['cases'])}")
    pins = ev.get('pinned', {})
    ids = {r['id'] for r in ev['cases']}
    if len(ids) != len(ev['cases']):
        raise ValueError('duplicate case IDs')
    counts, bad = {}, [f"UNKNOWN PIN: {key}" for key in sorted(set(pins) - ids)]
    for row, actual in zip(ev['cases'], actuals):
        kind = verdict(row, actual)
        counts[kind] = counts.get(kind, 0) + 1
        pin = pins.get(row['id'])
        same = kind.startswith('PASS') or kind.startswith('EXCLUDED')
        if same and pin is not None:
            bad.append(f"STALE PIN ({pin}): {row['id']}")
        elif not same and pin is None:
            bad.append(f"NEW DIVERGENCE ({kind}): {row['id']}")
        elif not same and pin != kind:
            bad.append(f"CHANGED PIN ({pin} -> {kind}): {row['id']}")
    return counts, bad


def replay(args):
    total, errors = {}, []
    for path in args.spec:
        spec = load_spec(path)
        ev = evidence(spec)
        t = time.monotonic()
        stdin = "".join(json.dumps({"source": r['source']}) + "\n" for r in ev['cases'])
        p = subprocess.run([args.vm2, '--gc-floor', '1', '--eval-stream'], input=stdin,
                           capture_output=True, text=True, timeout=600, check=True)
        actuals = []
        for line in p.stdout.splitlines():
            out = json.loads(line)
            result = out.get('result') or {}
            actuals.append(result['value'] if result.get('type') == 'string' else f"VM-ERROR {out.get('error')}")
        counts, bad = ratchet(ev, actuals)
        if args.pin:
            ev['pinned'] = {r['id']: verdict(r, a) for r, a in zip(ev['cases'], actuals)
                            if not verdict(r, a).startswith(('PASS', 'EXCLUDED'))}
            atomic_json(EVID / f"{spec['name']}.json", ev)
            counts, bad = ratchet(ev, actuals)
        total[spec['name']] = counts
        errors += bad
        print(f"{spec['name']}: {len(ev['cases'])} cases in {time.monotonic()-t:.2f}s  {json.dumps(counts, sort_keys=True)}")
        for error in bad[:args.show]:
            print('  ' + error)
    if args.json:
        atomic_json(Path(args.json), total)
    sys.exit(1 if errors else 0)


def infer(args):
    """Derive the shared coercion table from GNU's own signals: for each argument
    position, which predicate does GNU name when that position holds a bad value?"""
    for path in args.spec:
        spec = load_spec(path)
        ev = evidence(spec)
        name, lo, hi = spec["name"], *spec["arity"]
        sub = [s for s in ev["gnu"]["subrs"] if s[0] == name][0]
        print(f"{name}: GNU subr-arity ({sub[2]} . {sub[3]})  declared ({lo} . {hi})  {'OK' if [sub[2], sub[3]] == [lo, hi] else 'MISMATCH'}")
        n = hi if isinstance(hi, int) else lo + 1
        doms = [domain(spec, i) for i in range(n)]
        printed = {}
        for r in ev["cases"]:
            if r["family"] == "printer":
                src = r["source"][len("(prin1-to-string (condition-case e (list 'ok "):-len(") (error (list 'sig e))))")]
                printed[src] = r["expected"][4:-1]
        checks = [dict() for _ in range(n)]
        order, tails, accepted = {}, {}, [set() for _ in range(n)]
        for r, combo in zip([r for r in ev["cases"] if r["family"] == "dom"], itertools.product(*doms)):
            e = r["expected"]
            if e.startswith("(sig (wrong-type-argument "):
                pred, _, datum = e[len("(sig (wrong-type-argument "):-2].partition(" ")
                hit = [i for i, v in enumerate(combo) if printed.get(v) == datum]
                if len(hit) == 1:
                    checks[hit[0]].setdefault(pred, set()).add(combo[hit[0]])
                elif not hit:
                    tails[pred] = tails.get(pred, 0) + 1   # datum is a sub-object: GNU checked a TAIL
            elif e.startswith("(ok "):
                for i, v in enumerate(combo):
                    accepted[i].add(v)
        for i, c in enumerate(checks):
            print(f"   arg{i}: CHECK " + (", ".join(f"{p} (rejects {len(v)})" for p, v in sorted(c.items())) or "none observed")
                  + f"; accepts {len(accepted[i])}/{len(doms[i])} catalog values")
        if tails:
            print(f"   signals naming a sub-object (tail/element checks): {tails}")
        # check ORDER: a case where every position is individually rejected
        bad = [set().union(*c.values()) if c else set() for c in checks]
        for r, combo in zip([r for r in ev["cases"] if r["family"] == "dom"], itertools.product(*doms)):
            if n > 1 and all(combo[i] in bad[i] for i in range(n)):
                datum = r["expected"][len("(sig (wrong-type-argument "):-2].partition(" ")[2]
                first = [i for i, v in enumerate(combo) if printed.get(v) == datum]
                if len(first) == 1:
                    order[first[0]] = order.get(first[0], 0) + 1
        if order:
            print(f"   check order (position signalled when all are bad): {dict(sorted(order.items()))}")


def from_manifest(args):
    """Derive specs from the production manifest; independently check GNU arity."""
    man = json.loads(Path(args.manifest).read_text())
    selected = set(args.only or [])
    unknown = selected - {m['name'] for m in man}
    if unknown:
        raise ValueError(f"unknown manifest names: {', '.join(sorted(unknown))}")
    if selected:
        man = [m for m in man if m['name'] in selected]
    names = [m["name"] for m in man]
    with tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR")) as home:
        p = Path(home) / "c.json"; p.write_text("[]")
        r = subprocess.run([args.gnu, "-Q", "--batch", "-l", str(HERE / "gnu_driver.el")], capture_output=True, text=True, check=True,
                           env=dict(os.environ, HOME=home, PRIMH_CASES=str(p), PRIMH_NAMES=",".join(names)))
    head = json.loads([l for l in r.stderr.splitlines() if l.startswith("{")][0])
    out = Path(args.out); out.mkdir(exist_ok=True)
    if [r[0] for r in head['subrs']] != names:
        raise ValueError("manifest reply count/order mismatch")
    for m, (name, kind, lo, hi) in zip(man, head["subrs"]):
        if kind != 'subr':
            raise ValueError(f"{name}: GNU is not a native subr")
        if [lo, hi] != [m['min'], m['max']]:
            raise ValueError(f"{name}: GNU arity {[lo, hi]} disagrees with manifest {[m['min'], m['max']]}")
        spec = dict(name=name, arity=[m['min'], m['max']], effect=m["effect"].lower(), gnu_files=[m["gnu"].split(":")[0]],
                    gnu_def=m["gnu"], rust=m["rust"], args=[])
        (out / f"{name}.json").write_text(json.dumps(spec) + "\n")
        print(f"{name}: arity ({lo} . {hi}) from GNU, rust {m['rust']}, gnu {m['gnu']}")


def rust(args):
    out = [(HERE / "replay.rs.in").read_text()]
    for path in args.spec:
        spec = load_spec(path)
        evidence(spec)  # never emit a replay for stale evidence
        ident = spec['name'].replace('-', '_')
        rel = os.path.relpath(EVID / f"{spec['name']}.json", Path(args.out).resolve().parent)
        spec_rel = os.path.relpath(Path(path).resolve(), Path(args.out).resolve().parent)
        out.append(f"#[test] fn primh_{ident}() {{ replay(include_str!({json.dumps(rel)}), include_bytes!({json.dumps(spec_rel)})); }}")
    Path(args.out).write_text("\n".join(out) + "\n")
    print(f"wrote {args.out}")


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sp = ap.add_subparsers(dest="cmd", required=True)
    a = sp.add_parser("record"); a.add_argument("spec", nargs="+"); a.add_argument("--gnu", default="emacs"); a.add_argument("--gnu-src", default=str(GNU_SRC)); a.set_defaults(f=record)
    a = sp.add_parser("replay"); a.add_argument("spec", nargs="+"); a.add_argument("--vm2", required=True); a.add_argument("--show", type=int, default=4); a.add_argument("--json"); a.add_argument("--pin", action="store_true"); a.set_defaults(f=replay)
    a = sp.add_parser("infer"); a.add_argument("spec", nargs="+"); a.set_defaults(f=infer)
    a = sp.add_parser("from-manifest"); a.add_argument("manifest"); a.add_argument("--out", default=str(HERE / "specs-gen")); a.add_argument("--gnu", default="emacs"); a.add_argument("--only", action="append", help="select a manifest name (repeatable; unknown names fail)"); a.set_defaults(f=from_manifest)
    a = sp.add_parser("rust"); a.add_argument("spec", nargs="+"); a.add_argument("--out", required=True); a.set_defaults(f=rust)
    ns = ap.parse_args(); ns.f(ns)
