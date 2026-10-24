"""Focused regression tests; all failures must be runtime assertions."""
import copy
import unittest
import primh


class RatchetTests(unittest.TestCase):
    def row(self):
        return {"id": "x/0000", "source": "ignored", "expected": "(ok 1)", "gnu_regime_variant": "(ok 2)"}

    def test_alternate_regime_is_parity(self):
        ev = {"cases": [self.row()], "pinned": {}}
        counts, bad = primh.ratchet(ev, ["(ok 2)"])
        self.assertEqual(counts, {"PASS-REGIME": 1})
        self.assertEqual(bad, [])

    def test_new_divergence(self):
        _, bad = primh.ratchet({"cases": [self.row()]}, ["(ok 3)"])
        self.assertIn("NEW DIVERGENCE", "\n".join(bad))

    def test_stale_pin(self):
        _, bad = primh.ratchet({"cases": [self.row()], "pinned": {"x/0000": "VALUE"}}, ["(ok 1)"])
        self.assertIn("STALE PIN", "\n".join(bad))

    def test_alternate_regime_stales_pin(self):
        _, bad = primh.ratchet({"cases": [self.row()], "pinned": {"x/0000": "VALUE"}}, ["(ok 2)"])
        self.assertIn("STALE PIN", "\n".join(bad))

    def test_changed_class(self):
        _, bad = primh.ratchet({"cases": [self.row()], "pinned": {"x/0000": "DATA"}}, ["(ok 3)"])
        self.assertIn("CHANGED PIN", "\n".join(bad))

    def test_unknown_pin(self):
        _, bad = primh.ratchet({"cases": [self.row()], "pinned": {"gone": "VALUE"}}, ["(ok 1)"])
        self.assertIn("UNKNOWN PIN", "\n".join(bad))

    def test_reply_count(self):
        with self.assertRaisesRegex(ValueError, "reply count"):
            primh.ratchet({"cases": [self.row()]}, [])

    def test_timeout_excluded_not_pinned(self):
        r = self.row(); r["expected"] = "GNU-TIMEOUT"
        counts, bad = primh.ratchet({"cases": [r]}, ["(ok 3)"])
        self.assertEqual(counts, {"EXCLUDED-GNU-DIVERGES": 1})
        self.assertEqual(bad, [])


class RecordingTests(unittest.TestCase):
    def record(self, outcomes):
        import json
        import subprocess
        from pathlib import Path
        from types import SimpleNamespace
        from unittest.mock import patch
        rows = [dict(id=f'x/{i}', source=str(i)) for i in range(3)]
        head = {'subrs': [['x', 'subr', 1, 1]]}
        replies = [dict(id=r['id'], expected=f'(ok {i})', interp=f'(ok alt{i})')
                   for i, r in enumerate(rows)]
        calls = []
        def run(*args, **kwargs):
            batch = json.loads(Path(kwargs['env']['PRIMH_CASES']).read_text())
            calls.append([r['id'] for r in batch])
            timeout, indexes, change = outcomes[len(calls) - 1]
            stream = [dict(head), *[dict(replies[i]) for i in indexes]]
            if change:
                change(stream)
            text = '\n'.join(json.dumps(r) for r in stream) + '\n'
            if timeout:
                raise subprocess.TimeoutExpired(args[0], kwargs['timeout'], stderr=text.encode())
            return SimpleNamespace(stderr=text)
        with patch.object(primh.subprocess, 'run', side_effect=run):
            result = primh.gnu_rows('ignored', {'name': 'x', 'arity': [1, 1]}, rows, ['x'])
        return result, calls, replies

    def test_aggregate_timeout_retries_terminating_case_alone(self):
        (head, got), calls, replies = self.record([
            (True, [0], None), (False, [1], None), (False, [2], None)])
        self.assertEqual(calls, [['x/0', 'x/1', 'x/2'], ['x/1'], ['x/2']])
        self.assertEqual(got, {r['id']: (r['expected'], r['interp']) for r in replies})

    def test_individual_hang_excludes_only_confirmed_case(self):
        (_, got), calls, replies = self.record([
            (True, [0], None), (True, [], None), (False, [2], None)])
        self.assertEqual(calls, [['x/0', 'x/1', 'x/2'], ['x/1'], ['x/2']])
        self.assertEqual(got['x/0'], (replies[0]['expected'], replies[0]['interp']))
        self.assertEqual(got['x/1'], ('GNU-TIMEOUT', 'GNU-TIMEOUT'))
        self.assertEqual(got['x/2'], (replies[2]['expected'], replies[2]['interp']))

    def test_timeout_after_complete_stream_preserves_all_rows(self):
        (_, got), calls, replies = self.record([(True, [0, 1, 2], None)])
        self.assertEqual(len(calls), 1)
        self.assertEqual(got, {r['id']: (r['expected'], r['interp']) for r in replies})

    def test_isolated_timeout_after_complete_row_is_not_excluded(self):
        (_, got), calls, replies = self.record([
            (True, [0], None), (True, [1], None), (False, [2], None)])
        self.assertEqual(got['x/1'], (replies[1]['expected'], replies[1]['interp']))

    def test_invalid_header_count_order_or_regime_rejected(self):
        mutations = [
            lambda s: s[0].update(subrs=[['wrong', 'subr', 1, 1]]),
            lambda s: s.pop(),
            lambda s: s[1].update(id='x/2'),
            lambda s: s[1].pop('interp'),
        ]
        for mutation in mutations:
            with self.subTest(mutation=mutation), self.assertRaisesRegex(ValueError, 'GNU reply'):
                self.record([(False, [0, 1, 2], mutation)])

    def test_isolated_retry_header_is_validated(self):
        with self.assertRaisesRegex(ValueError, 'GNU reply'):
            self.record([(True, [0], None),
                         (True, [], lambda s: s[0].update(subrs=[])), (False, [2], None)])

    def test_gnu_process_crash_is_not_timeout_evidence(self):
        import subprocess
        from unittest.mock import patch
        with patch.object(primh.subprocess, 'run', side_effect=subprocess.CalledProcessError(-11, 'gnu')):
            with self.assertRaises(subprocess.CalledProcessError):
                primh.gnu_rows('ignored', {'name': 'x', 'arity': [1, 1]},
                               [{'id': 'x/0', 'source': 'nil'}], ['x'])

    def test_vm_crash_is_not_valid_timeout_exclusion(self):
        counts, bad = primh.ratchet({'cases': [{'id': 'x/0', 'expected': 'GNU-TIMEOUT'}]}, [None])
        self.assertEqual(counts, {'VM-CRASH': 1})
        self.assertIn('NEW DIVERGENCE (VM-CRASH)', '\n'.join(bad))


