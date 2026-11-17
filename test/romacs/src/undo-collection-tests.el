;;; undo-collection-tests.el --- What a collection does to undo lists -*- lexical-binding: t -*-

;; This file is part of Romacs.

;;; Commentary:

;; alloc.c garbage_collect, buffer.c compact_buffer and undo.c
;; truncate_undo_list: at a collection an undo list is cut to
;; `undo-limit' and `undo-strong-limit', `undo-outer-limit-function' is
;; offered a first record past `undo-outer-limit', and (MARKER . ADJUSTMENT)
;; entries whose marker nothing else holds go.
;;
;; Every test here passes on the pinned GNU as well: the sizes the limits
;; compare against are GNU's on a 64-bit build, a cons 16 bytes and a string
;; 32 and its characters.  An entry (1 . 2) is 32 bytes, and a boundary 16.
;;
;; What a marker nothing holds loses is tested in
;; crates/romacs-vm/tests/undo_collection.rs instead, in forms each run as a
;; request of its own: inside one, as here, a register a frame has finished with
;; still holds what it held.

;;; Code:

(require 'ert)
(require 'cl-lib)

(defmacro undo-collection-tests--with-limits (limits &rest body)
  "Run BODY in a temporary buffer with undo on and LIMITS, (LIMIT STRONG OUTER)."
  (declare (indent 1))
  `(with-temp-buffer
     (buffer-enable-undo)
     (let ((undo-limit ,(nth 0 limits))
           (undo-strong-limit ,(nth 1 limits))
           (undo-outer-limit ,(nth 2 limits))
           (undo-outer-limit-function nil))
       ,@body)))

(defun undo-collection-tests--collect ()
  "Collect twice."
  (garbage-collect)
  (garbage-collect)
  nil)

(defun undo-collection-tests--touch ()
  "Change the buffer, so the next collection truncates its undo list.
compact_buffer passes over a buffer unchanged since the last one."
  (let ((buffer-undo-list t))
    (insert "x")))

(defun undo-collection-tests--marker-entries (list)
  "How many (MARKER . ADJUSTMENT) entries LIST has."
  (cl-count-if (lambda (entry) (and (consp entry) (markerp (car entry)))) list))

