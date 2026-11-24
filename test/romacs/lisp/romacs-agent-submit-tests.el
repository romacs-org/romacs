;;; romacs-agent-submit-tests.el --- Asynchronous draft submission -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-log))
(require 'romacs-agent-claude)
(require 'romacs-agent-test-attached)
(require 'romacs-agent-test-stored)

(declare-function romacs-agent-submit "romacs-agent" (session-id intent callback))

(defun romacs-agent-submit-tests--record (session object &optional journal)
  "Keep the input JOURNAL (or OBJECT) as the supervisor would for SESSION."
  (write-region (concat (json-serialize (romacs-mcp--object "in" (or journal object))) "\n")
                nil (expand-file-name "journal.jsonl" (romacs-agent-dir session)) t 'silent))

(defun romacs-agent-submit-tests--inputs (session)
  "Read the user inputs back from SESSION's journal."
  (let (inputs)
    (romacs-agent--read-journal
     session (lambda (record)
               (when-let* ((input (gethash "in" record))
                           ((equal (gethash "type" input) "user")))
                 (push (romacs-agent--text-of (gethash "content" (gethash "message" input))) inputs))))
    (nreverse inputs)))

(defun romacs-agent-submit-tests--session (body)
  "Call BODY with a registered, attached Claude session and a private store."
  (romacs-agent-test-with-store
    (let* ((romacs-agent--sessions (make-hash-table :test #'equal))
           (id (romacs-agent--uuid))
           (dir (expand-file-name id (romacs-agent--state-root)))
           (session (romacs-agent--make :id id :dir dir :harness 'claude :state 'idle :title "S1 test")))
      (make-directory dir t)
      (puthash id session romacs-agent--sessions)
      (romacs-agent-test-attach session t)
      (unwind-protect
          (cl-letf (((symbol-function 'romacs-agent--line) #'romacs-agent-submit-tests--record))
            (funcall body session))
        (delete-process (romacs-agent-connection session))))))

(defun romacs-agent-submit-tests--await (predicate)
  "Wait for PREDICATE while servicing callback timers."
  (let ((deadline (+ (float-time) 3)))
    (while (and (not (funcall predicate)) (< (float-time) deadline))
      (accept-process-output nil 0.02))
    (should (funcall predicate))))

(defun romacs-agent-submit-tests--answer (session-id intent)
  "Submit INTENT to SESSION-ID and return its sole, deferred answer."
  (let (answers)
    (romacs-agent-submit session-id intent (lambda (answer) (push answer answers)))
    (should-not answers)
    (romacs-agent-submit-tests--await (lambda () answers))
    (let (drained)
      (run-at-time 0 nil (lambda () (setq drained t)))
      (romacs-agent-submit-tests--await (lambda () drained)))
    (should (= (length answers) 1))
    (should (equal (plist-get (car answers) :draft-id) (plist-get intent :draft-id)))
    (car answers)))

(defun romacs-agent-submit-tests--intent (&optional text kind)
  (list :draft-id (romacs-agent--uuid) :kind (or kind 'send)
        :text (or text "say once") :attachments nil))

(defun romacs-agent-submit-tests--refused (answer reason)
  (should (eq (plist-get answer :status) 'refused))
  (should (eq (plist-get answer :reason) reason))
  (should (stringp (plist-get answer :message)))
  (should-not (string-empty-p (plist-get answer :message))))

(ert-deftest romacs-agent-submit-validation ()
  (romacs-agent-test-with-store
    (dolist (intent (list (romacs-agent-submit-tests--intent "text" 'unknown)
                          (romacs-agent-submit-tests--intent "")))
      (romacs-agent-submit-tests--refused
       (romacs-agent-submit-tests--answer (romacs-agent--uuid) intent) 'invalid))))

(ert-deftest romacs-agent-submit-dedup ()
  (romacs-agent-submit-tests--session
   (lambda (session)
     (let* ((intent (romacs-agent-submit-tests--intent))
            (id (romacs-agent-id session))
            (first (romacs-agent-submit-tests--answer id intent)))
       (should (eq (plist-get first :status) 'accepted))
       ;; Dedup precedes discovery and dispatch, even if the harness has since ended.
       (setf (romacs-agent-state session) 'ended)
       (should (equal (romacs-agent-submit-tests--answer id intent) first))
       (should (equal (romacs-agent-submit-tests--inputs session) '("say once")))))))

(ert-deftest romacs-agent-submit-no-session ()
  (romacs-agent-test-with-store
    (romacs-agent-submit-tests--refused
     (romacs-agent-submit-tests--answer (romacs-agent--uuid) (romacs-agent-submit-tests--intent))
     'no-session)))

(ert-deftest romacs-agent-submit-closed ()
  (romacs-agent-submit-tests--session
   (lambda (session)
     (setf (romacs-agent-state session) 'ended)
     (romacs-agent-submit-tests--refused
      (romacs-agent-submit-tests--answer (romacs-agent-id session) (romacs-agent-submit-tests--intent))
      'closed)
     (should-not (romacs-agent-submit-tests--inputs session)))))

(ert-deftest romacs-agent-submit-owner-unavailable ()
  (romacs-agent-submit-tests--session
   (lambda (session)
     (cl-letf (((symbol-function 'romacs-agent--line) (lambda (&rest _) (error "Harness write failed"))))
       (romacs-agent-submit-tests--refused
        (romacs-agent-submit-tests--answer (romacs-agent-id session) (romacs-agent-submit-tests--intent))
        'owner-unavailable))
     (should-not (romacs-agent-submit-tests--inputs session)))))

(ert-deftest romacs-agent-submit-queue-position ()
  (romacs-agent-submit-tests--session
   (lambda (session)
     (setf (romacs-agent-state session) 'running)
     (romacs-agent-send-later session "already queued")
     (let ((answer (romacs-agent-submit-tests--answer
                    (romacs-agent-id session) (romacs-agent-submit-tests--intent "queue me" 'queue))))
       (should (eq (plist-get answer :status) 'accepted))
       (should (numberp (plist-get answer :queued)))
       (should (equal (mapcar #'romacs-agent-item-text (romacs-agent-queue session))
                      '("already queued" "queue me")))))))

(ert-deftest romacs-agent-submit-callback-once-after-return ()
  (romacs-agent-submit-tests--session
   (lambda (session)
     (should (eq (plist-get (romacs-agent-submit-tests--answer
                             (romacs-agent-id session) (romacs-agent-submit-tests--intent))
                            :status)
                 'accepted)))))

(defun romacs-agent-submit-tests--chat (session body)
  "Run BODY in the composer of SESSION's real chat, with a draft and a clean-up."
  (let ((chat (romacs-agent-log-chat session t)))
    (unwind-protect
        (with-current-buffer (romacs-agent-log-composer-buffer chat)
          (insert "  say once  ")
          (funcall body))
      (kill-buffer chat))))

(defun romacs-agent-submit-tests--ret ()
  "Invoke the binding RET has at point in the composer."
  (when-let* ((command (key-binding (kbd "RET"))) ((commandp command)))
    (call-interactively command)))

(defun romacs-agent-submit-tests--draft ()
  (buffer-substring-no-properties (point-min) (point-max)))

(defun romacs-agent-submit-tests--sending-face-p ()
  "Whether the prompt before the draft shows in the sending face."
  (let ((mark (overlay-get romacs-agent-compose--start 'before-string)))
    (eq (get-text-property (1- (length mark)) 'face mark) 'romacs-agent-compose-sending)))

(defun romacs-agent-submit-tests--held (body)
  "Run BODY in a chat whose submit callbacks are captured for explicit release."
  (romacs-agent-submit-tests--session
   (lambda (session)
     (romacs-agent-submit-tests--chat
      session
      (lambda ()
        (let (calls)
          (cl-letf (((symbol-function 'romacs-agent-submit)
                     (lambda (id intent callback) (push (list id intent callback) calls))))
            (funcall body (lambda () (reverse calls))))))))))

(defun romacs-agent-submit-tests--release (calls status &rest details)
  (should (= (length calls) 1))
  (pcase-let ((`(,_ ,intent ,callback) (car calls)))
    (funcall callback (append (list :draft-id (plist-get intent :draft-id) :status status) details))))

(ert-deftest romacs-agent-submit-composer-sending ()
  (romacs-agent-submit-tests--held
   (lambda (calls)
     (romacs-agent-submit-tests--ret)
     (should (equal (romacs-agent-submit-tests--draft) "  say once  "))
     (should (= (length (funcall calls)) 1))
     (let ((intent (nth 1 (car (funcall calls)))))
       (should (stringp (plist-get intent :draft-id)))
       (should (memq (plist-get intent :kind) '(send steer)))
       (should (equal (plist-get intent :text) "say once"))
       (should (plist-member intent :attachments))
       (should-not (plist-get intent :attachments)))
     (should-error (insert "x") :type '(buffer-read-only text-read-only))
     (should (romacs-agent-submit-tests--sending-face-p))
     (romacs-agent-submit-tests--ret)
     (should (= (length (funcall calls)) 1))
     (romacs-agent-submit-tests--release (funcall calls) 'accepted))))

(ert-deftest romacs-agent-submit-composer-accepted-clears-once ()
  (romacs-agent-submit-tests--held
   (lambda (calls)
     (let ((clears 0))
       (add-hook 'romacs-agent-log-draft-replaced-hook (lambda () (cl-incf clears)) nil t)
       (romacs-agent-submit-tests--ret)
       (should (= clears 0))
       (romacs-agent-submit-tests--release (funcall calls) 'accepted)
       (should (equal (romacs-agent-submit-tests--draft) ""))
       (should (= clears 1))
       (insert "new draft")
       (should (equal (romacs-agent-submit-tests--draft) "new draft"))))))

(ert-deftest romacs-agent-submit-composer-refused-keeps-draft-and-shows-message ()
  (romacs-agent-submit-tests--held
   (lambda (calls)
     (let ((reason "The owner is unavailable") shown)
       (cl-letf (((symbol-function 'message)
                  (lambda (format-string &rest args)
                    (when format-string (push (apply #'format format-string args) shown)))))
         (romacs-agent-submit-tests--ret)
         (romacs-agent-submit-tests--release (funcall calls) 'refused :reason 'owner-unavailable :message reason)
         (should (equal (romacs-agent-submit-tests--draft) "  say once  "))
         (should (member reason shown))
         (insert " retry"))))))

(ert-deftest romacs-agent-submit-refusal-can-retry-same-draft-id ()
  (romacs-agent-submit-tests--session
   (lambda (session)
     (romacs-agent-submit-tests--chat
      session
      (lambda ()
        (let ((submit (symbol-function 'romacs-agent-submit)) ids answers)
          (cl-letf (((symbol-function 'romacs-agent-submit)
                     (lambda (id intent callback)
                       (push (plist-get intent :draft-id) ids)
                       (funcall submit id intent
                                (lambda (answer) (funcall callback answer) (push answer answers))))))
            (setf (romacs-agent-state session) 'ended)
            (romacs-agent-submit-tests--ret)
            (romacs-agent-submit-tests--await (lambda () answers))
            (romacs-agent-submit-tests--refused (car answers) 'closed)
            (should-not (romacs-agent-submit-tests--inputs session))
            (setf (romacs-agent-state session) 'idle)
            (romacs-agent-submit-tests--ret)
            (romacs-agent-submit-tests--await (lambda () (>= (length answers) 2)))
            (should (= (length ids) 2))
            (should (stringp (cadr ids)))
            (should (equal (car ids) (cadr ids)))
            (should (eq (plist-get (car answers) :status) 'accepted))
            (should (equal (romacs-agent-submit-tests--inputs session) '("say once"))))))))))

(provide 'romacs-agent-submit-tests)
;;; romacs-agent-submit-tests.el ends here
