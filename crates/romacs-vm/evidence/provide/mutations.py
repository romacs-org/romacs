#!/usr/bin/env python3
import json, os, subprocess
from pathlib import Path
root = Path(__file__).resolve().parents[4]
source = root / 'crates/romacs-vm/src/loader.rs'
original = source.read_text()
out = Path(__file__).parent
mutants = {
    'ignore-subfeatures': ('if subfeature != Value::NIL {\n                    let key', 'if false {\n                    let key'),
    'all-subfeatures-present': ('let found = present && (subfeature == Value::NIL', 'let found = present || (subfeature == Value::NIL'),
    'cycles-answer-nil': ('if tail == tortoise { return Err(Fault::Circular(tail)); }', 'if tail == tortoise { return Ok(false); }'),
    'improper-list-answers-nil': ('if tail != Value::NIL { return Err(Fault::Wrong("listp", list)); }', 'if tail != Value::NIL { return Ok(false); }'),
}
results = []
try:
    for name, (old, new) in mutants.items():
        assert original.count(old) == 1
        source.write_text(original.replace(old, new))
        run = subprocess.run(['cargo','test','-p','romacs-vm','--jobs','3','--test','provide_subfeatures','--','--nocapture'], cwd=root, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        (out / (name + '.log')).write_text(run.stdout)
        assertion = 'PROVIDE_SUBFEATURES' if name in {'ignore-subfeatures', 'all-subfeatures-present'} else 'PROVIDE_GNU'
        killed = run.returncode != 0 and ('assertion `left == right` failed: ' + assertion) in run.stdout
        results.append({'name': name, 'exit': run.returncode, 'named_assertion_killed': killed})
        assert killed, run.stdout
finally:
    source.write_text(original)
    (out/'mutations.json').write_text(json.dumps(results, indent=2)+'\n')
print(json.dumps(results))
