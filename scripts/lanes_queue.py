"""Ephemeral, verdict-only batch integration; canonical refs are never written.

A verdict transfers only if an independent ordered replay has the identical
Git tree. Conflicts are ejected, never resolved. Activation is blocked until
fail-closed verification and deterministic generated artifacts land on master.
"""
import os as _portable_os
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time
import uuid


REQUIRED_MASTER_COMMITS = {
    'coverage-fail-closed': 'ebd8ad956c2957789d0a8378311d293cb708b0b8',
    'verification-holes': '8d7901c72dfe9d2e514a129042dd7ac893422890',
    'mutation-contract': 'd4c9b7d70359531b6c9c5504035fac31fe86fdd2',
    'deterministic-snapshot': None,
}
DEFAULT_NOTES = Path(_portable_os.environ['ROMACS_QUEUE_NOTES'])
SCRATCH = Path(str(_PortablePath(_portable_tempfile.gettempdir()) / ''))
SLOT = Path(_portable_os.environ['ROMACS_GATE_SLOT'])
CHECKER = SLOT.with_name('romacs-receipt-check.py')


class QueueRefused(RuntimeError):
    pass


class MergeConflict(QueueRefused):
    def __init__(self, tip):
        super().__init__(f'manual conflict resolution forbidden; eject {tip}')
        self.tip = tip


def require_dependencies(api, repo, master, requirements):
    missing = [name for name, commit in requirements.items()
               if not commit or not api.is_ancestor(repo, commit, master)]
    if missing:
        raise QueueRefused('fail-closed verification/determinism absent from master: ' + ', '.join(missing))


def assert_replay_tree(expected, observed):
    if expected != observed:
        raise QueueRefused(f'verdict transfer refused: gated tree {expected} != replay tree {observed}')


def select_batch(tips, build, gate):
    remaining, ejected = list(tips), []
    while remaining:
        try:
            candidate = build(remaining)
        except MergeConflict as conflict:
            if conflict.tip not in remaining:
                raise
            remaining.remove(conflict.tip)
            ejected.append(conflict.tip)
            continue
        if gate(candidate):
            return candidate, ejected
        culprit = remaining[-1]
        for count in range(1, len(remaining)):
            if not gate(build(remaining[:count])):
                culprit = remaining[count - 1]
                break
        remaining.remove(culprit)
        ejected.append(culprit)
    raise QueueRefused('no candidates remain after conflict/RED ejections')


def finish_batch(gated, replayed, prepare):
    assert_replay_tree(gated, replayed)
    prepare()


def atomic_json(path, value):
    temporary = path.with_name(path.name + '.tmp-' + uuid.uuid4().hex)
    with temporary.open('x') as output:
        json.dump(value, output, indent=2, sort_keys=True)
        output.write('\n')
        output.flush()
        os.fsync(output.fileno())
    os.replace(temporary, path)


GUARDIAN = '''import json,os,shutil,sys,tempfile
from pathlib import Path
root=Path(tempfile.mkdtemp(prefix="lanes-queue-",dir=sys.argv[1]))
try:
    print(root,flush=True)
    sys.stdin.buffer.read()
finally:
    shutil.rmtree(root)
'''


@contextlib.contextmanager
def private_queue(api, repo, root, receipt):
    """An independent EOF guardian removes private refs even on worker SIGKILL.

    Queue refs exist only in a private object store, never in the canonical
    repository. SIGKILL of the whole machine/guardian cannot execute cleanup;
    canonical master and canonical refs are nevertheless untouched by design.
    """
    guardian = subprocess.Popen([sys.executable, '-c', GUARDIAN, str(root)],
                                stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                text=True, start_new_session=True, close_fds=True)
    try:
        line = guardian.stdout.readline().strip()
        if not line:
            raise QueueRefused('queue cleanup guardian failed to start')
        path = Path(line) / 'repo'
        path.mkdir()
        api.git(path, 'init', '-q')
        common = api.git(repo, 'rev-parse', '--path-format=absolute', '--git-common-dir').stdout.strip()
        (path / '.git/objects/info/alternates').write_text(str(Path(common) / 'objects') + '\n')
        yield path
    finally:
        guardian.stdin.close()
        guardian.stdout.close()
        guardian.wait(timeout=30)


def tree(api, repo, commit):
    return api.git(repo, 'rev-parse', commit + '^{tree}').stdout.strip()


