#!/usr/bin/env python3
"""Extract literal Lisp from inherited test crates; no old runtime is used.
This counts Lisp-looking Rust string literals, not generated expressions or test
cases. Expected-output strings beginning with a quote are excluded. Rust format!
placeholders are retained, never guessed/substituted. Every rejection is saved.
"""
import argparse
import collections
import hashlib
import json
from pathlib import Path
import re
import subprocess

TOKEN = re.compile(r'//[^\n]*|/\*.*?\*/|r(\#*)".*?"\1|"(?:\\.|[^"\\])*"', re.S)

def decode(token):
    if token.startswith('r'):
        hashes = token.index('"') - 1
        return token[hashes + 2:-(hashes + 1)]
    body = token[1:-1]
    def escape(m):
        s = m.group(0)[1:]
        if s.startswith('u{'):
            return chr(int(s[2:-1].replace('_', ''), 16))
        if s.startswith('x'):
            return chr(int(s[1:], 16))
        if s[0] in '\n\r':
            return ''
        return {'n': '\n', 'r': '\r', 't': '\t', '0': '\0', '\\': '\\', '"': '"', "'": "'"}[s]
    return re.sub(r'\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|\r?\n\s*|.)', escape, body)

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[3]
    version = subprocess.check_output(['/usr/bin/emacs', '-Q', '--batch', '--eval', '(princ emacs-version)'], text=True)
    assert version == '32.0.50', version
    rows = []
    for crate in ['romacs-oracle-tests', 'romacs-melpa-tests']:
        for path in sorted((root / 'crates' / crate).rglob('*')):
            if path.suffix == '.el':
                rows.append({'crate': crate, 'file': str(path.relative_to(root)), 'line': 1, 'source': path.read_text()})
            elif path.suffix == '.rs':
                text = path.read_text()
                for match in TOKEN.finditer(text):
                    token = match.group()
                    if token.startswith('//') or token.startswith('/*'):
                        continue
                    source = decode(token)
                    if source.lstrip().startswith(('(', '[', "'", '`', ';')):
                        rows.append({'crate': crate, 'file': str(path.relative_to(root)), 'line': text.count('\n', 0, match.start()) + 1, 'source': source})
    process = subprocess.run([args.binary], input=''.join(json.dumps(r['source']) + '\n' for r in rows), text=True, capture_output=True, check=True)
    replies = [json.loads(line) for line in process.stdout.splitlines()]
    assert len(replies) == len(rows), (len(replies), len(rows))
    for row, reply in zip(rows, replies):
        row.update(reply)
        row['sha256'] = hashlib.sha256(row['source'].encode()).hexdigest()
    # Distinguish intentionally malformed test inputs/templates from actual
    # supported GNU syntax. This is read-only: no corpus expression is evaluated.
    import os
    oracle_input = Path(args.output).with_suffix('.sources.json')
    oracle_input.write_text(json.dumps([r['source'] for r in rows]))
    env = dict(os.environ, VM2_CORPUS_INPUT=str(oracle_input))
    oracle = subprocess.run(['/usr/bin/emacs', '-Q', '--batch', '-l', str(Path(__file__).with_name('source_corpus_oracle.el'))], env=env, text=True, capture_output=True, check=True)
    gnu = json.loads(oracle.stdout)
    assert len(gnu) == len(rows)
    for row, (forms, valid, error) in zip(rows, gnu):
        row.update(gnu_forms=forms, gnu_valid=valid, gnu_error=error)
    summaries = {}
    for crate in ['romacs-oracle-tests', 'romacs-melpa-tests']:
        own = [r for r in rows if r['crate'] == crate]
        valid = [r for r in own if r['gnu_valid']]
        supported = [r for r in valid if r['error'] is None]
        blockers = [r for r in valid if r['error']]
        first = dict(blockers[0]) if blockers else None
        if first:
            first['source'] = first['source'][:300]
        summaries[crate] = {
            'literal_sources': len(own), 'gnu_valid_sources': len(valid),
            'gnu_valid_fully_accepted': len(supported),
            'gnu_valid_accepted_top_level_forms': sum(r['forms'] for r in supported),
            'gnu_valid_rejected': len(blockers),
            'gnu_invalid_vm_accepted': sum(not r['gnu_valid'] and r['error'] is None for r in own),
            'first_valid_rejection': first,
            'valid_rejection_errors': dict(collections.Counter(r['error'].split(' at byte')[0] for r in blockers)),
        }
    Path(args.output).write_text(json.dumps({'scope': __doc__, 'summary': summaries, 'rows': rows}, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(summaries, ensure_ascii=False, indent=2))

if __name__ == '__main__':
    main()
