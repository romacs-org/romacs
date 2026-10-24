#!/usr/bin/env python3
"""Bounded exact-replacement mutants; restore source even on failure."""
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[4]
HERE = Path(__file__).resolve().parent
MUTANTS = [
    ('vector-is-obarray', 'src/prims/obarray.rs', 'cx.is_obarray(object)', 'cx.is_obarray(object) || cx.kind(object) == leaf::Kind::Vector', 'obarray_predicate_distinguishes_vectors', False),
    ('make-returns-nil', 'src/prims/obarray.rs', 'cx.make_obarray(size)', '{ let _ = (cx, size); Ok(Value::NIL) }', 'obarray_make', False),
    ('intern-returns-nil', 'src/prims/widen.rs', 'cx.intern_in(table, name)', '{ let _ = (table, name); Ok(Value::NIL) }', 'intern_custom_identity', False),
    ('soft-always-misses', 'src/prims/obarray.rs', 'cx.lookup_in(table, name)', '{ let _ = (table, name); Ok(Value::NIL) }', 'intern_soft_identity', False),
    ('unintern-does-not-remove', 'src/prims/obarray.rs', 'cx.unintern_in(table, name)', '{ let _ = (table, name); Ok(Value::TRUE) }', 'unintern_removal', False),
    ('mapatoms-skips-callbacks', 'src/prims/obarray.rs', 'walk(cx, vec![function, table, Value::integer(0).unwrap(), Value::UNBOUND])', '{ let _ = (cx, function, table); Ok(Step::Done(Value::NIL)) }', 'mapatoms_exact', False),
    ('gc-loses-symbol-identity', 'src/gc.rs', 't.visit_values(&mut |v| rewrite(v));', 't.visit_values(&mut |v| { rewrite(v); *v = Value::TRUE; });', 'obarray::tests::obarray_identity_moves_and_retention_is_flat', True),
    ('condition-registry-roots-dead-symbols', 'src/gc.rs', 'if !custom {', 'if true || !custom {', 'obarray::tests::obarray_condition_properties_do_not_root_dead_symbols', True),
    ('all-live-skips-symbol-sweep', 'src/gc.rs', 'if marked == before && obarray_symbols_live.len() == self.obarray_symbol_names.len() {', 'if marked == before {', 'obarray::tests::obarray_dead_symbol_reclaimed_with_fully_live_heap', True),
    ('compiled-operands-drop-symbols', 'src/gc.rs', '| Op::Define(slot, _) | Op::DefineMacro(slot, _) | Op::ResolveHead(_, slot) =>', '=>', 'obarray::tests::obarray_compiled_call_retains_symbol_slot', True),
]
results = []
for name, relative, old, new, test, unit in MUTANTS:
    path = ROOT / 'crates/romacs-vm' / relative
    original = path.read_bytes()
    text = original.decode()
    assert text.count(old) == 1, (name, text.count(old))
    log = HERE / (name + '.log')
    try:
        path.write_text(text.replace(old, new))
        command = ['cargo', 'test', '--jobs', '3', '-p', 'romacs-vm'] + (['--lib'] if unit else ['--test', 'obarray_substrate']) + [test, '--', '--nocapture', '--test-threads=1']
        with log.open('w') as stream:
            result = subprocess.run(command, cwd=ROOT, env=os.environ, stdout=stream, stderr=subprocess.STDOUT)
        output = log.read_text()
        decisive = {
            'condition-registry-roots-dead-symbols': 'a custom condition property cannot keep its dead owner alive',
            'all-live-skips-symbol-sweep': 'fully live object arena still sweeps dead symbols',
            'compiled-operands-drop-symbols': 'compiled custom-symbol operand retains its original slot',
        }.get(name)
        killed = result.returncode == 101 and 'test result: FAILED' in output and 'panicked at' in output and ((decisive in output) if decisive else ('assertion' in output or 'obarray_substrate.rs:' in output))
        results.append(dict(name=name, command=command, exit=result.returncode, behavioral_kill=killed, original_sha256=hashlib.sha256(original).hexdigest(), log=log.name))
        (HERE / 'mutations.json').write_text(json.dumps(results, indent=2) + '\n')
        assert killed, (name, output[-4000:])
    finally:
        path.write_bytes(original)
        assert path.read_bytes() == original
print(json.dumps(results, indent=2))