(defun undo-collection-tests--fill (entries)
  "Make the undo list ENTRIES, with a (MARKER . 1) for a new marker for each `m'.
No other reference to that marker is left when this returns."
  (let ((marker (point-marker)))
    (setq buffer-undo-list
          (mapcar (lambda (entry) (if (eq entry 'm) (cons marker 1) entry)) entries)))
  nil)

;;; Weak marker entries

(ert-deftest undo-collection-keeps-an-entry-whose-marker-is-held ()
  (with-temp-buffer
    (buffer-enable-undo)
    (insert "hello world")
    (setq buffer-undo-list nil)
    (let ((held (copy-marker 5)))
      (delete-region 3 8)
      (undo-collection-tests--collect)
      (should (= 1 (undo-collection-tests--marker-entries buffer-undo-list)))
      (should (eq (car (car (cdr buffer-undo-list)))
                  held))
      ;; The entry still does its job.
      (should (= (marker-position held) 3))
      (primitive-undo 1 buffer-undo-list)
      (should (equal (buffer-string) "hello world"))
      (should (= (marker-position held) 5)))))

(ert-deftest undo-collection-a-list-something-else-holds-keeps-its-markers ()
  ;; Held through the variable, the entries are reached before the list is
  ;; looked at, and so are their markers.
  (with-temp-buffer
    (buffer-enable-undo)
    (undo-collection-tests--fill '(m (1 . 2)))
    (let ((saved buffer-undo-list))
      (undo-collection-tests--collect)
      (should (eq buffer-undo-list saved))
      (should (= 1 (undo-collection-tests--marker-entries buffer-undo-list))))))

(ert-deftest undo-collection-an-improper-list-survives ()
  (with-temp-buffer
    (buffer-enable-undo)
    (setq buffer-undo-list (cons '(1 . 2) 3))
    (undo-collection-tests--collect)
    (should (equal buffer-undo-list '((1 . 2) . 3)))))

(ert-deftest undo-collection-cuts-an-improper-list-to-its-last-boundary ()
  ;; truncate_undo_list finds no nil at the end, so its walk was no whole one,
  ;; and it cuts where it last could.
  (undo-collection-tests--with-limits (1000 2000 100000)
    (undo-collection-tests--touch)
    (setq buffer-undo-list (cons '(1 . 2) (cons nil (cons '(3 . 4) 3))))
    (undo-collection-tests--collect)
    (should (equal buffer-undo-list '((1 . 2))))
    ;; With no boundary to cut at, nothing is kept.
    (undo-collection-tests--touch)
    (setq buffer-undo-list (cons '(1 . 2) 3))
    (undo-collection-tests--collect)
    (should (null buffer-undo-list))))

;;; Truncation

(ert-deftest undo-collection-cuts-a-long-history-to-the-limits ()
  (with-temp-buffer
    (buffer-enable-undo)
    (let ((undo-limit 1000) (undo-strong-limit 2000) (undo-outer-limit 100000))
      (dotimes (_ 2000) (insert "x") (undo-boundary))
      (should (= 4001 (length buffer-undo-list)))
      (garbage-collect)
      (should (= 42 (length buffer-undo-list))))))

(ert-deftest undo-collection-leaves-a-disabled-list-alone ()
  (with-temp-buffer
    (buffer-disable-undo)
    (insert "text")
    (undo-collection-tests--collect)
    (should (eq buffer-undo-list t))))

(ert-deftest undo-collection-leaves-a-short-list-alone ()
  (undo-collection-tests--with-limits (1000 2000 100000)
    (undo-collection-tests--touch)
    (let ((list (list '(1 . 2) nil '(3 . 4))))
      (setq buffer-undo-list list)
      (undo-collection-tests--collect)
      (should (eq buffer-undo-list list))
      (should (equal list '((1 . 2) nil (3 . 4)))))))

(ert-deftest undo-collection-keeps-the-first-record-over-the-limits ()
  (undo-collection-tests--with-limits (10 20 100000)
    (undo-collection-tests--touch)
    (setq buffer-undo-list
          (list (cons (make-string 1000 ?a) 1) '(5 . 6) nil '(1 . 2) nil '(3 . 4)))
    (undo-collection-tests--collect)
    (should (= 2 (length buffer-undo-list)))
    (should (equal (nth 1 buffer-undo-list) '(5 . 6)))))

(ert-deftest undo-collection-skips-a-leading-boundary ()
  (undo-collection-tests--with-limits (10 20 100000)
    (undo-collection-tests--touch)
    (setq buffer-undo-list (list nil '(1 . 2) nil '(3 . 4)))
    (undo-collection-tests--collect)
    (should (equal buffer-undo-list '(nil (1 . 2))))))

(ert-deftest undo-collection-past-the-limit-cuts-after-the-record ()
  ;; At the second boundary the size is 80: past `undo-limit', within
  ;; `undo-strong-limit'.  The record that passed the limit stays.
  (undo-collection-tests--with-limits (60 1000 100000)
    (undo-collection-tests--touch)
    (setq buffer-undo-list (list '(1 . 2) nil '(3 . 4) nil '(5 . 6) nil '(7 . 8)))
    (undo-collection-tests--collect)
    (should (equal buffer-undo-list '((1 . 2) nil (3 . 4))))))

(ert-deftest undo-collection-past-the-strong-limit-cuts-before-the-record ()
  ;; At the second boundary the size is 80: past `undo-strong-limit' too.
  ;; The record that passed it goes.
  (undo-collection-tests--with-limits (60 70 100000)
    (undo-collection-tests--touch)
    (setq buffer-undo-list (list '(1 . 2) nil '(3 . 4) nil '(5 . 6) nil '(7 . 8)))
    (undo-collection-tests--collect)
    (should (equal buffer-undo-list '((1 . 2))))))

(ert-deftest undo-collection-counts-a-deleted-string-as-gnu-does ()
  ;; Entry 32 and 16 for its cons, and 31 and the characters for its string:
  ;; (("abcdefghij" . 1)) is 73.  The boundary after it is past 72 and not 74.
  (dolist (limit-and-kept '((72 . 1) (74 . 3)))
    (undo-collection-tests--with-limits ((car limit-and-kept) 1000 100000)
      (undo-collection-tests--touch)
      (setq buffer-undo-list (list (cons "abcdefghij" 1) nil '(1 . 2) nil '(3 . 4)))
      (undo-collection-tests--collect)
      (should (= (cdr limit-and-kept) (length buffer-undo-list))))))

(ert-deftest undo-collection-passes-over-a-buffer-unchanged-since-the-last ()
  (undo-collection-tests--with-limits (40 80 100000)
    (undo-collection-tests--touch)
    (undo-collection-tests--collect)
    (let ((long (list '(1 . 2) nil '(3 . 4) nil '(5 . 6))))
      (setq buffer-undo-list long)
      (undo-collection-tests--collect)
      (should (= 5 (length buffer-undo-list)))
      ;; An edit, and the next one cuts it.
      (undo-collection-tests--touch)
      (undo-collection-tests--collect)
      (should (= 3 (length buffer-undo-list))))))

(ert-deftest undo-collection-reads-limits-as-the-buffer-sees-them ()
  (let ((tight (generate-new-buffer " *undo-tight*"))
        (loose (generate-new-buffer " *undo-loose*"))
        (undo-limit 100000)
        (undo-strong-limit 200000)
        (undo-outer-limit 1000000))
    (unwind-protect
        (progn
          (with-current-buffer tight
            (buffer-enable-undo)
            (setq-local undo-limit 40)
            (setq-local undo-strong-limit 80))
          (with-current-buffer loose (buffer-enable-undo))
          (dolist (buffer (list tight loose))
            (with-current-buffer buffer
              (undo-collection-tests--touch)
              (setq buffer-undo-list (list '(1 . 2) nil '(3 . 4) nil '(5 . 6)))))
          (undo-collection-tests--collect)
          (should (= 3 (length (buffer-local-value 'buffer-undo-list tight))))
          (should (= 5 (length (buffer-local-value 'buffer-undo-list loose)))))
      (kill-buffer tight)
      (kill-buffer loose))))

;;; `undo-outer-limit-function'

(ert-deftest undo-collection-offers-a-first-record-past-the-outer-limit ()
  (undo-collection-tests--with-limits (1000 2000 72)
    (let* ((asked nil) (buffer (current-buffer))
           (undo-outer-limit-function
            (lambda (size) (push (list size (current-buffer)) asked) t))
           (list (list (cons "abcdefghij" 1) nil '(1 . 2))))
      (undo-collection-tests--touch)
      (setq buffer-undo-list list)
      (garbage-collect)
      (should (equal asked (list (list 73 buffer)))))))

(ert-deftest undo-collection-does-not-offer-a-record-within-the-outer-limit ()
  (undo-collection-tests--with-limits (1000 2000 73)
    (let* ((asked nil)
           (undo-outer-limit-function (lambda (size) (push size asked) t)))
      (undo-collection-tests--touch)
      (setq buffer-undo-list (list (cons "abcdefghij" 1) nil '(1 . 2)))
      (garbage-collect)
      (should (null asked)))))

(ert-deftest undo-collection-a-non-nil-answer-leaves-the-list-to-the-function ()
  (undo-collection-tests--with-limits (10 20 40)
    (let ((undo-outer-limit-function (lambda (_) t))
          (list (list (cons "abcdefghij" 1) nil '(1 . 2) nil '(3 . 4))))
      (undo-collection-tests--touch)
      (setq buffer-undo-list list)
      (garbage-collect)
      (should (eq buffer-undo-list list))
      (should (= 5 (length buffer-undo-list))))))

(ert-deftest undo-collection-the-function-may-clear-the-list ()
  (undo-collection-tests--with-limits (10 20 40)
    (let ((undo-outer-limit-function
           (lambda (_) (setq buffer-undo-list nil) t)))
      (undo-collection-tests--touch)
      (setq buffer-undo-list (list (cons "abcdefghij" 1) nil '(1 . 2)))
      (garbage-collect)
      (should (null buffer-undo-list)))))

(ert-deftest undo-collection-a-nil-answer-truncates-as-without-the-function ()
  (undo-collection-tests--with-limits (10 20 40)
    (let* ((asked 0)
           (undo-outer-limit-function
            (lambda (_) (setq asked (1+ asked)) nil)))
      (undo-collection-tests--touch)
      (setq buffer-undo-list (list (cons "abcdefghij" 1) nil '(1 . 2) nil '(3 . 4)))
      (garbage-collect)
      (should (= asked 1))
      (should (= 1 (length buffer-undo-list))))))

(ert-deftest undo-collection-a-quit-leaves-the-offer-to-the-next-collection ()
  ;; compact_buffer does not get to BUF_COMPACT, and the next one asks again.
  (undo-collection-tests--with-limits (1000 2000 40)
    (let* ((asked 0)
           (undo-outer-limit-function
            (lambda (_)
              (setq asked (1+ asked))
              (if (= asked 1) (signal 'quit nil) t))))
      (undo-collection-tests--touch)
      (setq buffer-undo-list (list (cons "abcdefghij" 1) nil '(1 . 2)))
      (should (eq 'quit (condition-case nil (progn (garbage-collect) 'done) (quit 'quit))))
      (should (= asked 1))
      (garbage-collect)
      (should (= asked 2))
      ;; Answered, and the buffer has not changed since.
      (garbage-collect)
      (should (= asked 2)))))

(ert-deftest undo-collection-a-nil-answer-cuts-the-offered-buffers-list ()
  ;; Whichever buffer the function leaves current.
  (undo-collection-tests--with-limits (10 20 40)
    (let* ((other (current-buffer))
           (offered (generate-new-buffer "undo-collection-offered"))
           (undo-outer-limit-function
            (lambda (_) (set-buffer other) nil)))
      (unwind-protect
          (progn
            (with-current-buffer offered
              (buffer-enable-undo)
              (undo-collection-tests--touch)
              (setq buffer-undo-list (list (cons "abcdefghij" 1) nil '(1 . 2) nil '(3 . 4))))
            (setq buffer-undo-list (list '(1 . 2)))
            (garbage-collect)
            (should (= 1 (length (buffer-local-value 'buffer-undo-list offered)))))
        (kill-buffer offered)))))

(ert-deftest undo-collection-a-collection-in-the-function-does-not-ask-again ()
  (undo-collection-tests--with-limits (1000 2000 40)
    (let* ((asked 0)
           (undo-outer-limit-function
            (lambda (_) (setq asked (1+ asked)) (garbage-collect) t)))
      (undo-collection-tests--touch)
      (setq buffer-undo-list (list (cons "abcdefghij" 1) nil '(1 . 2)))
      (garbage-collect)
      (should (= asked 1)))))

(ert-deftest undo-collection-without-a-function-the-outer-limit-does-nothing ()
  (undo-collection-tests--with-limits (1000 2000 40)
    (undo-collection-tests--touch)
    (setq buffer-undo-list (list (cons "abcdefghij" 1) nil '(1 . 2)))
    (garbage-collect)
    (should (= 3 (length buffer-undo-list)))))

(ert-deftest undo-collection-a-non-integer-outer-limit-offers-nothing ()
  (undo-collection-tests--with-limits (1000 2000 nil)
    (let* ((asked nil)
           (undo-outer-limit-function (lambda (size) (push size asked) t)))
      (undo-collection-tests--touch)
      (setq buffer-undo-list (list (cons "abcdefghij" 1)))
      (garbage-collect)
      (should (null asked)))))

(ert-deftest undo-collection-the-shipped-function-warns-and-clears ()
  ;; simple.el's `undo-outer-limit-truncate', as a real session has it.
  (with-temp-buffer
    (buffer-enable-undo)
    (let ((undo-limit 1000) (undo-strong-limit 2000) (undo-outer-limit 40)
          (undo-outer-limit-function #'undo-outer-limit-truncate)
          (warning-minimum-level :emergency)
          (warning-minimum-log-level :emergency))
      (undo-collection-tests--touch)
      (setq buffer-undo-list (list (cons "abcdefghij" 1) nil '(1 . 2)))
      (garbage-collect)
      (should (null buffer-undo-list)))))

;;; Indirect buffers

(defmacro undo-collection-tests--with-indirect (clone &rest body)
  "Run BODY with `base' and an indirect `indirect' of it, cloned if CLONE."
  (declare (indent 1))
  `(let* ((base (generate-new-buffer "undo-collection-base"))
          (indirect nil))
     (unwind-protect
         (progn
           (with-current-buffer base
             (buffer-enable-undo)
             (insert "hello world")
             (setq buffer-undo-list nil))
           (setq indirect (make-indirect-buffer base "undo-collection-indirect" ,clone))
           ,@body)
       (when (buffer-live-p indirect) (kill-buffer indirect))
       (kill-buffer base))))

(ert-deftest undo-collection-truncates-a-list-through-its-indirect-buffer ()
  (undo-collection-tests--with-indirect nil
    (let ((undo-limit 1000) (undo-strong-limit 2000) (undo-outer-limit 100000))
      (with-current-buffer indirect
        (dotimes (_ 2000) (insert "x") (undo-boundary))
        (garbage-collect)
        (should (= 42 (length buffer-undo-list))))
      (should (= 42 (length (buffer-local-value 'buffer-undo-list base)))))))

(ert-deftest undo-collection-the-function-sees-the-base-buffer-current ()
  (undo-collection-tests--with-indirect nil
    (let* ((asked nil)
           (undo-limit 1000) (undo-strong-limit 2000) (undo-outer-limit 40)
           (undo-outer-limit-function
            (lambda (_) (push (current-buffer) asked) t)))
      (with-current-buffer indirect
        (let ((buffer-undo-list t)) (insert "x"))
        (setq buffer-undo-list (list (cons "abcdefghij" 1) nil '(1 . 2)))
        (garbage-collect))
      (should (equal asked (list base))))))

;;; A unibyte buffer

;;; A killed buffer

(ert-deftest undo-collection-a-killed-buffer-has-no-list ()
  (let ((buffer (generate-new-buffer "undo-collection-killed")))
    (with-current-buffer buffer
      (buffer-enable-undo)
      (insert "text"))
    (kill-buffer buffer)
    (undo-collection-tests--collect)
    (should (null (buffer-local-value 'buffer-undo-list buffer)))))

(provide 'undo-collection-tests)
;;; undo-collection-tests.el ends here
