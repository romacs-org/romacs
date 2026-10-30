#!/usr/bin/env python3
"""Live GNU answers for the supported VM2 text boundary."""
import json, subprocess, argparse
from pathlib import Path
from bench import read_form, GNU
if not __debug__: raise RuntimeError('evidence requires assertions; Python -O is forbidden')
CASES = [
 '(with-temp-buffer (insert "aλЖ🙂z") (vector (byte-to-position 3) (byte-to-position 4) (byte-to-position 0) (position-bytes 0) (position-bytes 9)))',
 '(with-temp-buffer (insert "λ") (let ((i 0) (s 0)) (while (< i 3) (setq s (+ s (char-after 1))) (defun char-after (p) 7) (setq i (1+ i))) s))',
 '(with-temp-buffer (insert "λ") (let ((i 0) (s 0)) (while (< i 3) (setq s (+ s (goto-char 1))) (defun goto-char (p) 7) (setq i (1+ i))) s))',
 '(with-temp-buffer (insert "λ") (let ((i 0) (s 0)) (while (< i 3) (setq s (+ s (or (search-forward "λ" nil t) 0))) (goto-char 1) (defun search-forward (s b n) 7) (setq i (1+ i))) s))',
 '(with-temp-buffer (insert "aλЖ🙂z") (vector (point) (point-min) (point-max) (buffer-size) (char-after 2) (position-bytes 4) (byte-to-position 6)))',
 '(with-temp-buffer (insert "aλЖ🙂z") (buffer-substring-no-properties 2 5))',
 '(with-temp-buffer (insert "aλЖ🙂z") (goto-char 3) (delete-region 3 4) (insert "Z") (vector (point) (buffer-substring-no-properties 1 6) (position-bytes 5)))',
 '(with-temp-buffer (insert "aλЖ🙂z") (narrow-to-region 2 5) (vector (point-min) (point-max) (char-after 1) (char-after 2) (position-bytes 5) (buffer-size)))',
 '(with-temp-buffer (insert "aλЖ🙂z") (narrow-to-region 2 4) (narrow-to-region 1 6) (vector (point-min) (point-max)))',
 '(with-temp-buffer (insert "abc") (narrow-to-region 2 3) (widen) (point-max))',
 '(with-temp-buffer (insert "aλЖ🙂z") (put-text-property 2 4 (quote x) (vector 7 "λ")) (goto-char 1) (insert "Q") (vector (get-text-property 3 (quote x)) (get-text-property 4 (quote x))))',
 '(with-temp-buffer (insert "aλЖ🙂z") (put-text-property 2 4 (quote x) 7) (delete-region 1 2) (vector (get-text-property 1 (quote x)) (get-text-property 2 (quote x)) (get-text-property 3 (quote x))))',
 '(with-temp-buffer (insert "aλ\nЖ🙂z") (goto-char 1) (vector (search-forward "\n" nil t) (point)))',
 '(with-temp-buffer (insert "aλЖ🙂z") (goto-char 1) (vector (search-forward "Ж🙂" nil t) (point) (search-forward "q" nil t) (point)))',
 '(with-temp-buffer (insert "aλЖ🙂z") (goto-char 1) (search-forward "λ" 2 t))',
 '(with-temp-buffer (insert 955 1046 128578) (buffer-substring-no-properties 1 4))',
 '(with-temp-buffer (insert "λ") (vector (char-after) (char-after 2) (char-after 0) (char-after -1) (char-after 100)))',
 '(with-temp-buffer (insert "λ") (vector (goto-char 0) (point) (goto-char -1) (point) (goto-char 100) (point)))',
 '(with-temp-buffer (insert "aλ") (goto-char 1) (search-forward "q" nil 7) (point))',
 '(with-temp-buffer (insert "outer") (with-temp-buffer (insert "inner")) (buffer-substring-no-properties 1 (point-max)))',
 '(with-temp-buffer (insert "outer") (catch (quote end) (with-temp-buffer (insert "inner") (throw (quote end) 7))) (buffer-size))',
 '(with-temp-buffer (insert "λ") (condition-case nil (with-temp-buffer (car 1)) (wrong-type-argument 7)) (buffer-size))',
 '(let ((s "aλЖ🙂z")) (vector (length s) (aref s 2)))',
 '(vector (% -7 3) (% 7 -3) (% -7 -3))',
]
def main():
 p=argparse.ArgumentParser();p.add_argument('--binary',required=True);p.add_argument('--evidence',required=True);a=p.parse_args()
 rows=[]
 for s in CASES:
  # Byte-compiled, as Romacs compiles every form: a byte-op call such as
  # (char-after 1) is the primitive, not the function cell (VM2-SEALED1).
  oracle=subprocess.check_output([GNU,'-Q','--batch','--eval','(setq lexical-binding t byte-compile-warnings nil native-comp-enable-subr-trampolines nil)','--eval',f'(prin1 (funcall (byte-compile (lambda () {s}))))'],text=True)
  got=subprocess.run([a.binary],input=read_form(s),capture_output=True)
  row=dict(source=s,transport=json.loads(read_form(s)),expected=oracle,got=got.stdout.decode().strip(),error=got.stderr.decode(),passed=got.returncode==0 and got.stdout.decode().strip()==oracle)
  rows.append(row)
  if not row['passed']:print(json.dumps(row,ensure_ascii=False))
 Path(a.evidence).write_text(json.dumps(rows,ensure_ascii=False,indent=2)+'\n')
 print(f'{sum(r["passed"] for r in rows)}/{len(rows)} live GNU text cases')
 raise SystemExit(not all(r['passed'] for r in rows))
if __name__=='__main__':main()
