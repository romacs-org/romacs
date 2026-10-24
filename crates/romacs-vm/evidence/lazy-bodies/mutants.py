#!/usr/bin/env python3
"""Apply one source mutant at a time, run the lazy-body tests, restore."""
import os as _portable_os
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import subprocess, sys, json, os, pathlib
ROOT = pathlib.Path(_portable_os.environ['ROMACS_MUTATION_ROOT'])
OUT = pathlib.Path(str(_PortablePath(_portable_tempfile.gettempdir()) / 'lazy-oracle/mutants'))
OUT.mkdir(exist_ok=True)
V = 'crates/romacs-vm/src/'
MUTANTS = {
 'gc-no-mark': (V+'gc.rs', "                    pending.push(lazy.body);\n", ""),
 'gc-no-forward': (V+'gc.rs', "                rewrite(&mut Arc::get_mut(lazy).expect(\"lazy body is private at a wait point\").body);\n", ""),
 'retire-keeps-body': (V+'request_code.rs', "                f.lazy = None;\n", ""),
 'leak-after-expansion': (V+'lib.rs', "        self.functions[id] = Arc::new(function);\n        Ok(())", "        self.functions[id] = Arc::new(function);\n        let leak = self.slot(\"lz-mutant-leak\");\n        self.slots[leak].value = lazy.body;\n        Ok(())"),
 'no-deferral': (V+'lib.rs', "        if !self.defer_expansion_errors || !matches!(f, Form::List(a) if !a.is_empty()) {", "        if true || !self.defer_expansion_errors || !matches!(f, Form::List(a) if !a.is_empty()) {"),
 'thunk-not-strict': (V+'lib.rs', "std::mem::replace(&mut self.defer_expansion_errors, !lazy.deferred);", "std::mem::replace(&mut self.defer_expansion_errors, true);"),
 'regime-always-eager': (V+'loader.rs', "        let lazy = matches!(self.slots[expander].function, f if f == Value::UNBOUND || f == Value::NIL);", "        let lazy = false && matches!(self.slots[expander].function, f if f == Value::UNBOUND || f == Value::NIL);"),
 'regime-always-lazy': (V+'loader.rs', "        let lazy = matches!(self.slots[expander].function, f if f == Value::UNBOUND || f == Value::NIL);", "        let lazy = true || matches!(self.slots[expander].function, f if f == Value::UNBOUND || f == Value::NIL);"),
 'no-propagate': (V+'source_cache.rs', "            self.propagate_next = lazy;\n", "            self.propagate_next = false;\n"),
 'ifp-always-t': (V+'prims/lang.rs', "    Ok(if cx.kind(object) == crate::prim::leaf::Kind::Closure { Value::TRUE } else { Value::NIL })", "    let _ = object; Ok(Value::TRUE)"),
 'captures-unmutated': (V+'lib.rs', "            capture_mutations: vec![true; names.len()],\n            mapped_code: None,\n            code: vec![],", "            capture_mutations: vec![false; names.len()],\n            mapped_code: None,\n            code: vec![],"),
 'force-keeps-lazy': (V+'lib.rs', "        function.capture_mutations = vec![true; names.len()];\n", "        function.capture_mutations = vec![true; names.len()];\n        function.lazy = Some(lazy.clone());\n"),
}
def run(name):
    path, old, new = MUTANTS[name]
    p = ROOT / path
    src = p.read_text()
    assert src.count(old) == 1, (name, src.count(old))
    p.write_text(src.replace(old, new, 1))
    try:
        env = dict(os.environ)
        log = OUT / f'{name}.log'
        with open(log, 'w') as f:
            r1 = subprocess.run(['cargo', 'test', '-p', 'romacs-vm', '--test', 'lazy_bodies', '--test', 'bootstrap_descriptors', '--no-fail-fast'], cwd=ROOT, stdout=f, stderr=subprocess.STDOUT, env=env)
            r2 = subprocess.run(['cargo', 'test', '-p', 'romacs-vm', '--lib', 'lazy_tests'], cwd=ROOT, stdout=f, stderr=subprocess.STDOUT, env=env)
        text = log.read_text()
        failed = sorted({l.split()[1] for l in text.splitlines() if l.startswith('test ') and l.rstrip().endswith('FAILED')})
        compile_error = 'error[E' in text or 'could not compile' in text
        return {'mutant': name, 'killed': bool(failed), 'failed_tests': failed, 'compile_error': compile_error}
    finally:
        p.write_text(src)
names = sys.argv[1:] or list(MUTANTS)
results = [run(n) for n in names]
for r in results:
    print(json.dumps(r))
(OUT / 'summary.json').write_text(json.dumps(results, indent=1))
