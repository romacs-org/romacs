#!/usr/bin/env python3
"""Replay G1a VM2 mutants. Run alone: temporarily edits only VM2 source."""
from pathlib import Path
import subprocess
import sys

crate = Path(__file__).resolve().parents[1]
source = crate / "src/display.rs"
original = source.read_text()
logs = Path(sys.argv[1])
logs.mkdir(parents=True, exist_ok=True)


def replace(text, old, new):
    assert text.count(old) == 1, old
    return text.replace(old, new)


extraction = '        let Object::Buffer(b) = self.object(self.slots[slot].value)? else {'
untraced = replace(original, '    theme: Theme,', '    theme: Theme,\n    raw: Option<Value>,')
untraced = replace(untraced, '            previous: None,', '            previous: None,\n            raw: None,')
untraced = replace(untraced, extraction,
                   '        let raw = *self.display_state.raw.get_or_insert(self.slots[slot].value);\n'
                   '        let Object::Buffer(b) = self.object(raw)? else {')
mutants = {
    "snapshot-stub": replace(original, extraction, '        return Err("snapshot stub".into());\n' + extraction),
    "untraced-window-handle": untraced,
    "wrong-colour": replace(original, 'foreground: color(Attribute::Foreground)?', 'foreground: [255, 255, 255]'),
    "frozen-version": replace(original, 'snapshot.buffer_version = self.display_state.version;', 'snapshot.buffer_version = 0;'),
}
try:
    for name, text in mutants.items():
        source.write_text(text)
        command = ['cargo', 'test', '--manifest-path', str(crate / 'Cargo.toml')]
        if name in ('wrong-colour', 'frozen-version'):
            command += ['--test', 'display', 'source_scene_snapshot_and_external_mutation']
        else:
            command += ['--lib', 'display::tests::snapshot_survives_source_string_and_property_compaction']
        run = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        (logs / (name + '.log')).write_text(run.stdout)
        assert run.returncode != 0 and 'test result: FAILED' in run.stdout, (name, run.returncode, run.stdout)
        if name == 'untraced-window-handle':
            assert 'target extraction after compaction' in run.stdout, run.stdout
        if name == 'snapshot-stub':
            low = subprocess.run(['cargo', 'test', '--manifest-path', str(crate / 'Cargo.toml'), '--test', 'display', 'source_scene_and_external_mutation_with_low_gc_floor'], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            (logs / 'low-floor-red.log').write_text(low.stdout)
            assert low.returncode != 0 and 'test result: FAILED' in low.stdout and 'low-floor snapshot' in low.stdout, low.stdout
        print(name, 'KILLED', run.returncode, flush=True)
finally:
    source.write_text(original)
