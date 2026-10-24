"""Kill filtering/arity implementation mutants without changing source files."""
import contextlib
import io
import unittest
import primh
import test_primh

source = primh.Path(primh.__file__).read_text()
mutants = {
    'ignore-selection': ("if selected:\n        man =", "if False:\n        man ="),
    'accept-unknown': ("if unknown:\n        raise", "if False:\n        raise"),
    'ignore-minimum': ("if [lo, hi] != [m['min'], m['max']]:", "if hi != m['max']:"),
    'ignore-maximum': ("if [lo, hi] != [m['min'], m['max']]:", "if lo != m['min']:"),
}
original = primh.from_manifest
for name, (old, new) in mutants.items():
    assert source.count(old) == 1, name
    namespace = dict(primh.__dict__, __name__='primh_mutant')
    exec(compile(source.replace(old, new), primh.__file__, 'exec'), namespace)
    primh.from_manifest = namespace['from_manifest']
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(test_primh.ManifestTests)
    log = io.StringIO()
    try:
        with contextlib.redirect_stdout(log):
            result = unittest.TextTestRunner(stream=log, verbosity=2).run(suite)
        print(f'=== MUTANT {name} ===\n{log.getvalue()}')
        assert result.failures and not result.errors, f'{name}: not killed by an assertion'
    finally:
        primh.from_manifest = original
print('All four manifest implementation mutants killed by assertion failures.')
