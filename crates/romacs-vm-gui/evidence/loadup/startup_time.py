#!/usr/bin/env python3
"""Launch-to-ready time of the real GUI per launch mode, on the compositor.

    startup_time.py BINARY OUT_DIR [REPS]

Time from the launch command to the VM's first `ready` reply on stdout (the
first frame has been painted and activated), with tools/romacs's
environment (jlib's command line), on Hyprland workspace 2. Prints one line
per run and the median per mode. The window is closed after each run.
"""
import json, os, statistics, sys, time
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'journey'))
import jlib  # noqa: E402

binary, out = sys.argv[1], Path(sys.argv[2])
reps = int(sys.argv[3]) if len(sys.argv) > 3 else 3
FLAGS = {'default': '', 'loadup': '--loadup', 'gnu-only': '--gnu-lisp'}
results = {}
for rep in range(reps):
    for mode, flag in FLAGS.items():
        d = out / f'{mode}-{rep}'; d.mkdir(parents=True, exist_ok=True)
        stdout = d / 'stdout.jsonl'
        if stdout.exists(): stdout.unlink()
        before = {c['pid'] for c in jlib.clients()}
        envs = f'ROMACS_LISP_DIR={jlib.ROOT}/lisp/gnu ROMACS_GUI_FONT=Iosevka ROMACS_GUI_FONT_SIZE=14'
        cmd = f'env -u RUST_MIN_STACK -u LD_LIBRARY_PATH {envs} {binary} {flag} >{stdout} 2>{d / "stderr.log"}'
        t0 = time.monotonic()
        jlib.hy('eval', f'hl.exec_cmd({json.dumps("sh -c " + json.dumps(cmd))}, {{workspace="2 silent"}})')
        ready = None
        while time.monotonic() - t0 < 120:
            try:
                line = stdout.read_text().splitlines()[:1]
                if line and json.loads(line[0]).get('ready'):
                    ready = time.monotonic() - t0; break
            except (FileNotFoundError, ValueError):
                pass
            time.sleep(0.01)
        own = [c for c in jlib.clients() if c['class'] == 'romacs' and c['pid'] not in before]
        for c in own:
            try: os.kill(c['pid'], 15)
            except ProcessLookupError: pass
        if ready is None: raise RuntimeError(f'{mode}: no ready reply')
        results.setdefault(mode, []).append(ready)
        print(f'{mode:10s} run {rep}: launch-to-ready {ready:.2f}s', flush=True)
        time.sleep(1.0)
for mode, xs in results.items():
    print(f'MEDIAN {mode:10s} {statistics.median(xs):.2f}s over {len(xs)} runs: {[round(x, 2) for x in xs]}')
