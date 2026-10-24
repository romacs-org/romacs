"""Bounded subject-mutant classifier, not a general test-adequacy proof.

Only these reviewed recipes are supported. A kill must reach an assertion;
registration errors, missing rows and arbitrary whole-body deletion never count.
"""
from pathlib import Path
import importlib.util
import json

ENGINES = ('romacs-mini', 'gnu-real', 'gnu-mini')


def load_focus():
    spec = importlib.util.spec_from_file_location('vacuity_focus', Path(__file__).with_name('probe-vacuity-focus.py'))
    if spec is None or spec.loader is None:
        raise ValueError('focused producer unavailable')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def recipes():
    forms = load_focus().FORMS
    result = {}
    for name, (file, original, _) in forms.items():
        if name == 'ert-test-deftest-lexical-binding-t':
            mutant = None  # No verified harness binding fault available.
        elif name == 'ert-test-body-runs':
            mutant = original.replace('(setq ert--test-body-was-run t)', 'nil')
        elif name == 'ert-test-should-error-argument':
            mutant = original.replace('(/ 1 0)', 'nil')
        else:
            mutant = original.replace('(format "%c" 0.5)', 'nil')
        if mutant == original:
            raise ValueError('subject recipe no longer changes its exact source slice')
        result[name] = (file, original, mutant)
    result['vclass-disabled-subject-control'] = ('classifier-control',
        '(defun vclass-subject () t)\n(ert-deftest vclass-disabled-subject-control () (vclass-subject))',
        '(defun vclass-subject () nil)\n(ert-deftest vclass-disabled-subject-control () (vclass-subject))')
    return result


def expected_cases():
    return [dict(file=file, test=name, mutation=mutation, engine=engine)
            for name, (file, _, mutant) in recipes().items()
            for mutation in (('original', 'subject-noop') if mutant else ('original',))
            for engine in ENGINES]


def classify(data):
    expected = expected_cases()
    key = lambda row: tuple(row[k] for k in ('file', 'test', 'mutation', 'engine'))
    rows = data['observations']
    if sorted(map(key, rows)) != sorted(map(key, expected)):
        raise ValueError('classifier requires exact reviewed case inventory')
    verdicts = {}
    for name, (file, _, mutant) in recipes().items():
        selected = [r for r in rows if r['test'] == name]
        if any('blocked' in r for r in selected):
            verdict = 'UNCLASSIFIED'
        elif any(r['result']['outcome'] != 'passed' for r in selected if r['mutation'] == 'original'):
            verdict = 'UNCLASSIFIED'
        elif mutant is None:
            verdict = 'UNCLASSIFIED'
        else:
            outcomes = [r['result']['outcome'] for r in selected if r['mutation'] == 'subject-noop']
            verdict = ('ASSERTION-SENSITIVE' if outcomes == ['failed'] * len(ENGINES)
                       else 'CANNOT-FAIL' if outcomes == ['passed'] * len(ENGINES)
                       else 'UNCLASSIFIED')
        verdicts[name] = verdict
    if verdicts['format-c-float'] != 'ASSERTION-SENSITIVE':
        raise ValueError('positive control failed: retained assertion must kill broken format subject')
    if verdicts['vclass-disabled-subject-control'] != 'CANNOT-FAIL':
        raise ValueError('negative control failed: disabled subject with no oracle must survive')
    return verdicts


def replay(data, audit, require_result, manifest):
    """Recompute outcomes from sealed raw output, not producer verdict strings."""
    focus = load_focus()
    replayed = []
    for row in data['observations']:
        file, original, mutant = recipes()[row['test']]
        form = original if row['mutation'] == 'original' else mutant
        paths = row['artifact_paths']
        probe = next(p for p in paths if not p.endswith(('.stdout', '.stderr')))
        text = require_result(manifest, audit, probe).read_text()
        if row['engine'] == 'romacs-mini':
            plan = json.loads(text)
            if plan['forms'][-1]['source'] != form:
                raise ValueError('probe differs from reviewed subject mutation')
        elif '\n' + form + '\n' not in text:
            raise ValueError('probe differs from reviewed subject mutation')
        raw = require_result(manifest, audit, probe + '.stdout').read_text()
        copy = dict(row)
        copy.pop('result', None)
        copy.pop('blocked', None)
        try:
            if row['exit_code'] != 0:
                raise ValueError('process did not complete')
            copy['result'] = (focus.parse_vm(raw, row['test']) if row['engine'] == 'romacs-mini'
                              else focus.parse_gnu(raw, row['test']))
        except (ValueError, json.JSONDecodeError) as error:
            copy['blocked'] = str(error)
        replayed.append(copy)
    return classify({'observations': replayed})
