#!/usr/bin/env python3
"""Mapcan-only mutants; require the final panic location and case message to match."""
import re, subprocess, sys
from pathlib import Path

ROOT = Path.cwd()
P = "crates/romacs-vm/src/prims/fns_map.rs"
L = "crates/romacs-vm/src/prim.rs"
E = "crates/romacs-vm/src/threaded.rs"
T = "crates/romacs-vm/tests/mapcan.rs"
PT = "crates/romacs-vm/tests/primh_mapcan.rs"


def line_of(path, marker):
    for i, l in enumerate((ROOT / path).read_text().splitlines(), 1):
        if marker in l:
            return i
    raise SystemExit(f"no line {marker!r} in {path}")


CHECK = (T, line_of(T, 'assert!(bad.is_empty(), "{} of {} rows differ from GNU'))
CONTENT = (T, line_of(T, '"{map}: identity and contents after moving collections inside the walk"'))
RETAINED = (T, line_of(T, '"{map}: the walk retained objects or bytes after its result was dropped"'))
PRIMH = (PT, line_of(PT, 'assert!(bad.is_empty(), "{} {}:'))
PANIC_FREE = (T, line_of(T, 'assert_eq!(shown, "(ok \\"String\'s bytes changed under a mapping function\\")", "{source}");'))

STASH_DECL = "const K_RES: usize = 0;"
STASH = ("thread_local! { static MUTANT_STASH: std::cell::RefCell<Vec<Value>> = const { std::cell::RefCell::new(Vec::new()) }; }\n"
         "const K_RES: usize = 0;")

MUTANTS = [
    ("walk-ignores-shortened-list", [(P, "None => return finish(cx, &state, i),", "None => Value::NIL,")],
     "mapcan", "mapcan_matches_gnu_values", CHECK, "(setcdr l nil)"),
    ("string-cursor-not-advanced", [(P, "state[R_CURSOR] = fixnum(at + width);", "state[R_CURSOR] = fixnum(at);")],
     "primh_mapcan", "primh_mapcan", PRIMH, "(string ?a ?λ ?雪 ?b)"),
    ("unibyte-high-bytes-widened", [(L, "(u32::from(bytes[0]), 1)", "(u32::from(bytes[0]) + if bytes[0] >= 128 { 0x3FFF00 } else { 0 }, 1)")],
     "mapcan", "mapcan_matches_gnu_values", CHECK, "(unibyte-string 255 65)"),
    ("chartable-accepted", [(P, "        return Err(Fault::Wrong(\"listp\", sequence));\n    }\n    state[R_N]", "    }\n    state[R_N]")],
     "mapcan", "mapcan_matches_gnu_values", CHECK, "make-keymap"),
    ("nconc-names-listp", [(P, "return Err(Fault::Wrong(\"consp\", list));", "return Err(Fault::Wrong(\"listp\", list));")],
     "mapcan", "mapcan_matches_gnu_values", CHECK, "consp 2"),
    ("nconc-nil-next-not-carried", [(P, "            args[k + 1] = last;", "            let _ = last;")],
     "mapcan", "mapcan_matches_gnu_values", CHECK, "(list a nil b)"),
    ("vector-index-frozen", [(P, "cx.vector_get(sequence, fixnum(i))?", "cx.vector_get(sequence, fixnum(0))?")],
     "primh_mapcan", "primh_mapcan", PRIMH, "(vector 1 \"s\" 'q)"),
    ("improper-list-walked-unchecked", [(P, "Ok(Step::Call { f: subr(\"length\"), args: vec![sequence], then: measured, saved })",
                                         "Ok(Step::Call { f: subr(if sequence.tag() == 3 { \"safe-length\" } else { \"length\" }), args: vec![sequence], then: measured, saved })")],
     "primh_mapcan", "primh_mapcan", PRIMH, "(cons 1 (cons 2 3))) (error e)) calls))"),
    ("string-cursor-unchecked", [(L, "            let width = match bytes[0] {\n                0x00..=0x7f => 1,\n                0xc0..=0xdf => 2,\n                0xe0..=0xef => 3,\n                0xf0..=0xf7 => 4,\n                0xf8 => 5,\n                _ => return None,\n            };\n            let whole = bytes.get(..width)?;\n            whole[1..].iter().all(|b| b & 0xc0 == 0x80).then(|| crate::lstring::string_char(whole))",
                                  "            Some(crate::lstring::string_char(bytes))")],
     "mapcan", "a_string_cursor_left_inside_a_character_is_an_error_not_a_panic", PANIC_FREE, "PANIC"),
    ("gc-saved-held-in-rust", [
        (E, STASH_DECL, STASH),
        (E, "        std::ptr::copy_nonoverlapping(saved.as_ptr(), k.add(K_SAVED), saved.len());",
            "        std::ptr::copy_nonoverlapping(saved.as_ptr(), k.add(K_SAVED), saved.len());\n        MUTANT_STASH.with(|s| *s.borrow_mut() = saved.clone());"),
        (E, "        let saved = std::slice::from_raw_parts(k.add(K_SAVED), n).to_vec();",
            "        let _ = n;\n        let saved = MUTANT_STASH.with(|s| s.borrow().clone());"),
     ], "mapcan", "mapcan_accumulates_across_moving_collections_inside_every_callback", CONTENT, "identity and contents"),
    ("gc-saved-held-in-rust/vector", None, "mapcan", "mapcan_vector_accumulates_across_moving_collections_inside_every_callback", CONTENT, "identity and contents"),
    ("gc-saved-held-in-rust/string", None, "mapcan", "mapcan_string_survives_moving_collections_inside_every_callback", CONTENT, "identity and contents"),
]
SHARED = {n: e for n, e, *_ in MUTANTS if e is not None}

