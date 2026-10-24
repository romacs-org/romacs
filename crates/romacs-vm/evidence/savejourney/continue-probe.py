import os as _portable_os
from pathlib import Path as _PortablePath
import pathlib, subprocess, json, hashlib
root=pathlib.Path(__file__).parent
oracle=pathlib.Path(_portable_os.environ.get('ROMACS_GNU_ORACLE', str(_PortablePath(_portable_os.environ.get('GNU_ORACLE_ROOT', _PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'gnu-emacs-pin')) / '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea/bin/emacs-32.0.50')))
expr='''(prin1 (let ((b (get-buffer-create " *sj-truename*"))) (unwind-protect (progn (with-current-buffer b (setq buffer-file-truename "/sj-proof")) (list (eq (get-truename-buffer "/sj-proof") b) (eq (funcall 'get-truename-buffer (intern "/sj-proof")) b) (get-truename-buffer "/missing") (condition-case e (get-truename-buffer 2) (error e)))) (kill-buffer b))))'''
r=subprocess.run([str(oracle),'--batch','-Q','--eval',expr],text=True,capture_output=True)
(root/'truename-live.json').write_text(json.dumps({'oracle':str(oracle),'sha256':hashlib.sha256(oracle.read_bytes()).hexdigest(),'expression':expr,'stdout':r.stdout,'stderr':r.stderr,'exit':r.returncode},indent=2)+'\n')
print(r.stdout,r.stderr,r.returncode)
