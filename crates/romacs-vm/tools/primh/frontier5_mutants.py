#!/usr/bin/env python3
"""Single-site mutants for the frontier5 primitives (tests/frontier5.rs).

Each mutant must apply exactly once, compile, and fail the named test AT ITS
TARGET ASSERTION: the panic location must be the named assertion line in
tests/frontier5.rs and the message must contain the expected marker. A
compile failure, a panic elsewhere, or a pass is reported as NOT KILLED.
Run in an owned checkout with the project environment. Restores every file.
"""
import json, re, subprocess, sys
from pathlib import Path

ROOT = Path.cwd()
P = "crates/romacs-vm/src/prims/frontier.rs"
G = "crates/romacs-vm/src/gc.rs"
L = "crates/romacs-vm/tools/primh/frontier5-ledger.json"
T = "crates/romacs-vm/tests/frontier5.rs"

def line_of(marker):
    for i, l in enumerate((ROOT / T).read_text().splitlines(), 1):
        if marker in l:
            return i
    raise SystemExit(f"no line {marker!r}")

CHECK = line_of('assert!(bad.is_empty()')
CONTENT = line_of('"{make}: contents after a moving collection"')
LEDGER_EQ = line_of('"pins and ledger differ"')

MUTANTS = [
    ("mv-init-ignored", P, "values.resize(n, init);", "values.resize(n, Value::NIL);", "make_vector_matches_gnu_values", CHECK, "(make-vector 3 'x)"),
    ("mv-negative-accepted", P, ".and_then(|n| usize::try_from(n).ok())\n        .ok_or(Fault::Wrong(\"wholenump\", length))?;", ".map(|n| n.unsigned_abs() as usize)\n        .ok_or(Fault::Wrong(\"wholenump\", length))?;", "make_vector_matches_gnu_values", CHECK, "(make-vector -1 nil)"),
    ("mv-memory-as-type", P, "values.try_reserve_exact(n).map_err(|_| Fault::Error(MEMORY_FULL.into()))?;", "values.try_reserve_exact(n).map_err(|_| Fault::Wrong(\"wholenump\", length))?;", "make_vector_impossible_length_signals_memory_full_text", CHECK, "Memory exhausted"),
    ("vc-skip-strings", P, "        } else if cx.string_view(arg).is_ok() {\n            values.extend(string_codes(cx, arg)?);", "        } else if cx.string_view(arg).is_ok() {", "vconcat_matches_gnu_values", CHECK, "(vconcat '(1 2) [3]"),
    ("vc-skip-nonsequence", P, "            return Err(Fault::Wrong(\"sequencep\", arg));\n        }\n    }\n    Ok(allocate_vector(cx, values))", "            continue;\n        }\n    }\n    Ok(allocate_vector(cx, values))", "vconcat_matches_gnu_values", CHECK, "(vconcat [1] 'sym)"),
    ("vc-raw-byte-widened", P, "Ok(cx.string_view(string)?.chars().map(|c| Value::integer(i64::from(c)).unwrap()).collect())", "let v = cx.string_view(string)?; let m = v.multibyte; Ok(v.chars().map(|c| Value::integer(i64::from(if !m && c >= 128 { c + crate::lstring::BYTE8_OFFSET } else { c })).unwrap()).collect())", "vconcat_matches_gnu_values", CHECK, "(vconcat (unibyte-string 255 65))"),
    ("tails-dotted-names-list", P, "if tail != Value::NIL { return Err(Fault::Wrong(\"listp\", tail)); }", "if tail != Value::NIL { return Err(Fault::Wrong(\"listp\", list)); }", "reverse_matches_gnu_values", CHECK, "(reverse '(1 . 2))"),
    ("tails-circular-names-list", P, "if tail == tortoise { return Err(Fault::Circular(tail)); }", "if tail == tortoise { return Err(Fault::Circular(list)); }", "reverse_matches_gnu_values", CHECK, "(list 0 1 2)"),
    ("tails-circular-names-list/vconcat", P, "if tail == tortoise { return Err(Fault::Circular(tail)); }", "if tail == tortoise { return Err(Fault::Circular(list)); }", "vconcat_matches_gnu_values", CHECK, "(list 0 1 2)"),
    ("rv-list-forward", P, "for car in list_elements(cx, seq)? { out = cx.cons(car, out); }", "for car in list_elements(cx, seq)?.into_iter().rev() { out = cx.cons(car, out); }", "reverse_matches_gnu_values", CHECK, "(reverse '(1 2 3))"),
    ("rv-vector-unreversed", P, "        items.reverse();\n        return Ok(allocate_vector(cx, items));", "        return Ok(allocate_vector(cx, items));", "reverse_matches_gnu_values", CHECK, "(reverse [1 2 3])"),
    ("rv-ascii-keeps-multibyte", P, "            return Ok(cx.byte_string(&bytes));", "            return Ok(cx.make_string(bytes, view.multibyte));", "reverse_matches_gnu_values", CHECK, "(reverse (string-to-multibyte \"abc\"))"),
    ("rv-multibyte-bytewise", P, "        let mut chars: Vec<u32> = view.chars().collect();\n        chars.reverse();", "        let mut chars: Vec<u32> = view.chars().collect();", "reverse_matches_gnu_values", CHECK, "(reverse \"aλ雪\")"),
    ("pz-even-accepted", P, "    if nargs % 2 == 0 {", "    if nargs % 2 == 0 && false {", "propertize_even_arity_names_the_symbol", CHECK, "(propertize \"a\" 'face)"),
    ("pz-arity-names-subr", P, "        let name = cx.intern(\"propertize\");", "        let name = cx.intern(\"propertize-subr\");", "propertize_even_arity_names_the_symbol", CHECK, "(propertize \"a\" 'face)"),
    ("pz-nonsymbol-key-accepted", P, "        Some(name) if cx.obarray_lookup(&name) == Some(key) => Ok(name),\n        _ => {", "        Some(name) if cx.obarray_lookup(&name) == Some(key) => Ok(name),\n        _ if true => Ok(String::from(\"nil\")),\n        _ => {", "propertize_unrepresentable_keys_are_refused_not_merged", CHECK, "(propertize \"a\" 7 1)"),
    ("pz-pairs-forward", P, "for pair in properties.chunks_exact(2).rev() {", "for pair in properties.chunks_exact(2) {", "propertize_matches_gnu_values", CHECK, "'f 1 'g 2"),
    ("pz-drop-existing", "crates/romacs-vm/src/prim.rs", "                copy.properties = s.properties.clone();", "                let _ = s;", "propertize_matches_gnu_values", CHECK, "(put-text-property 0 1 'x 1 s)"),
    ("pz-partial-range", "crates/romacs-vm/src/prim.rs", "let all = coordinates::ByteOffset(0)..coordinates::ByteOffset(copy.bytes().len());", "let all = coordinates::ByteOffset(0)..coordinates::ByteOffset(copy.bytes().len().min(1));", "propertize_matches_gnu_values", CHECK, "(text-properties-at 2 (propertize \"abc\""),
    ("pz-uninterned-merged", P, "        Some(name) if cx.obarray_lookup(&name) == Some(key) => Ok(name),", "        Some(name) if cx.obarray_lookup(&name).is_some() => Ok(name),", "propertize_unrepresentable_keys_are_refused_not_merged", CHECK, "make-symbol"),
    ("sh-no-sha3", P, "                 \"sha3-224\", \"sha3-256\", \"sha3-384\", \"sha3-512\"]", "                 ]", "secure_hash_algorithms_matches_gnu_values", CHECK, "(secure-hash-algorithms)"),
    ("gc-vector-untraced/make-vector", G, "                    data::Object::Vector(values) => pending.extend_from_slice(values),", "                    data::Object::Vector(values) => { let _ = values; }", "make_vector_result_retains_its_init_across_collection", CONTENT, "make-vector 3"),
    ("gc-vector-untraced/vconcat", G, "                    data::Object::Vector(values) => pending.extend_from_slice(values),", "                    data::Object::Vector(values) => { let _ = values; }", "vconcat_result_retains_its_elements_across_collection", CONTENT, "vconcat"),
    ("gc-vector-untraced/reverse", G, "                    data::Object::Vector(values) => pending.extend_from_slice(values),", "                    data::Object::Vector(values) => { let _ = values; }", "reverse_vector_result_retains_its_elements_across_collection", CONTENT, "reverse"),
    ("gc-props-untraced/propertize", G, "                        s.properties.visit_values(&mut |v| pending.push(*v))\n", "                        { let _ = s; }\n", "propertize_result_retains_property_values_across_collection", CONTENT, "propertize"),
    ("gc-vector-unforwarded/make-vector", G, "                    data::Object::Vector(values) => {\n                        for v in values {\n                            rewrite(v);\n                        }\n                    }", "                    data::Object::Vector(values) => { let _ = values; }", "make_vector_result_retains_its_init_across_collection", CONTENT, "make-vector 3"),
    ("gc-vector-unforwarded/vconcat", G, "                    data::Object::Vector(values) => {\n                        for v in values {\n                            rewrite(v);\n                        }\n                    }", "                    data::Object::Vector(values) => { let _ = values; }", "vconcat_result_retains_its_elements_across_collection", CONTENT, "vconcat"),
    ("gc-vector-unforwarded/reverse", G, "                    data::Object::Vector(values) => {\n                        for v in values {\n                            rewrite(v);\n                        }\n                    }", "                    data::Object::Vector(values) => { let _ = values; }", "reverse_vector_result_retains_its_elements_across_collection", CONTENT, "reverse"),
    ("gc-props-unforwarded/propertize", G, "                        s.properties.visit_values(&mut |v| rewrite(v))\n", "                        { let _ = s; }\n", "propertize_result_retains_property_values_across_collection", CONTENT, "propertize"),
    ("ledger-drops-a-pin", L, '  "make-vector/0499": {\n   "class": "VALUE",\n   "row": "VM2-EMPTY1"\n  },\n', '', "frontier5_pins_are_exactly_the_ledger", LEDGER_EQ, "pins and ledger differ"),
]

