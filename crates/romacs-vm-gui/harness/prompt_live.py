#!/usr/bin/env python3
"""A prompt read mid-command, on a real (headless) display, keys from a real
Wayland keyboard, against GNU Emacs driven by the SAME keystrokes.

Both editors run in their own private labwc (WLR_BACKENDS=headless,
WLR_RENDERER=pixman: valid for correctness, not for performance). Keys go
through the compositor's virtual keyboard (wtype), i.e. the editor's native
wl_keyboard path. Each state is captured with grim and its bottom (minibuffer)
row is read by tesseract. The EXPECTED text is GNU's: GNU 32.0.50 batch
(evidence/prompt/gnu-prompt.out) and GNU's own pixels read by the same OCR.
VM2's renderer is never its own oracle.

Steps, typed into *scratch* in both editors:
  prompt   (read-from-minibuffer "Name: ")  C-x C-e    -> minibuffer "Name:"
  typed    a b c                                      -> "Name: abc"
  returned RET                                        -> echo "\"abc\""
  quit     (condition-case q (read-from-minibuffer "Q: ") (quit (list 'quit q)))
           C-x C-e z z C-g                            -> echo "(quit (minibuffer-quit))"
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

import live_oracle as live

W, H = 800, 480
FORM1 = '(read-from-minibuffer "Name: ")'
FORM2 = "(condition-case q (read-from-minibuffer \"Q: \") (quit (list 'quit q)))"
EXPECT = {  # GNU 32.0.50: gnu-prompt.out and GNU's own screen (gnu/ run).
    'prompt': 'Name:', 'typed': 'Name: abc', 'returned': '"abc"',
    'quit-prompt': 'Q: zz', 'quit': "(quit (minibuffer-quit))",
}


def wtype(env, *args):
    subprocess.run(['/usr/bin/wtype', '-d', '40', *args], env=env, check=True, timeout=30)


def bottom_rgb(png: Path, rows: int) -> bytes:
    return subprocess.run(['magick', str(png), '-gravity', 'South', '-crop', f'{W}x{rows}+0+0', '+repage',
                          'rgb:-'], capture_output=True, check=True).stdout


def cursor_block(png: Path, rows: int):
    """Leftmost x of a solid block (>= 5 adjacent columns each inked over
    >= 14 of the bottom ROWS) or None. Editor-neutral: the same rule finds
    GNU's and VM2's box cursor; no glyph has a column that tall and wide."""
    raw = bottom_rgb(png, rows)
    px = [tuple(raw[i:i + 3]) for i in range(0, len(raw), 3)]
    background = max(set(px), key=px.count)
    ink = lambda p: sum(abs(p[k] - background[k]) for k in range(3)) > 60
    tall = [sum(ink(px[y * W + x]) for y in range(rows)) >= 14 for x in range(W)]
    run = 0
    for x, t in enumerate(tall):
        run = run + 1 if t else 0
        if run >= 5:
            return x - 4
    return None


def ocr_bottom(png: Path, out: Path, rows: int, limit) -> str:
    """OCR the minibuffer row left of LIMIT (the cursor block, when there is
    one): the cursor is checked separately, the text is read unrepaired."""
    crop = out.with_suffix('.mini.png')
    width = limit if limit else W
    raw = bottom_rgb(png, rows)
    light = sum(raw) / len(raw) > 127
    subprocess.run(['magick', str(png), '-gravity', 'South', '-crop', f'{W}x{rows}+0+0', '+repage', '+gravity',
                    '-crop', f'{width}x{rows}+0+0', '+repage', '-resize', '200%', '-colorspace', 'Gray',
                    *([] if light else ['-negate']), '-bordercolor', 'white', '-border', '20',
                    'PNG24:' + str(crop)], check=True)
    return subprocess.run(['tesseract', str(crop), '-', '--psm', '7'], capture_output=True, text=True,
                          check=True).stdout.strip()


def cursor_pixels(png: Path, rows: int, rgb=(0x90, 0x99, 0xd9), tol=12) -> int:
    """Pixels of the selected-cursor colour in the bottom ROWS (VM2's cursor
    is #9099d9, main.rs). GNU's screen shows a cursor in an active minibuffer;
    this checks VM2 draws one too, without trusting VM2's own diagnostics."""
    raw = subprocess.run(['magick', str(png), '-gravity', 'South', '-crop', f'{W}x{rows}+0+0', '+repage',
                          'rgb:-'], capture_output=True, check=True).stdout
    return sum(1 for i in range(0, len(raw), 3)
               if all(abs(raw[i + k] - rgb[k]) <= tol for k in range(3)))


def wait_prompt(log: Path, text: str, timeout: float, seen: int) -> int:
    """VM2 only: block until a prompt paint showing TEXT has RENDERED (the
    evaluator waited for the renderer). Synchronisation, not evidence."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        lines = log.read_text(errors='replace').splitlines()
        for n, line in enumerate(lines[seen:], seen):
            if '"prompt-paint"' in line and json.loads(line).get('minibuffer') == text:
                return n + 1
        time.sleep(.05)
    raise TimeoutError(f'no prompt paint showing {text!r}')


def run(args, editor: str) -> dict:
    out = args.output / editor
    out.mkdir(parents=True)
    with tempfile.TemporaryDirectory(prefix='gpr-', dir=args.runtime_root) as temp:
        runtime = Path(temp); runtime.chmod(0o700)
        config = runtime / 'config'; config.mkdir(); (config / 'rc.xml').write_text(live.RC_XML)
        home = runtime / 'home'; home.mkdir()
        env = {'PATH': '/usr/bin:/bin', 'HOME': str(home), 'LANG': 'C.UTF-8',
               'XDG_RUNTIME_DIR': str(runtime), 'XDG_CONFIG_HOME': str(home / 'config'),
               'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
               'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(runtime),
               'WLR_BACKENDS': 'headless', 'WLR_HEADLESS_OUTPUTS': '1', 'WLR_RENDERER': 'pixman',
               'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop'}
        comp = keyboard = proc = None
        result = {'editor': editor, 'states': {}}
        log = out / 'editor.stderr'
        with (out / 'compositor.log').open('w') as clog, log.open('w') as elog:
            try:
                comp = subprocess.Popen([live.LABWC, '-C', str(config)], env=env, stdout=clog, stderr=clog)
                deadline = time.time() + args.timeout
                while not any(p.is_socket() for p in runtime.glob('wayland-*')):
                    assert time.time() < deadline and comp.poll() is None, 'compositor did not start'
                    time.sleep(.05)
                env['WAYLAND_DISPLAY'] = next(p for p in runtime.glob('wayland-*') if p.is_socket()).name
                subprocess.run([live.RANDR, '--output', 'HEADLESS-1', '--custom-mode', f'{W}x{H}'], env=env, check=True)
                keyboard = subprocess.Popen(['/usr/bin/wtype', '-s', '600000', '-k', 'Shift_L'], env=env,
                                            stdout=elog, stderr=elog)
                time.sleep(.3)
                if editor == 'gnu':
                    # Blinking off so a capture never lands in the cursor's off phase.
                    cmd = ['/usr/bin/emacs-32.0.50', '-Q', '--eval', '(progn (setq inhibit-startup-screen t) (blink-cursor-mode -1))']
                    proc = subprocess.Popen(cmd, env=env, stdout=elog, stderr=elog)
                    time.sleep(4)
                else:
                    cmd = [args.binary, '--geometry', f'{W}x{H}']
                    proc = subprocess.Popen(cmd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=elog)
                    ready = json.loads(proc.stdout.readline())
                    assert ready.get('ready') is True, ready
                    time.sleep(1)
                result['command'] = cmd
                seen = 0

                def shot(state, vm_text=None):
                    nonlocal seen
                    if editor == 'vm2' and vm_text is not None and not args.sleep_sync:
                        seen = wait_prompt(log, vm_text, args.timeout, seen)
                    else:
                        time.sleep(1.2)
                    png = out / f'{state}.png'
                    time.sleep(args.settle)
                    assert proc.poll() is None, f'{editor} exited {proc.returncode} before capture'
                    subprocess.run([args.grim, str(png)], env=env, check=True, timeout=10)
                    rows = args.gnu_rows if editor == 'gnu' else args.rows
                    block = cursor_block(png, rows)
                    # Both cursors may blink (VM2's renderer at 500 ms, GNU's
                    # blink-cursor-mode is off here): look across a full period.
                    blinks = [block]
                    for i in range(4):
                        time.sleep(.3)
                        extra = out / f'{state}.blink{i}.png'
                        subprocess.run([args.grim, str(extra)], env=env, check=True, timeout=10)
                        blinks.append(cursor_block(extra, rows))
                    seen_block = next((b for b in blinks if b is not None), None)
                    text = ocr_bottom(png, png, rows, block if block is not None else seen_block)
                    result['states'][state] = {'png': png.name, 'ocr': text, 'cursor_block_x': seen_block,
                                               'cursor_blocks': blinks,
                                               'cursor_px': cursor_pixels(png, rows), 'rows': rows}

                wtype(env, FORM1)
                wtype(env, '-M', 'ctrl', '-k', 'x', '-k', 'e', '-m', 'ctrl')
                shot('prompt', 'Name: ')
                wtype(env, 'abc')
                shot('typed', 'Name: abc')
                wtype(env, '-k', 'Return')
                shot('returned')
                wtype(env, '-k', 'Return')
                wtype(env, FORM2)
                wtype(env, '-M', 'ctrl', '-k', 'x', '-k', 'e', '-m', 'ctrl')
                wtype(env, 'zz')
                shot('quit-prompt', 'Q: zz')
                wtype(env, '-M', 'ctrl', '-k', 'g', '-m', 'ctrl')
                shot('quit')
                result['alive'] = proc.poll() is None
            finally:
                live.stop(proc); live.stop(keyboard); live.stop(comp)
        live.save_json(out / 'result.json', result)
        return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--runtime-root', default=tempfile.gettempdir())
    p.add_argument('--timeout', type=float, default=30)
    p.add_argument('--settle', type=float, default=.3)
    p.add_argument('--rows', type=int, default=26, help='bottom pixel rows read as the minibuffer line')
    p.add_argument('--gnu-rows', type=int, default=19, help="GNU's echo row is shorter (its mode line sits above)")
    p.add_argument('--grim', default='grim'); p.add_argument('--magick', default='magick')
    p.add_argument('--only', choices=['vm2', 'gnu'])
    p.add_argument('--sleep-sync', action='store_true',
                   help="never read VM2's diagnostics: settle by time only, so the verdict rests on pixels alone")
    p.add_argument('--reread', type=Path, help='re-read the captures of an earlier run (no editors launched)')
    args = p.parse_args(); args.output.mkdir(parents=True, exist_ok=True)
    if args.reread:
        runs = {}
        for editor in ['vm2', 'gnu']:
            r = json.loads((args.reread / editor / 'result.json').read_text())
            for state, st in r['states'].items():
                png = args.reread / editor / st['png']
                st['ocr'] = ocr_bottom(png, args.output / f'{editor}-{state}.png', st['rows'], st['cursor_block_x'])
            runs[editor] = r
    else:
        runs = {e: run(args, e) for e in (['vm2', 'gnu'] if not args.only else [args.only])}
    verdict = {}
    for state, expected in EXPECT.items():
        row = {'expected': expected}
        for editor, r in runs.items():
            got = r['states'].get(state, {}).get('ocr')
            row[editor] = got
            row[editor + '_match'] = got == expected
        verdict[state] = row
    # An active prompt shows the selected cursor in the minibuffer row; the
    # echo area after the prompt has ended shows none (GNU's screens agree).
    cursor = {}
    for state in EXPECT:
        active = state in ('prompt', 'typed', 'quit-prompt')
        row = {'expected_cursor_in_minibuffer': active}
        for editor, r in runs.items():
            block = r['states'].get(state, {}).get('cursor_block_x')
            row[editor + '_block_x'] = block
            row[editor + '_match'] = (block is not None) == active
        if 'vm2' in runs:
            row['vm2_selected_colour_px'] = runs['vm2']['states'].get(state, {}).get('cursor_px')
        row['match'] = all(row[e + '_match'] for e in runs)
        cursor[state] = row
    summary = {'pass': all(all(v[e + '_match'] for e in runs) for v in verdict.values())
               and all(c['match'] for c in cursor.values()), 'states': verdict, 'cursor': cursor}
    live.save_json(args.output / 'summary.json', summary)
    print(json.dumps(summary, indent=1, ensure_ascii=False))
    return 0 if summary['pass'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
