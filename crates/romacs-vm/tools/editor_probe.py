#!/usr/bin/env python3
"""Probe editor reachability without inventing buffer primitives or timing failures."""
import argparse
import json
from pathlib import Path
import subprocess
from bench import ROOT, GNU, SOURCE, load, read_form, gnu_command


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', default=str(ROOT / 'target/release/romacs-vm'))
    parser.add_argument('--output', type=Path, default=ROOT / 'evidence/s2-editor.json')
    args = parser.parse_args()
    text = ROOT / 'src/lib.rs'
    walking = f'''(with-temp-buffer (insert-file-contents {json.dumps(str(text))})
      (let ((i (point-min)) (sum 0))
        (while (< i (point-max))
          (setq sum (+ sum (char-after i)) i (1+ i))) sum))'''
    insertion = next(body for name, _, body in load(SOURCE, 'original_cases').CASES if name == 'buf-insert')
    sources = [('real-file-char-walk', walking)]
    sources += [(f'exact-buf-insert-{n}', insertion.replace('N', str(n))) for n in (1, 2)]
    sources += [('buffer-size', '(buffer-size)'), ('char-after', '(char-after 1)'),
                ('point-min', '(point-min)'), ('insert', '(insert "abc")')]
    rows = []
    for name, source in sources:
        expected = subprocess.check_output(gnu_command(source), text=True).strip()
        data = read_form(source)
        vm = subprocess.run([args.binary], input=data, capture_output=True)
        ir = subprocess.run([args.binary, '--dump-ir'], input=data, capture_output=True)
        row = dict(name=name, source=source, transport=json.loads(data), expected=expected,
                   actual=vm.stdout.decode().strip(), exit=vm.returncode, error=vm.stderr.decode(),
                   ir=ir.stdout.decode(), ir_ok=ir.returncode == 0,
                   supported=vm.returncode == 0 and vm.stdout.decode().strip() == expected)
        rows.append(row)
        print(f'{name}: GNU={expected} VM2 exit={vm.returncode} {row["error"].strip()}')
    report = dict(text_file=str(text), text_bytes=text.stat().st_size,
                  measured=False, reason='No host-buffer state, host primitive registration or editor VM2 dependency exists.',
                  cases=rows)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    raise SystemExit(0 if rows[0]['supported'] else 2)

if __name__ == '__main__':
    main()
