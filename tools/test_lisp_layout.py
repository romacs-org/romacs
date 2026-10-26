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
                copyright_pattern = r'(?im)^;+.*Copyright.*(?:Free Software Foundation|FSF).*'
                if file == ROOT / 'rlisp/romacs-ert-batch.el':
                    # This adapter derives a GNU function and must retain its notice.
                    upstream = (ROOT / 'gnu-lisp/emacs-lisp/ert.el').read_text()
                    notices = re.findall(copyright_pattern, upstream)
                    self.assertTrue(notices, 'GNU_ERT_COPYRIGHT_MISSING')
                    for notice in notices:
                        self.assertIn(notice, source)
                    self.assertIn('Adapted from GNU ert.el', source)
                elif file == ROOT / 'rlisp/romacs-frame.el':
                    # Derives frame.el's background-mode function and faces.el's
                    # color API, and must retain both notices.
                    for upstream, notice in (('gnu-lisp/frame.el', ';; Copyright (C) 1993-1994, 1996-1997, 2000-2026 Free Software\n;; Foundation, Inc.'),
                                             ('gnu-lisp/faces.el', ';; Copyright (C) 1992-2026 Free Software Foundation, Inc.')):
                        self.assertIn(notice, (ROOT / upstream).read_text())
                        self.assertIn(notice, source)
                    self.assertIn('Adapted from GNU frame.el', source)
                elif file == ROOT / 'rlisp/romacs-keymap.el':
                    # Derived from GNU src/keymap.c and src/keyboard.c, outside gnu-lisp/.
                    self.assertIn(';; Copyright (C) 1985-1988, 1993-1995, 1998-2026 Free Software', source)
                    self.assertIn(';; Copyright (C) 1985-1989, 1993-1997, 1999-2026 Free Software Foundation,', source)
                    self.assertIn('Adapted from GNU src/keymap.c', source)
                elif file == ROOT / 'rlisp/romacs-unidata.el':
                    # Derived from admin/unidata/unidata-gen.el, outside gnu-lisp/.
                    self.assertIn(';; Copyright (C) 2008-2026 Free Software Foundation, Inc.', source)
                    self.assertIn('National Institute of Advanced Industrial Science and Technology (AIST)', source)
                    self.assertIn('Adapted from GNU admin/unidata/unidata-gen.el', source)
                else:
                    self.assertNotRegex(source, copyright_pattern)
                self.assertNotIn('This file is part of GNU Emacs.', source)


if __name__ == '__main__':
    unittest.main()
