#!/usr/bin/env python3
"""Record GNU's answers for cases.json -> gnu.json (one GNU process, pure).

Every case is `wrapper % form`, evaluated after `prelude`, printed with
`princ` on its own line between markers. Run from anywhere:

    python3 crates/romacs-vm/evidence/hash-read-syntax/oracle.py [--gnu /usr/bin/emacs]
"""
import argparse, hashlib, json, os, subprocess, tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--gnu', default='/usr/bin/emacs')
    args = ap.parse_args()
    spec = json.loads((HERE / 'cases.json').read_text())
    with tempfile.TemporaryDirectory(dir=os.environ.get('TMPDIR')) as home:
        driver = Path(home) / 'driver.el'
        lines = [';;; -*- lexical-binding: t -*-', spec['prelude']]
        for cid, form in spec['cases']:
            body = spec['wrapper'] % form
            lines.append(f'(princ "\\n@@CASE {cid}\\n") (princ (condition-case e {body} (error (format "DRIVER-ERROR %S" e)))) (princ "\\n@@END\\n")')
        driver.write_text('\n'.join(lines) + '\n')
        env = {'PATH': '/usr/bin:/bin', 'HOME': home, 'LANG': 'C.UTF-8'}
        out = subprocess.run([args.gnu, '-Q', '--batch', '-l', str(driver)], capture_output=True,
                             text=True, env=env, timeout=120, check=True).stdout
        version = subprocess.run([args.gnu, '--version'], capture_output=True, text=True,
                                 env=env, check=True).stdout.splitlines()[0]
    answers = {}
    for chunk in out.split('\n@@CASE ')[1:]:
        cid, rest = chunk.split('\n', 1)
        answers[cid] = rest.rsplit('\n@@END', 1)[0]
    assert [c for c, _ in spec['cases']] == list(answers), 'every case answered, in order'
    record = {
        'protocol': 'hash-read-syntax-v1',
        'gnu': version,
        'gnu_binary_sha256': sha(args.gnu),
        'cases_sha256': sha(HERE / 'cases.json'),
        'oracle_sha256': sha(__file__),
        'regime': 'GNU interpreted (load of a .el driver, lexical-binding t)',
        'cases': [{'id': c, 'form': f, 'gnu': answers[c]} for c, f in spec['cases']],
    }
    pending = HERE / 'gnu.json.pending'
    pending.write_text(json.dumps(record, indent=1, ensure_ascii=False) + '\n')
    pending.rename(HERE / 'gnu.json')
    print(f"recorded {len(answers)} cases from {version}")


if __name__ == '__main__':
    main()
