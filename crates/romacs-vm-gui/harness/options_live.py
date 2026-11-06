#!/usr/bin/env python3
"""Every automatic writer of settings, in a real session, leaves the init alone.

A server session in a private headless labwc, HOME, XDG and TMPDIR
(parity_live.py), driven by real keys through wtype and read over MCP.  It
runs each route that saves settings: `package-install' from a local archive,
`load-theme' answering its two questions, `customize-save-variable',
`customize-save-customized', `menu-bar-options-save', a saved face, the `!'
answer to unsafe local variables, `disable-command', `enable-command', and the
`y' answer to a disabled command.  After each step the init's SHA-256 must be
what it was, and at the end a new session must have every saved value.

  existing  an init the user wrote: it stays byte-identical.
  absent    no init yet: it stays absent, and settings still persist.
  nil       `custom-file' saved as nil, then naming the init.
  defaults  a saved value of an option Romacs's defaults also set survives.
  quick     -Q: nothing is written, and no ~/.emacs appears.
  view      the customize commands typed in a session: rows, no widgets, a
            value set and saved from a row, the init untouched.
  agent     the option tools over MCP: search, describe, set and save, a value
            that does not fit refused, the init untouched, and set and save
            gone with `romacs-mcp-full-access' off.

ROMACS_HARNESS_WLR_ENV puts the private compositor on the real GPU.

usage: options_live.py --romacs BIN --output DIR [--workflow NAME]... [--tools DIR]
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import traceback

import parity_live
from romacs_mcp_client import RomacsMcp
from server_live import Server, lisp, until

INIT = ''';; -*- lexical-binding: t -*-
;; Written by hand.  Nothing may change this file.
(setq inhibit-startup-screen t initial-scratch-message nil)
'''
PACKAGE = ''';;; optest-pkg.el --- A package for the journey -*- lexical-binding: t -*-
;; Version: 0.1
;; Package-Requires: ((emacs "24.1"))
;;; Commentary:
;; Nothing.
;;; Code:
(provide 'optest-pkg)
;;; optest-pkg.el ends here
'''
THEME = ''';; -*- lexical-binding: t -*-
(deftheme optest "A theme the session has not seen.")
(custom-theme-set-variables 'optest '(optest-theme-mark t))
(provide-theme 'optest)
'''
LOCALS = '''-*- optest-unsafe-local: 42 -*-
A file with a local variable nothing has declared safe.
'''


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest() if path.exists() else 'absent'


class Journey:
    def __init__(self, args, base, name, init=INIT):
        self.server = Server(args, base, name)
        self.init = self.server.home / '.config/romacs/init.el'
        self.custom = self.server.home / '.config/romacs/custom.el'
        if init is None:
            self.init.unlink()
        else:
            self.init.write_text(init)
        self.before = sha(self.init)
        self.steps = []
        self.archive = self.server.root / 'archive'
        self.archive.mkdir()
        (self.archive / 'optest-pkg-0.1.el').write_text(PACKAGE)
        (self.archive / 'archive-contents').write_text(
            '(1 (optest-pkg . [(0 1) ((emacs (24 1))) "A package for the journey" single nil]))\n')
        (self.server.home / '.config/romacs/optest-theme.el').write_text(THEME)
        self.locals = self.server.file('locals.txt', LOCALS)
        self.mcp = None

    def start(self):
        s = self.server
        s.client('-n')
        s.pid = int(s.eval('(emacs-pid)'))
        self.mcp = RomacsMcp.for_server(s, agent_tools=True)
        s.keyboard_ready()
        time.sleep(1)  # keyboard_ready's last C-g must not quit the next eval
        self.mcp.call('romacs_wait_idle')

    def stop(self):
        s = self.server
        pid = s.pid
        self.mcp.close()
        self.mcp = None
        s.client('--stop=none', check=False)
        until(lambda: not Path(f'/proc/{pid}').exists(), 15, 'server stopped')
        s.pid = None

    def eval(self, code):
        return self.mcp.call('romacs_eval', code=code)

    def keys(self, *groups):
        for group in groups:
            self.server.keys(*group)
            time.sleep(.15)

    def command(self, name):
        """M-x NAME RET, as typed."""
        self.keys(('-M', 'alt', '-k', 'x', '-m', 'alt'))
        self.prompt('M-x', name)

    def text(self, text):
        self.keys((text,), ('-k', 'Return'))

    def prompt(self, shown, text):
        self.wait_prompt(shown)
        self.text(text)

    def replace_value(self, shown, text):
        """At PROMPT showing the current value, clear it and enter TEXT."""
        self.wait_prompt(shown)
        self.keys(('-M', 'ctrl', '-k', 'a', '-k', 'k', '-m', 'ctrl'))
        self.text(text)

    def rows(self):
        observed = self.mcp.call('romacs_observe')
        return json.dumps(observed, ensure_ascii=False)

    def until(self, predicate, seconds, what):
        """Wait for PREDICATE; on a timeout, say what the session shows."""
        try:
            until(predicate, seconds, what)
        except RuntimeError as error:
            observed = self.mcp.call('romacs_observe')
            shown = {key: observed.get(key) for key in ('selectedBuffer', 'layers', 'minibuffer', 'echo')}
            messages = observed.get('messages', {}).get('lines', [])[-6:]
            raise RuntimeError(f'{error}; on screen: {json.dumps(shown, ensure_ascii=False)[:900]}; '
                               f'messages: {messages}') from None

    def wait_prompt(self, shown):
        self.until(lambda: shown in self.rows(), 20, f'prompt {shown}')

    def check(self, step):
        now = sha(self.init)
        self.steps.append({'step': step, 'init_unchanged': now == self.before, 'init': now})
        if now != self.before:
            raise RuntimeError(f'{step} changed the init')

    def idle(self):
        self.mcp.call('romacs_wait_idle')

    def package(self):
        self.eval(f'(setq package-archives (list (cons "local" {lisp(str(self.archive) + "/")})) '
                  'package-check-signature nil package-native-compile nil)')
        self.command('package-refresh-contents')
        self.until(lambda: self.eval("(and (require 'package) (assq 'optest-pkg package-archive-contents) t)") == 't',
              30, 'the archive read')
        self.command('package-install')
        self.prompt('Install package', 'optest-pkg')
        self.until(lambda: self.eval("(and (memq 'optest-pkg package-selected-packages) t)") == 't', 60, 'package selected')
        self.check('package-install')

    def theme(self):
        self.command('load-theme')
        self.prompt('Load custom theme', 'optest')
        self.wait_prompt('Really load?')
        self.keys(('y',))
        self.wait_prompt('Treat this theme as safe')
        self.keys(('y',))
        self.until(lambda: self.eval('(and (stringp (car custom-safe-themes)) t)') == 't', 20, 'theme hash kept')
        self.check('load-theme safe answer')

    def options(self):
        self.command('customize-save-variable')
        self.prompt('Set and save variable', 'kill-ring-max')
        self.replace_value('value for kill-ring-max', '123')
        self.until(lambda: self.eval('(eql kill-ring-max 123)') == 't', 20, 'saved option')
        self.check('customize-save-variable')
        self.command('customize-set-variable')
        self.prompt('Set variable', 'mark-ring-max')
        self.replace_value('value for mark-ring-max', '33')
        self.until(lambda: self.eval('(eql mark-ring-max 33)') == 't', 20, 'set option')
        self.command('customize-save-customized')
        self.idle()
        self.check('customize-save-customized')
        self.eval("(progn (require 'menu-bar) (customize-set-variable 'scroll-bar-mode nil) t)")
        self.command('menu-bar-options-save')
        self.idle()
        self.check('menu-bar-options-save')

    def face(self):
        self.eval("""(progn (defface optest-face '((t :weight normal)) "A face." :group 'emacs)
                       (custom-push-theme 'theme-face 'optest-face 'user 'set '((t :weight bold)))
                       (face-spec-set 'optest-face '((t :weight bold)) 'saved-face)
                       (custom-save-all) t)""")
        self.check('saved face')

    def local_variables(self):
        self.keys(('-M', 'ctrl', '-k', 'x', '-k', 'f', '-m', 'ctrl'))
        self.prompt('Find file', str(self.locals))
        self.wait_prompt('Please type')
        self.keys(('!',))
        self.until(lambda: self.eval("(and (assq 'optest-unsafe-local safe-local-variable-values) t)") == 't',
              20, 'local variable marked safe')
        self.check('safe local variable')

    def commands(self):
        self.command('disable-command')
        self.prompt('Disable command', 'forward-word')
        self.until(lambda: self.eval("(get 'forward-word 'disabled)") == 't', 20, 'command disabled')
        self.check('disable-command')
        self.command('enable-command')
        self.prompt('Enable command', 'narrow-to-region')
        self.until(lambda: self.eval("(get 'narrow-to-region 'disabled)") == 'nil', 20, 'command enabled')
        self.check('enable-command')
        self.command('downcase-region')
        self.wait_prompt('Use this command')
        self.keys(('y',))
        self.wait_prompt('Enable command for future editing sessions')
        self.keys(('y',))
        self.until(lambda: self.eval("(get 'downcase-region 'disabled)") == 'nil', 20, 'answered y')
        self.check('disabled command, y')

    def everything(self):
        self.package()
        self.theme()
        self.options()
        self.face()
        self.local_variables()
        self.commands()

    def persisted(self):
        return {
            'kill-ring-max': self.eval('(eql kill-ring-max 123)') == 't',
            'mark-ring-max': self.eval('(eql mark-ring-max 33)') == 't',
            'package-selected-packages':
                self.eval("(progn (require 'package) (and (memq 'optest-pkg package-selected-packages) t))") == 't',
            'custom-safe-themes': self.eval('(and (stringp (car custom-safe-themes)) t)') == 't',
            'safe-local-variable-values':
                self.eval("(and (assq 'optest-unsafe-local safe-local-variable-values) t)") == 't',
            'saved-face': self.eval("(and (get 'optest-face 'saved-face) t)") == 't',
            'disabled': self.eval("(get 'forward-word 'disabled)") == 't',
            'enabled': self.eval("(get 'narrow-to-region 'disabled)") == 'nil',
            'answered-y': self.eval("(get 'downcase-region 'disabled)") == 'nil',
        }

    def close(self):
        if self.mcp:
            self.mcp.close()
        self.server.close()


def whole_journey(args, base, name, init):
    j = Journey(args, base, name, init)
    try:
        j.start()
        j.everything()
        j.stop()
        j.start()
        libraries = j.eval("(list (featurep 'cus-edit) (featurep 'wid-edit))")
        persisted = j.persisted()
        j.check('restart')
        return {'steps': j.steps, 'persisted': persisted, 'init': j.before, 'cus_edit_wid_edit_after_start': libraries,
                'ok': all(persisted.values()) and all(s['init_unchanged'] for s in j.steps)}
    finally:
        j.close()


def existing(args, base):
    return whole_journey(args, base, 'ex', INIT)


def absent(args, base):
    report = whole_journey(args, base, 'ab', None)
    report['ok'] = report['ok'] and report['init'] == 'absent'
    return report


def nil_or_init(args, base):
    j = Journey(args, base, 'ni')
    try:
        j.start()
        j.eval('(customize-save-variable (quote custom-file) nil)')
        j.check('custom-file saved as nil')
        j.eval('(customize-save-variable (quote kill-ring-max) 121)')
        j.check('a save with custom-file nil')
        j.eval(f'(customize-save-variable (quote custom-file) {lisp(j.init)})')
        j.check('custom-file saved as the init')
        j.eval('(customize-save-variable (quote kill-ring-max) 122)')
        j.check('a save with custom-file the init')
        saved = 'kill-ring-max 122' in j.custom.read_text()
        return {'steps': j.steps, 'saved_beside': saved, 'ok': saved}
    finally:
        j.close()


def defaults(args, base):
    j = Journey(args, base, 'df')
    try:
        j.start()
        j.eval('(customize-save-variable (quote column-number-mode) nil)')
        j.eval("(customize-save-variable (quote display-line-numbers-type) t)")
        j.stop()
        j.start()
        values = {'column-number-mode': j.eval('column-number-mode'),
                  'display-line-numbers-type': j.eval('display-line-numbers-type')}
        return {'values': values, 'ok': values == {'column-number-mode': 'nil', 'display-line-numbers-type': 't'}}
    finally:
        j.close()


def quick(args, base):
    """-Q loads no saved settings, and `disable-command' run from the
    keyboard, which the probe records, writes no file."""
    j = Journey(args, base, 'qk')
    s = j.server
    ran, loaded = s.root / 'work/ran', s.root / 'work/loaded'
    j.custom.write_text(f'(write-region "" nil {lisp(loaded)})\n')
    saved = sha(j.custom)
    probe = s.root / 'work/probe.el'
    probe.write_text(f"""(advice-add 'disable-command :after
              (lambda (&rest _) (write-region "" nil {lisp(ran)} nil 'silent)))\n""")
    log = (s.root / 'quick.log').open('w')
    process = subprocess.Popen([args.romacs, '--loadup', '-Q', '-l', str(probe)], env=s.env, cwd=s.root / 'work',
                               stdin=subprocess.DEVNULL, stdout=log, stderr=log)

    def disabled():
        s.keys('-M', 'ctrl', '-k', 'g', '-m', 'ctrl')
        time.sleep(.3)
        s.keys('-M', 'alt', '-k', 'x', '-m', 'alt')
        time.sleep(1)
        s.keys('disable-command')
        s.keys('-k', 'Return')
        time.sleep(1)
        s.keys('forward-word')
        s.keys('-k', 'Return')
        time.sleep(1)
        return ran.exists()
    try:
        until(disabled, 60, 'disable-command ran under -Q')
        report = {'command_ran': True, 'dot_emacs': (s.home / '.emacs').exists(), 'custom_loaded': loaded.exists(),
                  'custom_unchanged': sha(j.custom) == saved, 'init_unchanged': sha(j.init) == j.before}
        report['ok'] = (report['init_unchanged'] and report['custom_unchanged']
                        and not report['dot_emacs'] and not report['custom_loaded'])
        return report
    finally:
        process.kill()
        process.wait()
        j.close()


def agent(args, base):
    j = Journey(args, base, 'ag')
    try:
        j.start()
        mcp = j.mcp
        instance = mcp.identity()

        def refused(tool, arguments):
            _, text, failed = mcp.tool(tool, {'instance': instance, **arguments})
            return failed, text

        def answer(tool, arguments):
            failed, text = refused(tool, arguments)
            if failed:
                raise RuntimeError(f'{tool}: {text}')
            return json.loads(text)
        found = answer('romacs_option_search', {'query': 'kill-ring-max'})
        described = answer('romacs_option_describe', {'name': 'kill-ring-max'})
        changed = answer('romacs_option_set', {'name': 'kill-ring-max', 'value': '61'})
        misfit = refused('romacs_option_set', {'name': 'kill-ring-max', 'value': '"text"'})
        trailing = refused('romacs_option_set', {'name': 'kill-ring-max', 'value': '61 62'})
        unknown = refused('romacs_option_describe', {'name': 'romacs-option-no-such'})
        face = refused('romacs_option_set', {'name': 'bold', 'value': '((t :weight bold))'})
        kept = j.eval('(eql kill-ring-max 61)') == 't'
        misspelled = refused('romacs_option_search', {'query': 'x', 'group': 'no-such-group'})
        wrong_kind = refused('romacs_option_search', {'query': 'x', 'kind': 'macro'})
        # A value past what a reply carries answers, without it.
        j.eval("""(defcustom optest-text "" "Text." :type 'string :group 'emacs)""")
        big = answer('romacs_option_set', {'name': 'optest-text', 'value': '"' + '\U0001F600' * 3000 + '"'})
        # A setter that waits for input ends at the budget, and the session goes on.
        j.eval("""(defcustom optest-asking nil "Asks." :type 'boolean :group 'emacs
                     :initialize #'custom-initialize-default
                     :set (lambda (symbol value) (y-or-n-p "Really? ") (set-default symbol value)))""")
        started = time.time()
        asked = refused('romacs_option_set', {'name': 'optest-asking', 'value': 't', 'budgetMs': 500})
        bounded = time.time() - started < 15
        j.mcp.call('romacs_wait_idle')
        saved = answer('romacs_option_save', {'name': 'kill-ring-max', 'value': '62'})
        j.check('romacs_option_save')
        told = j.eval('(with-current-buffer "*Messages*" (and (string-search "kill-ring-max: 62" (buffer-string)) t))') == 't'
        listed = mcp.tools()
        j.eval('(setq romacs-mcp-full-access nil)')
        limited = mcp.tools()
        checks = {
            'search': any(row['name'] == 'kill-ring-max' for row in found['options']) and found['scope'] == 'loaded',
            'describe': described['type'] == 'natnum' and described['state'] == 'standard',
            'set': changed['previous'] == described['value'] and changed['value'] == '61' and changed['state'] == 'set',
            'misfit': misfit[0] and ':type natnum' in misfit[1],
            'trailing': trailing[0], 'unknown': unknown[0], 'face': face[0] and 'read-only' in face[1],
            'refused-left-it': kept, 'misspelled-group': misspelled[0], 'wrong-kind': wrong_kind[0],
            'too-big-to-carry': big['value'] is None and big['state'] == 'set',
            'asking-setter-bounded': asked[0] and bounded,
            'save': saved['previous'] == '61' and saved['state'] == 'saved' and 'kill-ring-max 62' in j.custom.read_text(),
            'told-the-user': told,
            'listed': 'romacs_option_set' in listed and 'romacs_option_save' in listed,
            'off': 'romacs_option_set' not in limited and 'romacs_option_save' not in limited
                   and 'romacs_option_search' in limited and 'romacs_option_describe' in limited,
        }
        return {'checks': checks, 'ok': all(checks.values())}
    finally:
        j.close()


VIEW_FACTS = """(list (buffer-name) major-mode buffer-read-only (and custom-options t) (and widget-field-list t)
  (and (seq-some (lambda (overlay) (overlay-get overlay 'button)) (overlays-in (point-min) (point-max))) t)
  (and (widget-at (point-min)) t))"""


def view(args, base):
    j = Journey(args, base, 'vw')
    facts = {}
    try:
        j.start()

        def opened(command, shown, answer, buffer):
            j.command(command)
            j.prompt(shown, answer)
            j.until(lambda: buffer in j.eval('(buffer-name)'), 20, f'{command} shows {buffer}')
            facts[command] = j.eval(VIEW_FACTS)
            return facts[command]
        opened('customize-group', 'Customize group', 'killing', '*Customize Group: Killing*')
        opened('customize-apropos', 'Search for', 'kill-ring-max', '*Customize Apropos*')
        opened('customize-face', 'Customize face', 'bold', '*Customize Face: Bold*')
        opened('customize-option', 'Customize variable', 'kill-ring-max', '*Customize Option: Kill Ring Max*')
        j.keys(('-k', 'Return'))
        j.replace_value('Set kill-ring-max to', '77')
        j.until(lambda: j.eval('(eql kill-ring-max 77)') == 't', 20, 'a row set the value')
        row_set = j.eval('(buffer-substring-no-properties (line-beginning-position) (line-end-position))')
        j.keys(('s',))
        j.replace_value('Set kill-ring-max to', '78')
        j.until(lambda: j.eval('(eql kill-ring-max 78)') == 't', 20, 'a row saved the value')
        row_saved = j.eval('(buffer-substring-no-properties (line-beginning-position) (line-end-position))')
        j.check('saved from a row')
        widgetless = all(' nil nil nil nil)' in value and ' Custom-mode t ' in value for value in facts.values())
        saved = 'kill-ring-max 78' in j.custom.read_text()
        return {'facts': facts, 'row_set': row_set, 'row_saved': row_saved, 'steps': j.steps,
                'ok': widgetless and saved and '[set]' in row_set and '[saved]' in row_saved}
    finally:
        j.close()


WORKFLOWS = {'agent': agent, 'view': view, 'existing': existing, 'absent': absent, 'nil': nil_or_init, 'defaults': defaults, 'quick': quick}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--tools', default=parity_live.TOOLS)
    parser.add_argument('--workflow', action='append', choices=sorted(WORKFLOWS))
    args = parser.parse_args()
    args.romacs = str(Path(args.romacs).resolve())
    output = Path(args.output).resolve()
    output.mkdir(parents=True)
    report = {'binary': args.romacs, 'gpu': bool(os.environ.get('ROMACS_HARNESS_WLR_ENV')), 'workflows': {}}
    failed = False
    with tempfile.TemporaryDirectory(prefix='options-live-', dir=os.environ.get('TMPDIR')) as temporary:
        for name in args.workflow or sorted(WORKFLOWS):
            try:
                result = WORKFLOWS[name](args, Path(temporary))
            except Exception as error:
                result = {'ok': False, 'error': f'{type(error).__name__}: {error}', 'trace': traceback.format_exc()}
            report['workflows'][name] = result
            failed = failed or not result.get('ok')
            print(name, 'ok' if result.get('ok') else f'FAIL {result.get("error", "")}', flush=True)
    report['status'] = 'FAIL' if failed else 'PASS'
    (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False))
    print(report['status'])
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
