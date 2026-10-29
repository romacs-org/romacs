#!/usr/bin/env python3
"""Single-site mutants of the command-loop primitives, run in a scratch worktree.

Each mutant must apply exactly once; a compile failure is not a kill. A kill is
the fresh per-row corpus replay (`fresh_replay.py`, a new VM per row at
--gc-floor 1) failing on a row of the mutated primitive, or, for the table
mutants, the named `cargo test` failing at its assertion.

  python3 mutants.py WORKTREE TARGET_DIR OUT.json
"""
import json, subprocess, sys
from pathlib import Path

H = "crates/romacs-vm/tools/primh"
CI = "src/prims/callint.rs"
RH = "src/prims/eval_hooks.rs"
CMD = "src/command.rs"
PRIM = "src/prim.rs"

MUTANTS = [
    # run-hook-with-args
    ("rh-args-dropped", RH, "    Ok(Step::Call { f, args: args.to_vec(), then, saved })", "    Ok(Step::Call { f, args: vec![], then, saved })", ["run-hook-with-args"]),
    ("rh-t-not-spliced", RH, "        if element == Value::TRUE {", "        if false {", ["run-hook-with-args"]),
    ("rh-global-t-called", RH, "        if element != Value::TRUE {", "        if true {", ["run-hook-with-args"]),
    # Only differs on (lambda ...) list hook functions; the unit test asserts
    # which object reached funcall.
    ("rh-function-value-walked", RH, "        if cx.pair(val).is_none() || function_p(cx, val)? {", "        if cx.pair(val).is_none() {", "test:prims::t2_tests::run_hooks_calls_lambda_lists_whole"),
    # Every hook function receives only the first argument (the call site).
    ("rh-only-first-arg", RH, "    Ok(Step::Call { f, args: args.to_vec(), then, saved })", "    Ok(Step::Call { f, args: args[..args.len().min(1)].to_vec(), then, saved })", ["run-hook-with-args"]),
    # The first hook function gets every argument, the later ones only the
    # first (the saved registers, kept self-consistent).
    ("rh-later-only-first-arg", RH, "    saved.push(Value::integer(args.len() as i64).expect(\"argument count is a fixnum\"));\n    saved.extend_from_slice(args);", "    saved.push(Value::integer(args.len().min(1) as i64).expect(\"argument count is a fixnum\"));\n    saved.extend_from_slice(&args[..args.len().min(1)]);", ["run-hook-with-args"]),
    # The first version of rh-only-first-arg: it truncates the saved arguments
    # but keeps the full count, so split_args misreads the registers and the VM
    # panics on the first multi-argument, multi-function row. fresh_replay
    # aborts on the panic, which this runner reports as CRASH, not a kill.
    ("rh-saved-args-truncated", RH, "    saved.extend_from_slice(args);", "    saved.extend_from_slice(&args[..args.len().min(1)]);", ["run-hook-with-args"]),
    # prefix-numeric-value
    ("pnv-minus-is-one", CI, "        return fixnum(-1);", "        return fixnum(1);", ["prefix-numeric-value"]),
    ("pnv-cons-head-ignored", CI, "        if head.integer_value().is_some() {", "        if false {", ["prefix-numeric-value"]),
    # funcall-interactively
    ("fi-args-reversed", CI, "    Ok(Step::Call { f: function, args: args.to_vec(), then: done, saved: vec![] })", "    Ok(Step::Call { f: function, args: args.iter().rev().copied().collect(), then: done, saved: vec![] })", ["funcall-interactively"]),
    # interactive-form
    ("if-property-ignored", CI, "        if spec != Value::NIL {", "        if false {", ["interactive-form"]),
    ("if-subr-spec-p", CI, "        \"forward-char\" | \"backward-char\" | \"beginning-of-line\" | \"end-of-line\" | \"forward-line\" => \"^p\",", "        \"forward-char\" | \"backward-char\" | \"beginning-of-line\" | \"end-of-line\" | \"forward-line\" => \"p\",", ["interactive-form"]),
    ("if-lambda-modes-kept", CI, "                if cdr(cx, cdr(cx, spec)?)? == Value::NIL {", "                if true {", ["interactive-form"]),
    ("if-declare-not-skipped", CMD, "                } else if head_is(Some(form), \"declare\") {", "                } else if false && head_is(Some(form), \"declare\") {", ["interactive-form"]),
    # Review round 2 (Sol): defun metadata order, dotted (:documentation FORM . X).
    ("if-late-doc-accepted", CMD, "                    if doc.is_none() && !declared && iform.is_none() {", "                    if doc.is_none() {", ["interactive-form"]),
    ("if-defun-order-fixed", CMD, "                } else if head_is(Some(form), \"interactive\") {", "                } else if j == 0 && head_is(Some(form), \"interactive\") {", ["interactive-form"]),
    ("if-dotted-doc-quoted", CMD, "                Form::List(items) | Form::Dotted(items, _) if items.len() >= 2 => items[1].clone(),", "                Form::List(items) if items.len() >= 2 => items[1].clone(),", ["interactive-form", "call-interactively"]),
    ("if-doc-not-skipped", CMD, "            Some(Form::String(_) | Form::Bytes(_)) if !documented && body.len() > 1 => 1,", "            Some(Form::String(_) | Form::Bytes(_)) if !documented && body.len() > 1 => 0,", ["interactive-form"]),
    ("if-documentation-kept", CMD, "            (body[1..].to_vec(), &[])", "            (body.to_vec(), &[])", ["interactive-form"]),
    ("if-dotted-modes-lost", CMD, "            Form::Dotted(items, _) if items.len() >= 2 => items[1].clone(),", "            Form::Dotted(items, _) if items.len() >= 2 => sym(\"nil\"),", ["interactive-form"]),
    ("ci-spec-not-snapshot", CI, "    saved[SPEC] = cx.make_string(bytes, multibyte);", "    let _ = (bytes, multibyte); saved[SPEC] = specs;", ["call-interactively"]),
    # call-interactively
    ("ci-P-is-numeric", CI, "            b'P' => { let h = quotify(cx, prefix); push(cx, &mut saved, prefix, h) }", "            b'P' => { let n = prefix_numeric(cx, prefix); push(cx, &mut saved, n, n) }", ["call-interactively"]),
    ("ci-p-plus-one", CI, "            b'p' => { let n = prefix_numeric(cx, prefix); push(cx, &mut saved, n, n) }", "            b'p' => { let n = fixnum(prefix_numeric(cx, prefix).integer_value().unwrap() + 1); push(cx, &mut saved, n, n) }", ["call-interactively"]),
    ("ci-d-zero-based", CI, "                let point = fixnum(cx.point()?);", "                let point = fixnum(cx.point()? - 1);", ["call-interactively"]),
    ("ci-star-ignored", CI, "                if cx.variable_value(read_only)?.unwrap_or(Value::NIL) != Value::NIL {", "                if false {", ["call-interactively"]),
    ("ci-vars-not-restored", CI, "    for (i, name) in COMMAND_VARS.iter().enumerate() {\n        let symbol = cx.intern(name);\n        cx.variable_set(symbol, saved[SAVED_VARS + i])?;", "    for (i, name) in COMMAND_VARS.iter().enumerate().take(0) {\n        let symbol = cx.intern(name);\n        cx.variable_set(symbol, saved[SAVED_VARS + i])?;", ["call-interactively"]),
    ("ci-lexical-env-lost", CI, "                if own == specs && thunk != Value::NIL {", "                if false && own == specs && thunk != Value::NIL {", ["call-interactively"]),
    ("ci-no-history", CI, "    if from_tty || saved[RECORD] != Value::NIL {", "    if false {", ["call-interactively"]),
    ("ci-history-off-by-one", CI, "            for _ in 1..max {", "            for _ in 0..max {", ["call-interactively"]),
    ("ci-dotted-names-whole", CI, "        return Err(Fault::Wrong(\"listp\", tail));", "        return Err(Fault::Wrong(\"listp\", list));", ["call-interactively"]),
    ("ci-e-skips-next-event", CI, "                saved[NEXT_EVENT] = fixnum(after as i64);", "                saved[NEXT_EVENT] = fixnum(after as i64 + 1);", ["call-interactively"]),
    ("ci-plus-accepted", CI, "            b'+' => return Err(Fault::Error(\"‘+’ is not used in ‘interactive’ for ordinary commands\".into())),", "            b'+' => {}", ["call-interactively"]),
    ("ci-keys-not-checked", CI, "    if keys != Value::NIL && cx.kind(keys) != Kind::Vector {", "    if false {", ["call-interactively"]),
    # The closure -> spec table.
    ("table-not-compacting", PRIM, "                let table = hash::HashTable::new_local_bindings();", "                let table = hash::HashTable::new(hash::Test::Eq, hash::Weakness::Key, 65);", "test:callint_tests::dead_interactive_closures_release_their_specs"),
    ("table-strong", PRIM, "                let table = hash::HashTable::new_local_bindings();", "                let table = hash::HashTable::new(hash::Test::Eq, hash::Weakness::None, 65);", "test:callint_tests::"),
]


