"""Portable import receipt for sealed subject-only evidence.

The producer's exact-tree manifest is verified before import. Only the
ratchet output may change afterwards. All other tracked
bytes and modes remain bound; runtime/code changes require a fresh probe.
"""
import hashlib
import json
from pathlib import Path
import os
import re
import subprocess

from vacuity_classifier import replay

OUTPUTS = frozenset(('ledger/parity-ratchet.json',))
RECEIPT = 'VACUITY-VERDICT.json'
MEMBERS = 'MEMBER-EVIDENCE.json'
POLICY = 'gnu-tests/member-evidence-policy.json'


def git_root(root):
    """Where revision metadata lives, which is not always where measurement happens.

    The gate measures an extracted archive that has no .git, so revision identity
    cannot be read there. tools/gate exports ROMACS_VACUITY_GIT_ROOT naming the
    candidate worktree; use it ONLY when the measured tree genuinely has no .git,
    so a real repository is never silently overridden. ls-tree reads Git objects
    rather than working files, including an explicitly captured uncommitted tree.
    The variable is part of the gate's environment
    fingerprint, so its use is recorded in the receipt rather than hidden.
    """
    root = Path(root)
    if (root / '.git').exists():
        return root
    external = os.environ.get('ROMACS_VACUITY_GIT_ROOT')
    return Path(external) if external else root


def snapshot_tree(root):
    """Return the gate's captured tree for an extracted source archive."""
    root = Path(root)
    value = os.environ.get('ROMACS_VACUITY_GIT_TREE')
    if value is None or (root / '.git').exists():
        return None
    if not re.fullmatch(r'[0-9a-f]{40}|[0-9a-f]{64}', value):
        raise ValueError('invalid captured source tree')
    kind = subprocess.check_output(['git', '-C', str(git_root(root)), 'cat-file', '-t', value])
    if kind.strip() != b'tree':
        raise ValueError('captured source object is not a tree')
    return value


def source_identity(source):
    """The source fields bound by the build receipt."""
    keys = ('revision', 'tree', 'kind') if 'kind' in source else ('revision', 'tree')
    return {key: source[key] for key in keys}


def projection(root, revision=None):
    root = Path(root)
    working = revision is None or revision == 'HEAD'
    revision = revision or snapshot_tree(root) or 'HEAD'
    rows = subprocess.check_output(['git', '-C', str(git_root(root)), 'ls-tree', '-rz', revision])
    algorithm = subprocess.check_output(['git', '-C', str(git_root(root)), 'rev-parse', '--show-object-format']).decode().strip()
    digest = hashlib.sha256()
    for row in rows.split(b'\0'):
        if not row:
            continue
        metadata, raw_name = row.split(b'\t', 1)
        name = raw_name.decode()
        if name in OUTPUTS:
            continue
        mode, kind, oid = metadata.split()
        path = root / name
        if working:
            if any(p.is_symlink() for p in path.parents if p != root and p.is_relative_to(root)):
                raise ValueError('stale subject evidence: tracked parent is a symlink: ' + name)
            if path.is_symlink():
                import os
                content = os.fsencode(os.readlink(path))
                actual_mode = b'120000'
            else:
                content = path.read_bytes()
                actual_mode = b'100755' if path.stat().st_mode & 0o111 else b'100644'

            actual = hashlib.new(algorithm, b'blob ' + str(len(content)).encode() + b'\0' + content).hexdigest().encode()
            if mode != actual_mode or actual != oid:
                raise ValueError('stale subject evidence: tracked bytes or mode changed: ' + name)
        digest.update(row + b'\0')
    return digest.hexdigest()


def make_receipt(root, audit, manifest, verdicts):
    raw_manifest = (audit / 'audit-manifest.json').read_text()
    return {'schema': 1, 'projection': projection(root), 'excluded_outputs': sorted(OUTPUTS),
            'manifest': raw_manifest,
            'manifest_sha256': hashlib.sha256(raw_manifest.encode()).hexdigest(),
            'artifacts': {name: (audit / name).read_text() for name in manifest['results']},
            'verdicts': verdicts}