class IntegrityTests(unittest.TestCase):
    def test_atomic_write_and_cleanup(self):
        import json
        import tempfile
        from pathlib import Path
        with tempfile.TemporaryDirectory(dir=primh.HERE) as root:
            path = Path(root) / 'evidence.json'
            primh.atomic_json(path, {'value': 1})
            primh.atomic_json(path, {'value': 2})
            self.assertEqual(json.loads(path.read_text()), {'value': 2})
            self.assertEqual(list(Path(root).glob('*.pending')), [])

    def test_each_recorded_input_hash_is_enforced(self):
        import json
        import tempfile
        from pathlib import Path
        from unittest.mock import patch
        spec = primh.load_spec(primh.HERE / 'specs/cdr-safe.json')
        original = json.loads((primh.EVID / 'cdr-safe.json').read_text())
        for key in ('spec_sha256', 'generator_sha256', 'domains_sha256', 'driver_sha256', 'replay_sha256'):
            with self.subTest(key=key), tempfile.TemporaryDirectory(dir=primh.HERE) as root:
                changed = copy.deepcopy(original); changed[key] = '0' * 64
                primh.atomic_json(Path(root) / 'cdr-safe.json', changed)
                with patch.object(primh, 'EVID', Path(root)):
                    with self.assertRaisesRegex(ValueError, key):
                        primh.evidence(spec)

    def test_source_tampering_is_rejected(self):
        import json
        import tempfile
        from pathlib import Path
        from unittest.mock import patch
        spec = primh.load_spec(primh.HERE / 'specs/cdr-safe.json')
        original = json.loads((primh.EVID / 'cdr-safe.json').read_text())
        original['cases'][0]['source'] = 'nil'
        with tempfile.TemporaryDirectory(dir=primh.HERE) as root:
            primh.atomic_json(Path(root) / 'cdr-safe.json', original)
            with patch.object(primh, 'EVID', Path(root)):
                with self.assertRaisesRegex(ValueError, 'deterministic generation'):
                    primh.evidence(spec)


class ManifestTests(unittest.TestCase):
    def run_manifest(self, only, arity=(1, 1), declared=(1, 1)):
        import json
        import tempfile
        from pathlib import Path
        from types import SimpleNamespace
        from unittest.mock import patch
        rows = [dict(name='cdr-safe', min=declared[0], max=declared[1], effect='Pure',
                     gnu='data.c:688', rust='src/prims/fns.rs:87'),
                dict(name='romacs--t2-assoc', min=3, max=3, effect='Global',
                     gnu='fns.c:2006', rust='src/prims/callback.rs:14')]
        with tempfile.TemporaryDirectory(dir=primh.HERE) as root:
            path = Path(root) / 'manifest.json'
            path.write_text(json.dumps(rows))
            def reply(*args, **kwargs):
                names = kwargs['env']['PRIMH_NAMES'].split(',')
                head = {'subrs': [[n, 'subr', *(arity if n == 'cdr-safe' else (3, 3))] for n in names]}
                return SimpleNamespace(stderr=json.dumps(head))
            with patch.object(primh.subprocess, 'run', side_effect=reply) as run:
                primh.from_manifest(SimpleNamespace(manifest=path, only=only, out=Path(root)/'out', gnu='ignored'))
                return sorted(p.stem for p in (Path(root)/'out').glob('*.json')), run.call_args

    def test_filter_excludes_namespaced_extension(self):
        names, call = self.run_manifest(['cdr-safe', 'cdr-safe'])
        self.assertEqual(names, ['cdr-safe'])
        self.assertEqual(call.kwargs['env']['PRIMH_NAMES'], 'cdr-safe')

    def test_filter_unknown_name_rejected(self):
        with self.assertRaisesRegex(ValueError, 'unknown.*not-a-primitive'):
            self.run_manifest(['not-a-primitive'])

    def test_manifest_min_must_match_gnu(self):
        with self.assertRaisesRegex(ValueError, 'arity.*manifest'):
            self.run_manifest(['cdr-safe'], declared=(0, 1))

    def test_manifest_max_must_match_gnu(self):
        with self.assertRaisesRegex(ValueError, 'arity.*manifest'):
            self.run_manifest(['cdr-safe'], declared=(1, 2))

    def test_variadic_manifest_matches_gnu_many(self):
        names, _ = self.run_manifest(['cdr-safe'], arity=(1, 'many'), declared=(1, 'many'))
        self.assertEqual(names, ['cdr-safe'])


if __name__ == "__main__":
    unittest.main()