# Removing a vector's/string's MARK trace is caught first by the collector's
# own invariant (gc.rs "rewriting an unmarked root"), not by the test, so it
# proves nothing about the test. Those are kept to show it, and re-aimed as the
# *-unforwarded mutants (traced but not rewritten: a silently stale index),
# which must die at the retention test's content assertion.
INDIRECT = {"gc-vector-untraced/make-vector", "gc-vector-untraced/vconcat",
            "gc-vector-untraced/reverse", "gc-props-untraced/propertize"}

only = set(sys.argv[1:])
results = []
for name, path, old, new, test, line, marker in MUTANTS:
    if only and name not in only:
        continue
    f = ROOT / path
    orig = f.read_text()
    n = orig.count(old)
    if n != 1:
        results.append((name, test, f"NOT APPLIED ({n} sites)"))
        continue
    f.write_text(orig.replace(old, new, 1))
    try:
        r = subprocess.run(["cargo", "test", "-q", "-p", "romacs-vm", "--test", "frontier5", "--", "--exact", test],
                           capture_output=True, text=True, timeout=1200)
    finally:
        f.write_text(orig)
    out = r.stdout + r.stderr
    log = ROOT / f"crates/romacs-vm/tools/primh/logs/frontier5-mutant-{name.replace('/', '-')}.log"
    log.write_text(out)
    m = re.search(r"panicked at ([^\n]+):(\d+):\d+:\n(.*)", out)
    if "error[E" in out or "could not compile" in out:
        verdict = "COMPILE-ERROR (not red evidence)"
    elif r.returncode == 0:
        verdict = "SURVIVED"
    elif m and m.group(1).endswith("tests/frontier5.rs") and int(m.group(2)) == line and marker in out:
        verdict = f"KILLED at frontier5.rs:{line}"
    else:
        where = f"{m.group(1)}:{m.group(2)}" if m else "no panic location"
        verdict = f"DIED ELSEWHERE ({where})"
    results.append((name, test, verdict))
    print(f"{name:36} {test:62} {verdict}", flush=True)
assert all(Path(ROOT / p).read_text() for _, p, *_ in MUTANTS)
counted = [r for r in results if r[0] not in INDIRECT]
bad = [r for r in counted if not r[2].startswith("KILLED")]
print(f"{len(counted) - len(bad)}/{len(counted)} killed at their assertion; "
      f"{len(results) - len(counted)} mark-phase mutants reported, not counted (re-aimed)")
sys.exit(1 if bad else 0)
