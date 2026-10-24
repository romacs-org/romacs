#!/usr/bin/env python3
"""Apply one source mutant at a time, run the built-in variable tests, restore.

Each mutant breaks one thing the tests protect; a mutant is killed when a test
fails at an assertion (not a compile error). Logs: mutant-<name>.log; summary:
mutants-summary.json (failed tests and the first panic line of each).
"""
import json, os, pathlib, re, subprocess, sys
ROOT = pathlib.Path(__file__).resolve().parents[4]
OUT = pathlib.Path(__file__).resolve().parent
V = 'crates/romacs-vm/src/'
MUTANTS = {
    # Collector: forwarded storage not marked / not rewritten.
    'gc-no-mark': (V + 'gc.rs', "        self.forwarded.visit_values(&mut |v| pending.push(*v));\n", ""),
    'gc-no-rewrite': (V + 'gc.rs', "        self.forwarded.visit_values(&mut |v| rewrite(v));\n", ""),
    # Per-buffer values not traced with their buffer.
    'buffer-no-trace': (V + 'annotations.rs', "        self.per_buffer.visit_values(f);\n", ""),
    # Bool kind stores the raw value's truth wrongly (always true).
    'bool-always-true': (V + 'forwarded.rs', "            Fwd::Bool(i) => self.forwarded.bools[i as usize] = value != Value::NIL,", "            Fwd::Bool(i) => self.forwarded.bools[i as usize] = true,"),
    # Int kind accepts any value (CHECK_INTEGER removed).
    'int-no-check': (V + 'forwarded.rs', "                let n = value.integer_value().ok_or(Fault::Wrong(\"integerp\", value))?;", "                let n = value.integer_value().unwrap_or(0);"),
    # Predicate never applied to a per-buffer set.
    'per-buffer-no-predicate': (V + 'forwarded.rs', "                        Some(Predicate::Integerp) if value.integer_value().is_none() => {", "                        Some(Predicate::Integerp) if false => {"),
    # A per-buffer set does not make the variable local.
    'per-buffer-no-auto-local': (V + 'forwarded.rs', "                    self.per_buffer_mut(buffer).expect(\"live buffer\").set(i, current);\n                }\n            }\n        }\n        self.store_forwarded(fwd, value, buffer)", "                    let _ = current;\n                }\n            }\n        }\n        self.store_forwarded(fwd, value, buffer)"),
    # let of a non-local per-buffer variable binds the buffer, not the default.
    'let-never-default': (V + 'forwarded.rs', "        let let_default = matches!(fwd, Fwd::Buffer(i) if !self.per_buffer_local_p(buffer, i));", "        let let_default = false && matches!(fwd, Fwd::Buffer(i) if !self.per_buffer_local_p(buffer, i));"),
    # Unbinding restores nothing.
    'unbind-noop': (V + 'lib.rs', "                self.unbind_forwarded(fwd, value, owner, let_default);\n", ""),
    # set-default of a per-buffer variable applies set_internal (and its predicate).
    'set-default-is-set': (V + 'forwarded.rs', "            Fwd::Buffer(i) => {\n                self.forwarded.defaults[i as usize] = value;\n                Ok(())\n            }\n            _ => self.set_forwarded(slot, fwd, value, flag),", "            _ => self.set_forwarded(slot, fwd, value, flag),"),
    # makunbound of a built-in variable succeeds (stores UNBOUND).
    'makunbound-allowed': (V + 'forwarded.rs', "        if value == Value::UNBOUND {\n            return Err(Fault::Error(format!(\"{UNBINDABLE}{}\", self.symbol_names[slot])));\n        }\n", ""),
    # defvaralias may alias a built-in variable.
    'defvaralias-allows-builtin': (V + 'prims/vars.rs', "    if cx.variable_forwarded(new_alias)? {", "    if false && cx.variable_forwarded(new_alias)? {"),
    # Built-in variables are not marked special.
    'not-special': (V + 'lib.rs', "            vm.specials.insert(name.into());\n", ""),
    # Reads bypass the forwarding (redirect never installed).
    'no-redirect': (V + 'variable_runtime.rs', "                self.variable_redirects[slot] = Some(VariableRedirect::Forwarded(fwd));\n", ""),
    # after-load-alist / current-load-list (lread.c:5637, 5693). Each is
    # killed with the varh replay included (--with-varh).
    # Wrong C initial value: t, or a fresh cons, instead of nil.
    'after-load-alist-init-t': (V + 'forwarded.rs', 'name: "after-load-alist", gnu: "lread.c:5637", kind: Kind::Lisp(Init::Nil)', 'name: "after-load-alist", gnu: "lread.c:5637", kind: Kind::Lisp(Init::True)'),
    'current-load-list-init-cons': (V + 'forwarded.rs', 'name: "current-load-list", gnu: "lread.c:5693", kind: Kind::Lisp(Init::Nil)', 'name: "current-load-list", gnu: "lread.c:5693", kind: Kind::Lisp(Init::List(&[Init::Nil]))'),
    # Wrong forwarding kind: a DEFVAR_BOOL stores only t/nil.
    'current-load-list-as-bool': (V + 'forwarded.rs', 'name: "current-load-list", gnu: "lread.c:5693", kind: Kind::Lisp(Init::Nil)', 'name: "current-load-list", gnu: "lread.c:5693", kind: Kind::Bool(false)'),
    'after-load-alist-as-bool': (V + 'forwarded.rs', 'name: "after-load-alist", gnu: "lread.c:5637", kind: Kind::Lisp(Init::Nil)', 'name: "after-load-alist", gnu: "lread.c:5637", kind: Kind::Bool(false)'),
    # Not declared at all (an ordinary, void, non-special symbol).
    'after-load-alist-undeclared': (V + 'forwarded.rs', '    CVariable { name: "after-load-alist", gnu: "lread.c:5637", kind: Kind::Lisp(Init::Nil) },\n', ''),
    'current-load-list-undeclared': (V + 'forwarded.rs', '    CVariable { name: "current-load-list", gnu: "lread.c:5693", kind: Kind::Lisp(Init::Nil) },\n', ''),
}