def evidence_paths():
    receipt = os.environ.get('ROMACS_VACUITY_RECEIPT')
    members = os.environ.get('ROMACS_MEMBER_EVIDENCE')
    if not receipt and not members:
        raise ValueError('run-local evidence required: set ROMACS_VACUITY_RECEIPT or ROMACS_MEMBER_EVIDENCE')
    receipt = Path(receipt) if receipt else Path(members).parent / RECEIPT
    members = Path(members) if members else receipt.parent / MEMBERS
    if receipt.resolve().parent != members.resolve().parent:
        raise ValueError('member evidence and subject receipt must share a run directory')
    return receipt, members


def verify_receipt(root, path=None):
    root = Path(root)
    path = Path(path) if path is not None else evidence_paths()[0]
    receipt = json.loads(path.read_text())
    if receipt.get('schema') != 1 or receipt.get('excluded_outputs') != sorted(OUTPUTS):
        raise ValueError('invalid subject evidence receipt scope')
    manifest_text = receipt['manifest']
    if hashlib.sha256(manifest_text.encode()).hexdigest() != receipt['manifest_sha256']:
        raise ValueError('subject evidence manifest digest mismatch')
    manifest = json.loads(manifest_text)
    from vacuity_classifier import load_focus
    focus = load_focus()
    if (manifest['binaries']['gnu']['file']['sha256'] != focus.GNU_SHA
            or manifest['binaries']['gnu']['provenance'] != {
                'kind': 'native-gnu-pin', 'revision': focus.GNU_REV, 'sha256': focus.GNU_SHA}
            or manifest['binaries']['romacs']['provenance']['kind'] != 'build-receipt'):
        raise ValueError('subject evidence binary provenance mismatch')
    source = manifest['source']
    captured = snapshot_tree(root)
    if source.get('kind') == 'git-tree':
        if captured is None or source['revision'] != captured or source['tree'] != captured:
            raise ValueError('subject evidence captured source tree mismatch')
    elif source.get('kind') is not None or captured is not None:
        raise ValueError('subject evidence source regime mismatch')
    tree = subprocess.check_output(['git', '-C', str(git_root(root)), 'rev-parse', source['revision'] + '^{tree}']).decode().strip()
    if tree != source['tree']:
        raise ValueError('subject evidence source tree mismatch')
    if captured is None:
        subprocess.run(['git', '-C', str(git_root(root)), 'merge-base', '--is-ancestor', source['revision'], 'HEAD'], check=True)
    if projection(root, source['revision']) != receipt['projection'] or projection(root) != receipt['projection']:
        raise ValueError('stale subject evidence: source inputs changed')

    class Text:
        def __init__(self, value):
            self.value = value
        def read_text(self):
            return self.value

    def artifact(_manifest, _audit, name):
        text = receipt['artifacts'][name]
        record = manifest['results'][name]
        if hashlib.sha256(text.encode()).hexdigest() != record['sha256'] or len(text.encode()) != record['size']:
            raise ValueError('subject evidence artifact digest mismatch')
        return Text(text)

    if set(receipt['artifacts']) != set(manifest['results']):
        raise ValueError('subject evidence artifact inventory mismatch')
    for name in receipt['artifacts']:
        artifact(manifest, None, name)
    data = json.loads(artifact(manifest, None, 'member-results.json').read_text())
    if data.get('classification') != 'subject-only-v1':
        raise ValueError('subject evidence requires reviewed subject-only producer')
    verdicts = replay(data, None, artifact, manifest)
    if verdicts != receipt['verdicts']:
        raise ValueError('subject evidence verdict mismatch')
    return receipt, manifest, verdicts


def member_evidence(receipt, name):
    if name != 'ert-test-should-error-argument':
        raise ValueError('only the disputed should-error-argument entry may be promoted')
    if receipt['verdicts'][name] != 'ASSERTION-SENSITIVE':
        raise ValueError('only an assertion-sensitive verdict can replace historical evidence')
    return {'class': 'GENUINE', 'method': 'sealed-subject-v1',
            'why': 'Retained assertion rejects a no-error subject under romacs mini-ERT and pinned GNU real ERT; bounded subject sensitivity, not full harness parity.',
            'manifest_sha256': receipt['manifest_sha256'], 'recipe': name}