def run(cmd, cwd, env=None):
    p = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True)
    return p.returncode, p.stdout + p.stderr


def main():
    tree, target, out = Path(sys.argv[1]), sys.argv[2], Path(sys.argv[3])
    import os
    env = dict(os.environ, CARGO_TARGET_DIR=target)
    crate = tree / "crates/romacs-vm"
    vm = f"{target}/debug/romacs-vm"
    results = []
    rc, text = run(["cargo", "build", "-q", "-p", "romacs-vm", "--bin", "romacs-vm"], tree, env)
    assert rc == 0, text[-2000:]
    for spec in ["run-hook-with-args", "prefix-numeric-value", "funcall-interactively", "interactive-form", "call-interactively"]:
        rc, text = run(["python3", "crates/romacs-vm/tools/primh/fresh_replay.py", f"{H}/callint-specs/{spec}.json", "--vm2", vm, "--json", f"{H}/logs/callint/fresh-{spec}.json"], tree, env)
        assert rc == 0, f"baseline {spec} not green: {text[-1500:]}"
        print(f"baseline {spec}: {text.strip().splitlines()[-1][:200]}", flush=True)
    for name, rel, old, new, check in MUTANTS:
        path = crate / rel
        original = path.read_text()
        count = original.count(old)
        if count != 1:
            results.append(dict(id=name, applied=False, count=count))
            print(f"{name}: NOT APPLIED ({count} sites)", flush=True)
            continue
        path.write_text(original.replace(old, new))
        try:
            assert path.read_text() != original
            if isinstance(check, str):
                test = check.split(":", 1)[1]
                rc, text = run(["cargo", "test", "-q", "-p", "romacs-vm", "--lib", test], tree, env)
                compiled = "error[E" not in text
                killed = compiled and rc != 0 and "panicked" in text
                evidence = [l for l in text.splitlines() if "panicked" in l or "retained" in l or "assert" in l][:4]
            else:
                rc, text = run(["cargo", "build", "-q", "-p", "romacs-vm", "--bin", "romacs-vm"], tree, env)
                compiled = rc == 0
                killed, evidence = False, []
                if compiled:
                    for spec in check:
                        sp = f"{H}/callint-specs/{spec}.json"
                        rc2, text2 = run(["python3", "crates/romacs-vm/tools/primh/fresh_replay.py", sp, "--vm2", vm, "--json", f"{target}/mut-{name}.json"], tree, env)
                        try:
                            bad = json.loads(text2.strip().splitlines()[-1])["bad"]
                        except Exception:
                            bad = []
                            if "Traceback" in text2:
                                crash = [l for l in text2.splitlines() if "Error" in l or "panicked" in l][-2:]
                                evidence += ["CRASH (not an assertion kill): " + " | ".join(crash)]
                        hits = [b for b in bad if f"{spec}/" in b]
                        if rc2 != 0 and hits:
                            killed = True
                            evidence += hits[:3]
            results.append(dict(id=name, file=rel, old=old, new=new, applied=True, compiled=compiled, killed=killed, evidence=evidence))
            print(f"{name}: applied compiled={compiled} killed={killed} {evidence[:1]}", flush=True)
        finally:
            path.write_text(original)
    rc, text = run(["git", "diff", "--quiet"], tree)
    assert rc == 0, "worktree left dirty"
    out.write_text(json.dumps(dict(tree=str(tree), mutants=results), indent=1) + "\n")
    print(f"killed {sum(r.get('killed', False) for r in results)}/{len(results)}")


main()
