#!/usr/bin/env python3
"""A large file's faces arrive with nothing else happening: no key, no timer.

Opens FILE in one real session with cursor blink, show-paren and the
benchmark's hooks off, so no timer repaints the frame, and asserts that its
text shows plain while the first parse runs and that the frame changes when
the parse lands.  FILE must parse for longer than PLAIN seconds.

usage: treesit_landing_live.py --romacs BIN --output OUT FILE
"""
import argparse
import os
from pathlib import Path
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import open_fontify_live as ofl  # noqa: E402
import parity_live  # noqa: E402

PLAIN = 1.0  # seconds after RET: the prompt's fade is over, the parse is not
QUIET = """(blink-cursor-mode -1)
(show-paren-mode -1)
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--timeout', type=float, default=30)
    parser.add_argument('file', type=Path)
    args = parser.parse_args()
    args.gnu, args.grammars, args.mode, args.quiet, args.keep_frames = None, '', '1600x1000', 1.0, False
    args.output.mkdir(parents=True, exist_ok=True)
    args.cache = str(args.output / 'cache')
    os.makedirs(args.cache, exist_ok=True)
    parity_live.prepare_world(args.romacs, dict(os.environ, XDG_CACHE_HOME=args.cache), args.output)
    hooks = "(add-hook 'pre-command-hook #'ofl-pre)\n(add-hook 'post-command-hook #'ofl-post)\n"
    assert hooks in ofl.MEASURE
    ofl.MEASURE = ofl.MEASURE.replace(hooks, QUIET)
    with tempfile.TemporaryDirectory(prefix='landing-', dir=args.output, ignore_cleanup_errors=True) as temporary:
        session = ofl.Session('romacs', args, Path(temporary), args.file.resolve())
        try:
            session.start()
            session.keys('-M', 'ctrl', '-k', 'x', '-k', 'f', '-m', 'ctrl', gap=0.3)
            session.keys(session.file.name, gap=2.5)
            prompt = ofl.grab(session.env)[1]
            origin = time.time()
            session.keys('-k', 'Return', gap=0)
            cut = lambda frame: ofl.text_area(frame, *ofl.ROMACS_AREA)  # noqa: E731
            time.sleep(PLAIN)
            plain = ofl.grab(session.env)[1]
            if ofl.differing(cut(plain), cut(prompt)) < ofl.SAME:
                sys.exit('FAIL: no text after the prompt closed')
            while time.time() - origin < args.timeout:
                if ofl.differing(cut(ofl.grab(session.env)[1]), cut(plain)) >= ofl.SAME:
                    print(f'PASS: plain at {PLAIN} s, faces at {time.time() - origin:.2f} s')
                    return
                time.sleep(0.05)
            sys.exit(f'FAIL: no faces in {args.timeout} s')
        finally:
            session.stop()


if __name__ == '__main__':
    main()
