;;; romacs-agent-tools-guard-tests.el --- The summary boundary's guard, further -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; More reads under the guard of romacs-agent-tools-summary-tests: only
;; the owner lookup by id (`romacs-agent-tools--ask-owner') is exempt in
;; resolving a task, and the pools and a wake's rule read summaries.

;;; Code:

(require 'ert)
(require 'cl-lib)
(let ((load-path (cons (file-name-directory load-file-name) load-path)))
  (require 'romacs-agent-tools-summary-tests))

(ert-deftest romacs-agent-tools-guard-a-read-in-the-resolver-trips ()
  ;; A record read in the resolver, outside its owner lookup, is caught.
  ;; Called by name: a compiled call of an accessor is inlined, and no
  ;; advice sees it.
  (romacs-agent-tools-summary-tests--with
    (let ((read (lambda (&rest _)
                  (funcall 'romacs-agent-title
                           (gethash (romacs-agent-tools-summary-tests--id 5) romacs-agent--sessions)))))
      (unwind-protect
          (progn
            (advice-add 'romacs-agent-tools--resolve :before read)
            (should-error (romacs-agent-tools-summary-tests--guard
                           (lambda () (romacs-agent-tools-summary-tests--status 5)))
                          :type 'ert-test-failed))
        (advice-remove 'romacs-agent-tools--resolve read)))))

(ert-deftest romacs-agent-tools-guard-pools-read-summaries ()
  (romacs-agent-tools-summary-tests--with
    (setf (romacs-agent-state (gethash (romacs-agent-tools-summary-tests--id 2) romacs-agent--sessions)) 'running)
    (romacs-agent--summary-stale (romacs-agent-tools-summary-tests--id 2))
    (let* ((pools (romacs-agent-tools-summary-tests--guard #'romacs-agent-tools--pools))
           (claude (cl-find "claude" pools :key (lambda (pool) (gethash "harness" pool)) :test #'equal))
           (account (aref (gethash "accounts" claude) 0)))
      (should (equal (gethash "providerInstanceId" account) "claude"))
      ;; At work on it: tasks 2 and 3, running, and 4, waiting on an approval.
      (should (= (gethash "live" account) 3)))))

(ert-deftest romacs-agent-tools-guard-a-wake-rule-reads-the-summary ()
  (romacs-agent-tools-summary-tests--with
    (let ((quiet (gethash (romacs-agent-tools-summary-tests--id 3) romacs-agent--sessions))
          (ends (gethash (romacs-agent-tools-summary-tests--id 2) romacs-agent--sessions)))
      (should-not (romacs-agent-tools-summary-tests--guard
                   (lambda () (romacs-agent-tools--wakes-p quiet '(1 "done" nil))) t))
      (should (romacs-agent-tools-summary-tests--guard
               (lambda () (romacs-agent-tools--wakes-p ends '(1 "done" nil))) t)))))

(provide 'romacs-agent-tools-guard-tests)
;;; romacs-agent-tools-guard-tests.el ends here
