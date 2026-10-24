#!/usr/bin/env python3
"""Mutants for tests/hash_read_syntax.rs and the subr completion pin.

Each mutant edits one source file, asserts the edit applied, runs the named
test binaries, records whether a test FAILED at an assertion (not a compile
error, not an unwrap), and restores the file byte-for-byte (sha256 checked).

    python3 crates/romacs-vm/evidence/hash-read-syntax/mutants.py OUT.log
"""
import hashlib, os, re, subprocess, sys
ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))
TESTS = ['--test', 'hash_read_syntax']
MUTANTS = [
    ('reader-no-hash-table-dispatch', 'src/reader.rs',
     'if head == self.vm.intern("hash-table") {', 'if false && head == self.vm.intern("hash-table") {'),
    ('plist-last-pair-wins', 'src/hash.rs',
     '.chunks_exact(2)\n                .find(|pair| pair[0] == name)',
     '.chunks_exact(2)\n                .rfind(|pair| pair[0] == name)'),
    ('no-odd-data-check', 'src/hash.rs',
     'if items.len() % 2 == 1 {', 'if false && items.len() % 2 == 1 {'),
    # make-hash-table's own argument checks run BEFORE the data checks.
    ('data-check-after-make-hash-table', 'src/hash.rs',
     '        let data = get(self.intern("data"));\n',
     '        let _ = self.make_hash_table(&params)?;\n        let data = get(self.intern("data"));\n'),
    ('syntax-datum-drops-char', 'src/reader.rs',
     'let datum = format!("#s{}", other.map(String::from).unwrap_or_default());',
     'let datum = { let _ = other; "#s".to_owned() };'),
    ('reader-signal-ignored', 'src/prim.rs',
     'if let Some((condition, data)) = signal { Fault::Signal(condition, data) }',
     'if let Some((_condition, _data)) = signal.filter(|_| false) { Fault::Signal(_condition, _data) }'),
    ('printer-elements-json', 'src/eval_data.rs',
     'vm.eval_data_print(v, quoted, escape_newlines, depth + 1).map(String::into_bytes)',
     'vm.display(v).map(String::into_bytes)'),
    ('printer-format-elements-json', 'src/productivity.rs',
     'return self.print_table(t, |vm, v| vm.format_print(v, quoted, depth + 1));',
     'return self.print_table(t, |vm, v| vm.display(v).map(String::into_bytes));'),
    ('printer-drops-weakness', 'src/hash.rs',
     '        if t.weakness != Weakness::None {\n            out.push_str(" weakness ");',
     '        if false && t.weakness != Weakness::None {\n            out.push_str(" weakness ");'),
    ('gc-keys-claimed-unmoving', 'src/hash.rs',
     'None => t.insert(pair[0], pair[1], hash, moving),', 'None => t.insert(pair[0], pair[1], hash, { let _ = moving; false }),'),
    # Every table the reader builds is also consed onto a traced global:
    # reachable forever, so the heap must grow cycle after cycle.
    ('reader-retains-tables', 'src/reader.rs',
     '                Ok(table) => Ok(table),',
     '                Ok(table) => { let s = self.vm.slot("hrs--leak"); let old = if self.vm.slots[s].value == Value::UNBOUND { Value::NIL } else { self.vm.slots[s].value }; let cell = self.vm.list_value(&[table, old]); self.vm.slots[s].value = cell; Ok(table) }'),
    # GNU's dot test (lread.c:4126) replaced by VM2's general delimiter set.
    ('dot-token-uses-delimiters', 'src/reader.rs',
     '.is_none_or(|c| c as u32 <= 32 || c == \'\\u{a0}\' || "\\"\';([#?`,".contains(c))',
     '.is_none_or(Self::delimiter)'),
    # A `.' before `)`/`]` is the symbol `.`, not a refused dot token.
    ('lone-dot-before-closer-refused', 'src/reader.rs',
     "'.' if Self::dot_follows(self.peek()) =>",
     "'.' if Self::dot_follows(self.peek()) || matches!(self.peek(), Some(')' | ']')) =>"),
    # Without the arm a misplaced dot token (top level, tail, list start,
    # vector, record) reads as the symbol `.`.
    ('dot-token-read-as-symbol', 'src/reader.rs',
     "'.' if Self::dot_follows(self.peek()) =>", "'.' if false =>"),
    # A list start or a vector taking the dot as if it were an RE_list.
    ('dot-taken-anywhere', 'src/reader.rs',
     'if !vector && !values.is_empty() && self.dot_token() {', 'if self.dot_token() {'),
    # A closer in the tail position falls through to the generic closer path.
    ('dot-tail-closer-unchecked', 'src/reader.rs',
     'return Err(self.syntax(&closer.to_string()));', 'let _ = closer;'),
    # Only `)` checked in the tail: `(a . ]` falls to the generic closer path.
    ('dot-tail-bracket-unchecked', 'src/reader.rs',
     "if let Some(closer @ (')' | ']')) = self.peek() {", "if let Some(closer @ ')') = self.peek() {"),
]
log = []
ONLY = os.environ.get('MUTANTS_ONLY', '').split(',') if os.environ.get('MUTANTS_ONLY') else None
for name, rel, old, new in MUTANTS:
    if ONLY and name not in ONLY:
        continue
    path = os.path.join(ROOT, rel)
    original = open(path, 'rb').read()
    digest = hashlib.sha256(original).hexdigest()
    text = original.decode()
    assert text.count(old) == 1, f'{name}: pattern count {text.count(old)}'
    mutated = text.replace(old, new)
    assert mutated != text
    open(path, 'w').write(mutated)
    try:
        p = subprocess.run(['cargo', 'test', '-j', '6', '--no-fail-fast', *TESTS],
                           cwd=ROOT, capture_output=True, text=True)
    finally:
        open(path, 'wb').write(original)
    assert hashlib.sha256(open(path, 'rb').read()).hexdigest() == digest, f'{name}: restore failed'
    out = p.stdout + p.stderr
    compiled = 'error[' not in out and 'could not compile' not in out
    failed = re.findall(r'^test (\S+) \.\.\. FAILED$', out, re.M)
    at_assertion = re.findall(r"panicked at [^\n]*\n(assertion[^\n]*|[0-9]+ of [0-9]+ cases:|the window[^\n]*|keys lost[^\n]*|round [0-9][^\n]*)", out)
    divergences = sorted(set(re.findall(r'^(?:NEW DIVERGENCE|STALE PIN|CHANGED) ([a-z0-9-]+)', out, re.M)))
    verdict = 'COMPILE-ERROR' if not compiled else ('KILLED' if p.returncode != 0 and at_assertion else ('SURVIVED' if p.returncode == 0 else 'FAILED-NOT-AT-ASSERTION'))
    if os.environ.get('MUTANTS_DUMP'):
        open(os.environ['MUTANTS_DUMP'] + '.' + name, 'w').write(out)
    log.append(f'{name}: {verdict} failed={failed} first={at_assertion[:1]} cases={divergences[:8]}')
    print(log[-1], flush=True)
open(sys.argv[1], 'w').write('\n'.join(log) + '\n')
