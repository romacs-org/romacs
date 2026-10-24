import os as _portable_os
import hashlib, json, os, subprocess
from pathlib import Path
root = Path(_portable_os.environ['ROMACS_MUTATION_ROOT'])
out = Path(os.environ['TMPDIR']) / 'mdtrans-mutations'
out.mkdir(exist_ok=True)
rows = []
mutations = [
 ('no_translation', 'src/regexp.rs', 'if self.regexp.succeeded {', 'if false {', 'match_data_translate', 'translate_moves_all', 'TRANSLATE_REGISTERS'),
 ('no_end_translation', 'src/regexp.rs', '*end = (*end as i64).wrapping_add(delta).max(0) as usize;', '*end = (*end as i64).max(0) as usize;', 'match_data_translate', 'translate_moves_all', 'TRANSLATE_REGISTERS'),
 ('no_clamp', 'src/regexp.rs', '*start = (*start as i64).wrapping_add(delta).max(0) as usize;', '*start = (*start as i64).wrapping_add(delta) as usize;', 'match_data_translate', 'translate_moves_all', 'TRANSLATE_REGISTERS'),
 ('no_type_check', 'src/prims/search.rs', 'n.integer_value().ok_or(Fault::Wrong("fixnump", n))?', 'n.integer_value().unwrap_or(0)', 'match_data_translate', 'translate_checks_fixnum', 'TRANSLATE_FIXNUM'),
 ('lose_buffer_provenance', 'src/regexp.rs', 'pub(crate) fn regexp_translate_match_data(&mut self, delta: i64) {', 'pub(crate) fn regexp_translate_match_data(&mut self, delta: i64) {\n        let slot = self.names["#:vm2-match-buffer"];\n        self.slots[slot].value = Value::NIL;', 'match_data_translate', 'translate_preserves_buffer', 'TRANSLATE_BUFFER_PROVENANCE'),
 ('replay_no_translation', 'src/regexp.rs', 'if self.regexp.succeeded {', 'if false {', 'primh_match_data_translate', 'primh_match_data__translate', 'NEW DIVERGENCE (VALUE)'),
 ('frontier_wrong_line', 'tests/custom_bootstrap.rs', 'line 494 form 50:', 'line 495 form 50:', 'custom_bootstrap', 'custom_loads_and_files', 'CUSTOM_FILES_FRONTIER_MOVED'),
]
for name, relative, before, after, target, test, marker in mutations:
    path = root / 'crates/romacs-vm' / relative
    original = path.read_text()
    assert original.count(before) == 1, name
    try:
        path.write_text(original.replace(before, after))
        log = out / (name + '.log')
        with log.open('w') as stream:
            proc = subprocess.run(['cargo','test','-p','romacs-vm','--test',target,test,'--','--nocapture'], cwd=root, stdout=stream, stderr=subprocess.STDOUT)
        text = log.read_text()
        killed = proc.returncode == 101 and marker in text and 'test result: FAILED.' in text and 'could not compile' not in text
        rows.append(dict(name=name, exit=proc.returncode, marker=marker, killed=killed, log=str(log)))
        print(json.dumps(rows[-1]), flush=True)
        assert killed, name
    finally:
        path.write_text(original)
        assert hashlib.sha256(path.read_bytes()).hexdigest() == hashlib.sha256(original.encode()).hexdigest()
(out / 'receipt.json').write_text(json.dumps(rows, indent=2)+'\n')
