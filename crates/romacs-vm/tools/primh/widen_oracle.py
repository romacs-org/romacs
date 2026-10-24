#!/usr/bin/env python3
"""Recorded GNU proof for symbol-name, concat and indirect-function.

record runs live GNU first, atomically recording both regimes, never a candidate.
replay uses recorded evidence only; --gnu /nonexistent is deliberately ignored.
Each case gets a private GNU process / fresh candidate process, because function
cell mutation must not leak across cases. Both GNU regimes use idempotent setup.

Examples:
  python tools/primh/widen_oracle.py record --gnu /usr/bin/emacs
  python tools/primh/widen_oracle.py replay --vm2 /path/romacs-vm2 --gnu /nonexistent
  python tools/primh/widen_oracle.py replay --vm2 /path/romacs-vm2 --report-only
  python tools/primh/widen_oracle.py replay --vm2 /path/romacs-vm2 --pin vector

Pins are opt-in, exact-outcome ratchets, ONLY for cases explicitly tagged with a
known absent substrate below. No generic DATA/CONDITION/missing primitive pins.
A changed outcome or repaired pin fails until the pin is explicitly refreshed.
Report-only prints all divergences but does not certify acceptance.
"""
import os as _portable_os
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
SCRATCH = Path(str(_PortablePath(_portable_tempfile.gettempdir()) / ''))
SOURCE = Path(str(_PortablePath(_portable_os.environ.get('ROMACS_GNU_SOURCE', _PortablePath(_portable_os.environ.get('GNU_ORACLE_ROOT', _PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'gnu-emacs-pin')) / 'src')) / 'src'))
CASES = HERE / 'widen-cases.json'
EVIDENCE = HERE / 'evidence/widen.json'
PROTOCOL = 'widen-gnu-offline-v1'
NAMES = ['symbol-name', 'concat', 'indirect-function']
ARITIES = [['symbol-name', 'subr', 1, 1], ['concat', 'subr', 0, 'many'],
           ['indirect-function', 'subr', 1, 2]]
