#!/usr/bin/env python3
"""Fontsets on a real display: `query-fontset', `fontset-list',
`fontset-info' and `fontset-font' in Romacs's GUI (a private headless labwc)
against pinned GNU X11 on a private Xvfb, both reading the same forms; then
Romacs alone, whether a rule moves a character already on screen.

Run: fontset_live.py --binary BIN --output DIR [--tools DIR].
"""
import argparse
import hashlib
import json
import os
import secrets
import shutil
import socket
import subprocess
import tempfile
import time
from pathlib import Path

import emoji_live
import fileio_live as fio
import timers_live as tl

# Each form prints one line.  Frame font names differ between the two
# displays, so forms read rules and shapes, not the startup font's name.
FORMS = r'''
(length (fontset-list))
(mapcar (lambda (n) (and (string-match "fontset-[a-z]*\\'" n) (match-string 0 n))) (fontset-list))
(car (last (fontset-list)))
(query-fontset "fontset-default")
(query-fontset "-*-*-*-*-*-*-*-*-*-*-*-*-fontset-default")
(query-fontset "fontset-standard")
(and (query-fontset "fontset-startup") t)
(query-fontset "fontset-.*" t)
(query-fontset "*default")
(query-fontset "nope")
(query-fontset "")
(condition-case e (query-fontset 3) (error e))
(fontset-font t ?α)
(fontset-font t ?α t)
(fontset-font t ?a t)
(fontset-font nil ?a t)
(fontset-font nil ?α t)
(fontset-font t #x1f600 t)
(fontset-font t #xe000 t)
(fontset-font t #x4e00 t)
(fontset-font t #x20000 t)
(fontset-font t #x10ffff t)
(let ((i (fontset-info t))) (list (type-of i) (char-table-subtype i) (mapcar #'car (aref i ?a)) (mapcar #'car (aref i ?α)) (mapcar #'car (aref i #xe000)) (char-table-extra-slot i 0) (mapcar #'car (char-table-range i nil))))
(let ((i (fontset-info nil))) (list (mapcar #'car (aref i ?a)) (mapcar #'car (aref i ?α)) (mapcar #'car (char-table-range i nil)) (mapcar #'car (aref (char-table-extra-slot i 0) ?α))))
(progn (set-fontset-font t ?α "Noto Sans") (fontset-font t ?α t))
(progn (set-fontset-font t ?α '("Foo" . "iso10646-1") nil 'prepend) (fontset-font t ?α t))
(progn (set-fontset-font t ?α (font-spec :family "Bar" :registry "iso8859-7" :weight 'bold) nil 'append) (fontset-font t ?α t))
(mapcar #'car (aref (fontset-info t) ?α))
(progn (set-fontset-font t ?α nil) (fontset-font t ?α t))
(progn (set-fontset-font t ?β nil nil 'append) (fontset-font t ?β t))
(progn (set-fontset-font t ?γ nil nil 'prepend) (fontset-font t ?γ t))
(let ((i (fontset-info t))) (list (mapcar #'car (aref i ?β)) (mapcar #'car (aref i ?γ))))
(condition-case e (set-fontset-font t '(1 . 2) "X") (error e))
(condition-case e (set-fontset-font t '(0 . 127) nil) (error e))
(condition-case e (set-fontset-font t 'latin nil) (error e))
(progn (set-fontset-font nil nil "Y" nil 'prepend) (car (fontset-font nil ?δ t)))
(progn (set-fontset-font t 'lao '("Lao" . "iso10646-1") nil 'append) (length (delq nil (mapcar (lambda (entry) (equal (car entry) "Lao")) (fontset-font t ?ກ t)))))
(progn (set-fontset-font t 'ethiopic '("Eth" . "iso10646-1") nil 'prepend) (length (delq nil (mapcar (lambda (entry) (equal (car entry) "Eth")) (fontset-font t ?አ t)))))
(progn (new-fontset "-misc-fixed-medium-r-normal--0-0-0-0-c-0-fontset-test" '((greek "Foo" ("Bar" . "iso8859-7")) (latin . "Baz"))) (car (fontset-list)))
(query-fontset "fontset-test")
(fontset-font "fontset-test" ?α t)
(fontset-font "fontset-test" ?a)
(progn (new-fontset "-misc-fixed-medium-r-normal--0-0-0-0-c-0-fontset-test" '((han "Han"))) (list (fontset-font "fontset-test" ?α t) (car (fontset-font "fontset-test" #x4e00 t))))
(length (fontset-list))
(mapcar (lambda (font) (stringp font)) (seq-take (x-list-fonts "*") 2))
(let* ((object (car (internal-char-font nil ?a))) (query (query-font object))) (list (length query) (stringp (aref query 0)) (stringp (aref query 1)) (natnump (aref query 2)) (eq (car-safe (aref query 8)) 'opentype)))
(condition-case e (query-font 3) (error e))
(let ((object (car (internal-char-font nil ?a)))) (list (font-has-char-p object ?a) (font-has-char-p object #x10ffff) (condition-case e (font-has-char-p 3 ?a) (error e)) (condition-case e (font-has-char-p object 'x) (error e))))
(let ((entity (car (list-fonts (font-spec :family (symbol-name (font-get (car (internal-char-font nil ?a)) :family))))))) (let ((object (open-font entity 20))) (list (type-of object) (font-get object :size) (aref (query-font object) 2) (type-of (open-font entity)) (condition-case e (open-font 3) (error e)) (condition-case e (open-font entity 'x) (error e)))))
(let ((object (car (internal-char-font nil ?a)))) (list (font-match-p (font-spec :family (symbol-name (font-get object :family))) object) (font-match-p (font-spec :family "No Such") object) (font-match-p (font-spec :registry "iso10646-1") object) (font-match-p (font-spec :registry "koi8-r") object) (font-match-p (font-spec :script 'latin) object) (font-match-p (font-spec :lang 'ja) object) (condition-case e (font-match-p 3 object) (error e)) (condition-case e (font-match-p (font-spec) 3) (error e))))
(list (font-face-attributes "Foo-12") (font-face-attributes "Foo Bar:bold:italic") (font-face-attributes (font-spec :family "X" :size 20 :dpi 96 :weight 'bold :slant 'italic :width 'condensed)) (font-face-attributes (font-spec :size 12.5)) (condition-case e (font-face-attributes 3) (error e)))
(let* ((object (car (internal-char-font nil ?a))) (glyphs (font-get-glyphs object 0 2 "ab")) (g (aref glyphs 0))) (list (length glyphs) (length g) (aref g 0) (aref g 1) (aref g 2) (natnump (aref g 3)) (natnump (aref g 4)) (aref g 9) (aref (font-get-glyphs object 0 1 (vector #x10ffff)) 0) (font-get-glyphs object 0 0 "ab") (condition-case e (font-get-glyphs object 0 9 "ab") (error (car e))) (condition-case e (font-get-glyphs 3 0 1 "a") (error e)) (condition-case e (font-get-glyphs object 0 1 'x) (error e)) (with-temp-buffer (insert "xy") (length (font-get-glyphs object 1 3)))))
(eq (aref (query-font (car (internal-char-font nil ?a))) 0) (face-font 'default))
(list (font-match-p (font-spec :size 0) (font-spec :size 20)) (font-match-p (font-spec :size 20) (font-spec :size 20)) (font-match-p (font-spec) (font-spec :size 20)))
(let ((object (car (internal-char-font nil ?a)))) (list (font-match-p (font-spec :otf '(zzzz nil (kern))) object) (font-match-p (font-spec :otf '(zzzz)) object) (font-match-p (font-spec :script 'latin) (car (list-fonts (font-spec :family (symbol-name (font-get object :family))))))))
(list (font-face-attributes (font-spec :slant 'i :weight 'unspecified)) (font-face-attributes (font-spec :weight 'book)))
(let ((entity (car (list-fonts (font-spec :family (symbol-name (font-get (car (internal-char-font nil ?a)) :family))))))) (natnump (aref (query-font (open-font entity)) 2)))
(let ((object (car (internal-char-font nil ?a)))) (with-temp-buffer (set-buffer-multibyte nil) (insert "\351a") (list (aref (aref (font-get-glyphs object 2 3) 0) 2) (let ((g (aref (font-get-glyphs object 1 2) 0))) (and g (aref g 2))))))
(clear-font-cache)
'''

