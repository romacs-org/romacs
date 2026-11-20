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

(ert-deftest romacs-agent-stream-a-record-that-signals-loses-none-after-it ()
  ;; A record whose handling signals (here a notes-only host's
  ;; acknowledgement) is reported, and the records sent with it are
  ;; handled all the same, nothing signalled out of the filter.
  (let* ((session (romacs-agent--make :id "s" :harness 'claude))
         (process (make-process :name "romacs-stream-fixture" :command '("cat") :noquery t))
         (seen nil) (raised nil))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent--send-held) #'ignore)
                  ((symbol-function 'romacs-agent--notes-written) (lambda (_s) (error "Broken")))
                  ((symbol-function 'romacs-agent--raise) (lambda (_s event) (push event raised)))
                  ((symbol-function 'romacs-agent--record)
                   (lambda (_s record) (push (gethash "seq" record) seen))))
          (process-put process 'romacs-agent session)
          (process-put process 'notes-only t)
          (romacs-agent--filter process "{\"attached\":true}\n{\"seq\":2}\n{\"seq\":3}\n")
          (should (process-get process 'attached))
          (should (equal (reverse seen) '(2 3)))
          ;; Reported at once, in its chat: never left silent.
          (should (equal (mapcar (lambda (e) (plist-get e :type)) raised) '(error)))
          (should (string-search "Broken" (plist-get (car raised) :text))))
      (delete-process process))))

(ert-deftest romacs-agent-stream-output-while-handling-waits ()
  ;; Output that comes while a record is handled (its handler waited on a
  ;; process) is handled after the records before it, once each.
  (let* ((session (romacs-agent--make :id "s" :harness 'claude))
         (process (make-process :name "romacs-stream-fixture" :command '("cat") :noquery t))
         (seen nil))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent--kept) #'ignore)
                  ((symbol-function 'romacs-agent--record)
                   (lambda (_s record)
                     (let ((seq (gethash "seq" record)))
                       (push seq seen)
                       (when (eql seq 1) (romacs-agent--filter process "q\":3}\n{\"seq\":4}\n"))))))
          (process-put process 'romacs-agent session)
          (romacs-agent--filter process "{\"seq\":1}\n{\"seq\":2}\n{\"se")
          (should (equal (reverse seen) '(1 2 3 4)))
          (should-not (process-get process 'pending)))
      (delete-process process))))

(ert-deftest romacs-agent-stream-replay-ends-though-the-journal-grows ()
  ;; A replay reads what the journal held when it began: a record written
  ;; meanwhile does not keep it going.
  (let* ((dir (make-temp-file "romacs-stream-" t))
         (journal (expand-file-name "journal.jsonl" dir))
         (session (romacs-agent--make :id "g" :harness 'claude :dir dir :tools (make-hash-table :test #'equal)
                                      :streamed (make-hash-table :test #'equal)))
         (steps 0) (result :none) (romacs-agent--journal-slice 16))
    (unwind-protect
        (progn
          (with-temp-file journal
            (dotimes (i 3) (insert (json-serialize (romacs-mcp--object "seq" (1+ i) "in" (romacs-mcp--object "prompt" "p"))) "\n")))
          (cl-letf (((symbol-function 'run-at-time)
                     (lambda (_time _repeat f &rest args)
                       (when (< (cl-incf steps) 50)
                         (write-region "{\"seq\":99,\"in\":{\"prompt\":\"late\"}}\n" nil journal t 'silent)
                         (apply f args)))))
            (romacs-agent-events-async session (lambda (events) (setq result events)) 1))
          (should (listp result))
          (should (< steps 10)))
      (delete-directory dir t))))

;;; romacs-agent-stream-tests.el ends here
