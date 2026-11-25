;;; romacs-agent-pools-tests.el --- A harness's accounts as one pool -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(require 'romacs-test-notes)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-codex)
  (require 'romacs-agent-ui))

(defmacro romacs-agent-pools-tests--with (&rest body)
  "BODY with three Claude accounts and one Codex account, a private state
root and no limit known; `spend' marks an account's limit reached until
a time, `allow' marks it allowed."
  (declare (indent 0))
  `(let* ((root (make-temp-file "romacs-pools-" t))
          (process-environment (cons (concat "XDG_STATE_HOME=" root "/state") process-environment))
          (romacs-agent-providers (mapcar (lambda (name) (let ((dir (expand-file-name name root)))
                                                           (make-directory dir t)
                                                           (cons name dir)))
                                          '("claude" "claude2" "claude3" "codex")))
          (romacs-agent-pools nil)
          (romacs-agent--room (make-hash-table :test #'equal))
          (romacs-agent--room-loaded t)
          ;; No account asked again unless a test says.
          (romacs-agent-room-check-interval nil)
          (romacs-agent--room-timer nil)
          (romacs-agent--room-asking (make-hash-table :test #'equal))
          (romacs-agent--sessions (make-hash-table :test #'equal)))
     (cl-letf (((symbol-function 'romacs-agent--check-companions) #'ignore))
       (cl-flet ((spend (name resets &optional time)
                   (romacs-agent--note-room (romacs-agent--make :provider name)
                                            (list :status 'rejected :resets resets :window "five_hour" :time (or time 1.0))))
                 (allow (name &optional time)
                   (romacs-agent--note-room (romacs-agent--make :provider name)
                                            (list :status 'allowed :used 0.2 :window "five_hour" :time (or time 2.0))))
                 (names (instances) (mapcar (lambda (i) (plist-get i :name)) instances)))
         (unwind-protect (progn ,@body) (delete-directory root t))))))

(ert-deftest romacs-agent-pools-order-and-pick ()
  (romacs-agent-pools-tests--with
    (should (equal (names (romacs-agent-pool 'claude)) '("claude" "claude2" "claude3")))
    (should (equal (names (romacs-agent-pool 'codex)) '("codex")))
    ;; Its own order, when given.
    (let ((romacs-agent-pools '((claude "claude3" "claude"))))
      (should (equal (names (romacs-agent-pool 'claude)) '("claude3" "claude"))))
    (should (equal (plist-get (romacs-agent-pool-pick 'claude) :name) "claude"))
    ;; Spent until later: the next with room.
    (spend "claude" (+ (float-time) 3600))
    (should (equal (plist-get (romacs-agent-pool-pick 'claude) :name) "claude2"))
    (spend "claude2" (+ (float-time) 600))
    (spend "claude3" (+ (float-time) 7200))
    ;; None with room: the one back first.
    (should (equal (plist-get (romacs-agent-pool-pick 'claude) :name) "claude2"))
    ;; Its limit reset: new work draws on the first again.
    (spend "claude" (- (float-time) 1) 3.0)
    (should (equal (plist-get (romacs-agent-pool-pick 'claude) :name) "claude"))))

(ert-deftest romacs-agent-pools-a-harness-names-its-pool-an-instance-pins ()
  (romacs-agent-pools-tests--with
    (spend "claude" (+ (float-time) 3600))
    ;; The harness, a symbol: its pool.
    (should (equal (plist-get (romacs-agent--start-instance 'claude) :name) "claude2"))
    (should (equal (plist-get (romacs-agent--start-instance 'codex) :name) "codex"))
    ;; An instance's name pins it, even one named as its harness.
    (should (equal (plist-get (romacs-agent--start-instance "claude") :name) "claude"))
    (should (equal (plist-get (romacs-agent--start-instance "claude3") :name) "claude3"))
    ;; Nothing named: the first enabled instance's harness's pool.
    (should (equal (plist-get (romacs-agent--start-instance nil) :name) "claude2"))
    ;; An agent's tool call names a harness by its name.
    (should (eq (romacs-agent-pool-name "claude") 'claude))
    (should (equal (romacs-agent-pool-name "claude2") "claude2"))))

(ert-deftest romacs-agent-pools-pickers-name-harnesses ()
  (romacs-agent-pools-tests--with
    (cl-letf (((symbol-function 'romacs-agent--models) (lambda (_) '(("opus" :label "Opus")))))
      (let ((rows (romacs-agent--picker-rows)))
        (should (equal (delete-dups (mapcar (lambda (row) (plist-get (cdr row) :provider)) rows)) '(claude codex)))
        (should-not (cl-find-if (lambda (row) (string-match-p "claude2\\|Claude2" (car row))) rows)))
      ;; Accounts only when asked for.
      (should (member "claude2" (mapcar (lambda (row) (plist-get (cdr row) :provider)) (romacs-agent--picker-rows t))))
      (let ((romacs-agent-pick-accounts t))
        (should (member "claude3" (mapcar (lambda (row) (plist-get (cdr row) :provider)) (romacs-agent--picker-rows))))))))

(ert-deftest romacs-agent-pools-limits-are-kept-across-a-restart ()
  (romacs-agent-pools-tests--with
    (let ((writes 0))
      (cl-letf* ((keep (symbol-function 'romacs-agent--keep-room))
                 ((symbol-function 'romacs-agent--keep-room) (lambda () (cl-incf writes) (funcall keep))))
        (spend "claude" 5000.0 1.0)
        (allow "claude2" 2.0)
        ;; Each request's use alone is not written, only a 5% step of it.
        (romacs-agent--note-room (romacs-agent--make :provider "claude2")
                                 (list :status 'allowed :used 0.21 :window "five_hour" :time 3.0))
        (should (= writes 2))
        (romacs-agent--note-room (romacs-agent--make :provider "claude2")
                                 (list :status 'allowed :used 0.4 :window "five_hour" :time 4.0))
        (should (= writes 3))))
    ;; A new server reads what was kept.
    (let ((romacs-agent--room (make-hash-table :test #'equal)) (romacs-agent--room-loaded nil))
      (should (eq (plist-get (romacs-agent-room-of "claude") :status) 'rejected))
      (should (= (plist-get (romacs-agent-room-of "claude") :resets) 5000.0)))))

(ert-deftest romacs-agent-pools-move-on-to-the-next-account ()
  (romacs-agent-pools-tests--with
    (let ((session (romacs-agent--make :id "s" :harness 'claude :provider "claude" :state 'idle :caught-up t
                                       :dir (make-temp-file "s" t) :limited '(:provider "claude")))
          (selected nil) (queued nil) (moved nil))
      (puthash "s" session romacs-agent--sessions)
      (spend "claude" (+ (float-time) 3600))
      (cl-letf (((symbol-function 'romacs-agent-select)
                 (lambda (_session _model _effort provider) (setq selected provider) t))
                ((symbol-function 'romacs-agent--enqueue-first)
                 (lambda (_s item &optional done) (push item queued) (funcall done '(ok t))))
                ((symbol-function 'run-at-time) #'ignore))
        (let ((romacs-agent-moved-functions (list (lambda (_s from to limited &optional _kept) (push (list from to limited) moved)))))
          (should (equal (romacs-agent-move-on session) "claude2"))
          (should (equal selected "claude2"))
          ;; Told to go on only once the move took effect: first of what
          ;; waits, its own turn, all data (kept across a restart).
          (should-not queued)
          (romacs-agent--moved session "claude" "claude2" '(:provider "claude"))
          (should (string-match-p "claude's usage limit; you now run on claude2" (romacs-agent-item-text (car queued))))
          (should (equal (romacs-agent-item-own (car queued)) romacs-agent--limit-continue))
          (should (romacs-agent--keepable-p (car queued)))
          (should (equal moved '(("claude" "claude2" (:provider "claude")))))
          ;; A move that did not take effect: nothing to go on with.
          (setq queued nil)
          (romacs-agent--moved session "claude" "claude" '(:provider "claude"))
          (should-not queued)
          (should (equal (car moved) '("claude" "claude" (:provider "claude"))))
          ;; A move with no limit behind it: nothing to go on from.
          (romacs-agent--moved session "claude" "claude2" nil)
          (should-not queued)
          (should (= (length moved) 2)))
        (spend "claude2" (+ (float-time) 3600))
        (spend "claude3" (+ (float-time) 3600))
        (should-error (romacs-agent-move-on session) :type 'user-error)))))

(ert-deftest romacs-agent-pools-exposed-in-capabilities ()
  (romacs-agent-pools-tests--with
    (spend "claude" (+ (float-time) 3600))
    (allow "claude2")
    (let* ((pools (gethash "pools" (cl-letf (((symbol-function 'romacs-agent--want-models) #'ignore))
                                     (romacs-agent-tools--capabilities (romacs-mcp--object)))))
           (claude (cl-find "claude" pools :key (lambda (p) (gethash "harness" p)) :test #'equal)))
      (should (equal (gethash "next" claude) "claude2"))
      (should (equal (mapcar (lambda (a) (list (gethash "providerInstanceId" a) (gethash "room" a)))
                             (gethash "accounts" claude))
                     '(("claude" :false) ("claude2" t) ("claude3" t)))))))

(ert-deftest romacs-agent-pools-resume-at-the-reset ()
  ;; Every account spent, the first back first: it resumes by itself then,
  ;; journaled, kept across a restart, on the first with room.  The reset
  ;; is an hour ahead: a resume is never scheduled before now, and a loaded
  ;; run may take seconds to get there.
  (romacs-agent-pools-tests--with
    (let* ((dir (make-temp-file "s" t))
           (session (romacs-agent--make :id "r" :harness 'claude :provider "claude" :state 'idle :caught-up t
                                        :dir dir :done-seq 4 :input-seq 3 :limited '(:provider "claude")))
           (soon (+ (float-time) 3600)) (armed nil) (told nil) (moved nil)
           (romacs-agent-tools--resumes (make-hash-table :test #'eq :weakness 'key))
           (romacs-agent-switch-delay 0) (romacs-agent-resume-margin 1))
      (puthash "r" session romacs-agent--sessions)
      (romacs-agent--manifest session nil)
      (spend "claude" soon) (spend "claude2" (+ soon 600)) (spend "claude3" (+ soon 900))
      (cl-letf (((symbol-function 'run-at-time) (lambda (delay _ function &rest args) (push (list delay function args) armed) 'timer))
                ((symbol-function 'cancel-timer) #'ignore)
                ((symbol-function 'romacs-agent-tools--wake) #'ignore)
                ;; Its harness, ended meanwhile, is resumed to take it.
                ((symbol-function 'romacs-agent--continue) (lambda (s) (push 'continued told) s))
                ((symbol-function 'romacs-agent--admit)
                 (lambda (_s item first) (push (list (romacs-agent-item-text item) (romacs-agent-item-own item) first) told)))
                ((symbol-function 'romacs-agent-move-on) (lambda (_s why) (push why moved))))
        (romacs-agent-tools--switch-on-limit session '(:limit t))
        ;; Scheduled at the earliest reset plus its margin, said and kept.
        (should (= (romacs-agent-resume-time session) (+ soon 1)))
        (should (cl-find #'romacs-agent-tools--resume armed :key #'cadr))
        (let ((journal (mapconcat #'romacs-agent--json (romacs-agent--journal-records session) "\n")))
          (should (string-match-p "\"type\":\"resume\",\"state\":\"scheduled\"" journal)))
        ;; Kept in its manifest: a restarted server, loading it, arms it again.
        (let ((manifest (romacs-agent--read-json (expand-file-name "manifest.json" dir))))
          (should (= (gethash "resumeAt" manifest) (+ soon 1)))
          (clrhash romacs-agent-tools--resumes)
          (run-hook-with-args 'romacs-agent-loaded-functions session manifest))
        (should (= (romacs-agent-resume-time session) (+ soon 1)))
        ;; Due, its own account back: told to go on there, first of what waits.
        (spend "claude" (- (float-time) 1) 9.0)
        (romacs-agent-tools--resume session (+ soon 1))
        (should (equal told (list (list "Your usage limit has reset; go on from where you were."
                                        romacs-agent--limit-continue t)
                                  'continued)))
        (should-not (romacs-agent-resume-time session))
        (should (string-match-p "\"state\":\"fired\",\"on\":\"claude\""
                                (mapconcat #'romacs-agent--json (romacs-agent--journal-records session) "\n")))
        ;; Another account back first: it moves there.
        (spend "claude" (+ (float-time) 600) 10.0)
        (allow "claude2" 11.0)
        (romacs-agent-tools--arm-resume session 42.0)
        (romacs-agent-tools--resume session 42.0)
        (should (equal moved '("limit reset")))
        ;; A message cancels a waiting resume, journaled.
        (romacs-agent-tools--arm-resume session 43.0)
        (romacs-agent-tools--limit-on-event session '(:type input))
        (should-not (romacs-agent-resume-time session))
        ;; Its manifest says so at once: nothing written later brings it back.
        (should-not (gethash "resumeAt" (romacs-agent--read-json (expand-file-name "manifest.json" dir))))
        ;; A reset already used schedules nothing again.
        (spend "claude" soon 13.0) (spend "claude2" (+ soon 600) 13.0)
        (puthash session soon romacs-agent-tools--resets-used)
        (romacs-agent-tools--switch-on-limit session '(:limit t))
        (should-not (romacs-agent-resume-time session))
        ;; Turned off: nothing scheduled, none fires, none kept is armed again.
        (let ((romacs-agent-resume-at-reset nil))
          (spend "claude" (+ soon 1000) 14.0)
          (romacs-agent-tools--switch-on-limit session '(:limit t))
          (should-not (romacs-agent-resume-time session))
          (setq told nil moved nil)
          (romacs-agent-tools--arm-resume session 44.0)
          (romacs-agent-tools--resume session 44.0)
          (should-not (or told moved))
          (romacs-agent-tools--keep-resume session 45.0)
          (run-hook-with-args 'romacs-agent-loaded-functions session
                              (romacs-agent--read-json (expand-file-name "manifest.json" dir)))
          (should-not (romacs-agent-resume-time session))
          (should-not (gethash "resumeAt" (romacs-agent--read-json (expand-file-name "manifest.json" dir)))))))))

(ert-deftest romacs-agent-pools-settled-threads-do-not-continue-at-a-limit ()
  (romacs-agent-pools-tests--with
    (let* ((dir (expand-file-name "settled" root))
           (session (romacs-agent--make :id "settled" :harness 'claude :provider "claude"
                                        :state 'idle :caught-up t :dir dir :done-seq 4
                                        :limited '(:provider "claude")))
           (romacs-agent-tools--resumes (make-hash-table :test #'eq))
           (romacs-agent-event-functions (list #'romacs-agent-tools--limit-on-event))
           (time (+ (float-time) 3600)) callbacks)
      (make-directory dir)
      (puthash "settled" session romacs-agent--sessions)
      (romacs-agent--manifest session nil)
      (cl-letf (((symbol-function 'run-at-time) (lambda (_ _ f &rest args) (push (cons f args) callbacks) 'timer))
                ((symbol-function 'cancel-timer) #'ignore)
                ((symbol-function 'romacs-agent-tools--wake) #'ignore)
                ((symbol-function 'romacs-agent--stop-then) (lambda (_ then) (push (list then) callbacks)))
                ((symbol-function 'romacs-agent--admit) (lambda (&rest _) (ert-fail "Resumed")))
                ((symbol-function 'romacs-agent-resume) (lambda (&rest _) (ert-fail "Relaunched")))
                ((symbol-function 'romacs-agent-move-on) (lambda (&rest _) (ert-fail "Moved"))))
        (romacs-agent-tools--switch-on-limit session '(:limit t))
        (should (romacs-agent-tools--moving-p session))
        ;; A continuation waiting goes with it: a settled thread goes on from nothing.
        (romacs-agent--enqueue session (romacs-agent--limit-continuation "claude" "claude2" 4))
        (romacs-agent-tools--arm-resume session time)
        (romacs-agent-tools--keep-resume session time)
        (setf (romacs-agent-pending-selection session) (list :state 'armed :provider "other"))
        (romacs-agent--restart session)
        (romacs-agent-settle session)
        (should-not (romacs-agent-pending-selection session))
        (should-not (romacs-agent-resume-time session))
        (should-not (romacs-agent-tools--moving-p session))
        (should-not (romacs-agent-queue session))
        (while-let ((callback (pop callbacks))) (apply (car callback) (cdr callback)))
        (romacs-agent--raise session '(:type done :seq 5 :limit t))
        (romacs-agent-tools--rearm-resume session (romacs-mcp--object "resumeAt" time))
        (should (romacs-agent-settled-p session))
        (should-not callbacks)
        (should-not (gethash "resumeAt" (romacs-agent--read-json (expand-file-name "manifest.json" dir))))))))

(ert-deftest romacs-agent-pools-a-message-at-the-limit-keeps-the-resume ()
  ;; A delegated task stopped at its limit, every account spent: a message
  ;; from its parent cancels the resume, its turn ends at the limit again,
  ;; and the resume is scheduled again for the same reset.
  (romacs-agent-pools-tests--with
    (let* ((dir (make-temp-file "s" t))
           (session (romacs-agent--make :id "c" :harness 'claude :provider "claude3" :state 'idle :caught-up t
                                        :dir dir :done-seq 4 :input-seq 3 :limited '(:provider "claude3")
                                        :options '(:parent "p" :role "implementation")))
           (soon (+ (float-time) 600))
           (romacs-agent-tools--resumes (make-hash-table :test #'eq :weakness 'key))
           (romacs-agent-tools--resets-used (make-hash-table :test #'eq :weakness 'key))
           (romacs-agent-switch-delay 0) (romacs-agent-resume-margin 1))
      (ignore #'allow #'names)
      (puthash "c" session romacs-agent--sessions)
      (romacs-agent--manifest session nil)
      (spend "claude" soon) (spend "claude2" (+ soon 600)) (spend "claude3" (+ soon 900))
      (cl-letf (((symbol-function 'run-at-time) (lambda (&rest _) 'timer))
                ((symbol-function 'cancel-timer) #'ignore)
                ((symbol-function 'romacs-agent-tools--wake) #'ignore))
        (romacs-agent-tools--switch-on-limit session '(:limit t))
        (should (= (romacs-agent-resume-time session) (+ soon 1)))
        ;; Its parent's message: the resume is cancelled.
        (romacs-agent-tools--limit-on-event session '(:type input))
        (should-not (romacs-agent-resume-time session))
        ;; Its turn ends at the limit again: scheduled again.
        (romacs-agent-tools--switch-on-limit session '(:limit t))
        (should (= (romacs-agent-resume-time session) (+ soon 1)))
        (should (= (gethash "resumeAt" (romacs-agent--read-json (expand-file-name "manifest.json" dir))) (+ soon 1)))))))

(ert-deftest romacs-agent-pools-a-wait-is-its-limits-alone ()
  (romacs-agent-pools-tests--with
    (let* ((dir (make-temp-file "w" t))
           (session (romacs-agent--make :id "w" :harness 'claude :provider "claude" :state 'idle :caught-up t
                                        :dir dir :done-seq 4 :input-seq 3 :limited '(:provider "claude")))
           (armed nil) (moves 0) (selects 0)
           (romacs-agent-tools--resumes (make-hash-table :test #'eq :weakness 'key)))
      (puthash "w" session romacs-agent--sessions)
      (spend "claude" (+ (float-time) 600))
      (cl-letf (((symbol-function 'run-at-time) (lambda (_d _r function &rest args) (push (cons function args) armed) 'timer))
                ((symbol-function 'romacs-agent-tools--wake) #'ignore)
                ((symbol-function 'romacs-agent-select) (lambda (&rest _) (cl-incf selects) (= selects 1))))
        ;; A selection asked meanwhile (romacs_task_switch): the wait leaves
        ;; the move to it, and looks again later.
        (romacs-agent-tools--switch-on-limit session '(:limit t))
        (setf (romacs-agent-pending-selection session) '(:state armed :provider "claude2"))
        (let ((wait (cl-find #'romacs-agent-tools--hand-over armed :key #'car)))
          (setq armed nil)
          (cl-letf (((symbol-function 'romacs-agent-move-on) (lambda (&rest _) (cl-incf moves))))
            (apply (car wait) (cdr wait))))
        (should (= moves 0))
        (should (romacs-agent-tools--moving-p session))
        (should (equal (cdr (cl-find #'romacs-agent-tools--hand-over armed :key #'car)) (list session 4)))
        ;; The same move asked again while it is pending selects it once.
        (setf (romacs-agent-pending-selection session) nil)
        (romacs-agent-move-on session)
        (setf (romacs-agent-pending-selection session) '(:state armed :provider "claude2"))
        (cl-letf (((symbol-function 'romacs-agent--wanted) (lambda (_) '(:provider "claude2"))))
          (romacs-agent-move-on session))
        (should (= selects 1))
        (setf (romacs-agent-pending-selection session) nil)
        ;; The same limit reported again: one move, no second wait.
        (setq armed nil)
        (romacs-agent-tools--switch-on-limit session '(:limit t))
        (should-not armed)
        ;; The rest spent during the wait: said in the journal, with the reset.
        (spend "claude2" (+ (float-time) 600) 5.0) (spend "claude3" (+ (float-time) 900) 5.0)
        (romacs-agent-tools--hand-over session 4)
        (should-not (romacs-agent-tools--moving-p session))
        (should (string-match-p "\"to\":null,\"why\":\"every claude account is at its limit\",\"resets\":[0-9]"
                                (mapconcat #'romacs-agent--json (romacs-agent--journal-records session) "\n")))))))

;; A move at the limit holds the queue in the gate, its end from the
;; parent, and is armed again once a restart's replay is over.

(ert-deftest romacs-agent-pools-a-move-at-the-limit-holds-the-gate ()
  (let ((session (romacs-agent--make :id "g" :harness 'claude :provider "claude" :state 'idle :caught-up t
                                     :limited '(:provider "claude"))))
    (cl-letf (((symbol-function 'process-live-p) #'always)
              ((symbol-function 'romacs-agent--per-turn-p) #'ignore)
              ((symbol-function 'romacs-agent--folding-p) #'ignore)
              ((symbol-function 'romacs-agent--ready-p) #'always)
              ((symbol-function 'romacs-agent-stopped) #'ignore)
              ((symbol-function 'romacs-agent-tools--hand-on) #'ignore)
              ((symbol-function 'romacs-agent--dispatch-soon) #'ignore))
      (should (romacs-agent--open-p session nil))
      (setf (romacs-agent-limited session) '(:provider "claude" :moving 4))
      ;; Nothing goes, a hold's own prompt neither.
      (should-not (romacs-agent--open-p session nil))
      (should-not (romacs-agent--open-p session 'token))
      ;; Its kept state never says it moves: that wait is this process's.
      (let ((kept (car (read-from-string (romacs-agent--kept-text session)))))
        (should (equal (alist-get 'limited (plist-get kept :fields)) '(:provider "claude"))))
      ;; Not moved: its limit stays, its messages go.
      (romacs-agent-tools--not-moved session nil)
      (should (equal (romacs-agent-limited session) '(:provider "claude")))
      (should (romacs-agent--open-p session nil)))))

(ert-deftest romacs-agent-pools-a-limit-holds-its-end-while-it-may-move ()
  (should-not (romacs-agent-tools--wake-held-p nil t))
  (should-not (romacs-agent-tools--wake-held-p '(:provider "claude") nil))
  (should (romacs-agent-tools--wake-held-p '(:provider "claude" :moving 4) nil))
  ;; Replaying still: whether it moves is not known yet.
  (should (romacs-agent-tools--wake-held-p '(:provider "claude") t)))

(ert-deftest romacs-agent-pools-a-held-wake-is-bounded ()
  ;; Opus 1: a selection of a model alone at a limit held its failed end
  ;; for hours.  Sol S1: a selection to another account that never came
  ;; to it (its reattach retries spent) held the end, the hand-over
  ;; polling for ever.  Neither holds it now: only a move's bounded wait.
  (let* ((romacs-agent--sessions (make-hash-table :test #'equal))
         (session (romacs-agent--make :id "h" :harness 'claude :provider "claude" :state 'idle :caught-up t
                                      :dir (make-temp-file "h" t)
                                      :done-seq 4 :limited '(:provider "claude") :options '(:parent "p")
                                      :ends (list (list 4 "You've hit your limit" t "m-1"))
                                      :pending-selection '(:state armed :provider "claude" :model "sonnet")))
         (woken 0))
    (puthash "h" session romacs-agent--sessions)
    (cl-letf (((symbol-function 'romacs-agent-tools--hand-on) (lambda (_) (cl-incf woken)))
              ((symbol-function 'run-at-time) #'ignore)
              ((symbol-function 'romacs-agent--dispatch-soon) #'ignore))
      (romacs-agent-tools--wake session)
      (should (= woken 1))
      ;; Moving, a selection to another account on its way: held, waited out.
      (setf (romacs-agent-pending-selection session) '(:state armed :provider "claude2"))
      (romacs-agent-tools--arm-move session)
      (romacs-agent-tools--wake session)
      (romacs-agent-tools--hand-over session 4)
      (should (romacs-agent-tools--moving-p session))
      (should (= woken 1))
      ;; Never coming to it: the wait ends, the end goes to the parent, and
      ;; the selection is taken back (Sol S1: a late stop committed the move
      ;; after the parent was told it failed).
      (setf (romacs-agent-limited session)
            (plist-put (romacs-agent-limited session) :moving-since (- (float-time) romacs-agent-start-deadline 1)))
      (romacs-agent-tools--hand-over session 4)
      (should-not (romacs-agent-tools--moving-p session))
      (should (= woken 2))
      (should-not (romacs-agent-pending-selection session))
      ;; Its stop under way: it ends on the account it has.
      (setf (romacs-agent-pending-selection session) '(:state restarting :provider "claude2"))
      (romacs-agent-tools--arm-move session)
      (setf (romacs-agent-limited session)
            (plist-put (romacs-agent-limited session) :moving-since (- (float-time) romacs-agent-start-deadline 1)))
      (romacs-agent-tools--hand-over session 4)
      (should (equal (plist-get (romacs-agent-pending-selection session) :provider) "claude"))
      ;; A session a load replaced: its wait moves nothing and wakes no one.
      (let ((stale (romacs-agent--make :id "h" :harness 'claude :provider "claude" :done-seq 4
                                       :limited (list :provider "claude" :moving 4 :moving-since 0.0)
                                       :options '(:parent "p") :ends (list (list 4 "limit" t "m-1")))))
        (romacs-agent-tools--hand-over stale 4)
        (should (eql (romacs-agent-tools--moving-p stale) 4))
        (should (= woken 3))))))

(ert-deftest romacs-agent-pools-a-move-a-restart-cut-holds-from-its-load ()
  ;; Its harness stopped for a move, the editor gone before the selection
  ;; committed: loaded, it waits for that move as a live one does, so a
  ;; restart's wake of missed ends sends no failed one meanwhile.
  (let ((session (romacs-agent--make :id "l" :harness 'claude :provider "claude" :state 'ended :done-seq 4
                                     :limited '(:provider "claude")
                                     :pending-selection '(:state armed :provider "claude2")))
        (romacs-agent-switch-on-limit 'lossless) (armed nil))
    (cl-letf (((symbol-function 'run-at-time) (lambda (_d _r function &rest args) (push (cons function args) armed))))
      (run-hook-with-args 'romacs-agent-loaded-functions session (romacs-mcp--object))
      (should (eql (romacs-agent-tools--moving-p session) 4))
      (should (equal (car armed) (list #'romacs-agent-tools--hand-over session 4)))
      ;; A model alone pending: no move to wait for.
      (setf (romacs-agent-limited session) '(:provider "claude") armed nil
            (romacs-agent-pending-selection session) '(:state armed :provider "claude" :model "sonnet"))
      (romacs-agent-tools--limit-loaded session nil)
      (should-not (romacs-agent-tools--moving-p session))
      (should-not armed))))

(ert-deftest romacs-agent-pools-a-move-cancelled-as-its-harness-stops-goes-nowhere ()
  ;; Sol S3: cancelled (or settled) while its harness stops for the move,
  ;; the stop's callback committed, made a continuation and resumed it.
  (let ((session (romacs-agent--make :id "x" :harness 'claude :provider "claude" :state 'detached :cancelled t
                                     :limited '(:provider "claude")
                                     :pending-selection '(:state restarting :provider "claude2" :stopped t))))
    (cl-letf (((symbol-function 'romacs-agent--commit-switch) (lambda (_) (ert-fail "Committed")))
              ((symbol-function 'romacs-agent-resume) (lambda (_) (ert-fail "Resumed"))))
      (romacs-agent--restarted session)
      (setf (romacs-agent-cancelled session) nil)
      (cl-letf (((symbol-function 'romacs-agent-settled-p) #'always))
        (romacs-agent--restarted session)))
    (should (equal (romacs-agent-provider session) "claude"))
    (should-not (romacs-agent-queue session))))

(ert-deftest romacs-agent-pools-room-malformed-or-stale-is-none ()
  ;; Sol S4: a kept room file that is not a list aborted catching up; an
  ;; old report of room, from before the limit, released the queue to the
  ;; spent account.
  (romacs-agent-pools-tests--with
    (let ((romacs-agent--room (make-hash-table :test #'equal)) (romacs-agent--room-loaded nil))
      (make-directory (file-name-directory (romacs-agent--room-file)) t)
      (cl-letf (((symbol-function 'romacs-agent--room-watch) #'ignore))
        (dolist (text '("42" "\"x\"" "((\"claude2\" :status allowed :time \"bad\") (\"claude3\" :status rejected :time 1.0 :resets \"x\"))"
                        "(nil 3 (\"claude\" :status allowed :time 5.0) (7 . 8))"))
          (write-region text nil (romacs-agent--room-file) nil 'silent)
          (setq romacs-agent--room-loaded nil)
          (clrhash romacs-agent--room)
          (should (hash-table-p (romacs-agent--room-table)))))
      (should (equal (hash-table-keys romacs-agent--room) '("claude")))
      ;; Kept again over a bad file: merged, no error.
      (write-region "((\"claude2\" :status allowed :time \"bad\"))" nil (romacs-agent--room-file) nil 'silent)
      (romacs-agent--merge-room)
      (should (equal (sort (mapcar #'car (romacs-agent--room-kept)) #'string<) '("claude"))))
    ;; Room back only since the limit: a report after it, or its reset passed.
    (puthash "claude" (list :status 'allowed :time 50.0) romacs-agent--room)
    (should-not (romacs-agent-tools--room-back-p "claude" 100.0))
    (should (romacs-agent-tools--room-back-p "claude" 40.0))
    (puthash "claude" (list :status 'rejected :resets (- (float-time) 1) :time 50.0) romacs-agent--room)
    (should (romacs-agent-tools--room-back-p "claude" 100.0))
    ;; A limit of unknown time (an older snapshot): only its reset counts.
    (puthash "claude" (list :status 'allowed :time 50.0) romacs-agent--room)
    (should-not (romacs-agent-tools--room-back-p "claude" nil))
    (remhash "claude" romacs-agent--room)
    (should-not (romacs-agent-tools--room-back-p "claude" 100.0))
    ;; As its replay ends, an allowed report older than its limit: it moves.
    (let ((session (romacs-agent--make :id "o" :harness 'claude :provider "claude" :state 'idle :caught-up t
                                       :done-seq 4 :input-seq 4 :limited '(:provider "claude" :time 100.0)))
          (romacs-agent-switch-on-limit 'lossless))
      (puthash "o" session romacs-agent--sessions)
      (puthash "claude" (list :status 'allowed :time 50.0) romacs-agent--room)
      (cl-letf (((symbol-function 'run-at-time) #'ignore))
        (run-hook-with-args 'romacs-agent-caught-up-functions session))
      (should (eql (romacs-agent-tools--moving-p session) 4)))))

(ert-deftest romacs-agent-pools-a-restart-arms-the-move-again ()
  (let ((limited '(:provider "claude")))
    (should (romacs-agent-tools--moves-again-p limited t nil nil))
    (should-not (romacs-agent-tools--moves-again-p nil t nil nil))
    ;; Busy, asked since, or room known back.
    (should-not (romacs-agent-tools--moves-again-p limited nil nil nil))
    (should-not (romacs-agent-tools--moves-again-p limited t t nil))
    (should-not (romacs-agent-tools--moves-again-p limited t nil t)))
  ;; As its replay ends: armed again, its end held, a selection pending or
  ;; its room unknown too; with room back, its end goes.
  (romacs-agent-pools-tests--with
    (let* ((session (romacs-agent--make :id "a" :harness 'claude :provider "claude" :state 'idle :caught-up t
                                        :done-seq 4 :input-seq 4 :limited '(:provider "claude")
                                        :ends (list (list 4 "You've hit your limit" t "m-1"))
                                        :options '(:parent "p")))
           (romacs-agent-switch-on-limit 'lossless) (armed nil) (woken 0))
      (puthash "a" session romacs-agent--sessions)
      (puthash "claude" (list :status 'rejected :resets (+ (float-time) 600) :time 1.0) romacs-agent--room)
      (cl-letf (((symbol-function 'run-at-time) (lambda (_d _r function &rest args) (push (cons function args) armed)))
                ((symbol-function 'romacs-agent-tools--hand-on) (lambda (_) (cl-incf woken))))
        (run-hook-with-args 'romacs-agent-caught-up-functions session)
        (should (eql (romacs-agent-tools--moving-p session) 4))
        (should (equal (cl-find #'romacs-agent-tools--hand-over armed :key #'car)
                       (list #'romacs-agent-tools--hand-over session 4)))
        (should (= woken 0))
        (dolist (state (list (lambda () (setf (romacs-agent-pending-selection session) '(:state armed :provider "claude2")))
                             (lambda () (remhash "claude" romacs-agent--room))))
          (setf (romacs-agent-limited session) '(:provider "claude") armed nil)
          (funcall state)
          (run-hook-with-args 'romacs-agent-caught-up-functions session)
          (should (eql (romacs-agent-tools--moving-p session) 4))
          (should (= woken 0)))
        (setf (romacs-agent-pending-selection session) nil)
        (setf (romacs-agent-limited session) '(:provider "claude" :time 2.0) armed nil)
        (puthash "claude" (list :status 'allowed :time 3.0) romacs-agent--room)
        (run-hook-with-args 'romacs-agent-caught-up-functions session)
        (should-not (romacs-agent-tools--moving-p session))
        (should-not armed)
        (should (= woken 1))
        ;; Its replay moved it (no limit left): its ends go all the same.
        (setf (romacs-agent-limited session) nil)
        (run-hook-with-args 'romacs-agent-caught-up-functions session)
        (should (= woken 2))
        ;; Every account spent: said, and resumed at the first reset, as live.
        (setf (romacs-agent-limited session) '(:provider "claude"))
        (puthash "claude" (list :status 'rejected :resets (+ (float-time) 600) :time 4.0) romacs-agent--room)
        (dolist (name '("claude2" "claude3"))
          (puthash name (list :status 'rejected :resets (+ (float-time) 900) :time 4.0) romacs-agent--room))
        (cl-letf (((symbol-function 'romacs-agent-tools--spent) (lambda (_) (push 'spent armed)))
                  ((symbol-function 'romacs-agent-tools--schedule-resume) (lambda (_) (push 'resume armed))))
          (run-hook-with-args 'romacs-agent-caught-up-functions session))
        (should (equal armed '(resume spent)))
        (should-not (romacs-agent-tools--moving-p session))))))

(ert-deftest romacs-agent-pools-a-limit-again-is-the-same-move ()
  ;; A limit reported again while it moves (a duplicate completion) keeps
  ;; the move; moved, every end from the first on is answered by going on.
  (let* ((dir (make-temp-file "d" t))
         (session (romacs-agent--make :id "d" :harness 'claude :provider "claude" :state 'idle :caught-up t
                                      :dir dir :done-seq 3 :limited '(:provider "claude" :moving 3)
                                      :ends (list (list 3 "limit" t "m-1") (list 1 "earlier" nil nil))))
         (romacs-agent-tools--answered (make-hash-table :test #'equal)))
    (let ((completed (romacs-agent--completed-turn (list :turns 1 :provider "claude" :limited (romacs-agent-limited session))
                                                   '(:seq 4 :limit t :text "limit" :error t) nil)))
      (should (eql (plist-get (plist-get completed :limited) :moving) 3))
      (should-not (plist-get (plist-get (romacs-agent--completed-turn (list :turns 1 :provider "claude")
                                                                      '(:seq 4 :limit t) nil)
                                        :limited)
                             :moving)))
    (push (list 4 "limit" t "m-1") (romacs-agent-ends session))
    (romacs-agent-tools--moved session "claude" "claude2" (romacs-agent-limited session))
    (should (equal (romacs-agent-ends session) '((1 "earlier" nil nil))))
    ;; A move a restart applied as it loaded (no wait armed): its last end.
    (setf (romacs-agent-done-seq session) 5)
    (push (list 5 "limit" t "m-2") (romacs-agent-ends session))
    (romacs-agent-tools--moved session "claude" "claude2" '(:provider "claude"))
    (should (equal (romacs-agent-ends session) '((1 "earlier" nil nil))))))

(ert-deftest romacs-agent-pools-a-move-applied-as-it-loads-goes-on ()
  ;; Its harness stopped for the move, the editor gone before it came back:
  ;; the move applied as it loads resumes it to take its continuation.
  (let* ((romacs-agent--sessions (make-hash-table :test #'equal))
         (session (romacs-agent--make :id "g" :harness 'claude :state 'ended))
         (opened 0))
    (puthash "g" session romacs-agent--sessions)
    (cl-letf (((symbol-function 'romacs-agent--opened) (lambda (_) (cl-incf opened))))
      (romacs-agent--go-on session)
      (should (= opened 1))
      ;; Settled, cancelled, or no longer the session loaded: nothing.
      (romacs-agent--go-on (romacs-agent--make :id "g" :harness 'claude :state 'ended))
      (setf (romacs-agent-cancelled session) t)
      (romacs-agent--go-on session)
      (should (= opened 1)))))

(ert-deftest romacs-agent-pools-an-attachment-gone-wakes-what-it-held ()
  ;; Ends held while it replayed go once its attachment's status changes;
  ;; not at a limit, nothing of its was held.
  (let* ((session (romacs-agent--make :id "r" :harness 'claude :options '(:parent "p") :limited '(:provider "claude")
                                      :ends (list (list 4 "limit" t nil))))
         (romacs-agent-tools--scheduled (make-hash-table :test #'equal))
         (scheduled nil))
    (cl-letf (((symbol-function 'run-at-time) (lambda (&rest args) (push args scheduled))))
      (romacs-agent-tools--on-event session '(:type status :text "detached"))
      (should (= (length scheduled) 1))
      (clrhash romacs-agent-tools--scheduled)
      (setf (romacs-agent-limited session) nil)
      (romacs-agent-tools--on-event session '(:type status :text "detached"))
      (should (= (length scheduled) 1))
      (setf (romacs-agent-limited session) '(:provider "claude") (romacs-agent-ends session) nil)
      (romacs-agent-tools--on-event session '(:type status :text "detached"))
      (should (= (length scheduled) 1)))))

(ert-deftest romacs-agent-pools-a-message-unsure-is-told-to-the-parent ()
  ;; Kept messages whose sending its records could not say: its parent is
  ;; told once, a turn of its own; a task with no parent has it in its chat.
  (let ((delivered nil)
        (parent (romacs-agent--make :id "p" :harness 'claude :state 'idle))
        (romacs-agent--sessions (make-hash-table :test #'equal))
        (event (list :type 'error :unsure t :text "may not have reached it")))
    (puthash "p" parent romacs-agent--sessions)
    (cl-letf (((symbol-function 'romacs-agent-tools--deliver)
               (lambda (session text _delivered _valid _messages kind) (push (list session text kind) delivered))))
      (romacs-agent-tools--say-unsure (romacs-agent--make :id "c" :options '(:parent "p")) event)
      (should (equal delivered (list (list parent "may not have reached it" 'next))))
      (romacs-agent-tools--say-unsure (romacs-agent--make :id "o") event)
      (romacs-agent-tools--say-unsure (romacs-agent--make :id "c" :options '(:parent "p")) '(:type error :text "other"))
      (should (= (length delivered) 1)))))

(ert-deftest romacs-agent-pools-the-room-file-has-one-writer ()
  (romacs-agent-pools-tests--with
    (let* ((file (romacs-agent--room-file)) (lock (concat file ".lock")) (seen nil))
      (romacs-agent--with-lock file (lambda () (setq seen (file-directory-p lock))))
      (should seen)
      (should-not (file-exists-p lock))
      ;; Held by another: not run, tried again shortly.
      (make-directory lock t)
      (let ((retried nil))
        (cl-letf (((symbol-function 'run-at-time) (lambda (&rest args) (setq retried args))))
          (romacs-agent--with-lock file (lambda () (setq seen 'unlocked))))
        (should-not (eq seen 'unlocked))
        (should (eq (nth 2 retried) #'romacs-agent--with-lock)))
      ;; A crashed writer's lock is taken over.
      (set-file-times lock (time-subtract (current-time) 60))
      (romacs-agent--with-lock file (lambda () (setq seen 'ran)))
      (should (eq seen 'ran))
      (should-not (file-exists-p lock)))))

(ert-deftest romacs-agent-pools-switching-to-a-harness-is-its-pool ()
  (romacs-agent-pools-tests--with
    (let ((session (romacs-agent--make :id "x" :harness 'claude :provider "claude"))
          (to nil))
      (puthash "x" session romacs-agent--sessions)
      (cl-letf (((symbol-function 'romacs-agent-tools--caller) (lambda () "x"))
                ((symbol-function 'romacs-agent-switch-harness-soon) (lambda (_task provider &rest _) (setq to provider) 'now)))
        (romacs-agent-tools--switch (romacs-mcp--object "provider" "codex"))
        (should (eq to 'codex))))))

;;; Asking spent accounts again

(ert-deftest romacs-agent-pools-claude-usage-as-a-report ()
  (let* ((object (json-parse-string
                  "{\"five_hour\":null,\"seven_day\":{\"utilization\":1.0,\"resets_at\":\"2026-11-23T05:59:59.888143+00:00\"},\"seven_day_opus\":{\"utilization\":100.0,\"resets_at\":null}}"
                  :object-type 'hash-table :null-object nil :false-object nil))
         (report (romacs-agent--claude-usage-report object 5.0)))
    ;; Percentages there, fractions here; null unused; one not named, not said.
    (should (eq (plist-get report :status) 'rejected))
    (should (equal (mapcar (lambda (w) (list (plist-get w :name) (plist-get w :used))) (plist-get report :windows))
                   '(("five_hour" 0.0) ("seven_day" 0.01) ("seven_day_opus" 1.0))))
    (remhash "seven_day_opus" object)
    (setq report (romacs-agent--claude-usage-report object 5.0))
    (should (eq (plist-get report :status) 'allowed))
    (should (= (plist-get report :used) 0.01))
    ;; Its fraction of a second may be dropped.
    (should (<= (abs (- (plist-get report :resets) 1795413600)) 1))
    (should (= (plist-get report :time) 5.0))
    (puthash "five_hour" (json-parse-string "{\"utilization\":100.0,\"resets_at\":null}" :object-type 'hash-table) object)
    (should (eq (plist-get (romacs-agent--claude-usage-report object 6.0) :status) 'rejected))
    (should-not (romacs-agent--claude-usage-report (make-hash-table :test #'equal) 7.0))))

(ert-deftest romacs-agent-pools-claude-usage-keeps-its-token-off-the-command-line ()
  (let* ((home (make-temp-file "claude-home" t)) (runs nil) (sent nil) (got 'none)
         (keep (lambda (expires)
                 (with-temp-file (expand-file-name ".credentials.json" home)
                   (insert (format "{\"claudeAiOauth\":{\"accessToken\":\"sekrit\",\"expiresAt\":%d}}"
                                   (round (* 1000 expires))))))))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent--claude-run)
                   (lambda (_name command _environment then) (push (cons command then) runs) 'process))
                  ((symbol-function 'process-send-string) (lambda (_p text) (push text sent)))
                  ((symbol-function 'process-send-eof) #'ignore))
          ;; Fresh: asked at once, the token on curl's stdin.
          (funcall keep (+ (float-time) 3600))
          (romacs-agent--claude-usage home (lambda (report) (setq got report)))
          (should (equal (car (car (car runs))) "curl"))
          (should-not (cl-find "sekrit" (car (car runs)) :test #'string-search))
          (should (equal sent '("Authorization: Bearer sekrit\n")))
          (funcall (cdr (car runs)) "{\"seven_day\":{\"utilization\":40.0,\"resets_at\":null}}\n200" 0)
          (should (= (plist-get got :used) 0.4))
          ;; Refused: not known.
          (funcall (cdr (car runs)) "{}\n401" 0)
          (should-not got)
          ;; Expired: Claude Code renews it in place first; still expired, not known.
          (setq runs nil got 'none)
          (funcall keep (- (float-time) 10))
          (romacs-agent--claude-usage home (lambda (report) (setq got report)))
          (should (equal (cdr (car (car runs))) '("auth" "status" "--json")))
          (funcall (cdr (car runs)) "" 0)
          (should-not got)
          ;; Renewed meanwhile: asked.
          (setq runs nil got 'none)
          (romacs-agent--claude-usage home (lambda (report) (setq got report)))
          (funcall keep (+ (float-time) 3600))
          (funcall (cdr (car runs)) "" 0)
          (should (equal (car (car (car runs))) "curl"))
          ;; No sign-in: not known, nothing run.
          (delete-file (expand-file-name ".credentials.json" home))
          (setq runs nil got 'none)
          (romacs-agent--claude-usage home (lambda (report) (setq got report)))
          (should-not (or runs got)))
      (delete-directory home t))))

(ert-deftest romacs-agent-pools-a-spent-pool-is-asked-and-resumes-early ()
  ;; Every account spent, the first back days away: asked, one has room
  ;; already, and the session waiting for that reset goes on now.
  (romacs-agent-pools-tests--with
    (let* ((dir (make-temp-file "s" t))
           (session (romacs-agent--make :id "e" :harness 'claude :provider "claude2" :state 'idle :caught-up t
                                        :dir dir :done-seq 4 :input-seq 3 :limited '(:provider "claude2")))
           (later (+ (float-time) (* 3 86400))) (asked nil) (moved nil) (timers nil)
           (romacs-agent-room-check-interval 600)
           (romacs-agent-tools--resumes (make-hash-table :test #'eq :weakness 'key))
           (romacs-agent-tools--resets-used (make-hash-table :test #'eq :weakness 'key))
           (romacs-agent-switch-delay 0) (romacs-agent-resume-margin 1))
      (puthash "e" session romacs-agent--sessions)
      (romacs-agent--manifest session nil)
      (cl-letf (((symbol-function 'run-at-time) (lambda (delay repeat function &rest _)
                                                  (push (list delay repeat function) timers) (list 'timer delay)))
                ((symbol-function 'cancel-timer) (lambda (timer) (setq timers (cl-remove (cadr timer) timers :key #'car))))
                ((symbol-function 'romacs-agent-tools--wake) #'ignore)
                ((symbol-function 'romacs-agent-move-on) (lambda (_s why) (push why moved)))
                ((symbol-function 'romacs-agent--claude-usage)
                 (lambda (home callback) (push (file-name-nondirectory home) asked)
                   ;; The others' usage not known now.
                   (funcall callback
                            (and (equal (file-name-nondirectory home) "claude")
                                 (list :type 'rate-limit :status 'allowed :used 0.01 :window "seven_day"
                                       :windows (list (list :name "five_hour" :status 'allowed :used 0.0)
                                                      (list :name "seven_day" :status 'allowed :used 0.01))
                                       :time (float-time)))))))
        (dolist (spent `(("claude" . ,later) ("claude2" . ,(+ later 600)) ("claude3" . ,(+ later 900))))
          (romacs-agent-note-room (car spent) (list :status 'rejected :resets (cdr spent) :window "seven_day" :time 1.0)))
        ;; Spent: asked again every interval.
        (should (cl-find #'romacs-agent-check-room timers :key #'caddr))
        (romacs-agent-tools--switch-on-limit session '(:limit t))
        (should (= (romacs-agent-resume-time session) (+ later 1)))
        (romacs-agent-check-room)
        (should (equal (sort asked #'string<) '("claude" "claude2" "claude3")))
        ;; The first has room: the wait is over, moved there.
        (should (romacs-agent-room "claude"))
        (should-not (romacs-agent-resume-time session))
        (should (equal moved '("limit reset")))
        ;; Its next turn stops at a limit again, the same resets ahead:
        ;; it waits for them again.
        (romacs-agent-note-room "claude" (list :status 'rejected :resets later :window "seven_day" :time (float-time)))
        (romacs-agent-tools--switch-on-limit session '(:limit t))
        (should (romacs-agent-resume-time session))
        ;; An answer not known changes nothing; none asked twice at once.
        (setq asked nil)
        (puthash "claude2" t romacs-agent--room-asking)
        (romacs-agent-check-room)
        (should (equal (sort asked #'string<) '("claude" "claude3")))
        ;; None spent: asked no longer.
        (dolist (name '("claude" "claude2" "claude3"))
          (romacs-agent-note-room name (list :status 'allowed :used 0.2 :window "seven_day" :time (float-time))))
        (should-not (cl-find #'romacs-agent-check-room timers :key #'caddr))))))

;;; romacs-agent-pools-tests.el ends here
