#!/usr/bin/env python3
"""Check required self-reported diagnostics, separately from the pixel oracle."""
import argparse
import json
from pathlib import Path


def check(root):
    reports = []
    for variant in ('normal', 'low-gc', 'forced-gc'):
        records = []
        for line in (root / variant / 'binary.stderr').read_text().splitlines():
            try:
                item = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(item, dict):
                records.append(item)
        paints = [r for r in records if r.get('diagnostic') == 'paint']
        assert len(paints) == 2, (variant, 'expected two paint diagnostics', paints)
        assert all(r['unresolved_face_text_total'] == 0 for r in paints), (variant, paints)
        assert paints[1]['buffer_version'] > paints[0]['buffer_version'], (variant, paints)
        gc = [r for r in records if r.get('diagnostic') == 'gc-between-snapshot-and-paint']
        if variant == 'forced-gc':
            assert len(gc) == 2 and all(r['witness_moved'] is True and
                                       r['collections_after'] > r['collections_before'] for r in gc), gc
        reports.append({'variant': variant, 'paints': paints, 'moving_gc': gc})
    return {'diagnostic_requirements_hold': True,
            'not_pixel_acceptance': True,
            'note': 'Renderer/VM self-reports are diagnostics, not independent evidence.',
            'runs': reports}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('root', type=Path)
    args = ap.parse_args()
    result = check(args.root)
    (args.root / 'diagnostics.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))


if __name__ == '__main__':
    main()
