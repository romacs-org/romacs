;;; byte_op_kernels.el --- Byte-op semantics for the oracle test  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; `bok-run' answers every case below as one string, one line per case, so
;; `byte_op_kernels.rs' can compare Romacs with pinned GNU line by line.
;; `bok-compile' is `byte-compile' in GNU, so its calls are byte codes, and
;; `identity' in Romacs, which compiles everything.  A case runs as a compiled
;; call of the operator and as a `funcall' of its symbol, so both the kernel
;; and the primitive answer.

;;; Code:

(defvar bok-buffer nil)
(defvar bok-marker nil)
(defvar bok-unset-marker nil)

(defun bok-show (thunk)
  "THUNK's value printed, or the condition it signalled."
  (let ((print-circle t))
    (condition-case err
        (prin1-to-string (funcall thunk))
      (error (format "signal %S" err)))))

(defun bok-numbers ()
  (list 0 1 -1 2 -2 3 7 -7 1000
        most-positive-fixnum (1- most-positive-fixnum)
        most-negative-fixnum (1+ most-negative-fixnum)
        (1+ most-positive-fixnum) (1- most-negative-fixnum)
        (expt 2 64) (- (expt 2 64))
        0.0 -0.0 1.0 -1.0 1.5 -1.5 2.0 0.5 1e308 -1e308 5e-324
        1.0e+INF -1.0e+INF 0.0e+NaN -0.0e+NaN
        9007199254740992.0 9007199254740993 9007199254740992
        2305843009213693952.0 -2305843009213693952.0
        bok-marker bok-unset-marker
        nil t 'a "s" '(1) [1]))

(defconst bok-binary
  `((+ . ,(bok-compile (lambda (a b) (+ a b))))
    (- . ,(bok-compile (lambda (a b) (- a b))))
    (* . ,(bok-compile (lambda (a b) (* a b))))
    (/ . ,(bok-compile (lambda (a b) (/ a b))))
    (% . ,(bok-compile (lambda (a b) (% a b))))
    (max . ,(bok-compile (lambda (a b) (max a b))))
    (min . ,(bok-compile (lambda (a b) (min a b))))
    (< . ,(bok-compile (lambda (a b) (< a b))))
    (> . ,(bok-compile (lambda (a b) (> a b))))
    (<= . ,(bok-compile (lambda (a b) (<= a b))))
    (>= . ,(bok-compile (lambda (a b) (>= a b))))
    (= . ,(bok-compile (lambda (a b) (= a b))))
    (/= . ,(bok-compile (lambda (a b) (/= a b))))
    (eql . ,(bok-compile (lambda (a b) (eql a b))))
    (equal . ,(bok-compile (lambda (a b) (equal a b))))))

(defconst bok-unary
  `((1+ . ,(bok-compile (lambda (a) (1+ a))))
    (1- . ,(bok-compile (lambda (a) (1- a))))
    (- . ,(bok-compile (lambda (a) (- a))))
    (abs . ,(bok-compile (lambda (a) (abs a))))
    (length . ,(bok-compile (lambda (a) (length a))))))

(defun bok-numeric-lines ()
  (let ((lines nil) (values (bok-numbers)))
    (dolist (op bok-binary)
      (dolist (a values)
        (dolist (b values)
          (push (format "%s %S %S: %s | %s" (car op) a b
                        (bok-show (lambda () (funcall (cdr op) a b)))
                        (bok-show (lambda () (funcall (car op) a b))))
                lines))))
    (dolist (op bok-unary)
      (dolist (a values)
        (push (format "%s %S: %s | %s" (car op) a
                      (bok-show (lambda () (funcall (cdr op) a)))
                      (bok-show (lambda () (funcall (car op) a))))
              lines)))
    (nreverse lines)))

(defun bok-cycle (n)
  "A circular list of N ones."
  (let ((list (make-list n 1)))
    (setcdr (last list) list)
    list))

(defun bok-nest (n leaf)
  "LEAF inside N conses, each in the car."
  (let ((x leaf))
    (dotimes (_ n) (setq x (list x)))
    x))

(defun bok-vnest (n leaf)
  "LEAF inside N one-element vectors."
  (let ((x leaf))
    (dotimes (_ n) (setq x (vector x)))
    x))

(defconst bok-equal (bok-compile (lambda (a b) (equal a b))))

