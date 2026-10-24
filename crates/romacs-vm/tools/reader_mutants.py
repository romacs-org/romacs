#!/usr/bin/env python3
"""Compile behavioral reader mutants, restore source bytes on every exit."""
import argparse
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]
CASES = {
    'deferred-unread': ('src/reader.rs',
        '                self.unread_lookahead()?;\n                if !escaped {',
        '                // mutant: leave delimiter consumed\n                if !escaped {',
        'reader_nested_callback_unread_is_immediate'),
    'raw-byte-as-unicode': ('src/reader.rs',
        'let c = bytes[0] as u32;\n                    (if c >= 128 { c + 0x3fff00 } else { c }, 1)',
        'let c = bytes[0] as u32;\n                    (c, 1)', 'reader_bytes_unibyte_literal'),
    'copy-ascii-from-source': ('src/char_table.rs',
        'let ascii = Self::ascii_from_root(&root);',
        'let ascii = self.ascii.clone();',
        'review_independent_ascii_is_preserved_in_copy_and_rejected_by_mapping'),
    'missing-dot-unread': ('src/reader.rs',
        '            46 if Self::dot_follows(self.peek()) => {\n                self.unread_lookahead()?;',
        '            46 if Self::dot_follows(self.peek()) => {\n                // mutant: signal while the delimiter remains consumed',
        'reader_error_callback_collection_preserves_condition'),
    'lost-subtable-kind': ('src/prim.rs',
        'Some(data::Object::SubCharTable(_)) => Kind::SubCharTable,',
        'Some(data::Object::SubCharTable(_)) => Kind::Invalid,',
        'review_subtable_type_and_copy_sequence_contract'),
}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    results = []
    for name, (relative, old, new, test) in CASES.items():
        path = ROOT / 'crates/romacs-vm' / relative
        original = path.read_bytes()
        text = original.decode()
        if text.count(old) != 1:
            raise RuntimeError(f'{name}: expected one mutation site')
        try:
            path.write_text(text.replace(old, new))
            result = subprocess.run(['cargo', 'test', '-p', 'romacs-vm', '--lib', test],
                                    cwd=ROOT, capture_output=True, text=True)
            log = args.output.with_name(args.output.stem + '-' + name + '.log')
            log.write_text(result.stdout + result.stderr)
            killed = result.returncode != 0 and 'test result: FAILED.' in result.stdout and test in result.stdout
            results.append({'mutant': name, 'test': test, 'exit': result.returncode,
                            'behavioral_kill': killed, 'log': str(log)})
        finally:
            path.write_bytes(original)
    args.output.write_text(json.dumps(results, indent=2) + '\n')
    print(json.dumps(results, indent=2))
    return 0 if all(row['behavioral_kill'] for row in results) else 1

if __name__ == '__main__':
    raise SystemExit(main())
