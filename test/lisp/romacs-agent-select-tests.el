;;; romacs-agent-select-tests.el --- A new model or effort in a session -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-ui))

(defmacro romacs-agent-select-tests--with (fields &rest body)
  "BODY with `session', of FIELDS, recorded in a private directory."
  (declare (indent 1))
  `(let* ((dir (make-temp-file "romacs-select-" t))
          (session (romacs-agent--make ,@fields :id "s" :harness 'claude :provider "claude" :dir dir
                                       :host-socket (expand-file-name "none.sock" dir))))
     (unwind-protect
         (progn (romacs-agent--manifest session nil) ,@body)
       (delete-directory dir t))))

(defun romacs-agent-select-tests--subtypes (lines)
  (mapcar (lambda (line) (gethash "subtype" (gethash "request" (cdr line)))) lines))

(ert-deftest romacs-agent-select-claude-lines ()
  (romacs-agent-select-tests--with (:options '(:model "a" :effort "low"))
    ;; Each of what changes, and nothing else.
    (should (equal (romacs-agent-select-tests--subtypes (romacs-agent--claude-select session "b" "low"))
                   '("set_model")))
    (should (equal (romacs-agent-select-tests--subtypes (romacs-agent--claude-select session "a" "high"))
                   '("apply_flag_settings")))
    (let ((lines (romacs-agent--claude-select session "b" "high")))
      (should (equal (romacs-agent-select-tests--subtypes lines) '("set_model" "apply_flag_settings")))
      (should (equal (gethash "model" (gethash "request" (cdr (nth 0 lines)))) "b"))
      (should (equal (gethash "effortLevel" (gethash "settings" (gethash "request" (cdr (nth 1 lines))))) "high"))
      (should-not (equal (car (nth 0 lines)) (car (nth 1 lines)))))
    ;; What a live harness cannot clear takes a new one.
    (should (eq (romacs-agent--claude-select session "a" nil) 'restart))
    (should (eq (romacs-agent--claude-select session nil "low") 'restart))))

(ert-deftest romacs-agent-select-nothing-to-change ()
  (romacs-agent-select-tests--with (:options '(:model "a" :effort "low") :state 'idle)
    (should-not (romacs-agent-select session "a" "low"))
    (should-not (romacs-agent-pending-selection session))))

(ert-deftest romacs-agent-select-holds-messages-until-confirmed ()
  (romacs-agent-select-tests--with (:options '(:model "a") :state 'idle :caught-up t)
    (let ((written nil) (sent nil) (romacs-agent-select-timeout 3600))
      (cl-letf (((symbol-function 'romacs-agent--line) (lambda (_session object) (push object written)))
                ((symbol-function 'romacs-agent--harness-live-p) (lambda (_session) t))
                ((symbol-function 'romacs-agent--harness-free-p) (lambda (_session) t))
                ((symbol-function 'romacs-agent--input) (lambda (_session text &rest _) (push text sent)))
                ((symbol-function 'romacs-agent--note) #'ignore))
        (romacs-agent-select session "b" "high")
        (should (= (length written) 2))
        (should (eq (plist-get (romacs-agent-pending-selection session) :state) 'applying))
        (romacs-agent-send session "waits")
        (romacs-agent--send-next session)
        (should-not sent)
        (should (= (romacs-agent-queue-length session) 1))
        (let ((ids (plist-get (romacs-agent-pending-selection session) :waiting)))
          ;; Answered out of order, as Claude Code does; another id is not one.
          (should-not (romacs-agent--select-answered session "other" t))
          (should (romacs-agent--select-answered session (cadr ids) t))
          (should (romacs-agent-pending-selection session))
          (should-not sent)
          (should (romacs-agent--select-answered session (car ids) t)))
        (should-not (romacs-agent-pending-selection session))
        (should (equal sent '("waits")))
        (should (equal (plist-get (romacs-agent-options session) :model) "b"))
        (should (equal (plist-get (romacs-agent-options session) :effort) "high"))))))

(ert-deftest romacs-agent-select-refused-restarts ()
  (romacs-agent-select-tests--with (:state 'idle)
    (let ((restarted nil))
      (setf (romacs-agent-pending-selection session) (list :state 'applying :waiting '("x") :model "b" :effort nil))
      (cl-letf (((symbol-function 'romacs-agent--restart) (lambda (_session) (setq restarted t))))
        (should (romacs-agent--select-answered session "x" nil))
        (should restarted)))))

(ert-deftest romacs-agent-select-restart-leaves-messages-to-the-resume ()
  ;; Nothing takes them while the harness is gone: the resume delivers them.
  (dolist (state '(ended detached))
    (romacs-agent-select-tests--with (:state state)
      (let ((resumed nil))
        (romacs-agent--enqueue session (list "held" nil nil))
        (setf (romacs-agent-pending-selection session) (list :state 'restarting :stopped t :model "b"))
        (cl-letf (((symbol-function 'romacs-agent-resume) (lambda (_session) (setq resumed t)))
                  ((symbol-function 'romacs-agent--input) (lambda (&rest _) (error "No harness to take it"))))
          (romacs-agent--restarted session))
        (should resumed)
        ;; Attached: the held ones go.
        (should (eq (plist-get (romacs-agent-pending-selection session) :state) 'resuming))
        (romacs-agent-send session "held too")
        (should (= (romacs-agent-queue-length session) 2))
        (let ((sent nil))
          (cl-letf (((symbol-function 'romacs-agent--input) (lambda (_session text &rest _) (push text sent))))
            (setf (romacs-agent-state session) 'starting)
            (romacs-agent--settle session))
          (should-not (romacs-agent-pending-selection session))
          (should (equal (car (last sent)) "held")))))))

(ert-deftest romacs-agent-select-resuming-leaves-nothing-to-restore ()
  ;; Restarted on the selection: the launch's manifest says it took effect,
  ;; so a later server keeps it and applies nothing.
  (romacs-agent-select-tests--with (:state 'ended :options '(:model "a"))
    (setf (romacs-agent-pending-selection session) (list :state 'restarting :stopped t :model "b" :effort "high"))
    (cl-letf (((symbol-function 'romacs-agent-resume)
               (lambda (session) (romacs-agent--manifest session nil))))
      (romacs-agent--restarted session))
    (let ((manifest (romacs-agent--read-json (expand-file-name "manifest.json" dir))))
      (should-not (gethash "switch" manifest))
      (should (equal (gethash "model" (gethash "options" manifest)) "b")))
    (let ((romacs-agent--sessions (make-hash-table :test #'equal)))
      (let ((loaded (romacs-agent--load dir)))
        (should-not (romacs-agent-pending-selection loaded))
        (should (equal (plist-get (romacs-agent-options loaded) :model) "b"))
        (should (equal (plist-get (romacs-agent-options loaded) :effort) "high"))))))

(ert-deftest romacs-agent-select-restart-with-the-harness-already-gone ()
  ;; No unit, and no connection for a sentinel to report on.
  (romacs-agent-select-tests--with (:state 'ended)
    (let ((resumed nil))
      (setf (romacs-agent-pending-selection session) (list :state 'applying :waiting '("x") :model "b"))
      (cl-letf (((symbol-function 'romacs-agent-resume) (lambda (_session) (setq resumed t))))
        (romacs-agent--restart session))
      (should resumed)
      (should (equal (plist-get (romacs-agent-options session) :model) "b")))))

(ert-deftest romacs-agent-select-armed-died-keeps-messages ()
  (romacs-agent-select-tests--with (:state 'ended)
    (romacs-agent--enqueue session (list "held" nil nil))
    (cl-letf (((symbol-function 'romacs-agent--input) (lambda (&rest _) (error "No harness to take it"))))
      (romacs-agent-select session "b" nil))
    (should-not (romacs-agent-pending-selection session))
    (should (= (romacs-agent-queue-length session) 1))))

(ert-deftest romacs-agent-select-superseding-sent-requests-restarts ()
  ;; A/low asked to go to B/high, one request confirmed: C/low cannot be
  ;; reached from what the harness may be on, so a new one is.
  (romacs-agent-select-tests--with (:state 'idle :options '(:model "a" :effort "low"))
    (let ((restarted nil) (romacs-agent-select-timeout 3600))
      (setf (romacs-agent-pending-selection session)
            (list :state 'applying :waiting '("x") :model "b" :effort "high"
                  :timer (run-at-time 3600 nil #'ignore)))
      (cl-letf (((symbol-function 'romacs-agent--restart)
                 (lambda (session) (setq restarted (plist-get (romacs-agent-pending-selection session) :model)))))
        (should (romacs-agent-select session "c" "low")))
      (should (equal restarted "c")))))

(ert-deftest romacs-agent-select-back-cancels-what-is-armed ()
  (romacs-agent-select-tests--with (:state 'running :options '(:model "a"))
    (should (romacs-agent-select session "b" nil))
    (should (romacs-agent-pending-selection session))
    ;; What is armed again changes nothing; going back drops it.
    (should-not (romacs-agent-select session "b" nil))
    (should (romacs-agent-select session "a" nil))
    (should-not (romacs-agent-pending-selection session))
    (should-not (gethash "switch" (romacs-agent--read-json (expand-file-name "manifest.json" dir))))))

(ert-deftest romacs-agent-select-replay-leaves-the-live-selection ()
  ;; A chat replaying the journal meets the confirmation before the live
  ;; session's filter does: the live session's waiting stands.
  (romacs-agent-select-tests--with (:state 'idle :options '(:model "a") :buffer nil)
    (let ((switch (list :state 'applying :waiting '("x") :model "b" :effort nil)))
      (setf (romacs-agent-pending-selection session) switch)
      (let ((coding-system-for-write 'utf-8-unix))
        (write-region (concat (romacs-agent--json
                               (romacs-mcp--object
                                "seq" 1 "t" 1.0
                                "out" (romacs-mcp--object
                                       "type" "control_response"
                                       "response" (romacs-mcp--object "subtype" "success" "request_id" "x"))))
                              "\n")
                      nil (expand-file-name "journal.jsonl" dir) nil 'silent))
      (with-temp-buffer
        (romacs-agent-chat-mode)
        (let ((inhibit-read-only t))
          (setq romacs-agent-ui--session session
                romacs-agent-ui--tool-lines (make-hash-table :test #'equal)
                romacs-agent-ui--messages (make-hash-table :test #'equal)
                romacs-agent-ui--end (point-marker))
          (romacs-agent-ui--replay session)))
      (should (eq (romacs-agent-pending-selection session) switch))
      (should (equal (plist-get (romacs-agent-pending-selection session) :waiting) '("x")))
      (should (equal (plist-get (romacs-agent-options session) :model) "a")))))

(ert-deftest romacs-agent-select-armed-while-a-turn-runs ()
  (dolist (state '(running waiting-approval))
    (romacs-agent-select-tests--with (:state state)
      (cl-letf (((symbol-function 'romacs-agent--line) (lambda (&rest _) (error "Written mid-turn"))))
        (romacs-agent-select session "b" nil)
        (should (eq (plist-get (romacs-agent-pending-selection session) :state) 'armed))
        ;; It survives this server.
        (should (equal (gethash "model" (gethash "switch" (romacs-agent--read-json
                                                           (expand-file-name "manifest.json" dir))))
                       "b"))))))

(ert-deftest romacs-agent-select-with-no-harness-is-recorded-and-noted ()
  ;; An ended session: its resume uses the manifest's options.  The journal
  ;; has no supervisor to write it, and the divider stays in its place.
  (romacs-agent-select-tests--with (:state 'ended :options '(:model "a" :role "reviewer") :seq 7)
    (let ((events nil))
      (let ((romacs-agent-event-functions (list (lambda (_session event) (push event events)))))
        (should (romacs-agent-select session "b" "low")))
      (should-not (romacs-agent-pending-selection session))
      (should (equal (plist-get (romacs-agent-options session) :role) "reviewer"))
      (let ((manifest (romacs-agent--read-json (expand-file-name "manifest.json" dir))))
        (should (equal (gethash "model" (gethash "options" manifest)) "b"))
        (should (equal (gethash "effort" (gethash "options" manifest)) "low"))
        (should-not (gethash "switch" manifest)))
      (let ((event (cl-find 'selection events :key (lambda (e) (plist-get e :type)))))
        (should (equal (plist-get event :from) '(:model "a" :effort nil)))
        (should (equal (plist-get event :to) '(:model "b" :effort "low"))))
      (let ((record (with-temp-buffer
                      (insert-file-contents (expand-file-name "journal.jsonl" dir))
                      (json-parse-string (buffer-string)))))
        (should (= (gethash "seq" record) 8))
        (should (equal (gethash "type" (gethash "note" record)) "selection")))
      (should (= (romacs-agent-seq session) 8)))))

(ert-deftest romacs-agent-select-replayed-note-only-draws ()
  ;; A replay's selection neither changes the options nor writes anything.
  (romacs-agent-select-tests--with (:state 'idle :options '(:model "now") :context-window 1000)
    (romacs-agent--record
     session (romacs-mcp--object "seq" 1 "t" 1.0
                                 "note" (romacs-mcp--object
                                         "type" "selection"
                                         "from" (romacs-mcp--object "model" "a" "effort" :null)
                                         "to" (romacs-mcp--object "model" "b" "effort" "low"))))
    (should (equal (romacs-agent-options session) '(:model "now")))
    (should (equal (romacs-agent-model session) "b"))
    ;; The context window was the last model's.
    (should-not (romacs-agent-context-window session))))

(ert-deftest romacs-agent-select-free-harness ()
  (cl-flet ((free (state &optional per-turn live)
              (romacs-agent-select-tests--with (:state state :caught-up t
                                                       :harness (if per-turn 'codex 'claude))
                (cl-letf (((symbol-function 'process-live-p) (lambda (_process) live)))
                  (romacs-agent--harness-free-p session)))))
    (should-not (free 'running nil t))
    (should-not (free 'waiting-approval nil t))
    (should (free 'idle nil t))
    (should (free 'ended))
    (should (free 'starting nil t))
    ;; A per-turn harness's run must have retired.
    (should-not (free 'idle t t))
    (should (free 'idle t nil))
    ;; No supervisor left to hear from.
    (should (free 'detached))))

(provide 'romacs-agent-select-tests)
;;; romacs-agent-select-tests.el ends here