WITH_VARH = '--with-varh' in sys.argv
SUMMARY = next((a.split('=', 1)[1] for a in sys.argv if a.startswith('--summary=')), 'mutants-summary.json')

def run(name):
    path, old, new = MUTANTS[name]
    p = ROOT / path
    src = p.read_text()
    assert src.count(old) == 1, (name, src.count(old))
    p.write_text(src.replace(old, new, 1))
    try:
        log = OUT / f'mutant-{name}.log'
        with open(log, 'w') as f:
            subprocess.run(['cargo', 'test', '-p', 'romacs-vm', '--test', 'c_variables', '--no-fail-fast'], cwd=ROOT, stdout=f, stderr=subprocess.STDOUT)
            subprocess.run(['cargo', 'test', '-p', 'romacs-vm', '--lib', 'forwarded_tests'], cwd=ROOT, stdout=f, stderr=subprocess.STDOUT)
            if WITH_VARH:
                subprocess.run(['cargo', 'test', '-p', 'romacs-vm', '--test', 'varh'], cwd=ROOT, stdout=f, stderr=subprocess.STDOUT)
        text = log.read_text()
        failed = sorted({l.split()[1] for l in text.splitlines() if l.startswith('test ') and l.rstrip().endswith('FAILED')})
        panics = {}
        lines = text.splitlines()
        for i, l in enumerate(lines):
            m = re.match(r"thread '([^']+)' .* panicked at (.*)", l)
            if m and i + 1 < len(lines):
                panics[m.group(1)] = (m.group(2) + ' ' + lines[i + 1])[:300]
        compile_error = 'error[E' in text or 'could not compile' in text
        return {'mutant': name, 'killed': bool(failed) and not compile_error, 'failed_tests': failed, 'panics': panics, 'compile_error': compile_error}
    finally:
        p.write_text(src)

names = [a for a in sys.argv[1:] if not a.startswith('--')] or list(MUTANTS)
results = [run(n) for n in names]
for r in results:
    print(json.dumps({k: r[k] for k in ('mutant', 'killed', 'failed_tests', 'compile_error')}))
(OUT / SUMMARY).write_text(json.dumps(results, indent=1) + '\n')
