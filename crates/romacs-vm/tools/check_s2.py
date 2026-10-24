#!/usr/bin/env python3
"""Additional live GNU oracle probes; all expected answers come from GNU."""
import sys
from pathlib import Path
import check_lang

check_lang.CASES = {
    "optional/rest-placement": [
        "(funcall (lambda (a &optional b) (+ a b)) 4 5)",
        "(let ((f (lambda (a &optional b) (if b b a)))) (+ (funcall f 1 2) (funcall f 3)))",
        "(let ((x 7)) (funcall (lambda (a &optional b &rest xs) (+ x a (if b b 0) (length xs))) 1 2 3 4))",
        "(funcall (lambda (&rest xs) (length xs)))",
        "(funcall (lambda (&optional x &rest xs) (+ (if x x 9) (length xs))))",
        "(let ((f (funcall (lambda (&optional x) (lambda () (setq x (1+ x)))) 4))) (+ (funcall f) (funcall f)))",
        "(condition-case e (funcall (lambda (x &optional y) x)) (wrong-number-of-arguments 31))",
        "(condition-case e (funcall (lambda (x &optional y) x) 1 2 3) (wrong-number-of-arguments 32))",
    ],
    "apply-adaptation": [
        "(let ((i 0) (s 0)) (while (< i 100000) (setq s (apply '+ i '(1 2)) i (1+ i))) s)",
        "(let ((i 0) (s 0)) (while (< i 100000) (setq s (apply (lambda (a &optional b &rest xs) (+ a b (length xs))) i '(1 2 3)) i (1+ i))) s)",
        "(apply (lambda (a &optional b &rest xs) (+ a b (length xs))) 1 '(2 3 4))",
        "(let ((x 7)) (apply (lambda (a b c) (+ x a b c)) '(1 2 3)))",
        "(apply '+ nil)",
        "(funcall 'apply '+ '(2 3))",
        "(condition-case e (apply '+ '(1 2) 7) (wrong-type-argument 33))",
        "(let ((xs (cons 1 nil))) (setcdr xs xs) (condition-case e (apply '+ xs) (circular-list 34)))",
        "(progn (defun apply (&rest xs) 99) (apply '+ '(1 2)))",
    ],
    "one-shot-escape": [
        "(catch 'a (catch 'b (throw 'a 19)) 0)",
        "(catch 'a (catch 'a (throw 'a 2)) 3)",
        "(let ((x 1)) (+ (catch 'a (setq x 7) (throw 'a 2)) x))",
        "(catch nil (throw nil 4))",
        "(let ((i 0) (s 0)) (while (< i 1000) (setq s (+ s (catch 'x (throw 'x i))) i (1+ i))) s)",
        "(progn (defun throw (tag value) (+ value 1)) (catch 'x (throw 'x 3) 7))",
        "(catch 'x (condition-case e (signal 'error '(8)) (error (throw 'x (car (cdr e))))))",
    ],
    "condition-scope": [
        "(condition-case e (+ nil 2) (error e))",
        "(condition-case e (car 1) (error e))",
        "(condition-case e (length 7) (error e))",
        "(condition-case e (aref 7 0) (error e))",
        "(condition-case e (aset [1] 2 9) (error e))",
        "(condition-case e unbound (error e))",
        "(condition-case e (signal 2 nil) (error e))",
        "(condition-case e (signal 'quit nil) (error 1) (quit 2))",
        "(condition-case e (catch nil (throw nil 4)) (no-catch e))",
        "(condition-case e (+ nil 2) (wrong-type-argument 11) (error 12))",
        "(condition-case e (car 1) ((wrong-type-argument void-variable) 13) (error 14))",
        "(condition-case e unbound (void-variable 15))",
        "(condition-case e (signal 'error '(6 7)) (error (cdr e)))",
        "(condition-case outer (condition-case inner (signal 'error '(1)) (error (signal 'error '(2))) (t 99)) (error (car (cdr outer))))",
        "(condition-case e (catch 'x (signal 'error '(9))) (error (car (cdr e))))",
        "(let ((f (condition-case e (signal 'error '(8)) (error (lambda () (car (cdr e))))))) (funcall f))",
        "(let ((i 0) (s 0)) (while (< i 1000) (setq s (+ s (condition-case e (signal 'error (cons i nil)) (error (car (cdr e))))) i (1+ i))) s)",
    ],
    "sequence-literals": [
        '(copy-sequence "αβγ")',
        '(let ((s (copy-sequence "abc"))) (aset s 1 255) (aset s 1 90) (aref s 1))',
        '(aref "αβγ" 1)',
        '(condition-case e (let ((s (copy-sequence "αβγ"))) (aset s 1 955) (aref s 1)) (error 18))',
        "(length [a (1 2) 3])",
        "(car (aref [a (1 2) 3] 1))",
        "(let ((v [1 2])) (aset v 1 8) v)",
        "(let ((v [1 2])) (let ((w (copy-sequence v))) (aset w 1 8) (aref v 1)))",
        "(let ((i 0) (f (lambda () '(1 2)))) (setcar (funcall f) 9) (car (funcall f)))",
        "(condition-case e (aref [1] -1) (args-out-of-range 16))",
        "(condition-case e (aset [1] 1 9) (args-out-of-range 17))",
        "(plist-get '(a 1 b 2) 'b)",
        "(cdr (assq 'b (cons nil (cons (cons 'a 1) (cons (cons 'b 2) nil)))))",
    ],
    "place-evaluation-order": [
        "(let ((x (cons 1 2)) (i 0)) (setf (car (progn (setq i (1+ i)) x)) 9) (+ (* i 10) (car x)))",
        "(let ((x (cons 1 2)) (i 0)) (cl-incf (car (progn (setq i (1+ i)) x)) (progn (setcar x 100) 9)) (+ (* i 10) (car x)))",
        "(let ((x [1 2]) (i 0)) (setf (aref x (progn (setq i (1+ i)) 0)) 9) (+ (* i 10) (aref x 0)))",
        "(let ((x [1 2])) (cl-decf (aref x 1)) (aref x 1))",
        "(let ((x (cons nil nil))) (push 7 (car x)) (pop (car x)))",
        "(let ((x (cons nil nil)) (i 0)) (push (progn (setq i (1+ i)) i) (car (progn (setq i (+ i 10)) x))) (+ (* i 10) (car (car x))))",
    ],
}
if __name__ == '__main__':
    if '--evidence' not in sys.argv:
        sys.argv.extend(['--evidence', str(Path(__file__).resolve().parents[1] / 'evidence/s2-known-answers.json')])
    check_lang.main()
