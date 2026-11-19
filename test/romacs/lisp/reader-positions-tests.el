;;; reader-positions-tests.el --- Where a read leaves its stream -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)

;; The reader takes a buffer's characters ahead in runs, so it must leave
;; point, or the marker it reads from, exactly where pinned GNU does: after
;; a read, at end of file, after a syntax error, in a narrowed buffer, in a
;; unibyte buffer, across a `\N{NAME}' that calls Lisp, and where one
;; character read ahead is given back.

(defun reader-positions-tests--reads (text &optional narrow)
  "Three reads of TEXT in a buffer (narrowed to 1..NARROW): each value or
error and where point is after it."
  (with-temp-buffer
    (insert text)
    (when narrow (narrow-to-region 1 narrow))
    (goto-char (point-min))
    (let (out)
      (dotimes (_ 3)
        (push (condition-case e
                  (let ((v (read (current-buffer)))) (list :ok (if (stringp v) (append v nil) v) (point)))
                (error (list :err (car e) (point))))
              out))
      (nreverse out))))

(ert-deftest reader-positions-tests-a-read-leaves-its-stream-where-gnu-does ()
  (should (equal
           (list
            (reader-positions-tests--reads "(a b) c")
            (reader-positions-tests--reads "(a b")
            (reader-positions-tests--reads ")x")
            (reader-positions-tests--reads "abc def ghi" 6)
            (with-temp-buffer
              (insert "foo bar") (goto-char 3)
              (let ((m (copy-marker 1))) (list (read m) (marker-position m) (point) (read m) (marker-position m))))
            (with-temp-buffer
              (set-buffer-multibyte nil) (insert "\"\377a\" x") (goto-char 1)
              (let ((v (read (current-buffer)))) (list (append v nil) (multibyte-string-p v) (point))))
            (reader-positions-tests--reads "?\\N{LATIN SMALL LETTER A} z")
            (reader-positions-tests--reads "a(b)")
            (reader-positions-tests--reads "\"tex\\\"t\" 'q [v 1]")
            (list (read-from-string "(a b) c") (read-from-string "abc def" 4)
                  (condition-case e (read-from-string "(a") (error (car e)))))
           ;; Pinned GNU's answer.
           '
           (((:ok (a b) 6) (:ok c 8) (:err end-of-file 8)) ((:err end-of-file 5) (:err end-of-file 5) (:err end-of-file 5)) ((:err invalid-read-syntax 2) (:ok x 3) (:err end-of-file 3)) ((:ok abc 4) (:ok d 6) (:err end-of-file 6)) (foo 4 3 bar 8) ((255 97) nil 5) ((:ok 97 26) (:ok z 28) (:err end-of-file 28)) ((:ok a 2) (:ok (b) 5) (:err end-of-file 5)) ((:ok (116 101 120 34 116) 9) (:ok 'q 12) (:ok [v 1] 18)) (((a b) . 5) (def . 7) end-of-file)))))

;;; reader-positions-tests.el ends here
