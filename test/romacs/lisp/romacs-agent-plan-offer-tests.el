;;; romacs-agent-plan-offer-tests.el --- Session-owned offers to proceed -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

(require 'ert)
(require 'cl-lib)
(let ((load-path (cons (file-name-directory load-file-name) load-path)))
  (require 'romacs-agent-test-stored))
(require 'romacs-agent-summary)
(require 'romacs-agent-board)
(require 'romacs-agent-menu)
(require 'romacs-agent-codex)

(defconst romacs-agent-plan-offer-tests--id "aa710000-0000-4000-8000-000000000001")

(defun romacs-agent-plan-offer-tests--new ()
  (let* ((id romacs-agent-plan-offer-tests--id)
         (dir (expand-file-name id (romacs-agent--state-root)))
         (session (romacs-agent--make :id id :dir dir :harness 'codex :provider "codex"
                                      :cwd "/p/offer/" :title "Plan fixture" :state 'idle
                                      :options '(:plan t) :seq 40)))
    (make-directory dir t)
    (puthash id session romacs-agent--sessions)
    session))

(defmacro romacs-agent-plan-offer-tests--with (&rest body)
  "Run BODY with a private store, SESSION and captured supervisor NOTES."
  (declare (indent 0))
  `(romacs-agent-test-with-store
     (let* ((romacs-agent--sessions (make-hash-table :test #'equal))
            (romacs-agent--children (make-hash-table :test #'equal))
            (romacs-agent-log--chats (make-hash-table :test #'equal))
            (romacs-agent-writer-queued nil)
            (romacs-agent-board-source romacs-agent-log-local)
            (romacs-agent-board--unloaded nil)
            (romacs-agent-event-functions nil)
            (romacs-agent-loaded-functions nil)
            (romacs-agent-caught-up-functions nil)
            (romacs-agent-glyphs nil)
            (session (romacs-agent-plan-offer-tests--new))
            (notes nil))
       (cl-letf (((symbol-function 'romacs-agent--note)
                  (lambda (_owner note) (push note notes)))
                 ((symbol-function 'romacs-agent-history-items) #'ignore)
                 ((symbol-function 'romacs-agent--keep-soon) #'ignore))
         ,@body))))

(defun romacs-agent-plan-offer-tests--offer (session seq)
  "Deliver the public event decoded from a completed Codex plan item."
  (let ((event (car (romacs-agent--codex-item
                     session (romacs-mcp--object "type" "plan" "id" (format "plan-%d" seq)
                                                 "text" "1. Do the work") t))))
    (should (plist-get event :proceed))
    (setf (romacs-agent-seq session) seq)
    (romacs-agent--raise session (append event (list :seq seq :last t)))))

(defun romacs-agent-plan-offer-tests--summary ()
  (romacs-agent-summary romacs-agent-plan-offer-tests--id))

(defun romacs-agent-plan-offer-tests--current ()
  (funcall 'romacs-agent-summary-plan-offer (romacs-agent-plan-offer-tests--summary)))

(defun romacs-agent-plan-offer-tests--menu ()
  (keymap-popup--render (keymap-popup--meta romacs-agent-menu-map 'descriptions)))

(ert-deftest romacs-agent-plan-offer-with-no-chat ()
  (romacs-agent-plan-offer-tests--with
    (should (= 0 (hash-table-count romacs-agent-log--chats)))
    (romacs-agent-plan-offer-tests--offer session 41)
    (with-temp-buffer
      (romacs-agent-board-mode)
      (romacs-agent-board--render (current-buffer) 100)
      (should (string-match-p "Asks you +Plan fixture" (buffer-string))))
    (should (= 0 (hash-table-count romacs-agent-log--chats)))))

(ert-deftest romacs-agent-plan-offer-menu-waits-on-user ()
  (romacs-agent-plan-offer-tests--with
    (romacs-agent-plan-offer-tests--offer session 41)
    (with-temp-buffer
      (romacs-agent-log-mode)
      (romacs-agent-log--set-session session)
      (should (string-match-p "Waits on you" (romacs-agent-plan-offer-tests--menu))))
    (should (= 0 (hash-table-count romacs-agent-log--chats)))))

(ert-deftest romacs-agent-plan-offer-summary-waits-on-user ()
  (romacs-agent-plan-offer-tests--with
    (romacs-agent-plan-offer-tests--offer session 41)
    (should (= 0 (hash-table-count romacs-agent-log--chats)))
    (should (eq (funcall 'romacs-agent-summary-user-wait-kind (romacs-agent-plan-offer-tests--summary)) 'plan))
    (should (= (funcall 'romacs-agent-summary-user-wait-count (romacs-agent-plan-offer-tests--summary)) 1))
    (should (= (romacs-agent-plan-offer-tests--current) 41))))

(ert-deftest romacs-agent-plan-offer-requires-plan-mode ()
  (romacs-agent-plan-offer-tests--with
    (setf (romacs-agent-options session) nil)
    (romacs-agent-plan-offer-tests--offer session 41)
    (should-not (romacs-agent-plan-offer-tests--current))))

(ert-deftest romacs-agent-plan-offer-replaced-and-checked-before-proceed ()
  (romacs-agent-plan-offer-tests--with
    (romacs-agent-plan-offer-tests--offer session 41)
    (should (= (romacs-agent-plan-offer-tests--current) 41))
    (romacs-agent-plan-offer-tests--offer session 43)
    (should (= (romacs-agent-plan-offer-tests--current) 43))
    (should-error (funcall 'romacs-agent-proceed romacs-agent-plan-offer-tests--id 41) :type 'user-error)
    (should (= (romacs-agent-plan-offer-tests--current) 43))
    (should (plist-get (romacs-agent-options session) :plan))
    (should-not notes)
    (funcall 'romacs-agent-proceed romacs-agent-plan-offer-tests--id 43)
    (should-not (plist-get (romacs-agent-options session) :plan))
    (should-not (romacs-agent-plan-offer-tests--current))
    (should (equal (mapcar (lambda (note) (gethash "on" note)) notes) '(:false)))))

(ert-deftest romacs-agent-plan-offer-retires-on-plan-off ()
  (romacs-agent-plan-offer-tests--with
    (romacs-agent-plan-offer-tests--offer session 41)
    (should (= (romacs-agent-plan-offer-tests--current) 41))
    (romacs-agent-set-plan session nil)
    (should-not (romacs-agent-plan-offer-tests--current))
    (should-not (funcall 'romacs-agent-summary-user-wait-kind (romacs-agent-plan-offer-tests--summary)))))

(ert-deftest romacs-agent-plan-offer-retires-on-rollback ()
  (romacs-agent-plan-offer-tests--with
    (romacs-agent-plan-offer-tests--offer session 41)
    (should (= (romacs-agent-plan-offer-tests--current) 41))
    (romacs-agent--raise session '(:type rolled-back :seq 42 :to "prompt-39" :turns 1))
    (should-not (romacs-agent-plan-offer-tests--current))))

(ert-deftest romacs-agent-plan-offer-retires-on-new-prompt ()
  (romacs-agent-plan-offer-tests--with
    (romacs-agent-plan-offer-tests--offer session 41)
    (should (= (romacs-agent-plan-offer-tests--current) 41))
    (romacs-agent--raise session '(:type input :seq 42 :request "prompt-42" :text "Keep planning"))
    (should (eq (romacs-agent-state session) 'running))
    (should-not (romacs-agent-plan-offer-tests--current))
    (should-not (funcall 'romacs-agent-summary-user-wait-kind (romacs-agent-plan-offer-tests--summary)))))

(ert-deftest romacs-agent-plan-offer-internal-input-keeps-the-offer ()
  (romacs-agent-plan-offer-tests--with
    (romacs-agent-plan-offer-tests--offer session 41)
    (should (= (romacs-agent-plan-offer-tests--current) 41))
    (romacs-agent--raise session '(:type input :seq 42 :own "compact" :text "Compact"))
    (should (= (romacs-agent-plan-offer-tests--current) 41))))

(defun romacs-agent-plan-offer-tests--close-store ()
  (when (cdr romacs-agent-store--db)
    (sqlite-close (cdr romacs-agent-store--db))
    (setq romacs-agent-store--db nil)))

(ert-deftest romacs-agent-plan-offer-store-round-trip ()
  (romacs-agent-plan-offer-tests--with
    (romacs-agent-plan-offer-tests--offer session 41)
    (romacs-agent-plan-offer-tests--offer session 43)
    (romacs-agent--write-json
     (expand-file-name "manifest.json" (romacs-agent-dir session))
     (romacs-mcp--object "id" romacs-agent-plan-offer-tests--id "harness" "codex"
                         "cwd" "/p/offer/" "title" "Plan fixture"
                         "options" (romacs-mcp--object "plan" t)))
    (romacs-agent-store-put session)
    (romacs-agent-plan-offer-tests--close-store)
    (clrhash romacs-agent--sessions)
    (let ((db (romacs-agent-store--open)))
      (should (equal (sqlite-select db "select plan_offer from sessions where id = ?"
                                    (list romacs-agent-plan-offer-tests--id))
                     '((43))))
      (should (equal (sqlite-select db "pragma user_version") '((7)))))
    (should (= (romacs-agent-plan-offer-tests--current) 43))
    (should-not (gethash romacs-agent-plan-offer-tests--id romacs-agent--sessions))
    (should-error (funcall 'romacs-agent-proceed romacs-agent-plan-offer-tests--id 41) :type 'user-error)
    (should (= (romacs-agent-plan-offer-tests--current) 43))
    (should-not notes)
    (should (romacs-agent-load-session romacs-agent-plan-offer-tests--id))
    (should (= (romacs-agent-plan-offer-tests--current) 43))
    (should-error (funcall 'romacs-agent-proceed romacs-agent-plan-offer-tests--id 41) :type 'user-error)
    (should-not notes)
    (funcall 'romacs-agent-proceed romacs-agent-plan-offer-tests--id 43)
    (should-not (romacs-agent-plan-offer-tests--current))
    (should (= (length notes) 1))))

(defun romacs-agent-plan-offer-tests--v6-store ()
  "Create a store with the v6 sessions columns independently of the reader."
  (let ((db (sqlite-open (romacs-agent-store--file))))
    (unwind-protect
        (progn
          (sqlite-execute db
            "create table sessions (
               id text primary key, dir text not null, harness text, provider text, model text,
               title text, cwd text, parent text, state text, last_time real, cost real, transcript text,
               input_seq integer, done_seq integer, seen_seq integer, settled_seq integer, attention integer,
               effort text, plan integer, permission_mode text, context_used integer, context_window integer,
               resume_time real, seq integer, request text, failed integer)")
          (sqlite-execute db
            "insert into sessions (id, dir, harness, title, state, plan) values (?, '/v6', 'codex', 'Kept', 'idle', 1)"
            (list romacs-agent-plan-offer-tests--id))
          (sqlite-pragma db "user_version = 6"))
      (sqlite-close db))))

(ert-deftest romacs-agent-plan-offer-migrates-v6 ()
  (romacs-agent-plan-offer-tests--with
    (remhash (romacs-agent-id session) romacs-agent--sessions)
    (romacs-agent-plan-offer-tests--v6-store)
    (let ((db (romacs-agent-store--open)))
      (should (equal (sqlite-select db "pragma user_version") '((7))))
      (should (equal (sqlite-select db "select title, plan, plan_offer from sessions")
                     '(("Kept" 1 nil))))
      (should (equal (romacs-agent-summary-title (romacs-agent-plan-offer-tests--summary)) "Kept"))
      (should-not (romacs-agent-plan-offer-tests--current)))))

(ert-deftest romacs-agent-plan-offer-chat-divider-and-header ()
  (romacs-agent-plan-offer-tests--with
    (with-temp-buffer
      (romacs-agent-log-mode)
      (setq romacs-agent-log--session session
            romacs-agent-log--end (copy-marker (point-min) t))
      (let ((romacs-agent-event-functions
             (list (lambda (owner event) (romacs-agent-log--draw-live owner event)))))
        (romacs-agent-plan-offer-tests--offer session 41))
      (should (string-match-p "plan ready: .*proceeds, leaving plan mode · type to keep planning" (buffer-string)))
      (should (string-match-p "Needs approval.*1 waits on you" header-line-format)))))

(provide 'romacs-agent-plan-offer-tests)
;;; romacs-agent-plan-offer-tests.el ends here
