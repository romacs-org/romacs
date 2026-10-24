#!/usr/bin/env python3
import os, subprocess
from pathlib import Path
root = Path(__file__).resolve().parents[4]
p = root/'crates/romacs-vm/src/annotation_primitives.rs'
old = p.read_text()
needle = 'if self.buffer_state(buffer).is_ok_and(|state| !state.live) {'
assert old.count(needle) == 1
try:
    p.write_text(old.replace(needle, 'if false {'))
    env = dict(os.environ, RUST_MIN_STACK='33554432')
    outputs = []
    for test, assertion in [('minor_mode_dead_buffer', 'MINOR_MODE_DETACHED_MATCH_DATA'), ('match_data_dead_buffer', 'DEAD_BUFFER_MATCH_DATA')]:
        r = subprocess.run(['cargo','test','-p','romacs-vm','--jobs','3','--test',test,'--','--nocapture'], cwd=root, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        outputs.append(r.stdout)
        assert r.returncode != 0 and ('assertion `left == right` failed: ' + assertion) in r.stdout
        print('dead-buffer guard removed:', test, 'named assertion failed, exit',r.returncode)
    (Path(__file__).parent/'minor-mode-red.log').write_text('\n'.join(outputs))
finally:
    p.write_text(old)
