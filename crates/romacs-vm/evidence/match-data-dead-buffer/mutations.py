#!/usr/bin/env python3
"""Bounded source mutants; each must reach DEAD_BUFFER_MATCH_DATA."""
import hashlib
import json
import os
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[4]
source = root / 'crates/romacs-vm/src/annotation_primitives.rs'
out = Path(__file__).resolve().parent
original = source.read_text()
mutants = [
    ('signal-dead-buffer', 'if self.buffer_state(buffer).is_ok_and(|state| !state.live) {', 'if false {'),
    ('detached-live-position', 'None => Ok(Value::NIL),', 'None => Ok(Value::integer(1)?),'),
]
receipts = []
try:
    for name, old, new in mutants:
        assert original.count(old) == 1, name
        source.write_text(original.replace(old, new))
        command = ['cargo', 'test', '-p', 'romacs-vm', '--jobs', '3', '--test', 'match_data_dead_buffer', 'match_data_after_buffer_death_returns_detached_markers', '--', '--exact', '--nocapture']
        run = subprocess.run(command, cwd=root, capture_output=True, text=True)
        log = run.stdout + run.stderr
        (out / (name + '.log')).write_text(log)
        killed = run.returncode == 101 and 'assertion `left == right` failed: DEAD_BUFFER_MATCH_DATA' in log and '1 failed' in log
        receipts.append(dict(name=name, command=command, exit=run.returncode, killed_at_named_assertion=killed))
        assert killed, (name, log)
finally:
    source.write_text(original)
    (out / 'mutations.json').write_text(json.dumps({'restored_sha256': hashlib.sha256(source.read_bytes()).hexdigest(), 'mutants': receipts}, indent=2) + '\n')
print(json.dumps(receipts, indent=2))