def build_merges(api, private, base, tips, queue_ref=None, signed=False):
    """Native merges only: never accept manual resolutions or a queue parent."""
    api.git(private, 'checkout', '--detach', '-q', base)
    for tip in tips:
        result = api.git(private, '-c', 'core.hooksPath=/dev/null', '-c', 'merge.autoStash=false',
                         '-c', 'rerere.enabled=false', 'merge', '--no-ff', '--no-commit', tip, check=False)
        if result.returncode:
            conflicts = api.git(private, 'diff', '--name-only', '--diff-filter=U').stdout.strip()
            api.git(private, 'merge', '--abort', check=False)
            if not conflicts:
                raise QueueRefused(f'merge infrastructure failed; no tip ejected: {result.stderr.strip()}')
            raise MergeConflict(tip)
        if api.git(private, 'rev-parse', '-q', '--verify', 'MERGE_HEAD', check=False).returncode == 0:
            api.git(private, '-c', 'core.hooksPath=/dev/null', '-c', 'user.name=Thanos Apollo',
                    '-c', 'user.email=public@thanosapollo.org', '-c', f'commit.gpgsign={str(signed).lower()}',
                    'commit', '-q', '-m', f'{"Speculate" if queue_ref else "Merge"} accepted tip {tip}')
    commit = api.rev(private, 'HEAD')
    if queue_ref:
        api.git(private, 'update-ref', queue_ref, commit)
    return dict(commit=commit, tree=tree(api, private, commit), tips=list(tips))


def strict_receipt(receipt, expected_tree):
    if not (receipt.get('schema') == 2 and receipt.get('verdict') == 'GREEN'
            and receipt.get('complete') is True and receipt.get('tree') == expected_tree
            and receipt.get('tier') == 2 and receipt.get('mode') == 'full' and receipt.get('expr') == 'all()'):
        raise QueueRefused('candidate lacks an exact-tree complete schema-2 GREEN full receipt')


def run_checked(command, cwd=None, env=None):
    result = subprocess.run([str(x) for x in command], cwd=cwd, env=env,
                            capture_output=True, text=True)
    if result.returncode:
        raise QueueRefused(f'command refused ({result.returncode}): {command}\n{result.stdout}{result.stderr}')
    return result.stdout


def validate_candidate_receipt(api, canonical, tip, path):
    path = Path(path).absolute()
    receipt = json.loads(path.read_text())
    expected_tree = tree(api, canonical, tip)
    strict_receipt(receipt, expected_tree)
    match = re.fullmatch(r'([A-Za-z0-9_-]+)-[0-9]{8}-[0-9]{6}-[^/]+', path.parent.name)
    if path.name != 'receipt.json' or not match:
        raise QueueRefused('candidate receipt has no explicit gate run identity')
    gate_bytes = api.git(canonical, 'show', tip + ':scripts/gate').stdout.encode()
    run_checked([sys.executable, CHECKER, '--run-dir', path.parent, '--tree', expected_tree,
                 '--label', match[1], '--gate-sha', hashlib.sha256(gate_bytes).hexdigest(),
                 '--runs-root', path.parent.parent])


