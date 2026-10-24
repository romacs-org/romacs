#!/usr/bin/env python3
"""Isolated build.rs integration tests; no cargo or shared target writes.

python tools/registry_generator_test.py --sha2 /path/to/libsha2-HASH.rlib
Use an rlib produced by the current rustc. --build-script accepts a mutation copy.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

PARSER = argparse.ArgumentParser(description=__doc__)
PARSER.add_argument('--sha2', required=True, type=Path)
PARSER.add_argument('--build-script', type=Path, default=Path(__file__).resolve().parents[1] / 'crates/romacs-vm/build.rs')
ARGS, TEST_ARGS = PARSER.parse_known_args()


def declaration(name='fresh', ident='fresh', args='cx, a: Value'):
    return f'defsubr! {{ name = "{name}", gnu = "fns.c:1", effect = Pure,\nfn {ident}({args}) {{ Ok(Value::NIL) }} }}\n'


class RegistryGenerator(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix='registry-generator-', dir=os.environ.get('TMPDIR'))
        cls.base = Path(cls.temp.name)
        cls.binary = cls.base / 'generator'
        proc = subprocess.run(['rustc', '--edition=2021', str(ARGS.build_script), '--extern', f'sha2={ARGS.sha2}', '-L', f'dependency={ARGS.sha2.parent}', '-o', str(cls.binary)], capture_output=True, text=True)
        if proc.returncode:
            raise RuntimeError(proc.stderr)

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def run_generator(self, files, legacy='("legacy", Primitive::Legacy),'):
        case = tempfile.TemporaryDirectory(dir=self.base)
        self.addCleanup(case.cleanup)
        base = Path(case.name)
        root = base / 'crates/vm'
        out = base / 'out'
        (root / 'src/prims').mkdir(parents=True)
        out.mkdir()
        (base / 'Cargo.lock').write_text('')
        (root / 'Cargo.toml').write_text('')
        (root / 'build.rs').write_text(ARGS.build_script.read_text())
        (root / 'src/lib.rs').write_text(f'const PRIMITIVES: &[(&str, Primitive)] = &[{legacy}];')
        (root / 'src/prims/mod.rs').write_text('use super::*;')
        for name, text in files.items():
            path = root / 'src/prims' / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        proc = subprocess.run([str(self.binary)], cwd=root, env={**os.environ, 'OUT_DIR': str(out), 'RUSTC': 'rustc'}, capture_output=True, text=True)
        return proc, out

    def success(self, files, **kwargs):
        proc, out = self.run_generator(files, **kwargs)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        return out

    def rejects(self, files, diagnostic, **kwargs):
        proc, _ = self.run_generator(files, **kwargs)
        self.assertNotEqual(proc.returncode, 0, 'generator accepted forbidden/malformed source')
        self.assertIn(diagnostic, proc.stderr)

    def test_discovery_formatting_and_arity(self):
        source = '''use super::*;
crate::prim::defsubr /* gap */ ! (
 name = "zeta", gnu = "fns.c:2", effect = Reads,
 fn zeta(cx, a: Value, b: Integer,) { Ok(a) }
);
defsubr!{name="alpha",gnu="x:1",effect=Pure,fn alpha(cx) { Ok(Value::NIL) }}
'''
        out = self.success({'z.rs': source})
        manifest = json.loads((out / 'primitives.json').read_text())
        self.assertEqual([(r['name'], r['min'], r['max']) for r in manifest], [('alpha', 0, 0), ('zeta', 2, 2)])
        table = (out / 'generated_prims.rs').read_text()
        self.assertLess(table.index('alpha::DESC'), table.index('zeta::DESC'))
        self.assertEqual([r['rust'] for r in manifest], ['src/prims/z.rs:6', 'src/prims/z.rs:2'])

    def test_comments_strings_chars_and_lifetimes_are_not_code(self):
        source = '''// defsubr! { name = "fake" static thread_local! {}
/* nested /* static */ defsubr! broken */
const TEXT: &str = r###"defsubr! { static thread_local! {} }"###;
const MORE: &str = "static \\" defsubr! nope";
const BYTES: &[u8] = br##"static defsubr! {"##;
fn borrow(x: &'static str) { let _ = '"'; let _ = '\\''; }
''' + declaration()
        out = self.success({'a.rs': source})
        self.assertEqual([r['name'] for r in json.loads((out / 'primitives.json').read_text())], ['fresh'])

    def test_duplicate_lisp_name_different_format(self):
        self.rejects({'a.rs': declaration(), 'b.rs': declaration(ident='other').replace('defsubr! { name', 'defsubr ! {\n name')}, 'declared twice')

    def test_duplicate_rust_ident(self):
        self.rejects({'a.rs': declaration() + declaration('another')}, 'Rust identifier')

    def test_legacy_collision(self):
        self.rejects({'a.rs': declaration('legacy')}, 'legacy')

    def test_decoded_names_and_json(self):
        out = self.success({'a.rs': declaration('line\\n\\u{7f}\\"name')})
        self.assertEqual(json.loads((out / 'primitives.json').read_text())[0]['name'], 'line\n\x7f"name')
        self.rejects({'a.rs': declaration('fresh') + declaration('fre\\u{73}h', 'other')}, 'declared twice')

    def test_malformed_declarations(self):
        for source in [declaration().replace('gnu = "fns.c:1", ', ''), declaration().replace('effect = Pure', 'effect = "Pure"'), declaration().replace('a: Value', 'a'), 'defsubr! nope', declaration().rstrip()[:-1], declaration().replace('a: Value', 'a: Option<Value>')]:
            with self.subTest(source=source):
                self.rejects({'a.rs': source}, 'primitive')

    def test_bans_recursively_and_mod_rs(self):
        for path in ['mod.rs', 'a.rs', 'nested/deep.rs']:
            for code in ['static CACHE: u8 = 0;', 'static mut CACHE: u8 = 0;', 'std::thread_local /* gap */ ! { static X: u8 = 0; }', 'thread_local! {}']:
                with self.subTest(path=path, code=code):
                    self.rejects({path: code}, 'forbidden')

    def test_recursive_discovery_and_root_mod(self):
        out = self.success({'nested/deep.rs': declaration('deep'), 'mod.rs': declaration('root', 'root')})
        self.assertEqual([r['name'] for r in json.loads((out / 'primitives.json').read_text())], ['deep', 'root'])
        self.assertIn('prims::root::DESC', (out / 'generated_prims.rs').read_text())

    def test_output_is_deterministic_and_modules_sorted(self):
        files = {'z.rs': declaration('z', 'z'), 'a.rs': declaration('a', 'a')}
        one = self.success(files)
        two = self.success(dict(reversed(list(files.items()))))
        for name in ['generated_prims.rs', 'primitives.json']:
            self.assertEqual((one / name).read_bytes(), (two / name).read_bytes())
        modules = (one / 'prims_mods.rs').read_text()
        self.assertLess(modules.index('mod a;'), modules.index('mod z;'))


if __name__ == '__main__':
    unittest.main(argv=[__file__, *TEST_ARGS])
