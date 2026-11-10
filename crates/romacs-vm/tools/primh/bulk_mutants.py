#!/usr/bin/env python3
"""bulk_mutants.py: prove `tests/bulk_core.rs` fails when what it protects breaks.

Each mutant edits exactly one implementation site, runs the corpus test, and
must FAIL at a corpus assertion -- the bulk test's `core-group violations` or
primh's `NEW DIVERGENCE` (tests/primh.rs) -- naming at least one
case of the expected primitive. A compile failure, or a failure anywhere else,
does not count as a kill. The tree is restored after every mutant, also on
interrupt. Run from the repository root with the lane's cargo environment:

    python3 crates/romacs-vm/tools/primh/bulk_mutants.py [--only NAME]
"""
import argparse, re, subprocess, sys
from pathlib import Path

S = Path("crates/romacs-vm/src")
MUTANTS = [
    # (name, file, old, new, case-id prefix that must appear as a violation)
    ("arity-data-dropped-for-car", S / "tails.rs",
     "        Car | Cdr | Cons | Length", "        Cdr | Cons | Length", "car/"),
    ("tortoise-teleports-too-early", S / "tails.rs",
     "if l & 4095 == 0 && l.is_power_of_two()", "if l & 3 == 0 && l.is_power_of_two()", "length/"),
    ("length-dotted-names-whole-list", S / "tails.rs",
     'TailEnd::Dotted(t) if x.tag() == 3 => wrong(self, "listp", t)',
     'TailEnd::Dotted(_) if x.tag() == 3 => wrong(self, "listp", x)', "length/"),
    ("length-wrong-signal-symbol", S / "tails.rs",
     '_ => wrong(self, "sequencep", x),', '_ => wrong(self, "listp", x),', "length/"),
    ("append-walks-from-head-not-cdr", S / "tails.rs",
     "let rest = self.cons_pair(v).map_or(Value::NIL, |(_, cdr)| cdr);", "let rest = v;", "append/"),
    # (Deleting the in-place write is not a usable mutant: on a lasso that does
    # not cycle through SEQ the unmutated walk never returns, so it hangs
    # rather than failing. Observed: primh's replay ran 16 minutes.)
    ("nreverse-accepts-dotted-lists", S / "tails.rs",
     "            if tail != Value::NIL {\n                return Err(\"list required\".into());",
     "            if false {\n                return Err(\"list required\".into());", "nreverse/"),
    ("nreverse-loses-vectors", S / "tails.rs",
     "            v.reverse();\n", "", "nreverse/"),
    ("nreverse-wrong-signal-symbol", S / "tails.rs",
     'wrong(self, "arrayp", x)', 'wrong(self, "sequencep", x)', "nreverse/"),
    ("substring-drops-to-from-range-data", S / "prims/fns_core.rs",
     "Err(Fault::Range3(string, from, to))", "Err(Fault::Range(string, from))", "substring/"),
    ("substring-requires-from", S / "prims/fns_core.rs",
     "fn substring(cx, string: Value; optional from: Value, to: Value)",
     "fn substring(cx, string: Value, from: Value; optional to: Value)", "substring/"),
    ("substring-nil-from-rejected", S / "prims/fns_core.rs",
     "let f = bound(from, 0)?;", "let f = bound(if from == Value::NIL { Value::TRUE } else { from }, 0)?;", "substring/"),
    ("substring-drops-text-properties", S / "prim.rs",
     "                slice.properties = s.properties.slice(range);\n", "                let _ = s;\n", "substring/"),
    ("funcall-string-is-void-function", S / "threaded.rs",
     '    "invalid-function"\n}', '    "void-function"\n}', "mapcar/"),
    ("closure-arity-max-is-many-with-rest", S / "threaded.rs",
     "let max = Value::integer(f.arity as i64).unwrap();",
     'let max = if f.rest { let slot = e.vm.slot("many"); Value::reference(slot + 1, 7) } else { Value::integer(f.arity as i64).unwrap() };',
     "mapcar/"),
    # Removing car's registration outright aborts the test process with a
    # native stack overflow (not at the corpus assertion), so the removal
    # proof is done per path instead: generic subr entry, then fast kernel.
    ("direct-call-names-cell-value", S / "threaded.rs",
     "return raise(e, condition, &[Value::reference(i.b + 1, 7)]);", "return raise(e, condition, &[target]);", "mapcar/"),
    ("non-lambda-cons-is-void-function", S / "threaded.rs",
     "is_ok_and(|(head, _)| head == lambda)", "is_ok_and(|(head, _)| head == lambda || head != lambda)", "mapcar/"),
    ("car-generic-path-returns-cdr", S / "traversal.rs",
     "Ok(if p == Car { pair.0 } else { pair.1 })", "Ok(pair.1)", "car/"),
    ("car-fast-kernel-returns-cdr", S / "traversal.rs",
     "Some(if p == Car { car } else { cdr })", "Some({ let _ = car; cdr })", "car/"),
    ("stringp-accepts-vectors", S / "bootstrap.rs",
     "                if self.str_view(a[0]).is_some()\n",
     "                if self.str_view(a[0]).is_some() || self.object_if(a[0]).is_some_and(|o| matches!(o, data::Object::Vector(_)))\n",
     "stringp/"),
    ("cdr-of-nil-signals-in-generic-path", S / "traversal.rs",
     "            Car | Cdr => {\n                if x == Value::NIL {\n                    return Ok(x);\n                }",
     "            Car | Cdr => {\n                if x == Value::NIL && p == Car {\n                    return Ok(x);\n                }", "cdr/"),
    ("memq-circular-data-dropped", S / "tails.rs",
     'TailEnd::Circular(t) => Some(("circular-list", vec![t])),\n                _ => None,',
     'TailEnd::Circular(_) => Some(("circular-list", vec![])),\n                _ => None,', "memq/"),
]


