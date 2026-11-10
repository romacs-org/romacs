;;; fns-weak-tests.el --- Weak hash tables after a collection -*- lexical-binding: t -*-

;; Adapted from GNU's feature/igc3 test/src/fns-tests.el, which added these for
;; bugs that branch had.  GNU runs the populating step on a thread to keep stray
;; references off a conservative stack.  Romacs has no threads and exact roots,
;; so it runs on this one.

;; The four `ft-weak-*-removal' tests are the acceptance tests for tracing
;; register files only to each engine's top and clearing the dead tail above
;; it: with the whole file traced, each found one or two entries left, held by
;; stale registers.  Their `-wide' twins collect from a frame wider than the
;; populating one, whose registers are not yet written at the collection: they
;; lie below the top and can still hold what the populating callee left, so
;; only per-register liveness (stack maps) frees those, and they are expected
;; to fail until it lands (DIVERGENCES VM2-GC-FINALIZER).  Whether they fail
;; depends on frame layout: the wide frame's unwritten registers must lie where
;; the populating callee's were, so a change to register allocation can turn
;; them into unexpected passes with no change to the collector.

;;; Code:

(require 'ert)
(require 'cl-lib)

(defun ft--init-rng () (random "weak-hashtable-tests"))
(defun ft--nentries () 50)

(defun ft--format-component (num key? dead?)
  (format "%02d-%s-%s" num (if key? "key" "val") (if dead? "dead" "alive")))

(defun ft--parse-component (string)
  (or (string-match "^\\([0-9]+\\)-\\(key\\|val\\)-\\(dead\\|alive\\)$"
                    string)
      (error "Invalid argument: %S" string))
  (vector (string-to-number (match-string 1 string))
          (equal (match-string 2 string) "key")
          (equal (match-string 3 string) "dead")))

(defun ft--component-num (string) (aref (ft--parse-component string) 0))

(defun ft--dead-component (string key?)
  (ft--format-component (ft--component-num string) key? t))

;; Create NENTRIES pairs of strings and put them into TABLE.  Randomly
;; select a subset of the strings as "dead".  Return the list of pairs
;; where the "dead" strings are replaced the with nil.
(defun ft--populate-hashtable (table nentries)
  (let ((pairs '()))
    (dotimes (i nentries)
      (let* ((r (random 4))
             (key (cl-ecase r
                    ((0 2) (ft--format-component i t nil))
                    ((1 3) (ft--format-component i t t))))
             (val (cl-ecase r
                    ((0 1) (ft--format-component i nil nil))
                    ((2 3) (ft--format-component i nil t)))))
        (puthash key val table)
        (cl-ecase r
          (0 (push (cons key val) pairs))
          (1 (push (cons nil val) pairs))
          (2 (push (cons key nil) pairs))
          (3 ))))
    (nreverse pairs)))

(defun ft--hash-table-entries (table)
  (let ((entries '()))
    (maphash (lambda (k v) (push (cons k v) entries))
             table)
    entries))

(defun ft--check-entry (weakness key1 val1 key2 val2)
  (cl-ecase weakness
    (key
     (should (eq key1 key2))
     (cond (val1 (should (eq val1 val2)))
           (t (should (equal (ft--dead-component key1 nil)
                             val2)))))
    (value
     (should (eq val1 val2))
     (cond (key1 (should (eq key1 key2)))
           (t (should (equal (ft--dead-component val1 t)
                             key2)))))
    (key-and-value
     (should (eq key1 key2))
     (should (eq val1 val2)))
    (key-or-value
     (cond (key1 (should (eq key1 key2)))
           (t (should (equal (ft--dead-component val1 t)
                             key2))))
     (cond (val1 (should (eq val1 val2)))
           (t (should (equal (ft--dead-component key1 nil)
                             val2)))))))

(defun ft--check-entries (table pairs)
  (let* ((w (hash-table-weakness table))
         (expected (cl-ecase w
                     (key (cl-remove nil pairs :key #'car))
                     (value (cl-remove nil pairs :key #'cdr))
                     (key-and-value
                      (cl-remove-if (lambda (e)
                                      (not (and (car e) (cdr e))))
                                    pairs))
                     (key-or-value
                      (cl-remove-if (lambda (e)
                                      (not (or (car e) (cdr e))))
                                    pairs))))
         (actual (sort (ft--hash-table-entries table)
                       :key #'car :lessp #'string<)))
    (cl-loop for (k1 . v1) in expected
             for (k2 . v2) in actual
             do (ft--check-entry w k1 v1 k2 v2))
    (should (= (length expected) (length actual)))
    (should (= (hash-table-count table) (length expected)))))

(defun ft--gc (_execute-finalizers)
  (garbage-collect))

;; Test that weakly held objects are no longer in a hash table after a
;; GC cycle.
(defun ft--test-weak-removal (weakness)
  (let* ((_ (ft--init-rng))
         (table (make-hash-table :weakness weakness))
         (f (lambda () (ft--populate-hashtable table (ft--nentries))))
         (pairs (funcall f)))
    (ft--gc t)
    (ft--check-entries table pairs)))

(ert-deftest ft-weak-key-removal () (ft--test-weak-removal 'key))
(ert-deftest ft-weak-value-removal () (ft--test-weak-removal 'value))
(ert-deftest ft-weak-and-removal () (ft--test-weak-removal 'key-and-value))
(ert-deftest ft-weak-or-removal () (ft--test-weak-removal 'key-or-value))

;; A collecting frame 32 registers wider, none of them written before the
;; collection.
(defun ft--gc-wide (_execute-finalizers)
  (garbage-collect)
  (let* ((a0 (list 1)) (a1 (list a0)) (a2 (list a1)) (a3 (list a2)) (a4 (list a3))
         (a5 (list a4)) (a6 (list a5)) (a7 (list a6)) (a8 (list a7)) (a9 (list a8))
         (a10 (list a9)) (a11 (list a10)) (a12 (list a11)) (a13 (list a12))
         (a14 (list a13)) (a15 (list a14)) (a16 (list a15)) (a17 (list a16))
         (a18 (list a17)) (a19 (list a18)) (a20 (list a19)) (a21 (list a20))
         (a22 (list a21)) (a23 (list a22)) (a24 (list a23)) (a25 (list a24))
         (a26 (list a25)) (a27 (list a26)) (a28 (list a27)) (a29 (list a28))
         (a30 (list a29)) (a31 (list a30)))
    a31))

(defun ft--test-weak-removal-wide (weakness)
  (let* ((_ (ft--init-rng))
         (table (make-hash-table :weakness weakness))
         (f (lambda () (ft--populate-hashtable table (ft--nentries))))
         (pairs (funcall f)))
    (ft--gc-wide t)
    (ft--check-entries table pairs)))

(ert-deftest ft-weak-key-removal-wide ()
  :expected-result :failed
  (ft--test-weak-removal-wide 'key))
(ert-deftest ft-weak-value-removal-wide ()
  :expected-result :failed
  (ft--test-weak-removal-wide 'value))
(ert-deftest ft-weak-and-removal-wide ()
  :expected-result :failed
  (ft--test-weak-removal-wide 'key-and-value))
(ert-deftest ft-weak-or-removal-wide ()
  :expected-result :failed
  (ft--test-weak-removal-wide 'key-or-value))

(defun ft--test-puthash (weakness)
  (let ((h (make-hash-table :weakness weakness))
        (a (string ?a))
        (b (string ?b))
        (c (string ?c)))
    (puthash a a h)
    (should (eq (gethash a h) a))
    (puthash a b h)
    (should (eq (gethash a h) b))
    (puthash a c h)
    (should (eq (gethash a h) c))))

(ert-deftest ft-puthash-weak ()
  (dolist (w '(nil key value key-and-value key-or-value))
    (ft--test-puthash w)))

(defun ft--test-puthash-types (weakness test)
  (let ((objects
         (list 0 1 0.0
               (cons nil nil) '(nil . nil) '#1=(#1# . #1#)
               (make-symbol "uninterned") 'ft--test-puthash-types
               nil t 'error
               [] [1] (vector 2) (ash 1 128)
               "" "a" (string ?a) (make-string 10 255 t)))
        (ht (make-hash-table :weakness weakness :test test)))
    (dolist (key objects)
      (dolist (value objects)
        (puthash key value ht)
        (should (eq (gethash key ht 'test) value))
        (should (= (hash-table-count ht) 1))
        (remhash key ht)
        (should (eq (gethash key ht 'test) 'test))
        (should (= (hash-table-count ht) 0))))))

(ert-deftest ft-puthash-types-weak ()
  (dolist (w '(nil key value key-and-value key-or-value))
    (dolist (test '(eq eql equal))
      (ft--test-puthash-types w test))))

(defun ft--test-weak-fixnums (weakness test)
  (let ((h (make-hash-table :weakness weakness :test test)))
    (dotimes (i 32)
      (puthash i (lognot 2) h))
    (ft--gc t)))

(ert-deftest ft-weak-fixnums ()
  (dolist (w '(key value key-and-value key-or-value))
    (dolist (test '(eq eql equal))
      (ft--test-weak-fixnums w test))))

(defun ft--test-weak-fixnums2 (weakness test)
  (let ((h (make-hash-table :weakness weakness :test test)))
    (dotimes (i 3)
      (cl-ecase i
        (#b00 (dotimes (i 10)
                (puthash i (lognot i) h)))
        (#b01 (dotimes (i 10)
                (puthash i (cons nil nil) h)))
        (#b10 (dotimes (i 10)
                (puthash (cons nil nil) i h)))))
    (ft--gc t)))

(ert-deftest ft-weak-fixnums2 ()
  (dolist (w '(key value key-and-value key-or-value))
    (dolist (test '(eq eql equal))
      (ft--test-weak-fixnums2 w test))))

(defun ft--test-ephemeron-table (weakness)
  (let* ((h (make-hash-table :weakness weakness :test 'eq))
         (n 1000))
    (dotimes (i n)
      (let* ((obj (cons 'a i)))
        (puthash obj obj h)))
    (ft--gc t)
    (should (< (length (ft--hash-table-entries h)) n))
    (should (< (hash-table-count h) n))))

(ert-deftest ft-ephemeron-table ()
  (dolist (w '(key value key-and-value key-or-value))
    (ft--test-ephemeron-table w)))

;; This test is supposed to show a problem when finalizers are used to
;; remove hash table entries.  It's incorrect to "arm" the finalizer as
;; long as the entry can be "reached" by using an equal key.
(ert-deftest ft-weak-equal-table ()
  (let* ((h (make-hash-table :weakness 'key-or-value :test #'equal))
         (n 10)
         (root nil))
    (dotimes (i n)
      (let* ((key (list 'key i))
             (val (list 'val i)))
        (puthash key val h)))
    (should (= (hash-table-count h) n))
    (ft--gc nil)
    (setq root (gethash '(key 0) h))
    (ft--gc t)
    (should (< (hash-table-count h) n))
    (should (equal root (gethash '(key 0) h)))))

(provide 'fns-weak-tests)
;;; fns-weak-tests.el ends here
