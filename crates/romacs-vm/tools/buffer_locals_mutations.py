"""Run bounded behavioral mutants in the owning worktree; always restore bytes."""
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]
SRC = ROOT / 'crates/romacs-vm/src'
OUT = ROOT / 'crates/romacs-vm/evidence/buffer-local-substrate'
OUT.mkdir(exist_ok=True)
MUTANTS = [
 ('permanent', 'prim/leaf/buffer_locals.rs', 'if kill_permanent == Value::NIL && (property != Value::NIL || builtin_permanent) {', 'if false && kill_permanent == Value::NIL && (property != Value::NIL || builtin_permanent) {', 'buffer_local_substrate', 'kill_all_keeps_permanent'),
 ('flag', 'forwarded.rs', '*value = None;', '*value = Some(Value::integer(70).unwrap());', 'buffer_local_substrate', 'clear_one_removes_value'),
 ('global', 'prim/leaf/buffer_locals.rs', 'else { return Ok(false) };\n        Ok(self.vm.declaration_hash_get', 'else { return Ok(true) };\n        Ok(self.vm.declaration_hash_get', 'buffer_local_substrate', 'global_is_not_a_local'),
 ('buffer-read', 'prim/leaf/buffer_locals.rs', 'self.vm.slots[local.index(7).unwrap() - 1].value\n            } else', 'self.vm.slots[slot].value\n            } else', 'buffer_local_substrate', 'other_buffer_read'),
 ('inventory', 'prim/leaf/buffer_locals.rs', 'Ok(self.vm.list_value(&entries))\n    }', 'Ok(Value::NIL)\n    }', 'buffer_local_substrate', 'other_buffer_read'),
 ('ordinary-clear', 'prim/leaf/buffer_locals.rs', 'self.vm.declaration_hash_remove(table, buffer).map_err(Fault::Error)?;', 'let _ = (table, buffer);', 'buffer_local_substrate', 'kill_one_ignores_permanent'),
 ('hook-order', 'prims/buffer_locals.rs', 'let hook = cx.intern("change-major-mode-hook");', 'let hook = cx.intern("buffer-list-update-hook");', 'buffer_local_substrate', 'kill_all_permanent_argument'),
 ('mode-reset', 'forwarded.rs', 'self.per_buffer_mut(buffer).expect("buffer").clear_local(i);\n            self.forwarded_buffer_value(buffer, i);', 'self.forwarded_buffer_value(buffer, i);', 'buffer_local_substrate', 'mode_fields_are_inherently_local'),
 ('overlay-buffer', 'prim/leaf/overlays.rs', 'Ok(Object::Overlay(o)) => Ok(o.buffer),', 'Ok(Object::Overlay(_o)) => Ok(Value::NIL),', 'overlay_missing', 'missing_overlay'),
 ('overlay-delete', 'prim/leaf/overlays.rs', 'for overlay in overlays {', 'for overlay in overlays.into_iter().take(0) {', 'overlay_missing', 'missing_overlay'),
 ('overlay-previous', 'prim/leaf/overlays.rs', 'previous = previous.max(endpoint);', 'previous = previous;', 'overlay_missing', 'missing_overlay'),
 ('void-hook', 'prim/leaf/buffer_locals.rs', 'Err(Fault::VoidVariable(_)) => return Err(Fault::Wrong("arrayp", self.vm.forwarded_unbound_symbol())),', 'Err(Fault::VoidVariable(_)) => continue,', 'buffer_local_substrate', 'permanent_hook_void_condition'),
 ('builtin-identity', 'prim/leaf/buffer_locals.rs', 'let builtin_permanent = slot == self.vm.truncate_lines_slot', 'let builtin_permanent = self.vm.symbol_names[slot] == "truncate-lines"', 'buffer_local_substrate', 'builtin_permanence'),
 ('mode-identity', 'forwarded.rs', 'self.forwarded.fundamental_mode_name\n        } else {', 'self.materialize(Init::Str("Fundamental"))\n        } else {', 'buffer_local_substrate', 'fundamental_mode_name_identity'),
]
results = []
for name, relative, old, new, suite, test in MUTANTS:
    path = SRC / relative
    original = path.read_bytes()
    text = original.decode()
    if text.count(old) != 1:
        raise RuntimeError(f'{name}: mutation site is not unique')
    command = ['cargo', 'test', '-p', 'romacs-vm', '--jobs', '3', '--test', suite, test, '--', '--nocapture']
    try:
        path.write_text(text.replace(old, new))
        run = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=120)
        output = run.stdout.decode(errors='replace')
        (OUT / f'mutant-{name}.log').write_bytes(run.stdout)
        killed = run.returncode == 101 and 'panicked at' in output and 'test result: FAILED. 0 passed; 1 failed;' in output and 'error[E' not in output
        results.append({'name': name, 'command': command, 'exit': run.returncode, 'killed_at_behavioral_assertion': killed})
        print(name, 'KILLED' if killed else 'INVALID OR SURVIVED', flush=True)
        if not killed:
            raise RuntimeError(f'{name}: not a behavioral kill; see retained log')
    finally:
        path.write_bytes(original)
        if path.read_bytes() != original:
            raise RuntimeError(f'{name}: source restoration failed')
    (OUT / 'mutations.json').write_text(json.dumps(results, indent=2) + '\n')
print(f'{len(results)}/{len(MUTANTS)} mutants killed; all source bytes restored')