def run(name):
    """Run the bulk corpus test and the primh replays; collect both logs."""
    out, code = "", 0
    for test, filt in (("bulk_core", "bulk_core_matches_gnu_case_for_case"), ("primh", "primh_")):
        p = subprocess.run(["cargo", "test", "-p", "romacs-vm", "--test", test, "--", filt],
                           capture_output=True, text=True)
        out += f"\n=== {test}\n" + p.stdout + p.stderr
        code = code or p.returncode
    return code, out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", action="append")
    a = ap.parse_args()
    killed, survived = [], []
    for name, path, old, new, prefix in MUTANTS:
        if a.only and name not in a.only:
            continue
        text = path.read_text()
        if text.count(old) != 1:
            sys.exit(f"{name}: mutation site not unique in {path} ({text.count(old)} matches); refusing")
        try:
            path.write_text(text.replace(old, new))
            code, out = run(name)
        finally:
            path.write_text(text)
        compiled = "error[E" not in out and "could not compile" not in out
        hits = re.findall(r"(?:NEW DIVERGENCE|CHANGED PIN|STALE PIN)[^\n]*?: (" + re.escape(prefix) + r"\d{4})", out)
        primh = len(re.findall(r"NEW DIVERGENCE \([A-Z-]+\): " + re.escape(prefix), out.split("\n=== primh\n")[-1]))
        ok = code != 0 and compiled and hits and ("core-group violations" in out or "NEW DIVERGENCE" in out)
        (killed if ok else survived).append(name)
        n = re.search(r"(\d+) core-group violations", out)
        print(f"{'KILLED ' if ok else 'SURVIVED'} {name}: exit {code}, compiled {compiled}, "
              f"bulk {n.group(1) if n else 0} violations, primh {primh} new, {len(hits)} naming {prefix}* e.g. {hits[:2]}", flush=True)
    print(f"\n{len(killed)} killed, {len(survived)} survived: {survived}")
    sys.exit(1 if survived else 0)


if __name__ == "__main__":
    main()