def gate_candidate(api, private, candidate, label, output, sequence):
    before = set((api.REAL_HOME / f'.cache/lanes/{label}/.cache/emax-gate/runs').glob('*'))
    command = [api.DEFAULT_REPO / 'scripts/lane-run', label, '-C', private, '--',
               'bash', SLOT, 'run', label, '--', 'python3', 'scripts/gate',
               '--lane', label, '--jobs', '3', '--mem', '34G', 'full']
    result = subprocess.run([str(x) for x in command], capture_output=True, text=True)
    log = output / f'gate-{sequence}.log'
    log.write_text(result.stdout + result.stderr)
    matches = re.findall(r'^gate: (GREEN|RED): full \([0-9]+s\); logs in (/\S+)$', result.stdout, re.M)
    if len(matches) != 1:
        raise QueueRefused(f'gate did not emit exactly one run identity; see {log}')
    verdict, directory = matches[0]
    run = Path(directory)
    runs_root = api.REAL_HOME / f'.cache/lanes/{label}/.cache/emax-gate/runs'
    if run.parent.resolve() != runs_root.resolve() or run in before or run.is_symlink():
        raise QueueRefused('gate run is preexisting, redirected, or symlinked')
    receipt_path = run / 'receipt.json'
    fd = os.open(receipt_path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(fd) as source:
        receipt = json.load(source)
    if verdict == 'RED':
        if result.returncode == 0 or receipt.get('verdict') != 'RED' or receipt.get('tree') != candidate['tree']:
            raise QueueRefused('contradictory RED gate identity')
        return False, receipt, run
    if result.returncode:
        raise QueueRefused(f'GREEN output with nonzero gate exit {result.returncode}')
    strict_receipt(receipt, candidate['tree'])
    gate_sha = hashlib.sha256((private / 'scripts/gate').read_bytes()).hexdigest()
    preexisting = output / f'before-{sequence}.txt'
    preexisting.write_text('\n'.join(sorted(p.name for p in before)) + '\n')
    run_checked([sys.executable, CHECKER, '--run-dir', run, '--tree', candidate['tree'],
                 '--label', label, '--gate-sha', gate_sha, '--preexisting', preexisting,
                 '--runs-root', runs_root])
    reused = run_checked([api.DEFAULT_REPO / 'scripts/lane-run', label, '-C', private, '--',
                          'python3', 'scripts/gate', '--lane', label, 'reuse', run, 'full'])
    if reused.strip() != 'gate: reuse GREEN':
        raise QueueRefused('gate reuse failed exact environment certification')
    if api.git(private, 'status', '--porcelain', '--untracked-files=all').stdout:
        raise QueueRefused('gate dirt changed the candidate checkout')
    return True, receipt, run


def transfer_objects(api, private, canonical, commit):
    packed = subprocess.run(['git', '-C', str(private), 'pack-objects', '--stdout', '--revs', '--local'],
                            input=(commit + '\n').encode(), capture_output=True, check=True)
    result = subprocess.run(['git', '-C', str(canonical), 'index-pack', '--stdin'],
                            input=packed.stdout, capture_output=True)
    if result.returncode:
        raise QueueRefused('cannot import off-ref replay objects for exact record preparation')
    if api.rev(canonical, commit) != commit:
        raise QueueRefused('imported replay object failed readback')


def prepare_record(api, canonical, notes, commit, output, notes_commit):
    """Prepare once in an isolated committed-notes snapshot, never live notes."""
    snapshot = output / 'record'
    snapshot.mkdir()
    archive = subprocess.run(['git', '-C', str(notes), 'archive', notes_commit], capture_output=True, check=True)
    import tarfile
    import io
    with tarfile.open(fileobj=io.BytesIO(archive.stdout)) as source:
        source.extractall(snapshot, filter='data')
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1')
    result = run_checked([sys.executable, snapshot / 'reference/record_integrity.py', '--root', snapshot,
                          '--prepare-target', commit], env=env)
    run_checked([sys.executable, snapshot / 'reference/record_integrity.py', '--root', snapshot,
                 '--landing-target', commit], env=env)
    (output / 'record-preparation.log').write_text(result)
    return snapshot


def add_parser(sub):
    parser = sub.add_parser('queue', help='verdict-only ordered batch; never moves master')
    parser.add_argument('--tip', action='append', required=True, help='ordered immutable candidate/ref; repeat')
    parser.add_argument('--receipt', action='append', required=True, help='exact-tree candidate receipt, in tip order')
    parser.add_argument('--output', type=Path, required=True, help='new evidence directory outside every worktree')
    parser.add_argument('--lane', default='bq', help='gate environment/slot label (at most four alphanumerics)')
    parser.add_argument('--notes', type=Path, default=DEFAULT_NOTES)
    parser.add_argument('--dry-run', action='store_true', required=True, help='required; publication is owner-only')


def cmd_queue(args, registry, api):
    output = args.output.resolve()
    canonical = Path(args.repo).resolve()
    receipt_path = output / 'receipt.json'
    receipt_created = False
    receipt = dict(schema=2, verdict='RED', complete=False, tree=None, tier=2, mode='full', expr='all()',
                   gate_sha=None, oracle_sha=None, runner_sha=None, outcomes={}, queue=True,
                   started=time.time(), attempts=[], ejected=[], master=None)
    saved = dict(os.environ)
    for key in list(os.environ):
        if key.startswith('GIT_') or key in ('BASH_ENV', 'ENV'):
            del os.environ[key]
    os.environ['HOME'] = str(api.REAL_HOME)
    os.environ['GNUPGHOME'] = str(api.REAL_HOME / '.gnupg')
    try:
        if not re.fullmatch('[A-Za-z0-9]{1,4}', args.lane):
            raise QueueRefused('queue lane must be at most four literal alphanumerics')
        if len(args.tip) != len(args.receipt) or not args.tip:
            raise QueueRefused('one explicit receipt is required per ordered tip')
        for line in api.git(canonical, 'worktree', 'list', '--porcelain').stdout.splitlines():
            if line.startswith('worktree ') and output.is_relative_to(Path(line[9:]).resolve()):
                raise QueueRefused('queue evidence must be outside every worktree')
        output.mkdir(parents=True, exist_ok=False)
        receipt_created = True
        atomic_json(receipt_path, receipt)
        master = api.rev(canonical, 'refs/heads/master')
        receipt['master'] = master
        require_dependencies(api, canonical, master, REQUIRED_MASTER_COMMITS)
        notes_commit = api.rev(args.notes, 'HEAD')
        run_checked([sys.executable, args.notes / 'reference/record_committed.py',
                     '--root', args.notes, '--code-repository', canonical])
        tips = [api.rev(canonical, tip) for tip in args.tip]
        if len(set(tips)) != len(tips):
            raise QueueRefused('duplicate queue tips')
        for index, (tip, path) in enumerate(zip(tips, args.receipt)):
            validate_candidate_receipt(api, canonical, tip, path)
            for commit in api.git(canonical, 'rev-list', f'{master}..{tip}').stdout.split():
                if api.git(canonical, 'verify-commit', commit, check=False).returncode:
                    raise QueueRefused(f'candidate contains unverifiable signature: {commit}')
            check = argparse.Namespace(repo=str(canonical), lane=None, tip=tip, base=None,
                                       surface=None, forbid=None, canonical=str(canonical), receipt=path,
                                       min_block=3, json_out=str(output / f'check-{index}.json'))
            if api.cmd_check(check, registry):
                raise QueueRefused(f'existing lanes check refused {tip}')
        with private_queue(api, canonical, SCRATCH, receipt_path) as private:
            ref = 'refs/heads/queue/' + time.strftime('%Y%m%d') + '-' + uuid.uuid4().hex[:12]
            receipt['ephemeral_ref'] = ref
            receipt['private_repository'] = str(private)
            def build(batch):
                return build_merges(api, private, master, batch, ref)
            def gate(candidate):
                expected_gate = api.git(canonical, 'rev-parse', master + ':scripts/gate').stdout.strip()
                actual_gate = api.git(private, 'rev-parse', candidate['commit'] + ':scripts/gate').stdout.strip()
                if expected_gate != actual_gate:
                    raise QueueRefused('a candidate may not certify a changed gate')
                green, result, directory = gate_candidate(api, private, candidate, args.lane, output,
                                                          len(receipt['attempts']))
                receipt['attempts'].append(dict(tips=candidate['tips'], tree=candidate['tree'],
                                                verdict=result['verdict'], run=str(directory)))
                receipt.update({key: result.get(key) for key in ('gate_sha', 'oracle_sha', 'runner_sha', 'outcomes')})
                atomic_json(receipt_path, receipt)
                return green
            candidate, ejected = select_batch(tips, build, gate)
            receipt['ejected'] = ejected
            current = api.rev(canonical, 'refs/heads/master')
            if current != master:
                raise QueueRefused('master advanced while gating; start a fresh batch')
            replay = build_merges(api, private, current, candidate['tips'], signed=True)
            receipt['replay_tree'] = replay['tree']
            def prepare():
                transfer_objects(api, private, canonical, replay['commit'])
                return prepare_record(api, canonical, args.notes, replay['commit'], output, notes_commit)
            finish_batch(candidate['tree'], replay['tree'], prepare)
            if api.rev(canonical, 'refs/heads/master') != master:
                raise QueueRefused('master advanced during record preparation; verdict is stale')
            if api.rev(args.notes, 'HEAD') != notes_commit:
                raise QueueRefused('written record advanced during the batch; prepare a fresh snapshot')
            receipt.update(tree=candidate['tree'], replay_commit=replay['commit'], tips=candidate['tips'],
                           verdict='GREEN', complete=True, record_preparations=1, dry_run=True)
            api.git(private, 'update-ref', '-d', ref)
        receipt['ephemeral_ref_discarded'] = True
        atomic_json(receipt_path, receipt)
        print(json.dumps(receipt, indent=2))
        return 0
    except (QueueRefused, OSError, ValueError, subprocess.SubprocessError, SystemExit) as error:
        receipt.update(verdict='RED', complete=False, reason=str(error))
        if receipt_created:
            atomic_json(receipt_path, receipt)
        print('lanes queue: REFUSED: ' + str(error), file=sys.stderr)
        return 2
    finally:
        os.environ.clear()
        os.environ.update(saved)