SETUP = ('(progn (setq inhibit-startup-screen t) '
         '(dolist (form (with-temp-buffer (insert {forms}) (goto-char (point-min)) '
         '(let (forms) (condition-case nil (while t (push (read (current-buffer)) forms)) (end-of-file nil)) (nreverse forms)))) '
         '(let ((value (condition-case err (eval form t) (error (list (quote signal) err))))) '
         '(write-region (concat (prin1-to-string value) "\\n") nil {state} t (quote silent)))) '
         '(write-region "ready\\n" nil {state} t (quote silent)))')


# Romacs alone: stages run from timers, each ending in a record the harness
# waits for before it takes a screenshot, then a `go' file moves on.
JOURNEY = r'''
(setq inhibit-startup-screen t)
(defvar journey-state {state})
(defvar journey-go (concat journey-state ".go"))
(defun journey-record (form)
  (write-region (concat (prin1-to-string form) "\n") nil journey-state t 'silent))
(defun journey-family ()
  (seq-find (lambda (family) (and (find-font (font-spec :family family))
                                  (not (equal family (face-attribute 'default :family)))))
            '("DejaVu Sans Mono" "Noto Sans Mono" "Liberation Mono" "Noto Sans")))
(defvar journey-stages
  (list
   (lambda ()
     (switch-to-buffer (get-buffer-create "fontset"))
     (delete-other-windows)
     (insert "ЖЖЖЖЖЖЖЖ ЖЖЖЖЖЖЖЖ\nabcdefgh\n")
     (goto-char (point-max))
     (redisplay t)
     (list 'painted))
   (lambda ()
     ;; No font query since painting: what fontset-info names, painting opened.
     (let ((opened (seq-mapcat #'cdr (aref (fontset-info t) ?Ж))))
       (list 'opened (and opened t) (and (stringp (car opened)) t))))
   (lambda ()
     (set-fontset-font t ?Ж (font-spec :family (journey-family)))
     (redisplay t)
     (list 'rule (font-get (car (internal-char-font nil ?Ж)) :family)))
   (lambda ()
     (set-fontset-font t ?Ж nil)
     (redisplay t)
     (list 'stop (fontset-font t ?Ж t) (font-get (car (internal-char-font nil ?Ж)) :family)))
   (lambda ()
     (let ((before (window-font-width))
           (family (journey-family)))
       (set-fontset-font nil ?Ж (font-spec :family family :registry "iso10646-1"))
       (set-fontset-font nil 'latin (font-spec :family family))
       (redisplay t)
       (list 'latin (equal (face-attribute 'default :family) family)
             (equal (car (fontset-font nil ?Ж)) family)
             (and (fontset-font nil ?a) t) (/= before (window-font-width)))))
   (lambda ()
     ;; A change of the default family: the frame keeps its fontset, and the
     ;; override keeps its character, past every warmed answer.
     (let ((override (font-get (car (internal-char-font nil ?Ж)) :family)))
       (set-face-attribute 'default nil :family "Iosevka")
       (redisplay t)
       (list 'family (eq override (font-get (car (internal-char-font nil ?Ж)) :family))
             (equal (face-attribute 'default :family) "Iosevka"))))
   (lambda ()
     ;; A size alone: ASCII takes it, and the override stays.
     (let ((before (window-font-height))
           (override (font-get (car (internal-char-font nil ?Ж)) :family)))
       (set-fontset-font nil 'latin (font-spec :size 40))
       (redisplay t)
       ;; A size of zero is no size.
       (set-fontset-font nil 'latin (font-spec :size 0))
       (set-fontset-font nil 'latin (font-spec :size 0.0))
       (set-fontset-font nil 'latin "-*-*-*-*-*-*-0-*-*-*-*-*-iso10646-1")
       (list 'size (> (window-font-height) before)
             (eq override (font-get (car (internal-char-font nil ?Ж)) :family)))))))
(defun journey-step ()
  (when (file-exists-p journey-go)
    (delete-file journey-go)
    (journey-record (condition-case err (funcall (pop journey-stages)) (error (list 'failed err))))))
(write-region "" nil journey-go nil 'silent)
(run-with-timer 1 0.2 #'journey-step)
'''