only = set(sys.argv[1:])
unknown = only - {name for name, *_ in MUTANTS}
if unknown:
    raise SystemExit(f"unknown mutants: {sorted(unknown)}")
results = []
for name, edits, binary, test, (tfile, line), marker in MUTANTS:
    if only and name not in only:
        continue
    edits = edits if edits is not None else SHARED[name.split("/")[0]]
    originals, applied = {}, True
    for path, old, new in edits:
        f = ROOT / path
        originals.setdefault(path, f.read_text())
        text = f.read_text()
        if text.count(old) != 1:
            applied = False
            break
        f.write_text(text.replace(old, new, 1))
    try:
        if not applied:
            out, rc = f"NOT APPLIED: {old!r}", None
        else:
            r = subprocess.run(["cargo", "test", "-q", "-p", "romacs-vm", "--test", binary, "--", "--exact", test],
                               capture_output=True, text=True, timeout=2400)
            out, rc = r.stdout + r.stderr, r.returncode
    finally:
        for path, text in originals.items():
            (ROOT / path).write_text(text)
    log = ROOT / f"crates/romacs-vm/tools/primh/logs/mapcan-mutant-{name.replace('/', '-')}.log"
    log.write_text(out)
    panics = list(re.finditer(r"panicked at ([^\n]+?):(\d+):\d+:\n(.*)", out))
    m = panics[-1] if panics else None
    if rc is None:
        verdict = "NOT APPLIED"
    elif "error[E" in out or "could not compile" in out:
        verdict = "COMPILE-ERROR (not red evidence)"
    elif rc == 0:
        verdict = "SURVIVED"
    elif m and m.group(1).endswith(tfile) and int(m.group(2)) == line and marker in out:
        verdict = f"KILLED at {Path(tfile).name}:{line}"
    else:
        where = f"{m.group(1)}:{m.group(2)}" if m else "no panic location"
        verdict = f"DIED ELSEWHERE ({where})"
    results.append((name, test, verdict))
    print(f"{name:34} {test:72} {verdict}", flush=True)
for path in {p for _, e, *_ in MUTANTS if e for p, _, _ in e}:
    assert subprocess.run(["git", "diff", "--quiet", "--", path]).returncode in (0, 1)
bad = [r for r in results if not r[2].startswith("KILLED")]
print(f"{len(results) - len(bad)}/{len(results)} killed at their message-keyed target assertion")
sys.exit(1 if bad else 0)
