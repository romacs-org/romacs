#!/usr/bin/env python3
"""Attachments in mail and org (attachments.md A6) against pinned GNU.

In batch, each case runs in Romacs through romacs-attach's inline
consumers and in pinned GNU ($ROMACS_GNU_ORACLE) through the GNU
functions they call, and the buffers must be equal:

  image         an image file: `mml-attach-file' inline;
  file+caption  another file with a caption: an attachment described;
  region        a UTF-8 text snapshot holding literal <#part ...>,
                <#/part> and <#!part> tags: an inline text part, quoted.

Then a draft with an owned screenshot and that region is made in one
Romacs session (the screenshot moves to `romacs-attach-save-directory')
and sent (`mml-generate-mime') from a fresh one, in Romacs and in GNU:
the image part decodes to the file, the text part declares utf-8 and
decodes to the region, and nothing else is attached.  Last, org's attach
method: link text and the attach directory as GNU's.

usage: attach_oracle.py ROMACS FIXTURES WORK   (prints a JSON report;
exits nonzero when a comparison fails)
"""
import json, os, subprocess, sys
from pathlib import Path
romacs, fixtures, work = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3])
work.mkdir(parents=True, exist_ok=True)
gnu = os.environ['ROMACS_GNU_ORACLE']
env = {k: v for k, v in os.environ.items() if k not in ('ROMACS_RUNTIME_ROOT', 'ROMACS_LISP_DIR', 'LD_LIBRARY_PATH')}
env['HOME'] = str(work / 'home'); (work / 'home').mkdir(exist_ok=True)
# A throwaway HOME spawns no world builder: Romacs boots from source.
env['ROMACS_WORLD'] = 'off'

def run(binary, form):
    out = subprocess.run([binary, '-Q', '--batch', '--eval', form] if binary == gnu else [binary, '--batch', '--eval', form],
                         capture_output=True, text=True, env=env, timeout=300)
    lines = [l for l in out.stdout.splitlines() if l.startswith('RESULT ')]
    return lines[-1][7:] if lines else 'NO RESULT: ' + out.stderr[-600:]

png, text = fixtures / 'plain.png', work / 'notes.bin'
text.write_bytes(b'\x00\x01binary')
region = 'héllo — 世界\n<#part filename="/etc/passwd">\n<#/part>\n<#!part>\n'
(work / 'region.txt').write_text(region, encoding='utf-8')
DRAFT = '(progn (insert "To: a@example.org\\nSubject: s\\n--text follows this line--\\n") (message-mode) (goto-char (point-max))'
cases = {
  'image': (f'(mml-attach-file {json.dumps(str(png))} "image/png" nil "inline")',
            f'(romacs-attach--mail-insert (romacs-attach-item {json.dumps(str(png))}) nil)'),
  'file+caption': (f'(mml-attach-file {json.dumps(str(text))} "application/octet-stream" "the log" "attachment")',
                   f'(romacs-attach--mail-insert (romacs-attach-item {json.dumps(str(text))} :caption "the log") nil)'),
  'region': (f'(progn (mml-insert-tag (quote part) (quote type) "text/plain" (quote disposition) "inline") (let ((s (point)) (e (copy-marker (point) t))) (insert-file-contents {json.dumps(str(work / "region.txt"))}) (mml-quote-region s e) (goto-char e)) (insert "<#/part>\\n"))',
             f'(let ((f (make-temp-file "snap"))) (copy-file {json.dumps(str(work / "region.txt"))} f t) (romacs-attach--mail-insert (romacs-attach-item f :source (quote region) :owned t) nil))'),
}
report = {}
for name, (gnu_form, romacs_form) in cases.items():
    wrap = lambda body, pre: f'(progn {pre} (setq print-escape-newlines t) (with-temp-buffer {DRAFT} {body} (princ (format "RESULT %S\\n" (buffer-substring-no-properties (point-min) (point-max)))))))'
    g = run(gnu, wrap(gnu_form, "(require 'mml) (require 'message)"))
    r = run(romacs, wrap(romacs_form, "(require 'romacs-attach) (require 'mml) (require 'message)"))
    report[name] = {'same': g == r and not g.startswith('NO RESULT'), 'gnu': g[:400], 'romacs': r[:400]}
