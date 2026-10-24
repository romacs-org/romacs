#!/usr/bin/env python3
"""Record actual pinned GNU results with isolated real filesystem fixtures."""
import os as _portable_os
from pathlib import Path as _PortablePath
import hashlib, json, os, pathlib, subprocess, tempfile
HERE = pathlib.Path(__file__).resolve().parent
GNU = _portable_os.environ.get('ROMACS_GNU_ORACLE', str(_PortablePath(_portable_os.environ.get('GNU_ORACLE_ROOT', _PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'gnu-emacs-pin')) / '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea/bin/emacs-32.0.50'))
SRC = pathlib.Path(str(_PortablePath(_portable_os.environ.get('ROMACS_GNU_SOURCE', _PortablePath(_portable_os.environ.get('GNU_ORACLE_ROOT', _PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'gnu-emacs-pin')) / 'src')) / 'src/filelock.c'))
rows = []
with tempfile.TemporaryDirectory(prefix='filelock-gnu-', dir=os.environ['TMPDIR']) as tmp:
    p = pathlib.Path(tmp)/'target'
    lock = p.with_name('.#target')
    q = json.dumps(str(p))
    def record(name, form, data=None):
        lock.unlink(missing_ok=True)
        if data is not None:
            if isinstance(data, bytes): lock.write_bytes(data)
            else: lock.symlink_to(data)
        # Isolate these entry points from GNU's automatic modification locking.
        form = form.replace('(set-buffer-modified-p t)', '(let ((inhibit-modification-hooks t)) (set-buffer-modified-p t))')
        source = '(prin1 (condition-case e '+form+' (error e)))'
        run = subprocess.run([GNU, '-Q', '--batch', '--eval', "(require 'cl-lib)", '--eval', source], capture_output=True, text=True, timeout=20)
        rows.append(dict(name=name, source=source, status=run.returncode, stdout=run.stdout, stderr=run.stderr, lock_after=os.readlink(lock) if lock.is_symlink() else (lock.read_text() if lock.exists() else None)))
    record('callable', "(list (fboundp 'lock-buffer) (fboundp 'unlock-buffer) (fboundp 'file-locked-p) (functionp #'lock-buffer) (with-temp-buffer (list (lock-buffer) (funcall #'unlock-buffer))))")
    record('lock-type', '(lock-buffer 17)')
    record('query-type', '(file-locked-p 17)')
    record('foreign', f'(file-locked-p {q})', 'alice@foreign.invalid.23:17')
    record('regular', f'(file-locked-p {q})', b'bob@foreign.invalid.42')
    record('malformed', f'(file-locked-p {q})', b'garbage')
    record('empty', f'(file-locked-p {q})', b'')
    record('stale', f'(progn (make-symbolic-link (concat "gone@" (system-name) ".-1") {json.dumps(str(lock))}) (file-locked-p {q}))')
    record('ownership', f'(with-temp-buffer (setq buffer-file-truename {q}) (list (lock-buffer) (file-locked-p {q}) (progn (set-buffer-modified-p t) (lock-buffer) (file-locked-p {q})) (progn (unlock-buffer) (file-locked-p {q}))))')
    record('decline', f'(with-temp-buffer (set-buffer-modified-p t) (cl-letf (((symbol-function \'ask-user-about-lock) (lambda (f o) (setq fl-owner o) nil))) (lock-buffer {q}) (list fl-owner (file-locked-p {q}))))', 'alice@foreign.invalid.23:17')
    record('steal', f'(with-temp-buffer (set-buffer-modified-p t) (cl-letf (((symbol-function \'ask-user-about-lock) (lambda (f o) t))) (lock-buffer {q}) (file-locked-p {q})))', 'alice@foreign.invalid.23:17')
    record('handlers', '(progn (defun fl-handler (op file) (setq fl-seen (list op file)) \'handled) (let ((file-name-handler-alist \'(("remote:" . fl-handler)))) (list (file-locked-p "remote:a") fl-seen (with-temp-buffer (set-buffer-modified-p t) (lock-buffer "remote:a") fl-seen) (with-temp-buffer (setq buffer-file-truename "remote:a") (set-buffer-modified-p t) (unlock-buffer) fl-seen))))')
    record('nil-name', f'(cl-letf (((symbol-function \'make-lock-file-name) (lambda (f) nil))) (file-locked-p {q}))')
    record('disabled', f'(with-temp-buffer (set-buffer-modified-p t) (let ((create-lockfiles nil)) (lock-buffer {q})) (file-locked-p {q}))')
    record('disabled-supersession', f'(with-temp-buffer (write-region "" nil {q} nil t) (setq buffer-file-truename {q}) (set-visited-file-modtime -1) (set-buffer-modified-p t) (cl-letf (((symbol-function \'userlock--ask-user-about-supersession-threat) (lambda (f) (setq fl-threat f)))) (let ((create-lockfiles nil)) (lock-buffer)) (equal fl-threat {q})))')
    record('unlock-malformed', f'(with-temp-buffer (setq buffer-file-truename {q}) (set-buffer-modified-p t) (cl-letf (((symbol-function \'userlock--handle-unlock-error) (lambda (e) (setq fl-error e)))) (list (unlock-buffer) fl-error)))', b'garbage')
(HERE/'gnu.json').write_text(json.dumps(dict(binary=GNU, binary_sha256=hashlib.sha256(pathlib.Path(GNU).read_bytes()).hexdigest(), source_sha256=hashlib.sha256(SRC.read_bytes()).hexdigest(), source=str(SRC), rows=rows), indent=2)+'\n')
(HERE/'source-filelock.c').write_bytes(SRC.read_bytes())
print(json.dumps(rows,indent=2))
