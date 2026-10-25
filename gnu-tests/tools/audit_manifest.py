"""Fail-closed local audit provenance, not a cryptographic build attestation.

API: begin_run(repo, *, audit_dir, binaries, harness_files, commands) -> state;
finish_run(state, audit_dir, *, result_files) -> manifest;
verify_manifest(audit_dir, repo=None) -> manifest.

The output directory must be absent or empty at begin_run: existing evidence
cannot be rebound. Finish only after all producers have exited. Commands are
nonempty argv lists, recorded as supplied, not proof that they were executed.
Binaries map role names to {path, provenance}. Native GNU provenance is
{kind: 'native-gnu-pin', revision: FULL_SHA, sha256: EXPECTED_SHA256}.
Other binaries require {kind: 'build-receipt', path: RECEIPT_JSON}. A receipt
has schema=1, source={revision, tree, optional kind}, binary_sha256, command=[...], exit_code=0,
tracked_clean_start=true, tracked_clean_end=true. The build producer must write
it from actual build observations; this API validates and binds those claims,
not independently proves that compilation happened. Never fabricate a receipt
for an ambient executable. All receipt/input/output bytes remain available.

Source checks compare raw tracked bytes/modes against the Git object, bypassing
index stat caches, filters, assume-unchanged and skip-worktree. They establish
start/end snapshots, not continuous observation between those boundaries.
Gate archives bind kind=git-tree to their submitted tree object. Ordinary
repositories continue to require clean committed source and index snapshots.
"""
from __future__ import annotations

import copy
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
from typing import NoReturn

NAME = 'audit-manifest.json'


def _fail(message) -> NoReturn:
    raise ValueError(message)


def _git(repo, *args):
    try:
        return subprocess.check_output(['git', '-C', str(repo), *args], stderr=subprocess.PIPE)
    except subprocess.CalledProcessError as error:
        raise ValueError('source Git lookup failed') from error


def _oid(value):
    return isinstance(value, str) and re.fullmatch(r'[0-9a-f]{40}|[0-9a-f]{64}', value)


