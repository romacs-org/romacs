"""Generate and validate run-local classifications from policy and sealed evidence."""
import json
from pathlib import Path

from vacuity_binding import MEMBERS, POLICY, evidence_paths, member_evidence, verify_receipt


def build(root, receipt):
    data = json.loads((Path(root) / POLICY).read_text())
    if data.get('schema') != 1 or not isinstance(data.get('members'), dict):
        raise ValueError('invalid historical member policy schema')
    if set(data) != {'schema', 'members'}:
        raise ValueError('historical member policy cannot contain run observations')
    for value in data['members'].values():
        if not isinstance(value, dict) or set(value) != {'class', 'method'}:
            raise ValueError('historical member policy requires only class and method')
        if value['class'] not in ('GENUINE', 'VACUOUS', 'WRONG-ERROR', 'WRONG-ERROR-DATA', 'HARNESS-SELF'):
            raise ValueError('historical member policy contains an unknown class')
        if value['method'] not in ('hand', 'inspection', 'mutation', 'void-catch'):
            raise ValueError('historical member policy contains an unknown method')
    name = 'ert-test-should-error-argument'
    key = 'lisp/emacs-lisp/ert-tests.el::' + name
    if key in data['members']:
        raise ValueError('historical member policy cannot replace the sealed subject')
    data['members'][key] = member_evidence(receipt, name)
    return data


def write(root, receipt_path):
    receipt, _, _ = verify_receipt(root, receipt_path)
    output = Path(receipt_path).parent / MEMBERS
    output.write_text(json.dumps(build(root, receipt), indent=1, sort_keys=True) + '\n')
    return output


def load(root, path=None):
    receipt_path, default = evidence_paths()
    path = Path(path) if path is not None else default
    if path.resolve().parent != receipt_path.resolve().parent:
        raise ValueError('member evidence and subject receipt must share a run directory')
    receipt, _, _ = verify_receipt(root, receipt_path)
    data = json.loads(path.read_text())
    if data != build(root, receipt):
        raise ValueError('member evidence differs from policy and sealed classification')
    return data['members']
