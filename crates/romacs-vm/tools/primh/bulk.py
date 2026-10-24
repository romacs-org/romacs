#!/usr/bin/env python3
"""bulk.py: GNU-recorded differential corpora for many primitives at once.

  bulk.py cases  GROUP                    # print the deterministic case list
  bulk.py record GROUP --gnu emacs        # live GNU -> bulk/GROUP.corpus.json
  bulk.py replay GROUP --bin EXAMPLE      # diagnostic: per-case verdicts, fresh VM per case
  bulk.py report GROUP                    # coverage: successful vs signalling cases

This is primh's convention applied to a GROUP of primitives instead of one
declaration: the same wrapper (`primh.py` WRAP), the same unchanged GNU driver
(`gnu_driver.el`: byte-compiled and interpreted regimes, GNU `subr-arity` in
the header), the same verdict classes, and a Rust replay that builds a FRESH
production VM (prelude installed, collection floor 1) for every row. It is a
sibling of primh.py rather than an edit of it, because primh.py's own bytes
are hashed into every existing primh evidence file.

Only `record` writes expectations, and only from GNU's output. The group spec
names inputs (argument domains and probes); it never names an answer.
"""
import argparse, hashlib, itertools, json, os, shutil, subprocess, sys, tempfile, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
BULK = HERE / "bulk"
WRAP = "(prin1-to-string (condition-case e (list 'ok {expr}) (error (list 'sig e))))"
PROTOCOL = "bulk-v1"


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


def spec_path(group):
    return BULK / f"{group}.spec.json"


def load(group):
    return json.loads(spec_path(group).read_text())


def dom(spec, tags):
    """Values carrying any of TAGS, in file order, de-duplicated by source."""
    seen, out = set(), []
    for v in spec["values"]:
        if set(v["tags"]) & set(tags) and v["src"] not in seen:
            seen.add(v["src"])
            out.append(v["src"])
    if not out:
        raise ValueError(f"empty domain for tags {tags}")
    return out


