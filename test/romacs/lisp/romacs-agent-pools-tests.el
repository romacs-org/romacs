;;; romacs-agent-pools-tests.el --- A harness's accounts as one pool -*- lexical-binding: t; -*-
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
        ;; Each request's use alone is not written.
        (romacs-agent--note-room (romacs-agent--make :provider "claude2")
                                 (list :status 'allowed :used 0.4 :window "five_hour" :time 3.0))
        (should (= writes 2))))
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
                ((symbol-function 'romacs-agent--enqueue) (lambda (_s item) (push (car item) queued))))
        (let ((romacs-agent-moved-functions (list (lambda (_s from to) (push (cons from to) moved)))))
          (should (equal (romacs-agent-move-on session) "claude2"))
          (should (equal selected "claude2"))
          ;; Told to go on only once the move took effect.
          (should-not queued)
          (romacs-agent--raise session '(:type selection :from (:provider "claude") :to (:provider "claude2")))
          (should (string-match-p "claude's usage limit; you now run on claude2" (car queued)))
          (should (equal moved '(("claude" . "claude2"))))
          ;; A move that did not take effect: nothing to go on with.
          (setq queued nil)
          (romacs-agent-move-on session)
          (romacs-agent--raise session '(:type selection :from (:provider "claude") :to (:provider "claude")))
          (should-not queued)
          (should (equal (car moved) '("claude" . "claude"))))
        (spend "claude2" (+ (float-time) 3600))
        (spend "claude3" (+ (float-time) 3600))
        (should-error (romacs-agent-move-on session) :type 'user-error)))))

(ert-deftest romacs-agent-pools-shown-to-agents-and-the-user ()
  (romacs-agent-pools-tests--with
    (should-not (romacs-agent-ui--pools-line))
    (spend "claude" (+ (float-time) 3600))
    (allow "claude2")
    (let ((line (romacs-agent-ui--pools-line)))
      (should (equal (substring-no-properties line) "⛽claude 2/3"))
      (should (string-match-p "\\`claude: rejected, resets " (get-text-property 0 'help-echo line))))
    (let* ((pools (gethash "pools" (romacs-agent-tools--capabilities (romacs-mcp--object))))
           (claude (cl-find "claude" pools :key (lambda (p) (gethash "harness" p)) :test #'equal)))
      (should (equal (gethash "next" claude) "claude2"))
      (should (equal (mapcar (lambda (a) (list (gethash "providerInstanceId" a) (gethash "room" a)))
                             (gethash "accounts" claude))
                     '(("claude" :false) ("claude2" t) ("claude3" t)))))))

;;; romacs-agent-pools-tests.el ends here