LIMITATIONS = {
    'vector': 'Vector sequence traversal / vector reader substrate is not yet exposed to Leaf.',
    'text-properties': 'Property-bearing string storage and property inspection are not implemented.',
    'unibyte': 'Distinct unibyte strings / Emacs raw-byte characters are not implemented.',
    'extended-character': 'GNU non-Unicode character codes exceed the Unicode-only string substrate.',
}
WRAP = "(prin1-to-string (condition-case e (list 'ok {expr}) (error (list 'sig e))))"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def atomic(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, pending = tempfile.mkstemp(prefix=path.name + '.', suffix='.pending', dir=path.parent)
    try:
        with os.fdopen(fd, 'w', encoding='utf-8') as f:
            json.dump(data, f, ensure_ascii=False, indent=2)
            f.write('\n')
            f.flush()
            os.fsync(f.fileno())
        os.replace(pending, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(pending):
            os.unlink(pending)


def runtime_concat(expr):
    """Keep GNU byte-optimize-concat from folding literal faults at compile time.

    Global SETQ operands are observable and cannot be propagated as constants.
    The measured operation remains a direct compiled CONCAT call, not advice or
    a replacement implementation. This tiny tokenizer covers generated forms.
    """
    if not expr.startswith('(concat '):
        return expr
    text, args, start, depth, string, escape = expr[8:-1], [], 0, 0, False, False
    for i, char in enumerate(text + ' '):
        if escape:
            escape = False
        elif char == '\\':
            escape = True
        elif char == '"':
            string = not string
        elif not string:
            if char in '([':
                depth += 1
            elif char in ')]':
                depth -= 1
            elif char.isspace() and depth == 0:
                if text[start:i].strip():
                    args.append(text[start:i].strip())
                start = i + 1
    if depth or string:
        raise ValueError('generated concat form is unbalanced')
    bindings = ' '.join(f'(setq widen-arg-{i} {arg})' for i, arg in enumerate(args))
    callargs = ' '.join(f'widen-arg-{i}' for i in range(len(args)))
    return f'(progn {bindings} (concat {callargs}))'


def generate() -> dict:
    rows = []

    def add(primitive, label, expr, limitation=None):
        measured = runtime_concat(expr) if primitive == 'concat' else expr
        row = dict(id=f'{primitive}/{label}', primitive=primitive, expression=expr,
                   source=WRAP.format(expr=measured))
        if limitation in ('unibyte', 'extended-character'):
            # GNU's printer can emit extended UTF-8 that Python/JSON cannot
            # represent. Ask the Lisp printer for lossless ASCII escapes, not
            # replacement decoding or dropped raw bytes.
            row['source'] = '(let ((print-escape-nonascii t) (print-escape-multibyte t)) ' + row['source'] + ')'
        if limitation:
            row['limitation'] = limitation
        rows.append(row)

    for label, value in [('nil', 'nil'), ('t', 't'), ('ordinary', "'widen-name"),
                         ('keyword', ':widen'), ('unicode', "'λ雪😀"),
                         ('empty-name', "'||"), ('escaped-name', "'a\\ b"),
                         ('integer', '7'), ('negative', '-1'), ('float', '1.5'),
                         ('string', '"hello"'), ('empty-string', '""'),
                         ('list', "'(a b)"), ('dotted', "'(a . b)"),
                         ('vector', '[65 66]')]:
        # GNU || means a symbol named ||, not an empty symbol. Use intern for empty.
        if label == 'empty-name':
            value = '(intern "")'
        add('symbol-name', label, f'(symbol-name {value})', 'vector' if label == 'vector' else None)
    for argc in [0, 2, 4]:
        add('symbol-name', f'arity-{argc}', "(apply #'symbol-name '(" + ' '.join(['1'] * argc) + '))')

    simple = [('zero', '(concat)'), ('empty', '(concat "")'), ('nil', '(concat nil)'),
              ('nils', '(concat nil nil nil)'), ('one', '(concat "abc")'),
              ('unicode', '(concat "λ" "雪" "😀")'),
              ('several', '(concat "a" "" "bc" "d")'),
              ('mixed-nil', '(concat nil "a" nil "β" nil)'),
              ('many', '(concat ' + ' '.join('"%s"' % (i % 10) for i in range(128)) + ')'),
              ('list-empty', "(concat '())"), ('list-ascii', "(concat '(65 0 66))"),
              ('list-unicode', "(concat '(955 38634 128512))"),
              ('list-mixed', "(concat '(65) \"β\" nil '(67 68))"),
              ('dotted', "(concat '(65 . 66))"),
              ('dotted-bad-first', "(concat '(bad . 66))"),
              ('dotted-tail', "(concat '(65 66 . bad))"),
              ('error-order', "(concat '(65 bad) 7)"),
              ('late-error', '(concat "ok" nil 7)'),
              # Lossless finite witness for this one-cell cyclic payload: retain
              # condition, car, input identity, self-edge and all remaining data.
              # Neither runtime's cyclic printer is part of concat's contract.
              ('cycle', '(let ((x (list 65))) (setcdr x x) (condition-case e (concat x) (error (let ((payload (car (cdr e)))) (list (car e) (car payload) (eq payload x) (eq (cdr payload) payload) (cdr (cdr e)))))))')]
    for label, expr in simple:
        add('concat', label, expr)
    for label, value in [('integer', '65'), ('float', '1.5'), ('symbol', "'bad"), ('t', 't')]:
        add('concat', 'bad-sequence-' + label, '(concat ' + value + ')')
    for label, value in [('negative', '-1'), ('too-large', '4194304'), ('float', '1.5'),
                         ('symbol', 'bad'), ('nil', 'nil'), ('t', 't'), ('string', '"a"'),
                         ('cons', '(65)'), ('unicode-max', '1114111')]:
        add('concat', 'character-' + label, "(concat '(" + value + '))')
    add('concat', 'character-beyond-unicode', "(concat '(1114112))", 'extended-character')
    for label, expr in [('empty', '(concat [])'), ('ascii', '(concat [65 66])'),
                        ('unicode', '(concat [955 38634 128512])'),
                        ('mixed', '(concat "a" [66] nil \'(67))'),
                        ('negative', '(concat [-1])'), ('bad', '(concat [65 bad])'),
                        ('dotted-precedence', '(concat [bad] 7)')]:
        add('concat', 'vector-' + label, expr, 'vector')
    for label, expr in [('raw', '(concat (unibyte-string 128 255))'),
                        ('mixed', '(concat (unibyte-string 255) "λ")'),
                        ('kind', '(multibyte-string-p (concat (unibyte-string 65)))')]:
        add('concat', 'unibyte-' + label, expr, 'unibyte')
    add('concat', 'properties-reader', '(concat #("ab" 0 1 (face bold)) "c")', 'text-properties')
    add('concat', 'properties-inspect', '(get-text-property 0 \'face (concat (propertize "a" \'face \'bold) "b"))', 'text-properties')

    objects = [('nil', 'nil'), ('t', 't'), ('unbound', "'widen-unbound"),
               ('integer', '7'), ('negative', '-2'), ('float', '1.5'),
               ('string', '"λ雪"'), ('list', "'(lambda (x) x)"),
               ('dotted', "'(a . b)"), ('vector', '[65 66]')]
    for label, obj in objects:
        for opt_label, opt in [('omitted', ''), ('nil', ' nil'), ('t', ' t'),
                               ('arbitrary', " '(anything . 9)")]:
            add('indirect-function', f'{label}-{opt_label}', '(indirect-function ' + obj + opt + ')',
                'vector' if label == 'vector' else None)
    # Every setup overwrites every cell used: the driver's compiled and interpreted
    # evaluations therefore begin in the same logical world without advice.
    for label, terminal in [('nil', 'nil'), ('integer', '42'), ('string', '"end"'),
                             ('list', "'(lambda (x) x)"), ('unbound', "'widen-z"),
                             ('t', 't')]:
        setup = "(fset 'widen-z nil) (fset 'widen-b " + terminal + ") (fset 'widen-a 'widen-b)"
        for opt_label, opt in [('omitted', ''), ('nil', ' nil'), ('t', ' t'), ('arbitrary', ' 17')]:
            add('indirect-function', f'chain-{label}-{opt_label}',
                '(progn ' + setup + " (indirect-function 'widen-a" + opt + '))')
    add('indirect-function', 'identity', '(let ((x (list 1 2))) (eq x (indirect-function x)))')
    add('indirect-function', 'self-cycle-fset',
        "(progn (fset 'widen-a nil) (fset 'widen-a 'widen-a) (indirect-function 'widen-a))")
    add('indirect-function', 'two-cycle-fset',
        "(progn (fset 'widen-a nil) (fset 'widen-b nil) (fset 'widen-a 'widen-b) (fset 'widen-b 'widen-a) (indirect-function 'widen-a))")
    for argc in [0, 3, 5]:
        add('indirect-function', f'arity-{argc}', "(apply #'indirect-function '(" + ' '.join(['1'] * argc) + '))')
    assert len({r['id'] for r in rows}) == len(rows)
    return dict(protocol=PROTOCOL, limitations=LIMITATIONS, cases=rows)


def identities():
    return dict(generator_sha256=sha(Path(__file__).read_bytes()),
                driver_sha256=sha((HERE / 'gnu_driver.el').read_bytes()),
                reusable_harness_sha256=sha((HERE / 'primh.py').read_bytes()),
                cases_sha256=sha(CASES.read_bytes()),
                gnu_sources={name: sha((SOURCE / name).read_bytes()) for name in ['data.c', 'fns.c', 'lread.c']})


def validate_rows(rows, cases):
    if not isinstance(rows, list) or len(rows) != len(cases):
        raise ValueError('reference count mismatch')
    if [r.get('id') for r in rows] != [r['id'] for r in cases]:
        raise ValueError('reference ID/order mismatch')
    for row, case in zip(rows, cases):
        if {k: row.get(k) for k in case} != case:
            raise ValueError('reference case content mismatch: ' + case['id'])
        for key in ['expected', 'interp']:
            if not isinstance(row.get(key), str) or row[key].startswith('HARNESS-ERROR'):
                raise ValueError('reference missing/invalid regime: ' + case['id'])


def load():
    spec = json.loads(CASES.read_text())
    if spec != generate():
        raise ValueError('cases do not match deterministic generation')
    ev = json.loads(EVIDENCE.read_text())
    if ev.get('protocol') != PROTOCOL:
        raise ValueError('wrong evidence protocol')
    for key, value in identities().items():
        if ev.get(key) != value:
            raise ValueError('evidence identity changed: ' + key)
    validate_rows(ev.get('cases'), spec['cases'])
    if ev.get('count') != len(spec['cases']) or ev.get('regime_count') != 2 * len(spec['cases']):
        raise ValueError('metadata count mismatch')
    if ev.get('gnu', {}).get('subrs') != ARITIES:
        raise ValueError('GNU arity/identity mismatch')
    if ev.get('limitations') != LIMITATIONS or ev.get('regimes') != ['GNU byte-compiled', 'GNU interpreted']:
        raise ValueError('evidence limitation/regime metadata mismatch')
    gnu_hash = ev.get('gnu_binary_sha256')
    if not isinstance(gnu_hash, str) or len(gnu_hash) != 64 or any(c not in '0123456789abcdef' for c in gnu_hash):
        raise ValueError('GNU binary hash missing/invalid')
    pins = ev.get('pins', {})
    known = {r['id']: r for r in spec['cases']}
    for key, pin in pins.items():
        if key not in known or known[key].get('limitation') not in LIMITATIONS:
            raise ValueError('pin outside documented limitations: ' + key)
        if pin.get('limitation') != known[key]['limitation'] or not isinstance(pin.get('actual'), str):
            raise ValueError('invalid exact-outcome pin: ' + key)
    return ev


def private_env(root):
    home, tmp = Path(root) / 'home', Path(root) / 'tmp'
    home.mkdir(mode=0o700)
    tmp.mkdir(mode=0o700)
    return dict(os.environ, HOME=str(home), TMPDIR=str(tmp), XDG_CONFIG_HOME=str(home / '.config'),
                XDG_CACHE_HOME=str(home / '.cache'), XDG_DATA_HOME=str(home / '.local/share'),
                LC_ALL='C.UTF-8')


def record(args):
    if EVIDENCE.exists() and not args.replace:
        raise ValueError('evidence exists; use record --replace explicitly')
    spec = generate()
    atomic(CASES, spec)
    inputs = identities()
    executable = Path(shutil.which(args.gnu) or args.gnu).resolve(strict=True)
    binary_hash = sha(executable.read_bytes())
    started = time.monotonic()
    rows, header = [], None
    for case in spec['cases']:
        with tempfile.TemporaryDirectory(prefix='widen-gnu-', dir=SCRATCH) as root:
            env = private_env(root)
            casefile = Path(root) / 'case.json'
            casefile.write_text(json.dumps([{'id': case['id'], 'source': case['source']}]), encoding='utf-8')
            env.update(PRIMH_CASES=str(casefile), PRIMH_NAMES=','.join(NAMES))
            p = subprocess.run([str(executable), '-Q', '--batch', '-l', str(HERE / 'gnu_driver.el')],
                               env=env, capture_output=True, text=True, timeout=args.timeout, check=True)
        replies = [json.loads(line) for line in p.stderr.splitlines() if line.startswith('{')]
        if len(replies) != 2 or replies[0].get('subrs') != ARITIES:
            raise ValueError('GNU header/count mismatch: ' + case['id'])
        if p.stdout.strip():
            raise ValueError('unexpected GNU stdout: ' + case['id'])
        if header is not None and header != replies[0]:
            raise ValueError('GNU header changed between cases')
        header = replies[0]
        row = dict(case, **replies[1])
        validate_rows([row], [case])
        rows.append(row)
    validate_rows(rows, spec['cases'])
    if sha(executable.read_bytes()) != binary_hash:
        raise ValueError('GNU executable changed during recording')
    if inputs != identities():
        raise ValueError('oracle inputs changed during recording')
    ev = dict(protocol=PROTOCOL, **inputs, gnu_binary=str(executable),
              gnu_binary_sha256=binary_hash, gnu=header,
              source_locations={'symbol-name': 'data.c:826', 'indirect-function': 'data.c:2557', 'concat': 'fns.c:737'},
              count=len(rows), regime_count=2 * len(rows),
              isolation='one GNU process per case; idempotent fset setup between compiled/interpreted evaluation; private HOME/TMPDIR under scratch',
              regimes=['GNU byte-compiled', 'GNU interpreted'],
              comparison='exact full printed value or complete condition/data; candidate compiled vs GNU byte-compiled (interpreted retained, never silently substituted)',
              replay='one fresh candidate process per case, --gc-floor 1 --eval-stream, one source request per process; no GNU execution',
              limitations=LIMITATIONS, cases=rows, pins={})
    atomic(EVIDENCE, ev)
    load()
    print(json.dumps(dict(action='record', cases=len(rows), regimes=2 * len(rows),
                          by_primitive=dict(Counter(r['primitive'] for r in rows)),
                          gnu_regime_differences=[r['id'] for r in rows if r['expected'] != r['interp']],
                          gnu=header, seconds=round(time.monotonic() - started, 2), evidence=str(EVIDENCE)), ensure_ascii=False))


def replay(args):
    ev = load()  # No args.gnu lookup, stat, access, or subprocess call here.
    executable = Path(args.vm2).resolve(strict=True)
    binary_hash = sha(executable.read_bytes())
    counts, failures, details, pending = Counter(), [], [], dict(ev.get('pins', {}))
    regime_matches = Counter()
    for row in ev['cases']:
        with tempfile.TemporaryDirectory(prefix='widen-vm-', dir=SCRATCH) as root:
            p = subprocess.run([str(executable), '--gc-floor', '1', '--eval-stream'],
                               input=json.dumps({'source': row['source']}, ensure_ascii=False) + '\n',
                               env=private_env(root), capture_output=True, text=True, timeout=args.timeout,
                               check=True)
        lines = p.stdout.splitlines()
        if len(lines) != 1:
            raise ValueError('candidate reply count mismatch: ' + row['id'])
        out = json.loads(lines[0])
        result = out.get('result') or {}
        actual = result.get('value') if result.get('type') == 'string' else 'VM-ERROR ' + json.dumps(out, sort_keys=True, ensure_ascii=False)
        if not isinstance(actual, str):
            raise ValueError('candidate result not a string: ' + row['id'])
        pin = pending.get(row['id'])
        limitation = row.get('limitation')
        for regime, key in [('GNU byte-compiled', 'expected'), ('GNU interpreted', 'interp')]:
            regime_matches[regime] += int(actual == row[key])
        if actual == row['expected'] and actual == row['interp']:
            status = 'STALE-PIN' if pin else 'PASS'
        elif pin:
            status = 'PINNED-PARTIAL' if pin['actual'] == actual else 'CHANGED-PIN'
        elif limitation in args.pin:
            pending[row['id']] = dict(limitation=limitation, actual=actual,
                                      reason=LIMITATIONS[limitation], candidate_sha256=binary_hash)
            status = 'PINNED-PARTIAL'
        else:
            status = 'PARTIAL-UNPINNED' if limitation else 'FAIL'
        counts[status] += 1
        if status not in ('PASS', 'PINNED-PARTIAL'):
            failures.append(row['id'])
        if status != 'PASS':
            detail = dict(id=row['id'], status=status, expected=row['expected'], interp=row['interp'], actual=actual)
            if limitation:
                detail['limitation'] = limitation
            details.append(detail)
    if sum(counts.values()) != ev['count']:
        raise ValueError('candidate total mismatch')
    if sha(executable.read_bytes()) != binary_hash:
        raise ValueError('candidate changed during replay')
    if args.pin:
        # Persist only deliberate class-limited exact observations; core failures
        # remain failures. A report is not a license to ratchet arbitrary faults.
        ev['pins'] = pending
        atomic(EVIDENCE, ev)
        load()
    print(json.dumps(dict(action='replay', cases=ev['count'], counts=dict(counts),
                          accepted=not failures, report_only=args.report_only,
                          exact_matches_by_reference_regime=dict(regime_matches),
                          candidate_regime='compiled',
                          candidate_sha256=binary_hash, gnu_invoked=False,
                          details=details), ensure_ascii=False, indent=2))
    return 0 if args.report_only or not failures else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='action', required=True)
    r = sub.add_parser('record')
    r.add_argument('--gnu', default='/usr/bin/emacs')
    r.add_argument('--replace', action='store_true')
    r.add_argument('--timeout', type=int, default=30)
    p = sub.add_parser('replay')
    p.add_argument('--vm2', required=True)
    p.add_argument('--gnu', default='/nonexistent', help='ignored; documents offline replay')
    p.add_argument('--timeout', type=int, default=30)
    p.add_argument('--pin', choices=sorted(LIMITATIONS), action='append', default=[])
    p.add_argument('--report-only', action='store_true')
    sub.add_parser('verify')
    args = parser.parse_args()
    if args.action == 'record':
        record(args)
        return 0
    if args.action == 'verify':
        ev = load()
        print(json.dumps(dict(action='verify', cases=ev['count'], regimes=ev['regime_count'], pins=len(ev['pins']))))
        return 0
    return replay(args)


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print('widen oracle error: ' + str(error), file=sys.stderr)
        sys.exit(2)