def journey(binary, args, output, report):
    """The JOURNEY stages, a screenshot after each: the records and shots."""
    tl.INIT = JOURNEY
    shots, records = [], []
    with tl.session(binary, args, output, 'journey', report, False) as (env, state, process, _log):
        go = Path(str(state) + '.go')
        for stage in range(7):
            fio.wait(lambda: state.exists() and len(tl.records(state)) > stage, args.timeout, f'stage {stage}', process)
            tl.quiet(process, 1, args.timeout)
            time.sleep(1)
            shot = output / f'journey-{stage}.png'
            fio.shot(shot, env, report)
            shots.append(shot)
            go.write_text('')
        records = tl.records(state)
        magick = shutil.which('magick', path=env['PATH']) or 'magick'
        pixels = [emoji_live.pixels(shot, magick) for shot in shots]
    return records, pixels


def differ(a, b):
    """Whether the first text line of A and B differ: the cursor is below."""
    width = a[0]
    return a[2][:width * 3 * 40] != b[2][:width * 3 * 40]


def run_gnu(timeout, forms):
    """Pinned GNU X11's lines."""
    tools = Path(os.environ['ROMACS_DISPLAY_TOOLS'])
    with tempfile.TemporaryDirectory(prefix='fontset-gnu-') as temporary:
        root = Path(temporary)
        number = emoji_live.free_display()
        display = f'127.0.0.1:{number}'
        auth = root / 'Xauthority'
        subprocess.run([tools / 'xauth', '-f', auth, 'add', display, 'MIT-MAGIC-COOKIE-1', secrets.token_hex(16)], check=True)
        server = subprocess.Popen([tools / 'Xvfb', f':{number}', '-screen', '0', '1280x800x24', '-nolisten', 'unix',
                                   '-listen', 'tcp', '-nolock', '-noreset', '-auth', auth],
                                  cwd=root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        emacs = None
        try:
            fio.wait(lambda: socket.socket().connect_ex(('127.0.0.1', 6000 + number)) == 0, 10, 'Xvfb', server)
            home = root / 'home'
            home.mkdir()
            state = root / 'state'
            env = {'PATH': os.environ['PATH'], 'HOME': str(home), 'DISPLAY': display, 'XAUTHORITY': str(auth),
                   'GDK_BACKEND': 'x11', 'LANG': 'C.UTF-8', 'TMPDIR': str(root)}
            if 'ROMACS_GATE_FONTCONFIG' in os.environ:
                env['FONTCONFIG_FILE'] = os.environ['ROMACS_GATE_FONTCONFIG']
            setup = SETUP.format(forms=json.dumps(forms, ensure_ascii=False), state=json.dumps(str(state)))
            emacs = subprocess.Popen([os.environ['ROMACS_GNU_GUI_BINARY'], '-Q', '--eval', setup], env=env, cwd=root,
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            fio.wait(lambda: state.exists() and state.read_text().endswith('ready\n'), timeout, 'GNU ready', emacs)
            return state.read_text().splitlines()[:-1]
        finally:
            if emacs is not None:
                emacs.kill()
                emacs.wait()
            server.terminate()
            server.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--tools', help='directory with labwc, wlr-randr, wtype, grim and magick')
    parser.add_argument('--timeout', type=float, default=120)
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'status': 'FAIL', 'failures': []}
    forms = FORMS.strip()
    tl.INIT = SETUP.format(forms=json.dumps(forms, ensure_ascii=False), state='{state}') + '\n'
    try:
        with tl.session(binary, args, output, 'fontset', report, False) as (env, state, process, log_path):
            fio.wait(lambda: state.exists() and tl.records(state)[-1:] == ['ready'], args.timeout, 'Romacs ready', process)
            romacs = tl.records(state)[:-1]
        gnu = run_gnu(args.timeout, forms)
        lines = forms.splitlines()
        report['failures'] = [{'form': form, 'gnu': g, 'romacs': r} for form, g, r in zip(lines, gnu, romacs) if g != r]
        if len(gnu) != len(lines) or len(romacs) != len(lines):
            report['failures'].append({'lines': [len(lines), len(gnu), len(romacs)]})
        records, shots = journey(binary, args, output, report)
        report['journey'] = records
        judged = {'painting opened fonts fontset-info names': records[1] == '(opened t t)',
                  'a rule repaints a shown character': differ(shots[1], shots[2]),
                  'a stop repaints it again': differ(shots[2], shots[3]) and not differ(shots[1], shots[3]),
                  'the ASCII font changes and the override stays': records[4] == '(latin t t t t)' and differ(shots[3], shots[4]),
                  'a family change keeps the override': records[5] == '(family t t)',
                  'a size alone reaches ASCII and keeps the override': records[6] == '(size t t)'}
        report['failures'] += [name for name, ok in judged.items() if not ok]
        report['status'] = 'PASS' if not report['failures'] else 'FAIL'
    except Exception as error:  # noqa: BLE001
        report['error'] = repr(error)
    finally:
        (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    print(report['status'], json.dumps({k: report.get(k) for k in ('failures', 'error')}, ensure_ascii=False)[:6000])
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