(defun bok-equal-lines ()
  (let ((self-car (list nil)) (self-car2 (list nil))
        (self-vec (vector nil 1)) (self-vec2 (vector nil 1))
        (self-rec (record 'r nil)) (self-rec2 (record 'r nil))
        (m1 (copy-marker bok-marker)) (m2 (copy-marker bok-marker))
        (m3 (copy-marker (1+ bok-marker)))
        (o1 (make-overlay 1 3 bok-buffer)) (o2 (make-overlay 1 3 bok-buffer))
        (o3 (make-overlay 1 4 bok-buffer))
        (table1 (make-char-table 'bok)) (table2 (make-char-table 'bok))
        (hash1 (make-hash-table)) (hash2 (make-hash-table))
        (cases nil))
    (setcar self-car self-car)
    (setcar self-car2 self-car2)
    (aset self-vec 0 self-vec)
    (aset self-vec2 0 self-vec2)
    (aset self-rec 1 self-rec)
    (aset self-rec2 1 self-rec2)
    (overlay-put o1 'face 'bold)
    (overlay-put o2 'face 'bold)
    (setq cases
          `(("cycle-same" ,(bok-cycle 2) ,(bok-cycle 2))
            ("cycle-lengths" ,(bok-cycle 2) ,(bok-cycle 3))
            ("cycle-one" ,(bok-cycle 1) ,(bok-cycle 1))
            ("cycle-against-list" ,(bok-cycle 2) ,(make-list 5000 1))
            ("self-car" ,self-car ,self-car2)
            ("self-vector" ,self-vec ,self-vec2)
            ("self-record" ,self-rec ,self-rec2)
            ("long-lists" ,(make-list 100000 'x) ,(make-list 100000 'x))
            ("long-lists-differ" ,(make-list 100000 'x) ,(append (make-list 99999 'x) '(y)))
            ("dotted" (1 2 . 3) (1 2 . 3))
            ("dotted-differ" (1 2 . 3) (1 2 . 4))
            ("floats" 1.0 1.0)
            ("zeros" 0.0 -0.0)
            ("nan" 0.0e+NaN 0.0e+NaN)
            ("nan-signs" 0.0e+NaN -0.0e+NaN)
            ("float-int" 1.0 1)
            ("bignums" ,(expt 2 70) ,(expt 2 70))
            ("strings" "abc" ,(concat "ab" "c"))
            ("unibyte-multibyte" "abc" ,(string-to-multibyte "abc"))
            ("raw-bytes" ,(string-to-unibyte "\300") ,(string-to-multibyte "\300"))
            ("multibyte" "é" ,(concat "é"))
            ("properties" ,(propertize "a" 'face 'bold) "a")
            ("bool-vectors" ,(bool-vector t nil) ,(bool-vector t nil))
            ("bool-vectors-differ" ,(bool-vector t nil) ,(bool-vector t t))
            ("vectors" [1 (2) "3"] ,(vector 1 (list 2) (concat "3")))
            ("vector-lengths" [1] [1 2])
            ("records" ,(record 'a 1) ,(record 'a 1))
            ("record-vector" ,(record 'a 1) [a 1])
            ("markers" ,m1 ,m2)
            ("markers-differ" ,m1 ,m3)
            ("unset-markers" ,(make-marker) ,(make-marker))
            ("overlays" ,o1 ,o2)
            ("overlays-differ" ,o1 ,o3)
            ("char-tables" ,table1 ,table2)
            ("hash-tables" ,hash1 ,hash2)
            ("closures" ,(bok-compile (lambda (x) x)) ,(bok-compile (lambda (x) x)))
            ("symbols" a a)
            ("symbol-string" a "a")))
    (dolist (n '(9 10 11 12 50 99 100 101 150 198 199 200 201 202 203 210 400))
      (push (list (format "nest-%d" n) (bok-nest n 1) (bok-nest n 1)) cases)
      (push (list (format "nest-differ-%d" n) (bok-nest n 1) (bok-nest n 2)) cases)
      (push (list (format "vnest-%d" n) (bok-vnest n 1) (bok-vnest n 1)) cases))
    (mapcar (lambda (c)
              (format "equal %s: %s | %s | %s" (car c)
                      (bok-show (lambda () (funcall bok-equal (nth 1 c) (nth 2 c))))
                      (bok-show (lambda () (funcall 'equal (nth 1 c) (nth 2 c))))
                      (bok-show (lambda () (and (member (nth 1 c) (list 0 (nth 2 c))) t)))))
            (reverse cases))))

(defconst bok-aset (bok-compile (lambda (a i v) (aset a i v))))
(defconst bok-length (bok-compile (lambda (a) (length a))))
(defconst bok-nthcdr (bok-compile (lambda (n l) (nthcdr n l))))
(defconst bok-member (bok-compile (lambda (e l) (member e l))))
(defconst bok-get (bok-compile (lambda (s p) (get s p))))
(defconst bok-list0 (bok-compile (lambda () (list))))
(defconst bok-list1 (bok-compile (lambda (a) (list a))))
(defconst bok-list3 (bok-compile (lambda (a b c) (list a b c))))

(defun bok-sequence-lines ()
  (let ((lines nil) (table (make-char-table 'bok)))
    (put 'bok-symbol 'bok-a 1)
    (put 'bok-symbol 'bok-b 2)
    (dolist (c `(("vector" ,(vector 1 2) 0 9) ("vector-end" ,(vector 1 2) 2 9) ("vector-negative" ,(vector 1 2) -1 9)
                 ("record" ,(record 'a 1) 1 9) ("record-end" ,(record 'a 1) 2 9)
                 ("string" ,(string-to-multibyte "abc") 1 ?z) ("string-wide" ,(string-to-multibyte "abc") 1 ?é)
                 ("string-raw" ,(string-to-multibyte "abc") 1 #x3fff80) ("string-bad" ,(string-to-multibyte "abc") 1 'x)
                 ("unibyte" ,(string-to-unibyte "abc") 1 ?z) ("unibyte-wide" ,(string-to-unibyte "abc") 1 ?é)
                 ("bool" ,(make-bool-vector 3 nil) 1 t) ("char-table" ,table ?a 9) ("char-table-bad" ,table -1 9)
                 ("list" ,(list 1 2) 0 9) ("nil" nil 0 9) ("float-index" ,(vector 1) 0.0 9) ("big-index" ,(vector 1) ,(expt 2 70) 9)))
      (let ((a (nth 1 c)) (i (nth 2 c)) (v (nth 3 c)))
        (push (format "aset %s: %s then %S" (car c) (bok-show (lambda () (funcall bok-aset a i v))) a) lines)))
    (dolist (c `(("nil" nil) ("list" (1 2 3)) ("dotted" (1 2 . 3)) ("cycle" ,(bok-cycle 3))
                 ("string" "abc") ("vector" [1 2]) ("record" ,(record 'a 1)) ("bool" ,(make-bool-vector 3 nil))
                 ("char-table" ,table) ("number" 1) ("symbol" a) ("float" 1.0)))
      (push (format "length %s: %s | %s" (car c) (bok-show (lambda () (funcall bok-length (nth 1 c))))
                    (bok-show (lambda () (funcall 'length (nth 1 c)))))
            lines))
    (dolist (n `(0 1 2 3 4 -1 127 128 1000 ,most-positive-fixnum ,(1+ most-positive-fixnum) 1.0 nil))
      (dolist (c `(("nil" nil) ("list" (1 2 3)) ("dotted" (1 2 . 3)) ("cycle" ,(bok-cycle 3)) ("number" 1)))
        (push (format "nthcdr %S %s: %s" n (car c)
                      (let ((r (bok-show (lambda () (let ((v (funcall bok-nthcdr n (nth 1 c)))) (if (consp v) (car v) v))))))
                        r))
              lines)))
    (dolist (c `(("hit" 2 (1 2 3)) ("miss" 9 (1 2 3)) ("string" "b" ("a" "b")) ("float" 1.0 (1 1.0))
                 ("list" (1) (0 (1))) ("nil-list" 1 nil) ("dotted" 9 (1 . 2)) ("dotted-hit" 1 (1 . 2))
                 ("not-list" 1 2) ("cycle" 9 ,(bok-cycle 3)) ("cycle-hit" 1 ,(bok-cycle 3))))
      (push (format "member %s: %s | %s" (car c)
                    (bok-show (lambda () (let ((v (funcall bok-member (nth 1 c) (nth 2 c)))) (if (consp v) (car v) v))))
                    (bok-show (lambda () (let ((v (funcall 'member (nth 1 c) (nth 2 c)))) (if (consp v) (car v) v)))))
            lines))
    (dolist (c `((bok-symbol bok-a) (bok-symbol bok-b) (bok-symbol bok-c) (nil bok-a) (t bok-a) (1 bok-a) ("s" bok-a) (:bok-key bok-a)))
      (push (format "get %S: %s | %s" c (bok-show (lambda () (funcall bok-get (nth 0 c) (nth 1 c))))
                    (bok-show (lambda () (funcall 'get (nth 0 c) (nth 1 c)))))
            lines))
    (push (format "list: %S %S %S" (funcall bok-list0) (funcall bok-list1 1) (funcall bok-list3 1 2 3)) lines)
    (nreverse lines)))

(defun bok-run ()
  "Every case, one per line."
  (with-current-buffer (setq bok-buffer (get-buffer-create "bok"))
    (insert "0123456789")
    (setq bok-marker (copy-marker 5))
    (setq bok-unset-marker (make-marker))
    (mapconcat #'identity
               (append (bok-numeric-lines) (bok-equal-lines) (bok-sequence-lines))
               "\n")))

;;; byte_op_kernels.el ends here