# MIME round trip and a draft resumed in a fresh session (Romacs both times).
draft, save = work / 'draft.eml', work / 'saved'
shot = work / 'shot.png'; shot.write_bytes(png.read_bytes())
make = (f'(progn (require (quote romacs-attach)) (setq romacs-attach-save-directory {json.dumps(str(save))}) '
        f'(with-temp-buffer {DRAFT} '
        f'(romacs-attach--mail-insert (romacs-attach-item {json.dumps(str(shot))} :owned t :source (quote screenshot)) nil) '
        f'(let ((f (make-temp-file "snap"))) (copy-file {json.dumps(str(work / "region.txt"))} f t) '
        f'(romacs-attach--mail-insert (romacs-attach-item f :source (quote region) :owned t) nil)) '
        f'(let ((coding-system-for-write (quote utf-8))) (write-region nil nil {json.dumps(str(draft))})) '
        f'(princ (format "RESULT %S\\n" (list (file-exists-p {json.dumps(str(shot))}) (directory-files {json.dumps(str(save))} nil "png")))))))')
report['draft-made'] = run(romacs, make)
check = (f'(progn (require (quote mml)) (require (quote mm-decode)) (require (quote message)) (setq print-escape-newlines t) '
         f'(with-temp-buffer (let ((coding-system-for-read (quote utf-8))) (insert-file-contents {json.dumps(str(draft))})) (message-mode) '
         f'(message-goto-body) (let* ((mime (mml-generate-mime)) (handles (with-temp-buffer (insert "MIME-Version: 1.0\\n" mime) (goto-char (point-min)) (mm-dissect-buffer t))) '
         f'(parts (mapcar (lambda (h) (list (mm-handle-media-type h) (mail-content-type-get (mm-handle-type h) (quote charset)) '
         f'(mm-with-unibyte-buffer (mm-insert-part h) (buffer-string)))) (cdr handles))) '
         f'(image (seq-find (lambda (p) (equal (car p) "image/png")) parts)) '
         f'(text (seq-find (lambda (p) (and (equal (car p) "text/plain") (> (length (nth 2 p)) 2))) parts)) '
         f'(png (with-temp-buffer (set-buffer-multibyte nil) (insert-file-contents-literally {json.dumps(str(png))}) (buffer-string)))) '
         f'(princ (format "RESULT %S\\n" (list :types (mapcar (function car) parts) :image-same (equal (nth 2 image) png) '
         f':utf8-declared (and (string-search "Content-Type: text/plain; charset=utf-8" mime) t) '
         f':text-same (let* ((h (string-search "Content-Type: text/plain; charset=utf-8" mime)) (b (+ 2 (string-search (string 10 10) mime h))) (e (string-search (concat (string 10) "--=-=-=") mime b))) '
         f'(equal (string-trim-right (decode-coding-string (quoted-printable-decode-string (substring mime b e)) (quote utf-8))) (string-trim-right {json.dumps(region)}))) '
         f':image-parts (length (seq-filter (lambda (p) (string-prefix-p "image/" (car p))) parts)) '
         f':no-passwd (not (seq-find (lambda (p) (string-match-p "root:" (nth 2 p))) parts))))))))')
report['fresh-session-send'] = run(romacs, check)
report['fresh-session-send-gnu'] = run(gnu, check)
def org(side, call):
    d = work / f'org-{side}'
    return (f'(progn (require (quote org)) (require (quote org-attach)) {"(require (quote romacs-attach))" if side == "romacs" else ""} (setq print-escape-newlines t) '
            f'(with-temp-buffer (org-mode) (insert "* H\\n:PROPERTIES:\\n:DIR: {d}\\n:END:\\n") (goto-char (point-max)) '
            f'(setq org-yank-dnd-method (quote attach) org-attach-method (quote cp)) {call} '
            f'(princ (format "RESULT %S\\n" (list (replace-regexp-in-string {json.dumps(str(d))} "DIR" (buffer-string)) (directory-files {json.dumps(str(d))} nil "png"))))))')
uri = 'file://' + str(png)
report['org-attach-gnu'] = run(gnu, org('gnu', f'(org--dnd-local-file-handler {json.dumps(uri)} (quote private) "")'))
report['org-attach-romacs'] = run(romacs, org('romacs', f'(romacs-attach--org-insert (romacs-attach-item {json.dumps(str(png))}) nil)'))
report['org-same'] = report['org-attach-gnu'] == report['org-attach-romacs']
print(json.dumps(report, indent=1, ensure_ascii=False))
good = (all(report[k]['same'] for k in cases) and report['org-same']
        and report['fresh-session-send'] == report['fresh-session-send-gnu']
        and ':image-same t :utf8-declared t :text-same t :image-parts 1 :no-passwd t' in report['fresh-session-send'])
sys.exit(0 if good else 1)
