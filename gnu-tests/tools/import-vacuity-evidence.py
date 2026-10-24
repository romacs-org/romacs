#!/usr/bin/env python3
"""Export sealed focused observations without assigning member classifications.

Historical HAND audits are deliberately unsupported. Run the mechanical
producer, then use AUDIT_DIR --format focused --output FILE. --check compares
against an existing export. Run-local MEMBER-EVIDENCE.json is generated beside its subject receipt.
The importer checks provenance, inventory and artifact integrity; it does not
replay producer parsers or independently certify their semantic observations.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import sys

from audit_manifest import verify_manifest, require_result, NAME as MANIFEST_NAME

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent.parent
GNU_REV = '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea'
GNU_SHA = 'a31ba7aa12a7e4bbdda6ab9e19dab8338340f81f8100c33ac3843d10e176c8d8'
CASE_KEYS = ('file', 'test', 'mutation', 'engine')


def build(audit: pathlib.Path) -> dict:
    raise ValueError('historical HAND audit unsupported; rerun mechanical producer')


def verified_run(audit: pathlib.Path) -> dict:
    manifest = verify_manifest(audit, repo=ROOT)
    binaries = manifest['binaries']
    if not {'gnu', 'romacs'} <= binaries.keys():
        raise ValueError('production audit requires both gnu and romacs binary roles')
    gnu = binaries['gnu']
    if (gnu['provenance'] != {'kind': 'native-gnu-pin', 'revision': GNU_REV, 'sha256': GNU_SHA}
            or gnu['file']['sha256'] != GNU_SHA):
        raise ValueError('GNU pin differs from required revision/hash')
    if binaries['romacs']['provenance'].get('kind') != 'build-receipt':
        raise ValueError('romacs binary requires build-receipt provenance')
    return manifest


def case_identity(row):
    if not isinstance(row, dict) or any(
            not isinstance(row.get(key), str) or not row[key].strip() for key in CASE_KEYS):
        raise ValueError('case requires nonempty file/test/mutation/engine identity')
    return tuple(row[key] for key in CASE_KEYS)


def validate_observations(data, manifest, audit):
    if not isinstance(data, dict) or type(data.get('schema')) is not int or data['schema'] != 1:
        raise ValueError('focused member-results requires schema 1')
    expected = data.get('expected_cases')
    rows = data.get('observations')
    if not isinstance(expected, list) or not expected or not isinstance(rows, list):
        raise ValueError('focused member-results requires nonempty expected_cases and observations')
    identities = []
    for case in expected:
        identity = case_identity(case)
        if set(case) != set(CASE_KEYS):
            raise ValueError('expected_cases must contain only identity fields')
        identities.append(identity)
    if len(set(identities)) != len(identities):
        raise ValueError('duplicate expected case identity')
    observed = [case_identity(row) for row in rows]
    if len(set(observed)) != len(observed) or set(observed) != set(identities):
        raise ValueError('observations do not match exact expected case inventory')
    for row in rows:
        if ('blocked' in row) == ('result' in row):
            raise ValueError('observation requires exactly one blocked or result status')
        if 'blocked' in row:
            if not isinstance(row['blocked'], str) or not row['blocked'].strip():
                raise ValueError('blocked status requires a reason')
        elif not isinstance(row['result'], dict) or not row['result']:
            raise ValueError('result status requires a nonempty object')
        paths = row.get('artifact_paths')
        if (not isinstance(paths, list) or len(paths) != 3
                or not all(isinstance(path, str) and path for path in paths)
                or len(set(paths)) != 3):
            raise ValueError('artifact_paths requires distinct probe, stdout and stderr paths')
        probes = [path for path in paths if not path.endswith(('.stdout', '.stderr'))]
        if len(probes) != 1 or set(paths) != {probes[0], probes[0] + '.stdout', probes[0] + '.stderr'}:
            raise ValueError('artifact_paths must name probe and its .stdout/.stderr')
        for path in paths:
            require_result(manifest, audit, path)


def provenance(audit: pathlib.Path, manifest: dict) -> dict:
    return {'probe_dir': str(audit), 'source': manifest['source'],
            'romacs_tree_probed': manifest['source']['tree'],
            'binaries': manifest['binaries'], 'commands': manifest['commands'],
            'harness': manifest['harness'],
            'manifest_sha256': hashlib.sha256((audit / MANIFEST_NAME).read_bytes()).hexdigest()}


def build_focused(audit: pathlib.Path) -> dict:
    manifest = verified_run(audit)
    observations = json.loads(require_result(manifest, audit, 'member-results.json').read_text())
    validate_observations(observations, manifest, audit)
    return {'schema': 2, 'about': 'Focused mechanical observations, not canonical member classifications.',
            'provenance': provenance(audit, manifest), 'observations': observations}


def import_subject(audit, output=None):
    from vacuity_classifier import replay
    from vacuity_binding import make_receipt
    manifest = verified_run(audit)
    data = json.loads(require_result(manifest, audit, 'member-results.json').read_text())
    validate_observations(data, manifest, audit)
    if data.get('classification') != 'subject-only-v1':
        raise ValueError('subject import requires reviewed subject-only producer')
    verdicts = replay(data, audit, require_result, manifest)
    print(json.dumps(verdicts, sort_keys=True))
    if verdicts['ert-test-should-error-argument'] != 'ASSERTION-SENSITIVE':
        raise ValueError('should-error-argument is not proven assertion-sensitive; no evidence changed')
    receipt = make_receipt(ROOT, audit, manifest, verdicts)
    if output is None:
        raise ValueError('subject import requires an external --output receipt path')
    if output.resolve().is_relative_to(ROOT.resolve()):
        raise ValueError('external subject receipt must be outside measured source')
    with output.open('x') as stream:
        stream.write(json.dumps(receipt, indent=1, sort_keys=True) + '\n')
    from member_evidence import write
    write(ROOT, output)


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('audit')
    ap.add_argument('--check', action='store_true')
    ap.add_argument('--format', choices=('legacy', 'focused', 'subject'), default='legacy')
    ap.add_argument('--output', type=pathlib.Path)
    args = ap.parse_args(argv)
    if args.format == 'subject':
        if args.check:
            ap.error('subject import does not support --check')
        try:
            import_subject(pathlib.Path(args.audit).resolve(), args.output)
        except (ValueError, OSError) as error:
            ap.exit(2, f'audit import refused: {error}\n')
        return 0
    output = args.output
    if output is None:
        ap.error('focused observations require a separate --output; never replace MEMBER-EVIDENCE.json')
    builder = build if args.format == 'legacy' else build_focused
    try:
        text = json.dumps(builder(pathlib.Path(args.audit).resolve()), indent=1, sort_keys=True) + '\n'
    except (ValueError, OSError) as error:
        ap.exit(2, f'audit import refused: {error}\n')
    if args.check:
        same = output.is_file() and output.read_text() == text
        print(f'{output.name}: {"up to date" if same else "DIFFERS from a regeneration"}')
        return 0 if same else 1
    output.write_text(text)
    print(f'wrote focused observations to {output}; no canonical class changes')
    return 0


if __name__ == '__main__':
    sys.exit(main())
