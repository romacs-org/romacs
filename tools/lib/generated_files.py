#!/usr/bin/env python3
"""No file a tool of ours makes from the repository's sources is tracked.

Such a file is made where it is consumed (AGENTS.md): a build script writes it
into OUT_DIR, Lisp computes it as it loads, a check computes its ledger.
OUTPUTS names each output that once was, or would be, a tracked file, and the
generator that makes it now; `check` fails while git tracks one. GNU's
recordings and imports from outside sources are not made from the repository:
they stay committed with their recorder, and tools/lib/smells.py holds every
tracked file that says a tool wrote it to its merge-base's, unless it declares
itself with `SMELL(generated): reason`.

usage: generated_files.py check [--root ROOT]
"""
import argparse
from pathlib import Path
import subprocess
import sys

# Output path -> its generator, where it is made instead.
OUTPUTS = {
    'assets/fonts/RomacsIcons.ttf': 'crates/romacs-icon-font/build.rs, into its OUT_DIR',
    'crates/romacs-vm/tests/all/modules.rs': 'crates/romacs-vm-test-build/build.rs, into its OUT_DIR',
}


def tracked(root, outputs=OUTPUTS):
    """The OUTPUTS git tracks in ROOT."""
    listed = subprocess.run(['git', 'ls-files', '-z', '--', *outputs], cwd=root, check=True,
                            stdout=subprocess.PIPE).stdout.decode()
    return sorted(name for name in listed.split('\0') if name)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('command', choices=('check',))
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()
    found = tracked(args.root)
    for name in found:
        print(f'generated-files: {name} is tracked; {OUTPUTS[name]} makes it: `git rm` it', file=sys.stderr)
    return 1 if found else 0


if __name__ == '__main__':
    sys.exit(main())
