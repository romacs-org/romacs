#!/usr/bin/env python3
"""Refresh checked-in marker/overlay answers from GNU 32.0.50."""
import argparse,json,subprocess
from pathlib import Path
import bench
from bench import read_form,GNU
CASES=[
'(let ((m (make-marker))) (vector (marker-position m) (marker-buffer m) (marker-insertion-type m)))',
'(with-temp-buffer (insert "abc") (let ((m (make-marker))) (vector (eq (set-marker m 2) m) (marker-position m) (eq (marker-buffer m) (current-buffer)))))',
'(with-temp-buffer (insert "aλЖz") (let ((l (copy-marker 2)) (r (copy-marker 2 t))) (goto-char 2) (insert "ξЮ") (vector (marker-position l) (marker-position r) (marker-insertion-type l) (marker-insertion-type r))))',
'(with-temp-buffer (insert "abcdef") (let ((l (copy-marker 4)) (r (copy-marker 2 t))) (delete-region 2 4) (goto-char 2) (insert "XY") (vector (marker-position l) (marker-position r))))',
'(with-temp-buffer (insert "abc") (let ((l (copy-marker 2)) (r (copy-marker 2 t))) (goto-char 2) (insert-before-markers "λ" 1046) (vector (marker-position l) (marker-position r) (point))))',
'(with-temp-buffer (insert "abc") (let ((m (copy-marker 2 t))) (set-marker m nil) (vector (marker-position m) (marker-buffer m) (marker-insertion-type m) (marker-position (copy-marker m)))))',
'(with-temp-buffer (insert "abc") (let ((m (copy-marker 2 t))) (set-marker-insertion-type m nil) (goto-char 2) (insert "Q") (vector (marker-position m) (marker-insertion-type m) (marker-insertion-type (copy-marker m t)))))',
'(with-temp-buffer (insert "abc") (let ((m (copy-marker 999))) (vector (marker-position m) (marker-position (copy-marker -9)))))',
'(with-temp-buffer (insert "abc") (let ((m (copy-marker 2 t)) (b (current-buffer))) (with-temp-buffer (insert "abcdef") (set-marker m 4) (vector (marker-position m) (eq b (marker-buffer m))))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 2 3))) (vector (overlay-start o) (overlay-end o) (eq (move-overlay o 1 4) o) (overlay-start o) (overlay-end o))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 2 3))) (vector (overlay-put o (quote x) 7) (overlay-get o (quote x)) (overlay-get o (quote missing)) (delete-overlay o) (overlay-start o) (overlay-end o) (overlay-get o (quote x)))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 2 3))) (goto-char 2) (insert "Q") (goto-char 4) (insert "R") (vector (overlay-start o) (overlay-end o))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 2 3 nil t t))) (goto-char 2) (insert "Q") (goto-char 4) (insert "R") (vector (overlay-start o) (overlay-end o))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 2 2 nil t nil))) (goto-char 2) (insert "Q") (vector (overlay-start o) (overlay-end o))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 2 2 nil nil t))) (goto-char 2) (insert "Q") (vector (overlay-start o) (overlay-end o))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 2 3 nil t nil))) (delete-region 1 4) (insert "Q") (vector (overlay-start o) (overlay-end o))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 1 4))) (vector (length (overlays-at 1)) (length (overlays-at 4)) (length (overlays-in 2 3)) (next-overlay-change 1) (next-overlay-change 4))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 2 2))) (vector (length (overlays-at 2)) (length (overlays-in 1 2)) (length (overlays-in 2 3)) (length (overlays-in 2 2)))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 1 4))) (overlay-put o (quote x) 7) (remove-overlays 1 4 (quote x) 9) (vector (overlay-start o) (length (overlays-in 1 4)))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 1 4))) (overlay-put o (quote x) 7) (remove-overlays 1 4 (quote x) 7) (vector (overlay-start o) (length (overlays-in 1 4)))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 1 4))) (remove-overlays 2 3) (vector (overlay-start o) (overlay-end o) (length (overlays-in 1 4)))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 1 3)) (p (make-overlay 2 4))) (overlay-put o (quote x) 7) (overlay-put p (quote x) 9) (overlay-put o (quote priority) 10) (put-text-property 1 4 (quote x) 3) (vector (get-char-property 1 (quote x)) (get-char-property 2 (quote x)) (get-char-property 3 (quote x)) (length (overlays-at 2)) (overlay-get p (quote x)))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 1 3))) (with-temp-buffer (insert "λЖz") (move-overlay o 2 4 (current-buffer)) (vector (overlay-start o) (overlay-end o) (length (overlays-at 2))))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 1 3))) (overlay-put o (quote evaporate) t) (delete-region 1 3) (vector (overlay-start o) (overlay-end o))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 1 3))) (with-temp-buffer (insert "abcdef") (move-overlay o 3 6) (vector (overlay-start o) (overlay-end o) (length (overlays-at 4))))))',
'(with-temp-buffer (insert "abc") (let ((m (copy-marker 2))) (vector (marker-position (set-marker m (make-marker))) (marker-buffer m))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 2 2 nil t nil))) (goto-char 2) (insert-before-markers "Q") (vector (overlay-start o) (overlay-end o))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 2 2))) (overlay-put o (quote evaporate) t) (vector (overlay-start o) (overlay-end o))))',
'(with-temp-buffer (insert "abcdef") (let ((o (make-overlay 2 5))) (overlay-put o (quote evaporate) t) (move-overlay o 3 3) (vector (overlay-start o) (overlay-end o))))',
'(with-temp-buffer (insert "abcdef") (let ((o (make-overlay 2 5))) (remove-overlays 1 4) (vector (overlay-start o) (overlay-end o))))',
'(with-temp-buffer (insert "abcdef") (let ((o (make-overlay 2 5))) (remove-overlays 4 6) (vector (overlay-start o) (overlay-end o))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 2 2))) (remove-overlays 2 2) (vector (overlay-start o) (overlay-end o))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 3 2))) (vector (overlay-start o) (overlay-end o))))',
'(with-temp-buffer (insert "abcdefghijklmnopqrst") (let ((a (make-overlay 1 10)) (b (make-overlay 5 20))) (overlay-put a (quote face) 1) (overlay-put b (quote face) 2) (get-char-property 7 (quote face))))',
'(with-temp-buffer (insert "abcdefghijklmnopqrst") (let ((b (make-overlay 5 20)) (a (make-overlay 1 10))) (overlay-put a (quote face) 1) (overlay-put b (quote face) 2) (get-char-property 7 (quote face))))',
'(with-temp-buffer (insert "abc") (make-overlay 4 4) (vector (length (overlays-in 1 4)) (length (overlays-in 2 4)) (length (overlays-in 4 4))))',
'(with-temp-buffer (insert "abcde") (make-overlay 2 4) (vector (length (overlays-in 2 2)) (length (overlays-in 3 3)) (length (overlays-in 4 4))))',
'(with-temp-buffer (insert "abcde") (make-overlay 1 5) (vector (length (overlays-in 1 1)) (length (overlays-in 3 3))))',
'(with-temp-buffer (insert "abcdef") (let ((o (make-overlay 2 5))) (remove-overlays 4 1) (vector (overlay-start o) (overlay-end o))))',
'(with-temp-buffer (insert "abcdefghi") (let ((o (make-overlay 2 8 nil t t))) (remove-overlays 4 6) (goto-char 2) (insert "X") (goto-char 5) (insert "Y") (let ((left (car (overlays-at 3)))) (vector (overlay-start left) (overlay-end left) (overlay-start o) (overlay-end o)))))',
'(with-temp-buffer (insert "abc") (let ((o (make-overlay 1 4))) (remove-overlays 1 4 nil 5) (vector (overlay-start o) (overlay-end o))))',
'(with-temp-buffer (insert "abcdef") (make-overlay 4 4) (narrow-to-region 2 4) (vector (length (overlays-in 2 4)) (length (overlays-in 1 4)) (length (overlays-in 4 4)) (length (overlays-at 4))))',
'(with-temp-buffer (insert "abcdef") (let ((o (make-overlay 4 4))) (narrow-to-region 2 4) (remove-overlays) (widen) (vector (overlay-start o) (length (overlays-in 1 7)))))',
'(with-temp-buffer (insert "abcdef") (make-overlay 4 4) (make-overlay 2 6) (narrow-to-region 2 4) (vector (length (overlays-in 2 4)) (length (overlays-in 4 4)) (length (overlays-in 3 4)) (length (overlays-in 1 7))))',
]
def main():
 p=argparse.ArgumentParser();p.add_argument('--binary',required=True);p.add_argument('--evidence',required=True);p.add_argument('--gnu',default=GNU);a=p.parse_args()
 bench.GNU=a.gnu
 version=subprocess.check_output([a.gnu,'--version'],text=True).splitlines()[0]
 assert version=='GNU Emacs 32.0.50',version
 rows=[]
 for source in CASES:
  expected=subprocess.check_output([a.gnu,'-Q','--batch','--eval','(setq lexical-binding t)','--eval',f'(prin1 (eval (read {json.dumps(source,ensure_ascii=False)}) t))'],text=True)
  transport=read_form(source)
  got=subprocess.run([a.binary],input=transport,capture_output=True)
  row=dict(source=source,transport=json.loads(transport),expected=expected,got=got.stdout.decode().strip(),error=got.stderr.decode(),passed=got.returncode==0 and got.stdout.decode().strip()==expected)
  rows.append(row)
  if not row['passed']: print(json.dumps(row,ensure_ascii=False))
 Path(a.evidence).write_text(json.dumps(rows,ensure_ascii=False,indent=2)+'\n')
 print(f'{sum(r["passed"] for r in rows)}/{len(rows)} live GNU {version} marker/overlay cases')
 raise SystemExit(not all(r['passed'] for r in rows))
if __name__=='__main__':main()
