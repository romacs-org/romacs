;;; romacs-agent-plan-offer-rules-tests.el --- When an offer to proceed stands -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A plan offered to proceed with asks its user only while its session
;; plans, goes on and is not put aside; it is proceeded with only once
;; every record of the session is folded in; the menu offers for it only
;; what proceeds.

;;; Code:

(require 'ert)
(require 'cl-lib)
(let ((load-path (cons (file-name-directory load-file-name) load-path)))
  (require 'romacs-agent-test-stored))
(require 'romacs-agent-summary)
(require 'romacs-agent-board)
(require 'romacs-agent-menu)

(defconst romacs-agent-plan-offer-rules-tests--id "aa720000-0000-4000-8000-000000000001")

(defmacro romacs-agent-plan-offer-rules-tests--with (&rest body)
  "Run BODY with a private store and `session', in plan mode, offering the
plan of seq 43."
  (declare (indent 0))
  `(romacs-agent-test-with-store
     (let* ((romacs-agent--sessions (make-hash-table :test #'equal))
            (romacs-agent--children (make-hash-table :test #'equal))
            (romacs-agent--summaries (make-hash-table :test #'equal))
            (romacs-agent-log--chats (make-hash-table :test #'equal))
            (romacs-agent-writer-queued nil)
            (romacs-agent-board-source romacs-agent-log-local)
            (romacs-agent-event-functions nil)
            (romacs-agent-glyphs nil)
            (id romacs-agent-plan-offer-rules-tests--id)
            (session (romacs-agent--make :id id :dir (expand-file-name id (romacs-agent--state-root))
                                         :harness 'codex :provider "codex" :cwd "/p/rules/" :title "Offered"
                                         :state 'idle :options '(:plan t) :seq 43 :done-seq 43 :plan-offer 43)))
       (make-directory (romacs-agent-dir session) t)
       (puthash id session romacs-agent--sessions)
       (cl-letf (((symbol-function 'romacs-agent--note) #'ignore)
                 ((symbol-function 'romacs-agent--manifest-put) #'ignore)
                 ((symbol-function 'romacs-agent--keep-soon) #'ignore)
                 ((symbol-function 'romacs-agent-history-items) #'ignore))
         ,@body))))

(defun romacs-agent-plan-offer-rules-tests--key ()
  (romacs-agent-summary-state-key (romacs-agent-summary romacs-agent-plan-offer-rules-tests--id)))

(defun romacs-agent-plan-offer-rules-tests--board (settled)
  "The board's thread rows as (ID KEY), its Settled section with SETTLED."
  (let ((view (list :settled settled :folds (make-hash-table :test #'equal))))
    (cl-loop for row in (romacs-agent-board-rows (romacs-agent-summary nil) nil view)
             when (eq (plist-get row :kind) 'thread)
             collect (list (plist-get row :id) (plist-get row :key)))))

(ert-deftest romacs-agent-plan-offer-rules-settled-is-settled ()
  (romacs-agent-plan-offer-rules-tests--with
    (should (eq (romacs-agent-plan-offer-rules-tests--key) 'question))
    (should (equal (romacs-agent-plan-offer-rules-tests--board nil) `((,id question))))
    (romacs-agent-settle session)
    (should (eq (romacs-agent-plan-offer-rules-tests--key) 'settled))
    ;; The board and its Settled section agree: it is settled.
    (should-not (romacs-agent-plan-offer-rules-tests--board nil))
    (should (equal (romacs-agent-plan-offer-rules-tests--board t) `((,id settled))))
    (romacs-agent-settle session t)
    (should (eq (romacs-agent-plan-offer-rules-tests--key) 'question))
    ;; Its activity acknowledged through its last record: settled too.
    (setf (romacs-agent-settled-seq session) 43)
    (romacs-agent--changed session)
    (should (romacs-agent-settled-p session))
    (should (eq (romacs-agent-plan-offer-rules-tests--key) 'settled))
    (should-not (romacs-agent-plan-offer-rules-tests--board nil))
    (should (equal (romacs-agent-plan-offer-rules-tests--board t) `((,id settled))))))

(ert-deftest romacs-agent-plan-offer-rules-one-rule-for-what-waits ()
  ;; Its board row, its summary's wait, its menu's "Waits on you" and its
  ;; chat's header say one thing: an offer of a thread put aside or gone
  ;; waits on no one.
  (romacs-agent-plan-offer-rules-tests--with
    (with-temp-buffer
      (romacs-agent-log-mode)
      (romacs-agent-log--set-session session)
      (cl-flet ((waits ()
                  (let ((summary (romacs-agent-summary id)))
                    (list (romacs-agent-summary-state-key summary)
                          (romacs-agent-summary-user-wait-kind summary) (romacs-agent-summary-user-wait-count summary)
                          (and (romacs-agent-menu--waits-p) t)
                          (romacs-agent-log--waiting-count)))))
        (should (equal (waits) '(question plan 1 t 1)))
        (romacs-agent-settle session)
        (should (equal (waits) '(settled nil 0 nil 0)))
        (romacs-agent-settle session t)
        (dolist (state '(ended detached))
          (setf (romacs-agent-state session) state)
          (romacs-agent--changed session)
          (should (equal (cdr (waits)) '(nil 0 nil 0))))
        (setf (romacs-agent-state session) 'idle (romacs-agent-cancelled session) t)
        (romacs-agent--changed session)
        (should (equal (waits) '(ended nil 0 nil 0)))))))

(ert-deftest romacs-agent-plan-offer-rules-only-a-session-going-on-is-asked ()
  (romacs-agent-plan-offer-rules-tests--with
    (dolist (state '(ended detached))
      (setf (romacs-agent-state session) state)
      (romacs-agent--changed session)
      (should-not (eq (romacs-agent-plan-offer-rules-tests--key) 'question)))
    (setf (romacs-agent-state session) 'idle (romacs-agent-cancelled session) t)
    (romacs-agent--changed session)
    (should (eq (romacs-agent-plan-offer-rules-tests--key) 'ended))))

(ert-deftest romacs-agent-plan-offer-rules-proceed-waits-for-the-records ()
  (romacs-agent-plan-offer-rules-tests--with
    (let ((connection (make-pipe-process :name "plan-offer-rules" :noquery t)))
      (unwind-protect
          (progn
            ;; Attached, its replay under way: what it offers is not known yet.
            (setf (romacs-agent-connection session) connection)
            (should-error (romacs-agent-proceed id 43) :type 'user-error)
            (should (plist-get (romacs-agent-options session) :plan))
            ;; The replay takes the plan back: the offer the row named is gone.
            (romacs-agent--raise session '(:type rolled-back :to "p40" :turns 1 :seq 44))
            (setf (romacs-agent-caught-up session) t)
            (should-error (romacs-agent-proceed id 43) :type 'user-error)
            (should (plist-get (romacs-agent-options session) :plan))
            ;; Offered again, caught up: proceeded with.
            (romacs-agent--raise session '(:type plan :text "1. Again" :proceed t :seq 45))
            (romacs-agent-proceed id 45)
            (should-not (plist-get (romacs-agent-options session) :plan)))
        (delete-process connection)))))

(ert-deftest romacs-agent-plan-offer-rules-an-offer-needs-plan-mode ()
  (romacs-agent-plan-offer-rules-tests--with
    ;; Left out of plan mode with an offer its records or row still name.
    (setf (romacs-agent-options session) nil)
    (romacs-agent--changed session)
    (should-not (romacs-agent-summary-plan-offer (romacs-agent-summary id)))
    (should-not (eq (romacs-agent-plan-offer-rules-tests--key) 'question))
    (should-error (romacs-agent-proceed id 43) :type 'user-error)
    ;; As its row says it, after a restart: plan 0, plan_offer 43.
    (should (romacs-agent-store--put session nil))
    (remhash id romacs-agent--sessions)
    (should (equal (sqlite-select (romacs-agent-store--open) "select plan, plan_offer from sessions") '((0 43))))
    (should-not (romacs-agent-summary-plan-offer (romacs-agent-summary id)))
    (should-not (romacs-agent-summary-user-wait-kind (romacs-agent-summary id)))
    (should-not (eq (romacs-agent-plan-offer-rules-tests--key) 'question))))

(ert-deftest romacs-agent-plan-offer-rules-the-menu-offers-what-proceeds ()
  (romacs-agent-plan-offer-rules-tests--with
    (with-temp-buffer
      (romacs-agent-log-mode)
      (romacs-agent-log--set-session session)
      (let ((menu (keymap-popup--render (keymap-popup--meta romacs-agent-menu-map 'descriptions))))
        (should (string-match-p "Waits on you" menu))
        (should (string-match-p "y +Allow" menu))
        (dolist (entry '("Allow for this session" "Deny" "Go to it"))
          (should-not (string-search entry menu)))))))

(provide 'romacs-agent-plan-offer-rules-tests)
;;; romacs-agent-plan-offer-rules-tests.el ends here
