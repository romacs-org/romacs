#!/usr/bin/env python3
"""Record pinned GNU results; fixture creation is outside the Lisp evaluator."""
import os as _portable_os
from pathlib import Path as _PortablePath
import hashlib, json, os, pathlib, subprocess, tempfile
ORACLE = _portable_os.environ.get('ROMACS_GNU_ORACLE', str(_PortablePath(_portable_os.environ.get('GNU_ORACLE_ROOT', _PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'gnu-emacs-pin')) / '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea/bin/emacs-32.0.50'))
HERE = pathlib.Path(__file__).resolve().parent
with tempfile.TemporaryDirectory(prefix='filesave-oracle-', dir=os.environ['TMPDIR']) as temp:
    root = pathlib.Path(temp)
    (root/'file').write_text('source'); (root/'file').chmod(0o640)
    (root/'readonly').write_text('readonly'); (root/'readonly').chmod(0o400)
    (root/'target').write_text('target'); (root/'dir').mkdir()
    (root/'rodir').mkdir(); (root/'rodir').chmod(0o500)
    (root/'link').symlink_to(root/'file'); (root/'dangling').symlink_to(root/'absent')
    def q(p): return json.dumps(str(root/p))
    forms = {
      'writable-file': f'(file-writable-p {q("file")})',
      'writable-readonly': f'(file-writable-p {q("readonly")})',
      'writable-directory': f'(file-writable-p {q("dir")})',
      'writable-missing': f'(file-writable-p {q("missing")})',
      'writable-missing-parent': f'(file-writable-p {q("missing/child")})',
      'writable-missing-trailing-slash': f'(file-writable-p {json.dumps(str(root/"missing")+"/")})',
      'writable-readonly-parent': f'(file-writable-p {q("rodir/child")})',
      'writable-dangling': f'(file-writable-p {q("dangling")})',
      'modes-file': f'(file-modes {q("file")})',
      'modes-dir': f'(file-modes {q("dir")})',
      'modes-missing': f'(file-modes {q("missing")})',
      'modes-enotdir': f'(file-modes {q("file/child")})',
      'modes-link': f'(file-modes {q("link")})',
      'modes-nofollow': f'(file-modes {q("link")} \'anything)',
      'modes-dangling': f'(file-modes {q("dangling")})',
      'set-nofollow': f'(set-file-modes {q("link")} 511 t)',
      'set-missing': f'(set-file-modes {q("missing")} 384)',
      'set-lowbits': f'(progn (set-file-modes {q("file")} 8576) (file-modes {q("file")}))',
      'rename-exists': f'(rename-file {q("file")} {q("target")})',
      'rename-same': f'(rename-file {q("file")} {q("file")})',
      'rename-missing': f'(rename-file {q("missing")} {q("new")})',
      'rename-replace': f'(progn (rename-file {q("file")} {q("target")} t) (list (file-exists-p {q("file")}) (file-exists-p {q("target")})))',
      'rename-directory': f'(progn (rename-file {q("target")} {q("dir/")[:-1]}/\") (file-exists-p {q("dir/target")}))',
      'temp-name': f'(let* ((p {q("prefix-")}) (n (make-temp-name p))) (list (substring n 0 (length p)) (- (length n) (length p)) (file-exists-p n) (multibyte-string-p n)))',
      'temp-nonexistent-parent': f'(let ((n (make-temp-name {q("missing/prefix-")}))) (file-exists-p n))',
      'temp-relative': '(substring (make-temp-name "relative-") 0 9)',
      'temp-no-handler': '(let ((file-name-handler-alist \'((".*" . broken)))) (substring (make-temp-name "relative-") 0 9))',
      'invalid-writable': '(file-writable-p 1)', 'invalid-temp': '(make-temp-name nil)',
      'invalid-modes': '(file-modes 1)', 'invalid-rename': '(rename-file 1 2)',
      'invalid-set-precedence': '(set-file-modes 1 nil)',
      'nul-modes': '(file-modes "a\\0b")',
    }
    for op, tail in [('file-writable-p',''),('file-modes'," 'nofollow"),('set-file-modes',' 384 t'),('rename-file',' "/filesave-magic/b" t')]:
        forms['handler-'+op] = f'(let ((file-name-handler-alist \'(("/filesave-magic/" . filesave-handler)))) ({op} "/filesave-magic/a"{tail}))'
    rows=[]
    for name, form in forms.items():
        wrapped = '(progn (defun filesave-handler (op &rest args) (cons op args)) (put \'filesave-handler \'operations \'(file-writable-p file-modes set-file-modes rename-file)) (prin1 (condition-case e '+form+' (error e))))'
        run=subprocess.run([ORACLE,'-Q','--batch','--eval',wrapped],text=True,capture_output=True,timeout=15)
        rows.append(dict(id=name,form=form.replace(temp,'$FIXTURE'),stdout=run.stdout.replace(temp,'$FIXTURE'),stderr=run.stderr.replace(temp,'$FIXTURE'),exit=run.returncode))
    doc={'oracle':ORACLE,'sha256':hashlib.sha256(pathlib.Path(ORACLE).read_bytes()).hexdigest(),'uid':os.getuid(),'rows':rows}
    (HERE/'gnu.json').write_text(json.dumps(doc,indent=2)+'\n')
    for row in rows: print(row['id'],row['stdout'],row['stderr'])
