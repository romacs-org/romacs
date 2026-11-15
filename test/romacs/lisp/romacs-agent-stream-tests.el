;;; romacs-agent-stream-tests.el --- A supervisor's records and a journal read in pieces -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))

(ert-deftest romacs-agent-stream-records-in-any-chunks ()
  ;; A record split over many chunks is handled once, whole; two in one
  ;; chunk, each; a record's start ending a chunk waits for its rest.
  (let* ((session (romacs-agent--make :id "s" :harness 'claude))
         (process (make-process :name "romacs-stream-fixture" :command '("cat") :noquery t))
         (seen nil)
         (long (make-string 200000 ?é)))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent--kept) #'ignore)
                  ((symbol-function 'romacs-agent--record)
                   (lambda (_s record) (push (gethash "seq" record) seen))))
          (process-put process 'romacs-agent session)
          (let ((line (encode-coding-string (concat (json-serialize (romacs-mcp--object "seq" 1 "text" long)) "\n") 'utf-8)))
            ;; 1,000 chunks, some cutting a character's bytes.
            (let ((step (/ (length line) 1000)))
              (cl-loop for at from 0 below (length line) by step
                       do (romacs-agent--filter process (substring line at (min (length line) (+ at step)))))))
          (romacs-agent--filter process "{\"seq\":2}\n{\"seq\":3}\n{\"se")
          (should (equal (reverse seen) '(1 2 3)))
          (romacs-agent--filter process "q\":4}\n")
          (should (equal (reverse seen) '(1 2 3 4)))
          (should-not (process-get process 'pending)))
      (delete-process process))))

(ert-deftest romacs-agent-stream-journal-lines-by-offset ()
  ;; A journal read a slice at a time, slices smaller than a line and
  ;; cutting characters: every line once, decoded, its last unended line
  ;; left for later.
  (let ((file (make-temp-file "romacs-journal-")) (romacs-agent--journal-slice 7))
    (unwind-protect
        (progn
          (let ((coding-system-for-write 'utf-8-unix))
            (write-region "{\"a\":\"ééé\"}\n{\"b\":\"a much longer line than a slice\"}\n{\"c\":1}\n{\"d\"" nil file nil 'silent))
          (let ((offset 0) (lines nil))
            (while offset
              (pcase (romacs-agent--journal-lines file offset)
                (`(,read . ,next) (setq lines (append lines read) offset next))
                (_ (setq offset nil))))
            (should (equal lines '("{\"a\":\"ééé\"}" "{\"b\":\"a much longer line than a slice\"}" "{\"c\":1}")))))
      (delete-file file))))

;;; romacs-agent-stream-tests.el ends here
