#!/usr/bin/env python3
"""Fresh-process GNU reader oracle. --replay never resolves or runs GNU.

Record: --vm2 BIN --gnu /usr/bin/emacs --gnu-root ../gnu-emacs-ref --record FILE
Replay: --vm2 BIN --replay FILE
GNU source implementation: src/lread.c read_char_escape/read_string_literal;
src/eval.c Ffunction. Fixtures store actual GNU results, not inferred values.
"""
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile


def cases():
    sources = []
    # All modifier families, orderings, repeats and ASCII control boundaries.
    bases = ['@', 'A', 'Z', '[', '\\', ']', '^', '_', '`', 'a', 'z', '?', '1', ' ', 'λ']
    prefixes = ['C-', '^', 'M-', 's-', 'H-', 'S-', 'A-', 'C-\\C-', 'M-\\C-', 'C-\\M-', 'M-\\S-\\A-\\H-\\s-\\C-']
    for prefix in prefixes:
        for base in bases:
            # A literal backslash base must itself be escaped.
            sources.append('?\\' + prefix + ('\\\\' if base == '\\' else base))
    for base in ['000', '001', '037', '077', '100', '177', '200', '377', '400', 'x80', 'x080', 'x4000001', 'x00000000000078', 'u0080', 'u03bb']:
        sources.extend(['?\\' + base, '?\\C-\\' + base, '?\\M-\\' + base])
    for literal in [r'"\C-a"', r'"\C- "', r'"\^ "', r'"\S-a"', r'"\S-Z"', r'"\M-x"', r'"\M-\C-a"', r'"\200"', r'"\x80"', r'"\u0080"', r'"\s-x"']:
        sources.append('(aref ' + literal + ' 0)')
    # Backquote data shape, nesting, evaluation order and non-destructive splicing.
    sources.extend([
        "`(a ,(+ 1 1) ,@(list 3 4) . z)",
        "`(a ,@1)",
        "(let ((x '(2 3))) `(1 . ,x))",
        "(let ((x 7)) `(a `(b ,x ,,x)))",
        "`[a ,(+ 1 1) ,@(list 3 4)]",
        "(let ((x '(2 3))) (list `(1 ,@x 4) x))",
        "(let ((x 0)) `(,(setq x (1+ x)) ,@(list (setq x (1+ x))) ,(setq x (1+ x))))",
        "(progn (defun oracle-f (x) (declare (pure t)) (+ x 1)) (oracle-f 4))",
        "(progn (defmacro lambda (&rest cdr) (list 'function (cons 'lambda cdr))) (funcall (lambda (x) (+ x 1)) 4))",
    ])
    return list(dict.fromkeys(sources))


def run_gnu(binary, source, env):
    # Parse the exact source string at runtime; never splice it into the wrapper.
    shape = """(defun reader-oracle-shape (x)
      (cond ((null x) '((type . "nil")))
            ((eq x t) '((type . "boolean") (value . t)))
            ((integerp x) (list '(type . "integer") (cons 'value x)))
            ((symbolp x) (list '(type . "symbol") (cons 'value (symbol-name x))))
            ((consp x) (list '(type . "cons") (cons 'car (reader-oracle-shape (car x))) (cons 'cdr (reader-oracle-shape (cdr x)))))
            ((vectorp x) (list '(type . "vector") (cons 'value (vconcat (mapcar #'reader-oracle-shape x)))))
            (t (error "unsupported oracle value"))))"""
    wrapper = '(progn (require (quote json)) ' + shape + ' (princ (json-encode (reader-oracle-shape (eval (car (read-from-string ' + json.dumps(source, ensure_ascii=False) + ')) t)))))'
    p = subprocess.run([binary, '-Q', '--batch', '--eval', wrapper], env=env, text=True, capture_output=True, timeout=20)
    if p.returncode:
        raise RuntimeError(f'GNU failed for {source}: {p.stderr}')
    return json.loads(p.stdout)


def run_vm(binary, source, env):
    p = subprocess.run([binary, '--gc-floor', '1', '--eval-stream'], input=json.dumps({'source': source}) + '\n', env=env, text=True, capture_output=True, timeout=20)
    if p.returncode:
        raise RuntimeError(f'VM2 exited {p.returncode} for {source}: {p.stderr}')
    response = json.loads(p.stdout)
    return response.get('result'), response.get('error')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--vm2', required=True)
    parser.add_argument('--gnu', default='/usr/bin/emacs')
    parser.add_argument('--gnu-root', type=Path)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--record', type=Path)
    mode.add_argument('--replay', type=Path)
    args = parser.parse_args()
    root = os.environ.get('TMPDIR', str(_PortablePath(_portable_tempfile.gettempdir()) / ''))
    with tempfile.TemporaryDirectory(prefix='reader-oracle-', dir=root) as home:
        env = dict(os.environ, HOME=home, XDG_CONFIG_HOME=home, XDG_CACHE_HOME=home)
        if args.record:
            if not args.gnu_root:
                parser.error('--record requires --gnu-root')
            revision = subprocess.check_output(['git', '-C', str(args.gnu_root), 'rev-parse', 'HEAD'], text=True).strip()
            lread = args.gnu_root / 'src/lread.c'
            fixture = {'protocol': 'reader-integers-and-forms-v1', 'gnu_revision': revision, 'lread_sha256': hashlib.sha256(lread.read_bytes()).hexdigest(), 'rows': [{'source': source, 'expected': run_gnu(args.gnu, source, env)} for source in cases()]}
            args.record.parent.mkdir(parents=True, exist_ok=True)
            args.record.write_text(json.dumps(fixture, indent=2, ensure_ascii=False) + '\n')
        else:
            fixture = json.loads(args.replay.read_text())
        if fixture['protocol'] != 'reader-integers-and-forms-v1':
            raise RuntimeError('unknown oracle protocol')
        failures = []
        for row in fixture['rows']:
            value, error = run_vm(args.vm2, row['source'], env)
            if error is not None or value != row['expected']:
                failures.append(dict(row, actual=value, error=error))
        print(json.dumps({'cases': len(fixture['rows']), 'passed': len(fixture['rows']) - len(failures), 'failures': failures}, ensure_ascii=False, indent=2))
        return bool(failures)


if __name__ == '__main__':
    raise SystemExit(main())
