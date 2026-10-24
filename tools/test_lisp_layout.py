#!/usr/bin/env python3
"""Check the ownership boundary of runtime Lisp sources."""
import os
import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
GUI_MODULES = (
    'startup', 'find-file', 'buffer-switch', 'extended-command',
    'editing', 'dired', 'install-command',
)


class LispLayoutTests(unittest.TestCase):
    def test_runtime_roots_are_explicit(self):
        for name in ('rlisp', 'gnu-lisp', 'rlisp/deps'):
            self.assertTrue((ROOT / name).is_dir(), f'MISSING_LISP_ROOT: {name}')
        for name in ('lisp', 'lisp-bundled', 'vendor-lisp'):
            self.assertFalse((ROOT / name).exists(), f'RETIRED_LISP_ROOT: {name}')

    def test_owned_modules_live_in_rlisp(self):
        for name in GUI_MODULES:
            self.assertTrue((ROOT / 'rlisp' / f'romacs-{name}.el').is_file(), name)
            self.assertFalse((ROOT / 'crates/romacs-vm-gui/src' / f'{name}.el').exists(), name)
        for name in ('prelude', 'minibuf', 'buffer_lifecycle'):
            self.assertTrue((ROOT / 'rlisp/vm' / f'{name}.el').is_file(), name)
            self.assertFalse((ROOT / 'crates/romacs-vm/src/prims' / f'{name}.el').exists(), name)

    def test_development_consumers_use_the_current_gnu_root(self):
        stale = re.compile(
            r'''(?:\}/lisp(?:/|\b)|\$(?:root|ROOT)/lisp(?:/|\b)|'''
            r'''(?:ROOT|root)\s*/\s*['"]lisp(?:/|['"]))'''
        )
        for directory in ('scripts', 'tools', 'crates', 'gnu-tests/tests'):
            for parent, children, files in os.walk(ROOT / directory):
                children[:] = [name for name in children if name not in ('target', '__pycache__')]
                for name in files:
                    file = pathlib.Path(parent) / name
                    if file == pathlib.Path(__file__) or file.suffix not in ('.py', '.sh', '.rs', '.nix'):
                        continue
                    with self.subTest(file=file.relative_to(ROOT)):
                        self.assertNotRegex(file.read_text(), stale, 'STALE_GNU_SOURCE_ROOT')

    def test_packages_and_temporary_dependencies_are_in_rlisp(self):
        for relative in ('gnosis/gnosis.el', 'hermes/hermes.el', 'jabber/jabber.el',
                         'emacs-lisp/keymap-popup.el', 'deps/fsm/fsm.el',
                         'deps/markdown-mode/markdown-mode.el', 'deps/websocket/websocket.el',
                         'deps/README.md'):
            self.assertTrue((ROOT / 'rlisp' / relative).is_file(), f'RLISP_PACKAGE_MISSING: {relative}')

    def test_owned_lisp_has_minimal_honest_headers(self):
        files = sorted((ROOT / 'rlisp').glob('*.el'))
        files += sorted((ROOT / 'rlisp/vm').glob('*.el'))
        files += [ROOT / 'rlisp/term/romacs-preload.el']
        self.assertTrue(files, 'NO_OWNED_LISP_SOURCES')
        for file in files:
            source = file.read_text()
            with self.subTest(file=file.relative_to(ROOT)):
                self.assertIn('SPDX-License-Identifier: GPL-3.0-or-later', source)
                self.assertIn(';;; Commentary:', source)
                self.assertNotRegex(source, r'(?im)^;+.*Copyright.*(?:Free Software Foundation|FSF)')
                self.assertNotIn('This file is part of GNU Emacs.', source)


if __name__ == '__main__':
    unittest.main()
