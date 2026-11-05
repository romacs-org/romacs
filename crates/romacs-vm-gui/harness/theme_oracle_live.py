#!/usr/bin/env python3
"""Theme semantics against pinned GNU, both engines graphical.

One script (theme_oracle_steps.el) runs in pinned GNU and in Romacs, each in
its own private headless labwc, and every step's dump is compared: test
faces' attributes per frame, `theme-face', frame parameters, the enabled
themes and an event log. Faces and attributes only one engine has are
skipped; what differs by design is in KNOWN with its reason.

usage: theme_oracle_live.py --binary PATH --gnu PATH --output DIR
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

import live_oracle as live

HERE = Path(__file__).resolve().parent
# (step, frame, face, attribute), * for any. Each differs by design:
KNOWN = {
    # The engines' fonts are their own, as is the pointer's colour.
    *(('*', '*', 'default', a) for a in (':family', ':foundry', ':height', ':width')),
    ('*', '*', 'param', 'mouse-color'),
    # GNU's initial frame has a border colour before any face sets one.
    ('baseline', 'root', 'param', 'border-color'),
    # A child frame showing its root's faces keeps no per-frame copy of
    # what they imply, and shows the root's `supports' answers
    # (VM2-CHILD-FRAME).
    *(('*', 'child', 'param', p) for p in ('background-mode', 'display-type', 'cursor-color', 'border-color')),
    ('*', 'child', 'ot-supports', ':foreground'),
}
# GNU fills `default''s slots from the frame's font when it opens one, and
# Romacs leaves them unspecified: these (attribute, GNU, Romacs) only.
FONT_FILLED = {(':slant', 'normal', 'unspecified'), (':weight', 'regular', 'unspecified')}
FONT_SLOTS = {attr: gnu for attr, gnu, _ in FONT_FILLED}
# Fewer steps than this in GNU's dump means the script stopped early.
MINIMUM_STEPS = 50


def known(key):
    return any(all(p in ('*', k) for p, k in zip(pattern, key)) for pattern in KNOWN)


def font_filled(key, gnu, romacs):
    """Whether a difference in `default' is GNU's frame font: a root frame's
    exact pair, or a child frame's, which shows its root's value."""
    step, frame, face, attr = key
    if face != 'default' or FONT_SLOTS.get(attr) != gnu[key]:
        return False
    if frame == 'child':
        return romacs[key] == romacs.get((step, 'root', face, attr))
    return (attr, gnu[key], romacs[key]) in FONT_FILLED


def differences(gnu, romacs):
    """The rows that disagree, and those one engine lacks: a step that never
    ran must not pass for want of anything to compare."""
    diffs = []
    for key in sorted(gnu.keys() & romacs.keys()):
        step, frame, face, attr = key
        if gnu[key] != romacs[key] and not known(key) and not font_filled(key, gnu, romacs):
            diffs.append({'step': step, 'frame': frame, 'face': face, 'attr': attr, 'gnu': gnu[key], 'romacs': romacs[key]})
    return diffs, sorted(key for key in gnu.keys() ^ romacs.keys() if not known(key))


def comparison_can_fail():
    """The verdict's own check: each corruption below must be reported."""
    root, child = ('s', 'root', 'default', ':weight'), ('s', 'child', 'default', ':weight')
    good = {root: 'bold', child: 'regular'}
    assert not differences(good, {root: 'bold', child: 'bold'})[0], 'a child shows its root'
    assert differences(good, {root: 'bold', child: 'ultrabold'})[0], 'a child differing from its root'
    plain = {('s', 'root', 'default', ':slant'): 'normal'}
    assert not differences(plain, {('s', 'root', 'default', ':slant'): 'unspecified'})[0], 'the font-filled pair'
    assert differences(plain, {('s', 'root', 'default', ':slant'): 'italic'})[0], 'a wrong value in a font-filled slot'
    assert differences({('s', 'root', 'x', ':box'): 't'}, {})[1], 'a row Romacs lacks'


def run_engine(name, command, args):
    """COMMAND in a private compositor; the path of its dump."""
    out = args.output / f'{name}.tsv'
    out.unlink(missing_ok=True)
    with tempfile.TemporaryDirectory(prefix='ot-', dir=os.environ.get('XDG_RUNTIME_DIR')) as tmp:
        root = Path(tmp)
        home, config = root / 'home', root / 'config'
        (home / 'config' / 'romacs').mkdir(parents=True)
        config.mkdir()
        (config / 'rc.xml').write_text(live.RC_XML)
        init = f'(load "{HERE / "theme_oracle_steps.el"}" nil t)\n'
        (home / 'config' / 'romacs' / 'init.el').write_text(init)
        (home / 'init.el').write_text(init)
        env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home), 'LANG': 'C.UTF-8',
               'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
               'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
               'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root), 'OT_OUT': str(out),
               'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
               'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop', 'GDK_BACKEND': 'wayland'}
        wlr = dict(item.split('=', 1) for item in os.environ.get('ROMACS_HARNESS_WLR_ENV', '').split())
        for key in ('CARGO_HOME', 'ROMACS_RUNTIME_ROOT'):
            if key in os.environ:
                env[key] = os.environ[key]
        procs = []
        try:
            with (args.output / f'{name}.log').open('w') as log:
                procs.append(subprocess.Popen([live.LABWC, '-C', str(config)], env={**env, **wlr}, stdout=log, stderr=log))
                end = time.time() + 15
                while not any(p.is_socket() for p in root.glob('wayland-*')):
                    assert time.time() < end, 'compositor'
                    time.sleep(.05)
                env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
                procs.append(subprocess.Popen(['/usr/bin/wtype', '-s', '600000', '-k', 'Shift_L'], env=env, stdout=log, stderr=log))
                subprocess.run([live.RANDR, '--output', 'HEADLESS-1', '--custom-mode', '1600x1000'], env=env, check=True, timeout=15)
                time.sleep(.3)
                procs.append(subprocess.Popen(command + ([] if name == 'romacs' else ['-Q', '-l', str(home / 'init.el')]),
                                              env=env if name == 'romacs' else {**env, **wlr}, cwd=home, stdin=subprocess.DEVNULL, stdout=log, stderr=log))
                time.sleep(args.settle)
                subprocess.run(['/usr/bin/wtype', '-k', 'F5'], env=env, check=True, timeout=30)
                end = time.time() + args.timeout
                while not (out.exists() and out.read_text(errors='replace').endswith('END\n')):
                    assert time.time() < end, f'{name}: no END in {out}'
                    time.sleep(.2)
        finally:
            for p in reversed(procs):
                if p.poll() is None:
                    p.terminate()
                    try:
                        p.wait(5)
                    except subprocess.TimeoutExpired:
                        p.kill()
    return out


def rows(path):
    table = {}
    for line in path.read_text(errors='replace').splitlines():
        cells = line.split('\t')
        if len(cells) == 5:
            table[tuple(cells[:4])] = cells[4]
    return table


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--binary', type=Path, required=True)
    ap.add_argument('--gnu', type=Path, default=os.environ.get('ROMACS_GNU_ORACLE'))
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--settle', type=float, default=12)
    ap.add_argument('--timeout', type=float, default=120)
    args = ap.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    comparison_can_fail()
    gnu = rows(run_engine('gnu', [str(args.gnu)], args))
    romacs = rows(run_engine('romacs', [str(args.binary.resolve()), '--loadup'], args))
    steps = {key[0] for key in gnu}
    if 'done' not in steps or len(steps) < MINIMUM_STEPS:
        raise SystemExit(f'GNU ran {len(steps)} steps, fewer than the {MINIMUM_STEPS} this oracle has')
    diffs, missing = differences(gnu, romacs)
    report = {'compared': len(gnu.keys() & romacs.keys()), 'differences': len(diffs), 'missing': len(missing),
              'first_missing': missing[:200], 'first': diffs[:200]}
    (args.output / 'report.json').write_text(json.dumps(report, indent=1) + '\n')
    print(json.dumps({'compared': report['compared'], 'differences': len(diffs), 'missing': len(missing)}))
    for key in missing[:30]:
        print('missing', key, 'gnu' if key in gnu else 'romacs')
    for d in diffs[:60]:
        print(f"{d['step']:32} {d['frame']:6} {d['face']:20} {d['attr']:18} gnu={d['gnu'][:50]} romacs={d['romacs'][:50]}")
    return 1 if diffs or missing else 0


if __name__ == '__main__':
    raise SystemExit(main())
