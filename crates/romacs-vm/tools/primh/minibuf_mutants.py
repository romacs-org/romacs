#!/usr/bin/env python3
"""Single-site implementation mutants for read-from-minibuffer / read-string.

Each mutant edits exactly one site of src/prims/minibuf.rs or minibuf.el,
asserts the edit applied, builds, and runs the recorded-corpus replay
(tests/primh_minibuf.rs) plus tests/minibuf.rs. A mutant is KILLED only when
the run fails at a corpus or test assertion; a compile error is not a kill.
Usage (owned checkout with the project environment): python3 minibuf_mutants.py
"""
import re, subprocess, sys
from pathlib import Path

RS = Path("crates/romacs-vm/src/prims/minibuf.rs")
EL = Path("crates/romacs-vm/src/prims/minibuf.el")
MUTANTS = [
    ("READ ignored", RS, "    if read == Value::NIL {\n        return Ok(Step::Done(val));", "    if true {\n        return Ok(Step::Done(val));"),
    ("read-string DEFAULT never substituted", RS, "    if empty && default != Value::NIL {", "    if false && empty && default != Value::NIL {"),
    ("read-string DEFAULT cons: whole list, not car", RS, "cx.pair(default).map_or(default, |(car, _)| car)", "default"),
    ("READ: empty input ignores DEFAULT-VALUE", RS, "    if cx.string_view(val).is_ok_and(|s| s.chars == 0) {\n        if cx.string_view(default)", "    if false {\n        if cx.string_view(default)"),
    ("READ: trailing garbage accepted", RS, "        if text[end..].iter().any(|&c| !matches!(c, ' ' | '\\t' | '\\n')) {", "        if false {"),
    ("INITIAL (STRING . POSITION) position ignored", RS, "if n < 1 { -chars } else { n - 1 - chars }", "0"),
    ("INITIAL cons car not checked", RS, "let chars = cx.string_view(string).map_err(|_| Fault::Wrong(\"stringp\", string))?.chars as i64;", "let chars = cx.string_view(string).map_or(0, |s| s.chars) as i64;"),
    ("PROMPT not checked", RS, "    if cx.string_view(prompt).is_err() {\n        return Err(Fault::Wrong(\"stringp\", prompt));", "    if false {\n        return Err(Fault::Wrong(\"stringp\", prompt));"),
    ("inhibit-interaction ignored", RS, "if cx.variable_value(inhibit)?.is_some_and(|v| v != Value::NIL) {", "if false {"),
    ("KEYMAP not checked", RS, "cx.keymap(keymap).ok_or(Fault::Wrong(\"keymapp\", keymap))?", "cx.keymap(keymap).unwrap_or(keymap)"),
    ("DEFAULT register swapped for PROMPT (read-string identity)", RS, "    let default = saved[1];\n    let empty", "    let default = saved[2];\n    let empty"),
    ("C-g returns instead of signalling", EL, "((functionp val) (funcall val))", "((functionp val) nil)"),
    ("no keyboard macro: reads anyway (no end-of-file)", EL, "  (unless executing-kbd-macro\n    (signal 'end-of-file", "  (unless t\n    (signal 'end-of-file"),
    ("unbound HIST variable left unbound", EL, "(unless (boundp histvar) (set histvar nil))", "nil"),
    ("prompt `field' property dropped", EL, "(put-text-property 1 (point) 'field t)", "nil"),
    ("minibuffer-allow-text-properties ignored", EL, "(setq val (if minibuffer-allow-text-properties", "(setq val (if nil"),
    ("caller's window not reselected", EL, "      (select-window calling-window)\n", "      nil\n"),
    ("minibuffer not emptied on exit", EL, "      (set-buffer buffer)\n      (erase-buffer)\n", "      (set-buffer buffer)\n"),
    # Not listed: moving the level/default restore after the window steps is an
    # EQUIVALENT mutant today. With the window-live-p guard and the guarded
    # set-buffer, no later unwind step can signal in VM2 (no kill-buffer), so
    # no test can observe the order; it is kept first defensively.
    ("unwind: dead calling window reselected", EL, "      (when (window-live-p calling-window)\n        (select-window calling-window))", "      (select-window calling-window)"),
    ("RET not bound to exit-minibuffer", EL, "  (define-key minibuffer-local-map \"\\r\" 'exit-minibuffer))", "  nil)"),
    ("recursion check removed", EL, "(when (and (> romacs--minibuf-level 0) (not enable-recursive-minibuffers))", "(when nil"),
    ("end of input does not end the prompt", EL, "      (if (and (consp event) (memq (car event) '(t no-record))) (cdr event) event))))", "      (if (and (consp event) (memq (car event) '(t no-record))) (cdr event) event))\n    (unless unread-command-events (throw 'exit \"end\"))))"),
]


def run():
    r = subprocess.run(["cargo", "test", "-p", "romacs-vm", "--test", "primh_minibuf", "--test", "minibuf"],
                       capture_output=True, text=True)
    return r.returncode, r.stdout + r.stderr


def main():
    rows, survivors = [], 0
    only = sys.argv[1:]
    for name, path, old, new in MUTANTS:
        if only and not any(o in name for o in only):
            continue
        original = path.read_text()
        if original.count(old) != 1:
            sys.exit(f"mutant site not unique/present: {name}")
        path.write_text(original.replace(old, new))
        try:
            assert path.read_text() != original, "mutant did not apply"
            code, log = run()
        finally:
            path.write_text(original)
        compile_error = "error[E" in log or "could not compile" in log and "test failed" not in log
        hits = sorted(set(re.findall(r"(NEW DIVERGENCE \([A-Z-]+\): [a-z-]+/\d+|CHANGED PIN[^:]*: [a-z-]+/\d+|STALE PIN \([A-Z-]+\): [a-z-]+/\d+|panicked at crates/romacs-vm/tests/minibuf\.rs:\d+)", log)))
        killed = code != 0 and not compile_error and bool(hits)
        survivors += not killed
        verdict = "KILLED" if killed else ("COMPILE-ERROR" if compile_error else "SURVIVED")
        rows.append((name, path.name, verdict, hits))
        print(f"{verdict:13} {path.name:12} {name}  [{len(hits)} assertion sites: {'; '.join(hits[:3])}]", flush=True)
    code, log = run()
    print(f"restored tree: cargo test exit {code}")
    sys.exit(1 if survivors or code else 0)


if __name__ == "__main__":
    main()
