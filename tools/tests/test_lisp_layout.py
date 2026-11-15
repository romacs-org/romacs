#!/usr/bin/env python3
"""Check the ownership boundary of runtime Lisp sources."""
import os
import pathlib
import re
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tools/lib'))
import lisp_layout  # noqa: E402
GUI_MODULES = (
    'startup', 'find-file', 'buffer-switch', 'workspace-completion',
    'editing', 'dired', 'install-command',
)


class LispLayoutTests(unittest.TestCase):
    def romacs_files(self):
        """Romacs's own Lisp: the runtime files the bundled-autoloads scan holds, and the VM's halves."""
        return sorted([*lisp_layout.scanned_files(ROOT), *(ROOT / 'lisp/vm').glob('*.el')])

    def test_runtime_roots_are_explicit(self):
        layout = lisp_layout.layout(ROOT)
        for name in ('lisp', 'lisp/vm', 'lisp/' + layout['packages'], *('lisp/' + d for d in layout['directories'])):
            self.assertTrue((ROOT / name).is_dir(), f'MISSING_LISP_ROOT: {name}')
        for name in ('lisp/gnu', 'lisp/romacs', 'gnu-lisp', 'rlisp', 'lisp-bundled', 'vendor-lisp'):
            self.assertFalse((ROOT / name).exists(), f'RETIRED_LISP_ROOT: {name}')

    def test_owned_modules_are_romacs_lisp(self):
        romacs = self.romacs_files()
        for name in GUI_MODULES:
            found = [p for p in romacs if p.name == f'romacs-{name}.el']
            self.assertEqual(len(found), 1, f'romacs-{name}.el: {found}')
            self.assertFalse((ROOT / 'crates/romacs-vm-gui/src' / f'{name}.el').exists(), name)
        for name in ('prelude', 'minibuf', 'buffer_lifecycle'):
            self.assertTrue((ROOT / 'lisp/vm' / f'{name}.el').is_file(), name)
            self.assertFalse((ROOT / 'crates/romacs-vm/src/prims' / f'{name}.el').exists(), name)

    def test_development_consumers_name_no_retired_tree(self):
        # One tree: lisp/gnu and lisp/romacs are gone.
        stale = re.compile(r'''(?:\}/lisp|\$(?:root|ROOT)/lisp|(?:ROOT|root)\s*/\s*['"]lisp)/(?:gnu|romacs)(?![\w-])''')
        for directory in ('scripts', 'tools', 'crates', 'test/gnu/tests'):
            for parent, children, files in os.walk(ROOT / directory):
                children[:] = [name for name in children if name not in ('target', '__pycache__')]
                for name in files:
                    file = pathlib.Path(parent) / name
                    # This test and the move's own test hold the retired trees as data.
                    if file in (pathlib.Path(__file__), ROOT / 'tools/tests/test_lisp_tree.py') or file.suffix not in ('.py', '.sh', '.rs', '.nix'):
                        continue
                    with self.subTest(file=file.relative_to(ROOT)):
                        self.assertNotRegex(file.read_text(), stale, 'STALE_LISP_ROOT')

    def test_bundled_packages_are_in_lisp_packages(self):
        for relative in ('gnosis/gnosis.el', 'jabber/jabber.el', 'keymap-popup/keymap-popup.el',
                         'fsm/fsm.el', 'README.md'):
            self.assertTrue((ROOT / 'lisp/packages' / relative).is_file(), f'BUNDLED_PACKAGE_MISSING: {relative}')

    def test_owned_lisp_has_minimal_honest_headers(self):
        files = [p for p in self.romacs_files() if p.relative_to(ROOT).parts[1] != 'packages']
        self.assertTrue(files, 'NO_OWNED_LISP_SOURCES')
        for file in files:
            source = file.read_text()
            with self.subTest(file=file.relative_to(ROOT)):
                self.assertIn('SPDX-License-Identifier: GPL-3.0-or-later', source)
                self.assertIn(';;; Commentary:', source)
                copyright_pattern = r'(?im)^;+.*Copyright.*(?:Free Software Foundation|FSF).*'
                if file == ROOT / 'lisp/emacs-lisp/romacs-ert-batch.el':
                    # This adapter derives a GNU function and must retain its notice.
                    upstream = (ROOT / 'lisp/emacs-lisp/ert.el').read_text()
                    notices = re.findall(copyright_pattern, upstream)
                    self.assertTrue(notices, 'GNU_ERT_COPYRIGHT_MISSING')
                    for notice in notices:
                        self.assertIn(notice, source)
                    self.assertIn('Adapted from GNU ert.el', source)
                elif file == ROOT / 'lisp/ui/romacs-frame.el':
                    # Derives frame.el's background-mode function and faces.el's
                    # color API, and must retain both notices.
                    for upstream, notice in (('lisp/frame.el', ';; Copyright (C) 1993-1994, 1996-1997, 2000-2026 Free Software\n;; Foundation, Inc.'),
                                             ('lisp/faces.el', ';; Copyright (C) 1992-2026 Free Software Foundation, Inc.')):
                        self.assertIn(notice, (ROOT / upstream).read_text())
                        self.assertIn(notice, source)
                    self.assertIn('Adapted from GNU frame.el', source)
                elif file == ROOT / 'lisp/ui/romacs-custom.el':
                    # Derives custom.el's enable-theme and disable-theme.
                    notices = re.findall(copyright_pattern, (ROOT / 'lisp/custom.el').read_text())
                    self.assertTrue(notices, 'GNU_CUSTOM_COPYRIGHT_MISSING')
                    for notice in notices:
                        self.assertIn(notice, source)
                    self.assertIn('Adapted from GNU custom.el', source)
                elif file == ROOT / 'lisp/completion/romacs-commands.el':
                    # Derives simple.el's read-extended-command-1.
                    notices = re.findall(copyright_pattern, (ROOT / 'lisp/simple.el').read_text())
                    self.assertTrue(notices, 'GNU_SIMPLE_COPYRIGHT_MISSING')
                    for notice in notices:
                        self.assertIn(notice, source)
                    self.assertIn('Adapted from GNU simple.el', source)
                elif file == ROOT / 'lisp/romacs-keymap.el':
                    # Derived from GNU src/keyboard.c, outside lisp/.
                    self.assertIn(';; Copyright (C) 1985-1989, 1993-1997, 1999-2026 Free Software Foundation,', source)
                    self.assertIn('Adapted from GNU src/keyboard.c', source)
                elif file == ROOT / 'lisp/international/romacs-unidata.el':
                    # Derived from admin/unidata/unidata-gen.el, outside lisp/.
                    self.assertIn(';; Copyright (C) 2008-2026 Free Software Foundation, Inc.', source)
                    self.assertIn('National Institute of Advanced Industrial Science and Technology (AIST)', source)
                    self.assertIn('Adapted from GNU admin/unidata/unidata-gen.el', source)
                elif file == ROOT / 'lisp/completion/romacs-orderless.el':
                    # Derived from orderless.el (GNU ELPA, FSF copyright),
                    # outside lisp/; retains its notice and names the commit.
                    self.assertIn(';; Portions Copyright (C) 2021-2026 Free Software Foundation, Inc.,', source)
                    self.assertIn('derived from orderless.el by', source)
                    self.assertIn('orderless commit e271a8e', source)
                elif file == ROOT / 'lisp/completion/romacs-complete.el':
                    # Derived from vertico.el and two of its extensions (GNU
                    # ELPA, FSF copyright); retains its notice and names the commit.
                    self.assertIn(';; Portions Copyright (C) 2021-2026 Free Software Foundation, Inc.,', source)
                    self.assertIn('derived from vertico.el by', source)
                    self.assertIn('vertico commit c60d802', source)
                elif file == ROOT / 'lisp/ui/romacs-theme.el':
                    # Ports color.el's sRGB, CIE XYZ and Oklab conversions and
                    # retains its notice.
                    self.assertIn(';; Copyright (C) 2010-2026 Free Software Foundation, Inc.', (ROOT / 'lisp/color.el').read_text())
                    self.assertIn(';; Portions Copyright (C) 2010-2026 Free Software Foundation, Inc., from\n;; GNU Emacs lisp/color.el', source)
                elif file == ROOT / 'lisp/ui/romacs-theme-modus.el':
                    # Adapts modus-themes.el (GNU ELPA, FSF copyright) as it
                    # loads; retains its notice and names the version.
                    self.assertIn(';; Copyright (C) 2019-2026  Free Software Foundation, Inc.', source)
                    self.assertIn(';; Adapted from modus-themes.el 5.3.0 by Protesilaos Stavrou', source)
                else:
                    self.assertNotRegex(source, copyright_pattern)
                self.assertNotIn('This file is part of GNU Emacs.', source)

    def test_ported_rust_keeps_its_notice(self):
        # src/fzf.rs ports fzf's matcher (MIT), which requires its notice.
        source = (ROOT / 'crates/romacs-vm/src/fzf.rs').read_text()
        self.assertTrue(source.startswith('// SPDX-License-Identifier: MIT\n'))
        self.assertIn('// Copyright (c) 2013-2026 Junegunn Choi', source)
        self.assertIn('commit\n// 26ff936df038b6a62ec42cf18db27b2ea2bb8ec3', source)
        self.assertIn('// The above copyright notice and this permission notice shall be included in', source)


if __name__ == '__main__':
    unittest.main()