def _source(repo):
    from vacuity_binding import git_root, snapshot_tree

    repo = Path(repo).resolve(strict=True)
    metadata = git_root(repo)
    captured = snapshot_tree(repo)
    revision = captured or _git(metadata, 'rev-parse', '--verify', 'HEAD').decode().strip()
    tree = _git(metadata, 'rev-parse', '--verify', f'{revision}^{{tree}}').decode().strip()
    algorithm = _git(metadata, 'rev-parse', '--show-object-format').decode().strip()
    for row in _git(metadata, 'ls-tree', '-rz', '--full-tree', revision).split(b'\0'):
        if not row:
            continue
        header, name = row.split(b'\t', 1)
        mode, kind, oid = header.split()
        path = repo / os.fsdecode(name)
        if any(p.is_symlink() for p in path.parents if p != repo and p.is_relative_to(repo)):
            _fail(f'tracked parent is a symlink: {name!r}')
        try:
            info = path.lstat()
            if mode == b'120000' and stat.S_ISLNK(info.st_mode):
                data = os.fsencode(os.readlink(path))
            elif mode in (b'100644', b'100755') and stat.S_ISREG(info.st_mode):
                actual_mode = b'100755' if info.st_mode & 0o111 else b'100644'
                if mode != actual_mode:
                    _fail(f'tracked mode mismatch: {name!r}')
                data = path.read_bytes()
            else:
                _fail(f'unsupported or mismatching tracked entry: {name!r}')
        except OSError as error:
            raise ValueError(f'tracked file unavailable: {name!r}') from error
        actual = hashlib.new(algorithm, b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest()
        if kind != b'blob' or actual != oid.decode():
            _fail(f'tracked bytes mismatch: {name!r}')
    if captured is None:
        if _git(metadata, 'diff', '--cached', '--name-only', revision, '--'):
            _fail('tracked index differs from source HEAD')
        if _git(metadata, 'rev-parse', '--verify', 'HEAD').decode().strip() != revision:
            _fail('source revision changed during snapshot')
    source = {'repo': str(repo), 'revision': revision, 'tree': tree,
              'tracked_clean_start': True, 'tracked_clean_end': True}
    if captured is not None:
        source['kind'] = 'git-tree'
    return source


def _file(path):
    path = Path(path).absolute()
    # Reject symlinks in every component before resolve, including the leaf.
    if any(p.is_symlink() for p in (path, *path.parents)):
        _fail(f'symlink evidence path: {path}')
    try:
        info = path.stat()
        if not stat.S_ISREG(info.st_mode):
            _fail(f'evidence is not regular: {path}')
        data = path.read_bytes()
    except OSError as error:
        raise ValueError(f'evidence file unavailable: {path}') from error
    return {'path': str(path.resolve()), 'sha256': hashlib.sha256(data).hexdigest(),
            'size': len(data), 'mode': stat.S_IMODE(info.st_mode)}


def _check_file(record):
    if not isinstance(record, dict) or _file(record['path']) != record:
        _fail('file digest/mode mismatch')


def _commands(commands):
    if not isinstance(commands, list) or not commands:
        _fail('commands must be nonempty argv lists')
    for command in commands:
        if not isinstance(command, list) or not command or not all(isinstance(s, str) and s for s in command):
            _fail('commands must be nonempty argv lists')


def _binary(record, source):
    _check_file(record['file'])
    if not record['file']['mode'] & 0o111:
        _fail('binary is not executable')
    provenance = record['provenance']
    if provenance.get('kind') == 'native-gnu-pin':
        if not _oid(provenance.get('revision')) or provenance.get('sha256') != record['file']['sha256']:
            _fail('native GNU pin mismatch')
    elif provenance.get('kind') == 'build-receipt':
        _check_file(provenance['receipt'])
        receipt = json.loads(Path(provenance['receipt']['path']).read_text())
        from vacuity_binding import source_identity
        expected_source = source_identity(source)
        if (receipt.get('schema') != 1 or receipt.get('source') != expected_source
                or receipt.get('binary_sha256') != record['file']['sha256']
                or type(receipt.get('exit_code')) is not int or receipt['exit_code'] != 0
                or receipt.get('tracked_clean_start') is not True
                or receipt.get('tracked_clean_end') is not True):
            _fail('build receipt source/binary/provenance mismatch')
        _commands([receipt.get('command')])
    else:
        _fail('binary requires native GNU pin or build-receipt provenance')


def _relative(audit, relative):
    rel = Path(relative)
    if rel.is_absolute() or not rel.parts or '..' in rel.parts or str(rel) == NAME:
        _fail('invalid result path')
    path = Path(audit) / rel
    if not path.absolute().is_relative_to(Path(audit).absolute()):
        _fail('result outside audit directory')
    return path


def begin_run(repo, *, audit_dir, binaries, harness_files, commands):
    """Capture pre-run source/inputs; refuse an existing evidence directory."""
    audit = Path(audit_dir).absolute()
    if any(p.is_symlink() for p in (audit, *audit.parents)):
        _fail('symlink audit directory')
    if audit.exists() and (not audit.is_dir() or any(audit.iterdir())):
        _fail('audit directory must be absent or empty; never rebind old outputs')
    source = _source(repo)
    _commands(commands)
    if not binaries or not harness_files:
        _fail('binaries and harness files are required')
    bound = {}
    for role, value in binaries.items():
        provenance = copy.deepcopy(value.get('provenance', {}))
        if provenance.get('kind') == 'build-receipt':
            provenance = {'kind': 'build-receipt', 'receipt': _file(provenance['path'])}
        bound[role] = {'file': _file(value['path']), 'provenance': provenance}
        if provenance.get('kind') == 'native-gnu-pin' and role != 'gnu':
            _fail('native GNU provenance is only valid for the gnu role')
        _binary(bound[role], source)
    harness = [_file(path) for path in harness_files]
    audit.mkdir(parents=True, exist_ok=True)
    return {'schema': 1, 'source': source, 'audit_dir': str(audit.resolve()),
            'binaries': bound, 'harness': harness, 'commands': copy.deepcopy(commands)}


def _verify_inputs(manifest, repo=None):
    source = manifest['source']
    if _source(source['repo'] if repo is None else repo) != source:
        _fail('source revision/tree differs from audit source')
    _commands(manifest['commands'])
    if not manifest['binaries'] or not manifest['harness']:
        _fail('missing audit inputs')
    for role, binary in manifest['binaries'].items():
        if binary['provenance'].get('kind') == 'native-gnu-pin' and role != 'gnu':
            _fail('native GNU provenance is only valid for the gnu role')
        _binary(binary, source)
    for record in manifest['harness']:
        _check_file(record)


def finish_run(start, audit_dir, *, result_files):
    """Verify ending inputs and exclusively seal settled result files."""
    manifest = copy.deepcopy(start)
    audit = Path(audit_dir).resolve(strict=True)
    if str(audit) != manifest['audit_dir']:
        _fail('audit directory differs from start')
    _verify_inputs(manifest)
    if not result_files or len(set(map(str, result_files))) != len(result_files):
        _fail('nonempty distinct results required')
    manifest['results'] = {str(p): _file(_relative(audit, p)) for p in result_files}
    with (audit / NAME).open('x') as stream:
        json.dump(manifest, stream, indent=2, sort_keys=True)
        stream.write('\n')
    return manifest


def verify_manifest(audit_dir, repo=None):
    """Validate source and every sealed input/result before consumption.

    Consumers must supply their trusted repository root. Omitting repo checks
    only the manifest's self-declared repository, suitable for generic tooling,
    not for admitting production evidence into a particular checkout.
    """
    audit = Path(audit_dir).resolve(strict=True)
    try:
        _file(audit / NAME)
        manifest = json.loads((audit / NAME).read_text())
        if manifest['schema'] != 1 or manifest['audit_dir'] != str(audit):
            _fail('manifest schema/directory mismatch')
        _verify_inputs(manifest, repo)
        if not manifest['results']:
            _fail('manifest has no results')
        for relative in manifest['results']:
            require_result(manifest, audit, relative)
        return manifest
    except (OSError, KeyError, TypeError, json.JSONDecodeError) as error:
        raise ValueError('missing or invalid audit manifest') from error


def require_result(manifest, audit_dir, relative):
    """Return an authenticated result path; unlisted inputs are forbidden."""
    if str(relative) not in manifest['results']:
        _fail(f'result not sealed in manifest: {relative}')
    record = manifest['results'][str(relative)]
    path = _relative(audit_dir, relative)
    if record['path'] != str(path.absolute()):
        _fail('sealed result path mismatch')
    _check_file(record)
    return path