def tuples(spec, name):
    p = spec["primitives"][name]
    if "variadic" in p:
        tags, top = p["variadic"]
        d = dom(spec, tags)
        out = []
        for w in range(0, top + 1):
            out += list(itertools.product(d, repeat=w))
        cap = p.get("max_tuples")
        if cap and len(out) > cap:
            stride = -(-len(out) // cap)
            out = out[::stride]
        return out
    return list(itertools.product(*[dom(spec, t) for t in p["args"]]))


def cases(spec, group):
    """Deterministic (id, p, family, source) rows; ids are stable."""
    rows = []
    for name in sorted(spec["primitives"]):
        out = []
        tup = tuples(spec, name)
        # 1. typed cartesian (or bounded variadic) product: the compiled call
        for t in tup:
            out.append(("dom", f"({name}{''.join(' ' + a for a in t)})"))
        # 2. the same tuples through FUNCALL and APPLY: VM2 compiles a direct
        #    call to a fast path but funcall/apply reach the generic subr entry,
        #    so both must be measured
        for t in tup[::3]:
            out.append(("funcall", f"(funcall #'{name}{''.join(' ' + a for a in t)})"))
        for t in tup[1::5]:
            out.append(("apply", f"(apply #'{name} (list{''.join(' ' + a for a in t)}))"))
        # 3. arity boundaries through APPLY (GNU's own subr-arity is in the header)
        p = spec["primitives"][name]
        if "variadic" not in p:
            n = len(p["args"])
            for k in sorted({max(n - 1, 0), n + 1, 0, n + 2}):
                if k != n:
                    out.append(("arity", f"(apply #'{name} '({' '.join(['1'] * k)}))"))
        # 4. GC pressure between argument construction and the call
        for t in tup[::29]:
            binds = " ".join(f"(a{i} {s})" for i, s in enumerate(t))
            args = "".join(f" a{i}" for i in range(len(t)))
            if binds:
                out.append(("gc", f"(let ({binds}) (let ((i 0)) (while (< i 3000) (list i i) (setq i (1+ i)))) ({name}{args}))"))
        # 5. aliasing: is the result EQ to an argument?
        for t in tup[::7]:
            if not t:
                continue
            binds = " ".join(f"(a{i} {s})" for i, s in enumerate(t))
            args = "".join(f" a{i}" for i in range(len(t)))
            eqs = " ".join(f"(eq r a{i})" for i in range(len(t)))
            out.append(("alias", f"(let ({binds}) (let ((r ({name}{args}))) (list {eqs})))"))
        # 6. hand-chosen probes (inputs only; GNU supplies every answer)
        for probe in spec.get("probes", {}).get(name, []):
            out.append(("probe", probe))
        for i, (fam, expr) in enumerate(out):
            rows.append(dict(id=f"{name}/{i:04d}", p=name, family=fam, source=WRAP.format(expr=expr)))
    # 7. the catalog prints itself: separates printer from primitive divergence
    printable = sorted({v["src"] for v in spec["values"] if v.get("printable", True)})
    for i, v in enumerate(printable):
        rows.append(dict(id=f"printer/{i:04d}", p="printer", family="printer", source=WRAP.format(expr=v)))
    return rows


def hashes(group):
    return dict(generator_sha256=sha(Path(__file__).read_bytes()),
                driver_sha256=sha((HERE / "gnu_driver.el").read_bytes()),
                spec_sha256=sha(spec_path(group).read_bytes()))


def gnu_run(gnu, names, rows, home, limit):
    env = dict(os.environ, HOME=home, PRIMH_NAMES=",".join(names))
    p = Path(home) / "cases.json"
    p.write_text(json.dumps([{"id": r["id"], "source": r["source"]} for r in rows]))
    env["PRIMH_CASES"] = str(p)
    r = subprocess.run([gnu, "-Q", "--batch", "-l", str(HERE / "gnu_driver.el")],
                       env=env, capture_output=True, text=True, timeout=limit)
    if r.returncode != 0:
        raise ValueError(f"GNU exited {r.returncode}: {r.stderr[-400:]}")
    replies = [json.loads(line) for line in r.stderr.splitlines() if line.startswith("{")]
    if not replies:
        raise ValueError("GNU reply missing header")
    return replies[0], replies[1:]


def record(a):
    spec = load(a.group)
    rows = cases(spec, a.group)
    names = sorted(spec["primitives"])
    t = time.monotonic()
    head, got = None, {}
    with tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR")) as home:
        # one GNU process per primitive (every case is self-contained: values
        # are read freshly per case and no case writes global state)
        for name in names + ["printer"]:
            batch = [r for r in rows if r["p"] == name]
            h, done = gnu_run(a.gnu, names, batch, home, 600)
            if head is not None and h != head:
                raise ValueError("GNU header changed between processes")
            head = h
            if [g.get("id") for g in done] != [r["id"] for r in batch]:
                raise ValueError(f"{name}: GNU reply count/order mismatch")
            for g in done:
                if not isinstance(g.get("expected"), str) or not isinstance(g.get("interp"), str):
                    raise ValueError(f"{g.get('id')}: GNU reply missing regime pair")
                got[g["id"]] = (g["expected"], g["interp"])
    arity = {}
    for name, kind, lo, hi in head["subrs"]:
        if kind != "subr":
            raise ValueError(f"{name}: GNU defines it in Lisp, not C; it does not belong in a primitive corpus")
        arity[name] = [lo, hi]
    # The interpreted regime calls the C DEFUN itself and is always required.
    # GNU's byte compiler can fail to compile a case at all: its optimizer
    # constant-folds e.g. (append '(1 . 2) x) and the fold's own signal escapes
    # `byte-compile` as an invalid function. That is a compiler defect, not
    # primitive behaviour, so such a row keeps only the interpreted answer and
    # says so in `gnu_compile_failure`; it is never silently dropped.
    harness = [i for i, v in got.items() if str(v[1]).startswith("HARNESS-ERROR")]
    if harness:
        raise ValueError(f"GNU interpreter driver failed on {len(harness)} case(s), e.g. {harness[:3]}; refusing evidence")
    for r in rows:
        compiled, interp = got[r["id"]]
        if compiled.startswith("HARNESS-ERROR"):
            r["expected"] = interp
            r["gnu_compile_failure"] = compiled
            continue
        r["expected"] = compiled
        if interp != compiled:
            r["gnu_regime_variant"] = interp
    ev = dict(protocol=PROTOCOL, group=a.group, **hashes(a.group),
              gnu_binary_sha256=sha(Path(shutil.which(a.gnu) or a.gnu).resolve().read_bytes()),
              gnu=head, arity=arity,
              regime="GNU byte-compiled (expected) and interpreted (gnu_regime_variant); "
                     "VM2 compiled, fresh production VM per case, prelude installed, collection floor 1",
              cases=rows)
    dst = BULK / f"{a.group}.corpus.json"
    atomic_json(dst, ev)
    print(f"recorded {len(rows)} cases over {len(names)} primitives in {time.monotonic()-t:.1f}s -> {dst}")


def corpus(group):
    ev = json.loads((BULK / f"{group}.corpus.json").read_text())
    for k, v in hashes(group).items():
        if ev.get(k) != v:
            raise ValueError(f"{group}: {k} changed since recording; re-record")
    want = [{k: r[k] for k in ("id", "p", "family", "source")} for r in ev["cases"]]
    if want != cases(load(group), group):
        raise ValueError("recorded cases differ from deterministic generation")
    return ev


def report(a):
    ev = corpus(a.group)
    per = {}
    for r in ev["cases"]:
        d = per.setdefault(r["p"], {"ok": 0, "sig": 0, "kinds": set(), "variant": 0})
        if r["expected"].startswith("(ok "):
            d["ok"] += 1
        else:
            d["sig"] += 1
            d["kinds"].add(r["expected"][6:].split(" ")[0].rstrip(")"))
        d["variant"] += "gnu_regime_variant" in r
    print(f"{len(ev['cases'])} cases; GNU {ev['gnu']['version']}")
    print(f"  {'primitive':12} {'arity':>10} {'ok':>5} {'signal':>7} {'regime':>7}  signal kinds")
    for p, d in sorted(per.items()):
        ar = ev["arity"].get(p, "")
        print(f"  {p:12} {str(ar):>10} {d['ok']:5} {d['sig']:7} {d['variant']:7}  {','.join(sorted(d['kinds']))}")


def replay(a):
    """Diagnostic replay through the example binary (fresh VM per case)."""
    corpus(a.group)
    r = subprocess.run([a.bin, str(BULK / f"{a.group}.corpus.json")], capture_output=True, text=True)
    if r.returncode not in (0, 1):
        sys.stderr.write(r.stderr)
        sys.exit(f"replay binary exited {r.returncode}")
    sys.stdout.write(r.stdout)


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sp = ap.add_subparsers(dest="cmd", required=True)
    c = sp.add_parser("cases"); c.add_argument("group")
    c.set_defaults(f=lambda a: [print(json.dumps(r, ensure_ascii=False)) for r in cases(load(a.group), a.group)])
    c = sp.add_parser("record"); c.add_argument("group"); c.add_argument("--gnu", default="emacs"); c.set_defaults(f=record)
    c = sp.add_parser("report"); c.add_argument("group"); c.set_defaults(f=report)
    c = sp.add_parser("replay"); c.add_argument("group"); c.add_argument("--bin", required=True); c.set_defaults(f=replay)
    ns = ap.parse_args(); ns.f(ns)
