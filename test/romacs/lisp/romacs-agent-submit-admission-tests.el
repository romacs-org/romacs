;;; romacs-agent-submit-admission-tests.el --- One admission for every draft -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A draft is sent once, whatever comes between its submit and its
;; answer: a repeat while it is admitted, a view that fails, a reply
;; lost, a restart of the serving editor, a narrowed or undone composer.
;; The asynchronous submit's own cases are romacs-agent-submit-tests.el.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-log)
  (require 'romacs-agent-service)
  (require 'romacs-agent-client)
  (require 'romacs-agent-remote))
(require 'romacs-agent-claude)
(require 'romacs-agent-test-attached)
(require 'romacs-agent-test-stored)
(require 'romacs-test-socket)

(defvar romacs-agent-remote-submit-deadline)

;;; A session, its journal and its answers

(defun romacs-agent-submit-admission-tests--record (session object &optional journal)
  "Keep the input JOURNAL (or OBJECT) as the supervisor would for SESSION."
  (write-region (concat (json-serialize (romacs-mcp--object "in" (or journal object))) "\n")
                nil (expand-file-name "journal.jsonl" (romacs-agent-dir session)) t 'silent))

(defun romacs-agent-submit-admission-tests--inputs (session)
  "The user inputs SESSION's journal holds, in order."
  (let (inputs)
    (romacs-agent--read-journal
     session (lambda (record)
               (when-let* ((input (gethash "in" record))
                           ((equal (gethash "type" input) "user")))
                 (push (romacs-agent--text-of (gethash "content" (gethash "message" input))) inputs))))
    (nreverse inputs)))

(defalias 'romacs-agent-submit-admission-tests--harness-line (symbol-function 'romacs-agent--line)
  "`romacs-agent--line' itself, for a test whose harness's line is written.")

(defun romacs-agent-submit-admission-tests--with (body &optional others)
  "Call BODY with a session, registered, attached and idle, in a store of
its own, its writes kept in its journal, and no draft answered before;
then with OTHERS (a number) more such sessions."
  (romacs-agent-test-with-store
    (let* ((romacs-agent--sessions (make-hash-table :test #'equal))
           (romacs-agent-event-functions nil)
           (romacs-agent-input-functions nil)
           (sessions
            (cl-loop for n from 0 to (or others 0)
                     collect (let* ((id (romacs-agent--uuid))
                                    (dir (expand-file-name id (romacs-agent--state-root)))
                                    (session (romacs-agent--make :id id :dir dir :harness 'claude :state 'idle
                                                                 :title (format "admission %d" n))))
                               (make-directory dir t)
                               (puthash id session romacs-agent--sessions)
                               (romacs-agent-test-attach session t)))))
      (unwind-protect
          (cl-letf (((symbol-function 'romacs-agent--line) #'romacs-agent-submit-admission-tests--record))
            (apply body sessions))
        (dolist (session sessions) (delete-process (romacs-agent-connection session)))))))

(defun romacs-agent-submit-admission-tests--await (predicate &optional seconds)
  "Wait for PREDICATE, timers run meanwhile, SECONDS at most (3 by default)."
  (let ((deadline (+ (float-time) (or seconds 3))))
    (while (and (not (funcall predicate)) (< (float-time) deadline))
      (accept-process-output nil 0.02))
    (should (funcall predicate))))

(defun romacs-agent-submit-admission-tests--settle ()
  "Let every timer due now run: answers given after their call."
  (let ((ran nil))
    (run-at-time 0 nil (lambda () (setq ran t)))
    (romacs-agent-submit-admission-tests--await (lambda () ran))))

(defun romacs-agent-submit-admission-tests--intent (&optional text kind id)
  (list :draft-id (or id (romacs-agent--uuid)) :kind (or kind 'send) :text (or text "say once") :attachments nil))

(defun romacs-agent-submit-admission-tests--submit (session intent)
  "Submit INTENT to SESSION: a cell whose car gathers its answers."
  (let ((answers (list nil)))
    (romacs-agent-submit (romacs-agent-id session) intent (lambda (answer) (push answer (car answers))))
    answers))

(defun romacs-agent-submit-admission-tests--pending-p (draft-id)
  "Whether this editor's draft DRAFT-ID is being admitted still."
  (eq (car-safe (gethash draft-id romacs-agent--submit-drafts)) 'pending))

(defun romacs-agent-submit-admission-tests--on-write (function body)
  "Call BODY with every line written to a harness then calling FUNCTION
with its session: as what a process's output, read while it is written,
runs inside an admission."
  (cl-letf* ((line (symbol-function 'romacs-agent--line))
             ((symbol-function 'romacs-agent--line)
              (lambda (session &rest args) (prog1 (apply line session args) (funcall function session)))))
    (funcall body)))

(defun romacs-agent-submit-admission-tests--queued (session id)
  "How many of SESSION's waiting messages are named ID."
  (cl-count id (romacs-agent-queue session) :key #'romacs-agent-item-id :test #'equal))

;;; A chat whose draft is sent

(defun romacs-agent-submit-admission-tests--chat (session text body)
  "Call BODY in the composer of SESSION's chat, TEXT typed in it."
  (let ((chat (romacs-agent-log-chat session t)))
    (unwind-protect
        (with-current-buffer (romacs-agent-log-composer-buffer chat)
          (goto-char (point-max))
          (insert text)
          (funcall body))
      (kill-buffer chat))))

(defun romacs-agent-submit-admission-tests--held (session text body)
  "Call BODY in SESSION's composer holding TEXT with a function that answers
the draft's submit (once RET has made it) with its status, accepted by
default."
  (romacs-agent-submit-admission-tests--chat
   session text
   (lambda ()
     (let (calls)
       (cl-letf (((symbol-function 'romacs-agent-submit)
                  (lambda (_id intent callback) (push (cons intent callback) calls))))
         (funcall body (lambda (&optional status)
                         (should (= (length calls) 1))
                         (funcall (cdr (car calls))
                                  (append (list :draft-id (plist-get (car (car calls)) :draft-id)
                                                :status (or status 'accepted))
                                          (and (eq status 'refused)
                                               (list :reason 'owner-unavailable :message "Not now")))))))))))

(defun romacs-agent-submit-admission-tests--draft ()
  (save-restriction (widen) (buffer-substring-no-properties (point-min) (point-max))))

(defun romacs-agent-submit-admission-tests--undo ()
  "Undo as its key does in the composer."
  (let ((last-command nil))
    (call-interactively (key-binding (kbd "C-/")))))

(defun romacs-agent-submit-admission-tests--answered ()
  (romacs-agent-submit-admission-tests--await (lambda () (not romacs-agent-log--sending))))

;;; A remote chat whose service is a stub

(defun romacs-agent-submit-admission-tests--client (minor)
  "A client whose service welcomed it at protocol 1.MINOR, connected to none."
  (romacs-agent-client--make :server (romacs-agent-rpc-object "version" (vector 1 minor))))

(defun romacs-agent-submit-admission-tests--remote (session text body &optional minor)
  "Call BODY in a remote chat's composer, SESSION's, holding TEXT, its service a stub
of protocol 1.MINOR (1.1 by default): BODY gets a function returning the
commands sent, (METHOD PARAMS CALLBACK) each, the oldest first."
  (let ((source (romacs-agent-remote--make :client (romacs-agent-submit-admission-tests--client (or minor 1))))
        (sent nil))
    (cl-letf (((symbol-function 'romacs-agent-client-live-p) (lambda (_) t))
              ((symbol-function 'romacs-agent-client-command)
               (lambda (_client method params callback) (push (list method params callback) sent))))
      (with-temp-buffer
        (romacs-agent-log-mode)
        (setq-local romacs-agent-log--session session
                    romacs-agent-log--source source
                    romacs-agent-log--end (point-min-marker)
                    romacs-agent-log--prompts nil)
        (set-marker-insertion-type romacs-agent-log--end t)
        (romacs-agent-log--separator t)
        (with-current-buffer (romacs-agent-log-composer-buffer (current-buffer))
          (insert text)
          (funcall body (lambda () (reverse sent))))))))

;;; The service, over its socket

(defun romacs-agent-submit-admission-tests--serving (body)
  "Call BODY with a session, as `romacs-agent-submit-admission-tests--with'
gives it, its editor serving on a socket of its own; then CONNECT, a
function making a client of an INSTANCE, and ASK, one sending a request
over a CLIENT, METHOD and PARAMS, that returns its (RESULT ERROR)."
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (let* ((root (romacs-test-socket-directory "rsa"))
            (socket (expand-file-name "s.sock" root))
            (romacs-agent-service--commands (make-hash-table :test #'equal))
            (clients nil))
       (unwind-protect
           (progn
             (romacs-agent-service-start socket)
             (funcall body session
                      (lambda (instance)
                        (let* ((romacs-agent-client-instance instance)
                               (client (romacs-agent-client-connect socket)))
                          (push client clients)
                          (romacs-agent-submit-admission-tests--await (lambda () (romacs-agent-client-revision client)))
                          client))
                      (lambda (client method params)
                        (let ((reply nil))
                          (romacs-agent-client-request client method params (lambda (r e) (setq reply (list r e))))
                          (romacs-agent-submit-admission-tests--await (lambda () reply))
                          reply))))
         (mapc #'romacs-agent-client-close clients)
         (romacs-agent-service-stop)
         (delete-directory root t))))))

(defun romacs-agent-submit-admission-tests--send (id cid text &rest more)
  "`agents/send''s params: TEXT to session ID by CID, mode now, and MORE."
  (apply #'romacs-agent-rpc-object "cid" cid "id" id "text" text "mode" "now" more))

;;; B1: a draft is admitted once, whatever signals around it

(ert-deftest romacs-agent-submit-admission-repeat-during-its-write-waits ()
  ;; The same draft submitted again as its first submit writes it (from
  ;; the input hook) waits for that answer: one input, both answered.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (let* ((intent (romacs-agent-submit-admission-tests--intent))
            (again nil)
            (romacs-agent-input-functions
             (list (lambda (&rest _)
                     (unless again
                       (setq again (romacs-agent-submit-admission-tests--submit session intent))))))
            (first (romacs-agent-submit-admission-tests--submit session intent)))
       (romacs-agent-submit-admission-tests--await (lambda () (and (car first) again (car again))))
       (romacs-agent-submit-admission-tests--settle)
       (should (= (length (car first)) 1))
       (should (= (length (car again)) 1))
       (should (equal (car (car first)) (car (car again))))
       (should (eq (plist-get (car (car first)) :status) 'accepted))
       (should (equal (romacs-agent-submit-admission-tests--inputs session) '("say once")))))))

(ert-deftest romacs-agent-submit-admission-survives-a-failing-view ()
  ;; A view that signals as the queue changes (`error' or `quit') neither
  ;; leaves an answer unsaid nor a message behind its answer: accepted is
  ;; one message waiting, refused none, and a retry adds none.
  (dolist (condition '(error quit))
    (romacs-agent-submit-admission-tests--with
     (lambda (session)
       (setf (romacs-agent-state session) 'running)
       (let* ((intent (romacs-agent-submit-admission-tests--intent "queue me" 'queue))
              (id (plist-get intent :draft-id))
              (armed t)
              (romacs-agent-event-functions
               (list (lambda (_session event)
                       (when (and armed (eq (plist-get event :type) 'changed)
                                  (> (romacs-agent-submit-admission-tests--queued session id) 0))
                         (setq armed nil)
                         (signal condition '("A view failed"))))))
              (answers (list nil)))
         (condition-case nil
             (romacs-agent-submit (romacs-agent-id session) intent (lambda (answer) (push answer (car answers))))
           (quit nil))
         (romacs-agent-submit-admission-tests--await (lambda () (car answers)))
         (romacs-agent-submit-admission-tests--settle)
         (should (= (length (car answers)) 1))
         (should (= (romacs-agent-submit-admission-tests--queued session id)
                    (if (eq (plist-get (car (car answers)) :status) 'accepted) 1 0)))
         (let ((retry (romacs-agent-submit-admission-tests--submit session intent)))
           (romacs-agent-submit-admission-tests--await (lambda () (car retry)))
           (should (eq (plist-get (car (car retry)) :status) 'accepted))
           (should (= (romacs-agent-submit-admission-tests--queued session id) 1))))))))

(ert-deftest romacs-agent-submit-admission-written-then-a-signal-is-accepted ()
  ;; A draft written before something after it signals (the next message
  ;; failing to go) went: accepted, and a retry sends nothing more.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (let* ((intent (romacs-agent-submit-admission-tests--intent))
            (dispatch-one (symbol-function 'romacs-agent--dispatch-one))
            (calls 0)
            (first (cl-letf (((symbol-function 'romacs-agent--dispatch-one)
                              (lambda (session)
                                (if (> (cl-incf calls) 1) (error "The next message failed")
                                  (funcall dispatch-one session)))))
                     (romacs-agent-submit-admission-tests--submit session intent))))
       (romacs-agent-submit-admission-tests--await (lambda () (car first)))
       (should (equal (romacs-agent-submit-admission-tests--inputs session) '("say once")))
       (should (eq (plist-get (car (car first)) :status) 'accepted))
       (let ((retry (romacs-agent-submit-admission-tests--submit session intent)))
         (romacs-agent-submit-admission-tests--await (lambda () (car retry)))
         (should (equal (romacs-agent-submit-admission-tests--inputs session) '("say once"))))))))

;;; B2: an edited retry under a sent draft's id

(ert-deftest romacs-agent-submit-admission-edited-retry-is-a-conflict ()
  ;; A draft accepted, its answer lost: its user edits it and sends it
  ;; again under its id.  The edit is not dropped: refused as a conflict,
  ;; kept in the composer under a new id, and sent by the next RET.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (romacs-agent-submit-admission-tests--chat
      session "say once"
      (lambda ()
        (let ((id (romacs-agent--uuid)))
          (setq romacs-agent-log--draft-id id)
          (romacs-agent-log-send)
          (romacs-agent-submit-admission-tests--answered)
          (should (equal (romacs-agent-submit-admission-tests--draft) ""))
          ;; Its answer lost: the composer still holds it, under its id.
          (setq romacs-agent-log--draft-id id)
          (insert "say once, edited")
          (romacs-agent-log-send)
          (romacs-agent-submit-admission-tests--answered)
          (should (equal (romacs-agent-submit-admission-tests--draft) "say once, edited"))
          (should-not (equal romacs-agent-log--draft-id id))
          (should (equal (romacs-agent-submit-admission-tests--inputs session) '("say once")))
          (romacs-agent-log-send)
          (romacs-agent-submit-admission-tests--answered)
          (should (equal (romacs-agent-submit-admission-tests--draft) ""))
          (should (equal (romacs-agent-submit-admission-tests--inputs session) '("say once" "say once, edited")))))))))

(ert-deftest romacs-agent-submit-admission-retry-under-another-kind-goes-once ()
  ;; A draft accepted, its answer lost, sent again under its id as
  ;; another kind (RET retried as C-c C-q): its text went already, so it
  ;; is answered as accepted, and goes once.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (romacs-agent-submit-admission-tests--chat
      session "say once"
      (lambda ()
        (let ((id (romacs-agent--uuid)))
          (setq romacs-agent-log--draft-id id)
          (romacs-agent-log-send)
          (romacs-agent-submit-admission-tests--answered)
          (setq romacs-agent-log--draft-id id)
          (insert "say once")
          (romacs-agent-log-queue)
          (romacs-agent-submit-admission-tests--answered)
          (should (equal (romacs-agent-submit-admission-tests--draft) ""))
          (should (equal (romacs-agent-submit-admission-tests--inputs session) '("say once")))))))))

;;; B3: a remote interrupt is one decision, the serving editor's

(ert-deftest romacs-agent-submit-admission-remote-interrupt-is-one-send ()
  ;; A remote C-c C-k with text is one `agents/send' of kind interrupt, its
  ;; answer the service's: never a second command, never an acceptance
  ;; the service did not give.
  (let ((session (romacs-agent--make :id "remote-interrupt" :harness 'claude :state 'running)))
    (dolist (text '("stop that"))
      (romacs-agent-submit-admission-tests--remote
       session text
       (lambda (sent)
         (let ((shown nil))
           (cl-letf (((symbol-function 'message)
                      (lambda (format &rest args) (when format (push (apply #'format-message format args) shown)))))
             (romacs-agent-log-interrupt-and-send)
             (should (equal (mapcar #'car (funcall sent)) '("agents/send")))
             (let ((params (nth 1 (car (funcall sent)))))
               (should (equal (gethash "mode" params) "interrupt"))
               (should (equal (gethash "text" params) text)))
             (funcall (nth 2 (car (funcall sent)))
                      nil (romacs-agent-rpc-object "message" "Nothing runs there"
                                                   "data" (romacs-agent-rpc-object "kind" "not-live")))
             (romacs-agent-submit-admission-tests--answered)
             (romacs-agent-submit-admission-tests--settle)
             (should (member "Nothing runs there" shown))
             (should (equal (romacs-agent-log--draft) text)))))))))

(ert-deftest romacs-agent-submit-admission-service-interrupt-goes-first-once ()
  ;; The serving editor takes an interrupt with text as one draft: its
  ;; message first of what waits, the turn interrupted once, a repeat
  ;; of its id under another cid neither.
  (romacs-agent-submit-admission-tests--serving
   (lambda (session connect ask)
     (setf (romacs-agent-state session) 'running)
     (romacs-agent-send-later session "already waiting")
     (let ((client (funcall connect "one")) (draft (romacs-agent--uuid)) (interrupts 0))
       (cl-letf (((symbol-function 'romacs-agent-interrupt) (lambda (_) (cl-incf interrupts))))
         (dolist (cid '("c1" "c2"))
           (let ((reply (funcall ask client "agents/send"
                                 (romacs-agent-submit-admission-tests--send (romacs-agent-id session) cid "stop that" "mode" "interrupt"
                                                                            "draftId" draft))))
             (should-not (cadr reply))
             (should (equal (gethash "state" (car reply)) "queued")))))
       (should (= interrupts 1))
       (should (equal (mapcar #'romacs-agent-item-text (romacs-agent-queue session)) '("stop that" "already waiting")))
       (should (romacs-agent-item-front (car (romacs-agent-queue session))))))))

;;; B4: a peer that never answers

(ert-deftest romacs-agent-submit-admission-remote-send-has-a-deadline ()
  ;; A service that never answers leaves the draft unconfirmed at the
  ;; deadline: writable again, under its id.  Its late reply touches no
  ;; later send of the draft.
  (let ((session (romacs-agent--make :id "remote-silent" :harness 'claude :state 'idle))
        (romacs-agent-remote-submit-deadline 0.2))
    (romacs-agent-submit-admission-tests--remote
     session "say once"
     (lambda (sent)
       (romacs-agent-log-send)
       (should romacs-agent-log--sending)
       (romacs-agent-submit-admission-tests--answered)
       (should (equal (romacs-agent-log--draft) "say once"))
       (let ((id romacs-agent-log--draft-id))
         (should id)
         (romacs-agent-log-send)
         (should (= (length (funcall sent)) 2))
         (should (equal (gethash "draftId" (nth 1 (nth 1 (funcall sent)))) id))
         ;; The first send's reply, late: the second waits on its own.
         (funcall (nth 2 (car (funcall sent))) (romacs-agent-rpc-object "state" "written") nil)
         (romacs-agent-submit-admission-tests--settle)
         (should (equal romacs-agent-log--sending id))
         (should (equal (romacs-agent-log--draft) "say once"))
         (funcall (nth 2 (nth 1 (funcall sent))) (romacs-agent-rpc-object "state" "written") nil)
         (romacs-agent-submit-admission-tests--answered)
         (should (equal (romacs-agent-log--draft) "")))))))

;;; B5: drafts are kept per caller, never by cid

(ert-deftest romacs-agent-submit-admission-callers-do-not-share-drafts ()
  ;; Two clients naming the same cid and no draft id send two messages.
  (romacs-agent-submit-admission-tests--serving
   (lambda (session connect ask)
     (dolist (instance '("one" "two"))
       (let ((reply (funcall ask (funcall connect instance) "agents/send" (romacs-agent-submit-admission-tests--send (romacs-agent-id session) "c1" "hi"))))
         (should-not (cadr reply))))
     (should (equal (romacs-agent-submit-admission-tests--inputs session) '("hi" "hi"))))))

;;; B6: the lock holds whatever the narrowing, and undo waits

(ert-deftest romacs-agent-submit-admission-lock-ignores-narrowing ()
  ;; Narrowed to part of the draft while it is sent, the chat is left with
  ;; nothing of it once it is accepted: no read-only tail.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (romacs-agent-submit-admission-tests--held
      session "say once, and a tail"
      (lambda (release)
        (romacs-agent-log-send)
        (narrow-to-region (point-min) (+ (point-min) 4))
        (funcall release)
        (widen)
        (should (equal (romacs-agent-submit-admission-tests--draft) ""))
        (should-not (text-property-not-all (point-min) (point-max) 'read-only nil))
        (insert "next")
        (should (equal (romacs-agent-submit-admission-tests--draft) "next")))))))

(ert-deftest romacs-agent-submit-admission-undo-leaves-a-sent-draft ()
  ;; Undo, its key in the composer, while the draft is sent changes
  ;; nothing: the composer is read-only, and refuses it; once the draft
  ;; is refused, undo works on it again.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (romacs-agent-submit-admission-tests--held
      session "say"
      (lambda (release)
        (undo-boundary)
        (insert " once")
        (undo-boundary)
        (romacs-agent-log-send)
        (should-error (romacs-agent-submit-admission-tests--undo) :type 'buffer-read-only)
        (should (equal (romacs-agent-submit-admission-tests--draft) "say once"))
        (should romacs-agent-log--sending)
        (funcall release 'refused)
        (romacs-agent-submit-admission-tests--undo)
        (should (equal (romacs-agent-submit-admission-tests--draft) "say")))))))

;;; N2: a draft id is a UUID

(ert-deftest romacs-agent-submit-admission-draft-id-is-a-uuid ()
  ;; A peer's draft id that is no UUID is refused, nothing sent: it names
  ;; the message in its session's records.
  (romacs-agent-submit-admission-tests--serving
   (lambda (session connect ask)
     (let ((reply (funcall ask (funcall connect "one") "agents/send"
                           (romacs-agent-submit-admission-tests--send (romacs-agent-id session) "c1" "hi" "draftId" "a\"b"))))
       (should (cadr reply))
       (should (equal (gethash "kind" (gethash "data" (cadr reply))) "bad-params"))
       (should-not (romacs-agent-submit-admission-tests--inputs session))))))

;;; N3: history while a draft is sent

(ert-deftest romacs-agent-submit-admission-history-waits-for-the-send ()
  ;; M-n, as a handoff's restore of the draft, while a recalled prompt is
  ;; sent: nothing changes, nothing signals.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (romacs-agent-submit-admission-tests--held
      session ""
      (lambda (release)
        (with-current-buffer romacs-agent-log--chat (setq-local romacs-agent-log--prompts (list "earlier prompt")))
        (romacs-agent-log-history-previous)
        (should (equal (romacs-agent-submit-admission-tests--draft) "earlier prompt"))
        (romacs-agent-log-send)
        (romacs-agent-log-history-next)
        (should (equal (romacs-agent-submit-admission-tests--draft) "earlier prompt"))
        (should romacs-agent-log--recalled)
        (funcall release)
        (should (equal (romacs-agent-submit-admission-tests--draft) "")))))))

;;; N4: an interrupt comes last

(ert-deftest romacs-agent-submit-admission-refused-interrupt-interrupts-nothing ()
  ;; An interrupt whose message cannot wait is refused, and the turn under
  ;; way goes on: interrupted only once nothing can fail.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (setf (romacs-agent-state session) 'running)
     (let ((interrupts 0))
       (cl-letf (((symbol-function 'romacs-agent-interrupt) (lambda (_) (cl-incf interrupts)))
                 ((symbol-function 'romacs-agent--enqueue-first) (lambda (&rest _) (error "The queue failed"))))
         (let ((answers (romacs-agent-submit-admission-tests--submit
                         session (romacs-agent-submit-admission-tests--intent "stop that" 'interrupt))))
           (romacs-agent-submit-admission-tests--await (lambda () (car answers)))
           (should (eq (plist-get (car (car answers)) :status) 'refused))
           (should (= interrupts 0))))))))

;;; A replayed answer says where its message is now

(ert-deftest romacs-agent-submit-admission-replay-after-the-queue ()
  ;; A queued draft's answer, given again once its message was written
  ;; and has left the queue, is the same answer, its place gone, and
  ;; nothing sent again.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (setf (romacs-agent-state session) 'running)
     (let* ((intent (romacs-agent-submit-admission-tests--intent "queue me" 'queue))
            (first (romacs-agent-submit-admission-tests--submit session intent)))
       (romacs-agent-submit-admission-tests--await (lambda () (car first)))
       (should (eql (plist-get (car (car first)) :queued) 1))
       ;; Its turn's end: the message waiting is written.
       (setf (romacs-agent-state session) 'idle)
       (romacs-agent--dispatch session)
       (should-not (romacs-agent-queue session))
       (setf (romacs-agent-state session) 'running)
       (let ((again (romacs-agent-submit-admission-tests--submit session intent)))
         (romacs-agent-submit-admission-tests--await (lambda () (car again)))
         (should (eq (plist-get (car (car again)) :status) 'accepted))
         (should-not (plist-member (car (car again)) :queued))
         (should-not (romacs-agent-queue session))
         (should (equal (romacs-agent-submit-admission-tests--inputs session) '("queue me"))))))))

;;; Round 4: one run of the serving editor, its peers and its folds

(ert-deftest romacs-agent-submit-admission-older-peer-interrupts-then-sends ()
  ;; A 1.0 service has no interrupt mode: C-c C-k with text is
  ;; `agents/interrupt', then a send of mode next, and an unanswered send
  ;; is said unconfirmed, nothing promised.
  (let ((session (romacs-agent--make :id "remote-old" :harness 'claude :state 'running))
        (romacs-agent-remote-submit-deadline 0.2))
    (romacs-agent-submit-admission-tests--remote
     session "stop that"
     (lambda (sent)
       (let ((shown nil))
         (cl-letf (((symbol-function 'message)
                    (lambda (format &rest args) (when format (push (apply #'format-message format args) shown)))))
           (romacs-agent-log-interrupt-and-send)
           (should (equal (mapcar #'car (funcall sent)) '("agents/interrupt" "agents/send")))
           (should (equal (gethash "mode" (nth 1 (nth 1 (funcall sent)))) "next"))
           (romacs-agent-submit-admission-tests--answered)
           (romacs-agent-submit-admission-tests--settle)
           (let ((said (seq-find (lambda (line) (string-search "unconfirmed" line)) shown)))
             (should said)
             (should-not (string-search "goes once" said))))))
     0)))

(ert-deftest romacs-agent-submit-admission-peers-sharing-a-draft-id-are-two-messages ()
  ;; Two callers naming one draft id send two messages, each its own queue
  ;; item, named apart: a queue is kept by session and item id.
  (romacs-agent-submit-admission-tests--serving
   (lambda (session connect ask)
     (setf (romacs-agent-state session) 'running)
     (let ((draft (romacs-agent--uuid)))
       (dolist (instance '("one" "two"))
         (should-not (cadr (funcall ask (funcall connect instance) "agents/send"
                                    (romacs-agent-submit-admission-tests--send
                                     (romacs-agent-id session) "c1" instance "mode" "next" "draftId" draft)))))
       (let ((queue (romacs-agent-queue session)))
         (should (equal (mapcar #'romacs-agent-item-text queue) '("one" "two")))
         (should (romacs-agent--uuid-p (romacs-agent-item-id (car queue))))
         (should-not (equal (romacs-agent-item-id (car queue)) (romacs-agent-item-id (cadr queue)))))))))

(ert-deftest romacs-agent-submit-admission-failed-fold-keeps-an-accepted-draft ()
  ;; A draft accepted while a worker folds its session is kept when the
  ;; fold fails: accepted, it is not dropped.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (let* ((id (romacs-agent-id session))
            (romacs-agent--folding (make-hash-table :test #'equal))
            (romacs-agent--fold-tries (make-hash-table :test #'equal))
            (romacs-agent--fold-parked (make-hash-table :test #'equal))
            (romacs-agent--fold-waiters (make-hash-table :test #'equal))
            (romacs-agent--fold-queue nil)
            (intent (romacs-agent-submit-admission-tests--intent "while folded" 'queue)))
       (puthash id (list session) romacs-agent--folding)
       (puthash id romacs-agent--fold-retries romacs-agent--fold-tries)
       (let ((answers (romacs-agent-submit-admission-tests--submit session intent)))
         (romacs-agent-submit-admission-tests--await (lambda () (car answers)))
         (should (eq (plist-get (car (car answers)) :status) 'accepted)))
       (let* ((said nil)
              (romacs-agent-event-functions
               (list (lambda (_session event)
                       (when (eq (plist-get event :type) 'error) (setq said (plist-get event :text)))))))
         (cl-letf (((symbol-function 'romacs-agent--restore-kept) #'ignore))
           (romacs-agent--folded-one session nil "the worker failed"))
         ;; Said to wait, not dropped.
         (should (string-search "1 message(s) of its user wait" said)))
       (should (= (romacs-agent-submit-admission-tests--queued session (plist-get intent :draft-id)) 1))))))

(ert-deftest romacs-agent-submit-admission-pending-draft-outlives-others ()
  ;; A draft being admitted while 256 others are accepted is known still:
  ;; sent again, it is answered, not sent again.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (let* ((intent (romacs-agent-submit-admission-tests--intent "the first"))
            (draft (plist-get intent :draft-id))
            (crowded nil)
            (romacs-agent-input-functions
             (list (lambda (&rest _)
                     (unless crowded
                       (setq crowded t)
                       (dotimes (n 256)
                         (romacs-agent-submit (romacs-agent-id session)
                                              (romacs-agent-submit-admission-tests--intent (format "other %d" n) 'queue)
                                              #'ignore))))))
            (first (romacs-agent-submit-admission-tests--submit session intent)))
       (romacs-agent-submit-admission-tests--await (lambda () (car first)))
       (let ((again (romacs-agent-submit-admission-tests--submit session intent)))
         (romacs-agent-submit-admission-tests--await (lambda () (car again)))
         (should (eq (plist-get (car (car again)) :status) 'accepted)))
       (should (= (romacs-agent-submit-admission-tests--queued session draft) 0))
       (should (equal (seq-filter (lambda (text) (equal text "the first"))
                                  (romacs-agent-submit-admission-tests--inputs session))
                      '("the first")))))))

(ert-deftest romacs-agent-submit-admission-quit-after-the-write-is-a-write ()
  ;; A quit once the line is written (as the write is committed) does not
  ;; make it unwritten: the draft is sent once, however often it comes.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (let* ((intent (romacs-agent-submit-admission-tests--intent))
            (written nil)
            (under-way (symbol-function 'romacs-agent--turn-under-way-p)))
       (cl-letf (((symbol-function 'romacs-agent--line)
                  (lambda (&rest args)
                    (apply #'romacs-agent-submit-admission-tests--record args)
                    (setq written t)))
                 ((symbol-function 'romacs-agent--turn-under-way-p)
                  (lambda (session)
                    (if (not written) (funcall under-way session)
                      (setq written nil)
                      (signal 'quit nil)))))
         (dotimes (_ 2)
           (let ((answers (list nil)))
             (condition-case nil
                 (romacs-agent-submit (romacs-agent-id session) intent (lambda (answer) (push answer (car answers))))
               (quit nil))
             (romacs-agent-submit-admission-tests--await (lambda () (car answers))))))
       (should (equal (romacs-agent-submit-admission-tests--inputs session) '("say once")))))))

(ert-deftest romacs-agent-submit-admission-empty-interrupt-is-an-interrupt ()
  ;; C-c C-k with nothing typed only interrupts, as `agents/interrupt' to
  ;; a service; a submit always has something to send.
  (let ((session (romacs-agent--make :id "remote-empty" :harness 'claude :state 'running)))
    (romacs-agent-submit-admission-tests--remote
     session ""
     (lambda (sent)
       (romacs-agent-log-interrupt-and-send)
       (should (equal (mapcar #'car (funcall sent)) '("agents/interrupt"))))))
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (let ((answers (romacs-agent-submit-admission-tests--submit
                     session (romacs-agent-submit-admission-tests--intent "" 'interrupt))))
       (romacs-agent-submit-admission-tests--await (lambda () (car answers)))
       (should (eq (plist-get (car (car answers)) :reason) 'invalid))))))

(ert-deftest romacs-agent-submit-admission-ended-harness-is-closed-whatever-its-state ()
  ;; A session whose harness is gone, though its records still say idle,
  ;; refuses a draft as closed: said where the refusal is made.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (delete-process (romacs-agent-connection session))
     (should (eq (romacs-agent-state session) 'idle))
     (let ((answers (romacs-agent-submit-admission-tests--submit session (romacs-agent-submit-admission-tests--intent))))
       (romacs-agent-submit-admission-tests--await (lambda () (car answers)))
       (should (eq (plist-get (car (car answers)) :reason) 'closed))))))

(ert-deftest romacs-agent-submit-admission-uuid-is-strict ()
  (should (romacs-agent--uuid-p (romacs-agent--uuid)))
  (should-not (romacs-agent--uuid-p "01234567----------------------------"))
  (should-not (romacs-agent--uuid-p "01234567-89ab-cdef-0123-456789abcdeff")))

(ert-deftest romacs-agent-submit-admission-remote-answer-says-its-place ()
  ;; The service says a queued draft's place, and a remote chat's answer
  ;; carries it.
  (romacs-agent-submit-admission-tests--serving
   (lambda (session connect ask)
     (setf (romacs-agent-state session) 'running)
     (let ((reply (funcall ask (funcall connect "one") "agents/send"
                           (romacs-agent-submit-admission-tests--send
                            (romacs-agent-id session) "c1" "later" "mode" "next" "draftId" (romacs-agent--uuid)))))
       (should (equal (gethash "state" (car reply)) "queued"))
       (should (eql (gethash "queued" (car reply)) 1)))))
  (let ((session (romacs-agent--make :id "remote-place" :harness 'claude :state 'running)) (got nil))
    (romacs-agent-submit-admission-tests--remote
     session ""
     (lambda (sent)
       (romacs-agent-log-source-submit (buffer-local-value 'romacs-agent-log--source romacs-agent-log--chat) session
                                       (romacs-agent-submit-admission-tests--intent "later" 'queue)
                                       (lambda (answer) (setq got answer)))
       (funcall (nth 2 (car (funcall sent))) (romacs-agent-rpc-object "state" "queued" "queued" 2) nil)
       (romacs-agent-submit-admission-tests--await (lambda () got))
       (should (eql (plist-get got :queued) 2))))))

(ert-deftest romacs-agent-submit-admission-deadline-lets-the-reply-go ()
  ;; At the deadline the client no longer waits on the send's reply.
  (let* ((client (romacs-agent-submit-admission-tests--client 1))
         (source (romacs-agent-remote--make :client client))
         (session (romacs-agent--make :id "remote-gone" :harness 'claude :state 'idle))
         (romacs-agent-remote-submit-deadline 0.1) (got nil))
    (cl-letf (((symbol-function 'romacs-agent-client-live-p) (lambda (_) t))
              ((symbol-function 'romacs-agent-rpc-send) #'ignore))
      (romacs-agent-log-source-submit source session (romacs-agent-submit-admission-tests--intent)
                                      (lambda (answer) (setq got answer)))
      (should (= (hash-table-count (romacs-agent-client-pending client)) 1))
      (romacs-agent-submit-admission-tests--await (lambda () got))
      (should (eq (plist-get got :status) 'unconfirmed))
      (should (= (hash-table-count (romacs-agent-client-pending client)) 0)))))

(ert-deftest romacs-agent-submit-admission-cleared-composer-takes-a-new-id ()
  ;; A refused draft keeps its id; emptied, the next draft takes another.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (romacs-agent-submit-admission-tests--chat
      session "first"
      (lambda ()
        (let ((ids nil))
          (cl-letf (((symbol-function 'romacs-agent-submit)
                     (lambda (_id intent callback)
                       (push (plist-get intent :draft-id) ids)
                       (romacs-agent-submit-answer
                        callback (romacs-agent-submit-refused intent 'owner-unavailable "Not now")))))
            (romacs-agent-log-send)
            (romacs-agent-submit-admission-tests--answered)
            (romacs-agent-log-send)
            (romacs-agent-submit-admission-tests--answered)
            (should (equal (car ids) (cadr ids)))
            (delete-region (point-min) (point-max))
            (insert "second")
            (romacs-agent-log-send)
            (romacs-agent-submit-admission-tests--answered)
            (should (= (length ids) 3))
            (should-not (equal (car ids) (cadr ids))))))))))

;;; Round 5: recall, per-caller answers, the welcome, views, refusals

(defun romacs-agent-submit-admission-tests--refusing (body)
  "Call BODY with `romacs-agent-submit' answering each draft refused, as
unconfirmed, and a function returning the draft ids it was given, the
newest first."
  (let ((ids nil))
    (cl-letf (((symbol-function 'romacs-agent-submit)
               (lambda (_id intent callback)
                 (push (plist-get intent :draft-id) ids)
                 (romacs-agent-submit-answer
                  callback (romacs-agent-submit-refused intent 'owner-unavailable "Not now")))))
      (funcall body (lambda () ids)))))

(ert-deftest romacs-agent-submit-admission-recall-keeps-the-draft-id ()
  ;; M-p then M-n give back an unconfirmed draft under its id: sent
  ;; again, it is the same draft.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (romacs-agent-submit-admission-tests--chat
      session "first"
      (lambda ()
        (with-current-buffer romacs-agent-log--chat (setq-local romacs-agent-log--prompts (list "earlier prompt")))
        (romacs-agent-submit-admission-tests--refusing
         (lambda (ids)
           (romacs-agent-log-send)
           (romacs-agent-submit-admission-tests--answered)
           (romacs-agent-log-history-previous)
           (romacs-agent-log-history-next)
           (should (equal (romacs-agent-submit-admission-tests--draft) "first"))
           (romacs-agent-log-send)
           (romacs-agent-submit-admission-tests--answered)
           (should (equal (car (funcall ids)) (cadr (funcall ids)))))))))))

(ert-deftest romacs-agent-submit-admission-undone-kill-keeps-the-draft-id ()
  ;; A draft killed, then the kill undone, is the same draft again.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (romacs-agent-submit-admission-tests--chat
      session "first"
      (lambda ()
        (romacs-agent-submit-admission-tests--refusing
         (lambda (ids)
           (romacs-agent-log-send)
           (romacs-agent-submit-admission-tests--answered)
           (undo-boundary)
           (delete-region (point-min) (point-max))
           (undo-boundary)
           (let ((last-command nil)) (undo))
           (should (equal (romacs-agent-submit-admission-tests--draft) "first"))
           (romacs-agent-log-send)
           (romacs-agent-submit-admission-tests--answered)
           (should (equal (car (funcall ids)) (cadr (funcall ids)))))))))))

(defun romacs-agent-submit-admission-tests--crowd (session scope n)
  "Submit N drafts of SCOPE's, to wait in SESSION's queue, and wait for them."
  (let ((answered 0))
    (dotimes (i n)
      (romacs-agent--submit-draft scope (romacs-agent-id session)
                                  (romacs-agent-submit-admission-tests--intent (format "%s %d" scope i) 'queue)
                                  (lambda (_) (cl-incf answered))))
    (romacs-agent-submit-admission-tests--await (lambda () (= answered n)))))

(ert-deftest romacs-agent-submit-admission-no-draft-is-pushed-out ()
  ;; A written draft is known however many callers' drafts follow it: a
  ;; thousand of them, its retry is answered, not sent again.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (let ((intent (romacs-agent-submit-admission-tests--intent "from a")) (answers nil))
       (romacs-agent--submit-draft "a" (romacs-agent-id session) intent (lambda (answer) (push answer answers)))
       (romacs-agent-submit-admission-tests--await (lambda () answers))
       (let ((answered 0))
         (dotimes (n 1000)
           (romacs-agent--submit-draft (format "b%d" n) (romacs-agent-id session)
                                       (romacs-agent-submit-admission-tests--intent (format "from b%d" n))
                                       (lambda (_) (cl-incf answered))))
         ;; A busy host takes seconds to answer a thousand drafts.
         (romacs-agent-submit-admission-tests--await (lambda () (= answered 1000)) 60))
       (romacs-agent--submit-draft "a" (romacs-agent-id session) intent (lambda (answer) (push answer answers)))
       (romacs-agent-submit-admission-tests--await (lambda () (cdr answers)))
       (should (eq (plist-get (car answers) :status) 'accepted))
       (should (equal (seq-filter (lambda (text) (equal text "from a"))
                                  (romacs-agent-submit-admission-tests--inputs session))
                      '("from a")))))))

(ert-deftest romacs-agent-submit-admission-a-waiting-draft-keeps-its-dedupe ()
  ;; A draft waiting in the queue is answered again by its place there,
  ;; however many answers were kept since.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (setf (romacs-agent-state session) 'running)
     (let ((intent (romacs-agent-submit-admission-tests--intent "from a" 'queue)) (answers nil))
       (romacs-agent--submit-draft "a" (romacs-agent-id session) intent (lambda (answer) (push answer answers)))
       (romacs-agent-submit-admission-tests--await (lambda () answers))
       (romacs-agent-submit-admission-tests--crowd session "a" 256)
       (romacs-agent--submit-draft "a" (romacs-agent-id session) intent (lambda (answer) (push answer answers)))
       (romacs-agent-submit-admission-tests--await (lambda () (cdr answers)))
       (should (eq (plist-get (car answers) :status) 'accepted))
       (should (eql (plist-get (car answers) :queued) 1))
       (should (= (cl-count "from a" (romacs-agent-queue session) :key #'romacs-agent-item-text :test #'equal) 1))))))

(ert-deftest romacs-agent-submit-admission-views-hear-of-an-answered-draft ()
  ;; A view told the session changed never finds its draft still being
  ;; admitted: its answer is kept first.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (let* ((intent (romacs-agent-submit-admission-tests--intent))
            (seen nil)
            (romacs-agent-event-functions
             (list (lambda (_session event)
                     (when (eq (plist-get event :type) 'changed)
                       (push (if (romacs-agent-submit-admission-tests--pending-p (plist-get intent :draft-id))
                                 'pending 'answered)
                             seen)))))
            (answers (romacs-agent-submit-admission-tests--submit session intent)))
       (romacs-agent-submit-admission-tests--await (lambda () (car answers)))
       (should seen)
       (should-not (memq 'pending seen))))))

(ert-deftest romacs-agent-submit-admission-ended-session-is-a-refusal-to-mcp ()
  ;; An ended session's refusal stays a `user-error': a tool call that
  ;; meets it is refused, said plainly, not failed.
  (let ((session (romacs-agent--make :id "ended" :harness 'claude :state 'ended :title "done"))
        (refused nil) (said nil))
    (should (equal (should-error (romacs-agent--message-ended session) :type 'user-error)
                   '(user-error "Session done has ended: resume it (r) to go on")))
    (cl-letf (((symbol-function 'romacs-mcp--available-tools)
               (lambda () (list (list "ended_tool" :schema (romacs-mcp--object "type" "object")
                                      :handler (lambda (_) (romacs-agent--message-ended session))))))
              (romacs-mcp-failure-functions
               (list (lambda (_peer _tool _arguments message reason) (setq refused reason said message)))))
      (romacs-mcp--call (romacs-mcp--object "name" "ended_tool" "arguments" (romacs-mcp--object)) nil))
    (should refused)
    (should (equal said "Session done has ended: resume it (r) to go on"))))

(ert-deftest romacs-agent-submit-admission-older-peer-interrupt-is-let-go ()
  ;; At the deadline a 1.0 service's send stops waiting on its interrupt
  ;; too.
  (let* ((client (romacs-agent-submit-admission-tests--client 0))
         (source (romacs-agent-remote--make :client client))
         (session (romacs-agent--make :id "remote-old-gone" :harness 'claude :state 'running))
         (romacs-agent-remote-submit-deadline 0.1) (got nil))
    (cl-letf (((symbol-function 'romacs-agent-client-live-p) (lambda (_) t))
              ((symbol-function 'romacs-agent-rpc-send) #'ignore))
      (romacs-agent-log-source-submit source session (romacs-agent-submit-admission-tests--intent "stop that" 'interrupt)
                                      (lambda (answer) (setq got answer)))
      (should (= (hash-table-count (romacs-agent-client-pending client)) 2))
      (romacs-agent-submit-admission-tests--await (lambda () got))
      (should (= (hash-table-count (romacs-agent-client-pending client)) 0)))))

;;; Round 6: the client sent on, live drafts everywhere, callers, one refusal

(ert-deftest romacs-agent-submit-admission-before-the-welcome-is-refused-at-once ()
  ;; Sent before its service's welcome, a draft is refused at once, still
  ;; connecting; it stays under its id, and goes once the welcome came.
  (let* ((client (romacs-agent-client--make))
         (source (romacs-agent-remote--make :client client))
         (session (romacs-agent--make :id "remote-early" :harness 'claude :state 'running))
         (got nil) (sent nil))
    (cl-letf (((symbol-function 'romacs-agent-client-live-p) (lambda (_) t))
              ((symbol-function 'romacs-agent-client-command)
               (lambda (_client method params _callback) (push (cons method params) sent))))
      (romacs-agent-log-source-submit source session (romacs-agent-submit-admission-tests--intent "stop that" 'interrupt)
                                      (lambda (answer) (setq got answer)))
      (romacs-agent-submit-admission-tests--settle)
      (should got)
      (should (eq (plist-get got :reason) 'owner-unavailable))
      (should (string-search "still connecting" (plist-get got :message)))
      (should-not sent)))
  (let ((session (romacs-agent--make :id "remote-early-chat" :harness 'claude :state 'idle)))
    (romacs-agent-submit-admission-tests--remote
     session "say once"
     (lambda (sent)
       (setf (romacs-agent-client-server (romacs-agent-remote-client (buffer-local-value 'romacs-agent-log--source romacs-agent-log--chat))) nil)
       (romacs-agent-log-send)
       (romacs-agent-submit-admission-tests--answered)
       (should-not (funcall sent))
       (should (equal (romacs-agent-log--draft) "say once"))
       (let ((id romacs-agent-log--draft-id))
         (should id)
         (setf (romacs-agent-client-server (romacs-agent-remote-client (buffer-local-value 'romacs-agent-log--source romacs-agent-log--chat)))
               (romacs-agent-rpc-object "version" [1 1]))
         (romacs-agent-log-send)
         (should (equal (gethash "draftId" (nth 1 (car (funcall sent)))) id)))))))

(ert-deftest romacs-agent-submit-admission-a-live-draft-conflicts-across-sessions ()
  ;; A draft id whose message waits in one session's queue, its answer
  ;; long pushed out, is a conflict sent to another session.
  (romacs-agent-submit-admission-tests--with
   (lambda (session other)
     (let ((intent (romacs-agent-submit-admission-tests--intent "here" 'queue)) (answers nil))
       (setf (romacs-agent-state session) 'running)
       (romacs-agent--submit-draft "a" (romacs-agent-id session) intent (lambda (answer) (push answer answers)))
       (romacs-agent-submit-admission-tests--await (lambda () answers))
       (romacs-agent-submit-admission-tests--crowd session "a" 256)
       (romacs-agent--submit-draft "a" (romacs-agent-id other) intent (lambda (answer) (push answer answers)))
       (romacs-agent-submit-admission-tests--await (lambda () (cdr answers)))
       (should (eq (plist-get (car answers) :reason) 'conflict))
       (should-not (romacs-agent-submit-admission-tests--inputs other))
       (should-not (romacs-agent-queue other))))
   1))

(ert-deftest romacs-agent-submit-admission-older-peer-send-settles-both ()
  ;; A 1.0 service's send answered, its client waits on no reply of it:
  ;; the interrupt's let go too.
  (let* ((client (romacs-agent-submit-admission-tests--client 0))
         (source (romacs-agent-remote--make :client client))
         (session (romacs-agent--make :id "remote-old-settled" :harness 'claude :state 'running))
         (got nil))
    (cl-letf (((symbol-function 'romacs-agent-client-live-p) (lambda (_) t))
              ((symbol-function 'romacs-agent-rpc-send) #'ignore))
      (romacs-agent-log-source-submit source session (romacs-agent-submit-admission-tests--intent "stop that" 'interrupt)
                                      (lambda (answer) (setq got answer)))
      (should (= (hash-table-count (romacs-agent-client-pending client)) 2))
      (romacs-agent-client--receive
       client (romacs-agent-rpc-object "id" (apply #'max (hash-table-keys (romacs-agent-client-pending client)))
                                       "result" (romacs-agent-rpc-object "state" "queued")))
      (romacs-agent-submit-admission-tests--await (lambda () got))
      (should (eq (plist-get got :status) 'accepted))
      (should (= (hash-table-count (romacs-agent-client-pending client)) 0)))))

(ert-deftest romacs-agent-submit-admission-narrowed-emptying-keeps-the-draft-id ()
  ;; Deleting what a narrowing shows of a draft leaves the rest, and the
  ;; draft keeps its id.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (romacs-agent-submit-admission-tests--chat
      session "first"
      (lambda ()
        (romacs-agent-submit-admission-tests--refusing
         (lambda (ids)
           (romacs-agent-log-send)
           (romacs-agent-submit-admission-tests--answered)
           (narrow-to-region (point-min) (+ (point-min) 2))
           (delete-region (point-min) (point-max))
           (widen)
           (should (equal (romacs-agent-submit-admission-tests--draft) "rst"))
           (romacs-agent-log-send)
           (romacs-agent-submit-admission-tests--answered)
           (should (equal (car (funcall ids)) (cadr (funcall ids)))))))))))

;;; The reshaped admission: one index, one hold, one predicate

(ert-deftest romacs-agent-submit-admission-ended-is-closed-before-dispatch ()
  ;; A session whose harness ended is answered closed before anything is
  ;; dispatched: nothing queued, nothing written.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (setf (romacs-agent-state session) 'ended)
     (let ((admitted 0))
       (cl-letf* ((admit (symbol-function 'romacs-agent--admit))
                  ((symbol-function 'romacs-agent--admit)
                   (lambda (&rest args) (cl-incf admitted) (apply admit args))))
         (let ((answers (romacs-agent-submit-admission-tests--submit session (romacs-agent-submit-admission-tests--intent))))
           (romacs-agent-submit-admission-tests--await (lambda () (car answers)))
           (should (eq (plist-get (car (car answers)) :reason) 'closed))))
       (should (= admitted 0))
       (should-not (romacs-agent-queue session))
       (should-not (romacs-agent-submit-admission-tests--inputs session))))))

;;; One critical section, one exit, no observer inside

(ert-deftest romacs-agent-submit-admission-nested-admission-holds-the-outer-views ()
  ;; A's write admits a draft to B, and B's queues a message for A: A's
  ;; views hear nothing until A's draft is answered.
  (romacs-agent-submit-admission-tests--with
   (lambda (a b)
     (let* ((intent (romacs-agent-submit-admission-tests--intent "to a"))
            (seen nil) (done nil)
            (romacs-agent-event-functions
             (list (lambda (session event)
                     (when (and (eq session a) (eq (plist-get event :type) 'changed))
                       (push (if (romacs-agent-submit-admission-tests--pending-p (plist-get intent :draft-id))
                                 'pending 'answered)
                             seen))))))
       (romacs-agent-submit-admission-tests--on-write
        (lambda (session)
          (unless (memq session done)
            (push session done)
            (cond ((eq session a)
                   (romacs-agent-submit (romacs-agent-id b) (romacs-agent-submit-admission-tests--intent "to b") #'ignore))
                  ((eq session b) (romacs-agent-send-later a "for a later")))))
        (lambda ()
          (let ((answers (romacs-agent-submit-admission-tests--submit a intent)))
            (romacs-agent-submit-admission-tests--await (lambda () (car answers))))))
       (should (memq b done))
       (should seen)
       (should-not (memq 'pending seen))))
   1))

(ert-deftest romacs-agent-submit-admission-other-sessions-wait-for-the-answer ()
  ;; Another session's changes while a draft is admitted (its process's
  ;; output read as the draft's line is written) are held, and told once,
  ;; after the answer.
  (romacs-agent-submit-admission-tests--with
   (lambda (session other)
     (let* ((told 0) (during nil)
            (romacs-agent-event-functions
             (list (lambda (changed event)
                     (when (and (eq changed other) (eq (plist-get event :type) 'changed)) (cl-incf told))))))
       (romacs-agent-submit-admission-tests--on-write
        (lambda (_)
          (unless during
            (romacs-agent--changed other)
            (romacs-agent--changed other)
            (setq during told)))
        (lambda ()
          (let ((answers (romacs-agent-submit-admission-tests--submit session (romacs-agent-submit-admission-tests--intent))))
            (romacs-agent-submit-admission-tests--await (lambda () (car answers))))))
       (should (eql during 0))
       (should (= told 1))))
   1))

(ert-deftest romacs-agent-submit-admission-written-is-said-before-any-hook ()
  ;; A queued draft written by a later dispatch is written in its index
  ;; before any hook runs: a hook that clears the queue leaves it known,
  ;; and its retry goes nowhere.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (setf (romacs-agent-state session) 'running)
     (let* ((intent (romacs-agent-submit-admission-tests--intent "once" 'queue))
            (cleared nil)
            (first (romacs-agent-submit-admission-tests--submit session intent)))
       (romacs-agent-submit-admission-tests--await (lambda () (car first)))
       (let ((romacs-agent-input-functions
              (list (lambda (&rest _) (unless cleared (setq cleared t) (romacs-agent--clear-queue session))))))
         (setf (romacs-agent-state session) 'idle)
         (romacs-agent--dispatch session))
       (should cleared)
       (setf (romacs-agent-state session) 'running)
       (let ((again (romacs-agent-submit-admission-tests--submit session intent)))
         (romacs-agent-submit-admission-tests--await (lambda () (car again)))
         (should (eq (plist-get (car (car again)) :status) 'accepted)))
       (should-not (romacs-agent-queue session))
       (should (equal (romacs-agent-submit-admission-tests--inputs session) '("once")))))))

(ert-deftest romacs-agent-submit-admission-hooks-run-once-it-is-answered ()
  ;; A hook of a message written while a draft is admitted runs once the
  ;; draft is answered: one that drops the draft finds it queued, its
  ;; answer kept, not one still being admitted.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (setf (romacs-agent-state session) 'running)
     (romacs-agent-send-later session "older")
     (setf (romacs-agent-state session) 'idle)
     (let* ((intent (romacs-agent-submit-admission-tests--intent "newer" 'queue))
            (draft-id (plist-get intent :draft-id))
            (at-hook 'none)
            (romacs-agent-input-functions
             (list (lambda (&rest _)
                     (when (eq at-hook 'none)
                       (setq at-hook (if (romacs-agent-submit-admission-tests--pending-p draft-id) 'pending 'answered))
                       (when-let* ((item (cl-find draft-id (romacs-agent-queue session)
                                                  :key #'romacs-agent-item-id :test #'equal)))
                         (romacs-agent-queue-drop session item))))))
            (answers (romacs-agent-submit-admission-tests--submit session intent)))
       (romacs-agent-submit-admission-tests--await (lambda () (car answers)))
       (should (eq at-hook 'answered))
       (should (eq (plist-get (car (car answers)) :status) 'accepted))
       (should-not (gethash draft-id romacs-agent--submit-drafts))))))

(ert-deftest romacs-agent-submit-admission-an-input-hook-never-sees-it-pending ()
  ;; A draft written at once: its input hook finds its answer kept.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (let* ((intent (romacs-agent-submit-admission-tests--intent))
            (at-hook 'none)
            (romacs-agent-input-functions
             (list (lambda (&rest _)
                     (setq at-hook (if (romacs-agent-submit-admission-tests--pending-p (plist-get intent :draft-id))
                                       'pending (car-safe (gethash (plist-get intent :draft-id) romacs-agent--submit-drafts)))))))
            (answers (romacs-agent-submit-admission-tests--submit session intent)))
       (romacs-agent-submit-admission-tests--await (lambda () (car answers)))
       (should (eq at-hook 'written))))))

(ert-deftest romacs-agent-submit-admission-a-quit-anywhere-answers-once ()
  ;; A quit before the admission's work leaves no reservation: it and a
  ;; repeat waiting on it are answered once, nothing sent, and a retry
  ;; is a fresh attempt.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (let* ((intent (romacs-agent-submit-admission-tests--intent "quit"))
            (first (list nil)) (repeat (list nil)) (once t)
            (advice (lambda (&rest _)
                      (when once
                        (setq once nil)
                        (romacs-agent-submit (romacs-agent-id session) intent (lambda (a) (push a (car repeat))))
                        (signal 'quit nil)))))
       (advice-add 'romacs-agent--submit-run :before advice)
       (unwind-protect
           (condition-case nil
               (romacs-agent-submit (romacs-agent-id session) intent (lambda (a) (push a (car first))))
             (quit nil))
         (advice-remove 'romacs-agent--submit-run advice))
       (romacs-agent-submit-admission-tests--await (lambda () (and (car first) (car repeat))))
       (romacs-agent-submit-admission-tests--settle)
       (should (= (length (car first)) 1))
       (should (= (length (car repeat)) 1))
       (should (eq (plist-get (car (car first)) :status) 'refused))
       (should-not (gethash (plist-get intent :draft-id) romacs-agent--submit-drafts))
       (should-not (romacs-agent-submit-admission-tests--inputs session))
       (let ((retry (romacs-agent-submit-admission-tests--submit session intent)))
         (romacs-agent-submit-admission-tests--await (lambda () (car retry)))
         (should (eq (plist-get (car (car retry)) :status) 'accepted)))
       (should (equal (romacs-agent-submit-admission-tests--inputs session) '("quit")))))))

(ert-deftest romacs-agent-submit-admission-unanswered-remote-send-is-unconfirmed ()
  ;; A remote send whose reply never comes is unconfirmed at the deadline:
  ;; not refused, the draft kept under its id, said so.
  (let ((session (romacs-agent--make :id "remote-unanswered" :harness 'claude :state 'idle))
        (romacs-agent-remote-submit-deadline 0.2) (shown nil) (got nil))
    (romacs-agent-submit-admission-tests--remote
     session "say once"
     (lambda (_sent)
       (romacs-agent-log-source-submit (buffer-local-value 'romacs-agent-log--source romacs-agent-log--chat) session
                                       (romacs-agent-submit-admission-tests--intent "say once")
                                       (lambda (answer) (setq got answer)))
       (romacs-agent-submit-admission-tests--await (lambda () got))
       (should (eq (plist-get got :status) 'unconfirmed))
       (cl-letf (((symbol-function 'message)
                  (lambda (format &rest args) (when format (push (apply #'format-message format args) shown)))))
         (romacs-agent-log-send)
         (let ((id romacs-agent-log--draft-id))
           (romacs-agent-submit-admission-tests--answered)
           (should (equal (romacs-agent-log--draft) "say once"))
           (should (equal romacs-agent-log--draft-id id))
           (should (seq-find (lambda (line) (string-search "Unconfirmed: send again, it goes once" line)) shown))))))))

(ert-deftest romacs-agent-submit-admission-an-edited-queued-draft-is-that-draft ()
  ;; A queued draft its user edits in the queue is the edited draft: its
  ;; retry with the new text is answered again, nothing sent.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (setf (romacs-agent-state session) 'running)
     (let* ((intent (romacs-agent-submit-admission-tests--intent "first text" 'queue))
            (first (romacs-agent-submit-admission-tests--submit session intent)))
       (romacs-agent-submit-admission-tests--await (lambda () (car first)))
       (should (romacs-agent-queue-edit session (car (romacs-agent-queue session)) "edited text"))
       (let ((again (romacs-agent-submit-admission-tests--submit
                     session (plist-put (copy-sequence intent) :text "edited text"))))
         (romacs-agent-submit-admission-tests--await (lambda () (car again)))
         (should (eq (plist-get (car (car again)) :status) 'accepted))
         (should (eql (plist-get (car (car again)) :queued) 1)))
       (should (= (length (romacs-agent-queue session)) 1))))))

;;; One commit point, answers before hooks, one guard

(defun romacs-agent-submit-admission-tests--quitting (session intent)
  "Submit INTENT to SESSION, a quit going on through the call: its answers."
  (let ((answers (list nil)))
    (condition-case nil
        (romacs-agent-submit (romacs-agent-id session) intent (lambda (answer) (push answer (car answers))))
      (quit nil))
    (romacs-agent-submit-admission-tests--settle)
    (romacs-agent-submit-admission-tests--settle)
    (car answers)))

(ert-deftest romacs-agent-submit-admission-a-quit-in-the-send-sends-at-most-once ()
  ;; C-g as the line goes: the draft may have gone, so it is unconfirmed
  ;; and goes no second time, its retry answered the same.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (let ((intent (romacs-agent-submit-admission-tests--intent "cut short")) (once t))
       (cl-letf (((symbol-function 'romacs-agent--line)
                  (lambda (&rest args)
                    (apply #'romacs-agent-submit-admission-tests--record args)
                    (when once (setq once nil) (signal 'quit nil)))))
         (should (eq (plist-get (car (romacs-agent-submit-admission-tests--quitting session intent)) :status)
                     'unconfirmed))
         (setf (romacs-agent-state session) 'idle)
         (should (eq (plist-get (car (romacs-agent-submit-admission-tests--quitting session intent)) :status)
                     'unconfirmed)))
       (should-not (romacs-agent-queue session))
       (should (equal (romacs-agent-submit-admission-tests--inputs session) '("cut short")))))))

(ert-deftest romacs-agent-submit-admission-a-quit-after-the-send-restores-nothing ()
  ;; A quit held back over the commit fires after it: the message stays
  ;; written, out of the queue, never sent again.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (let ((intent (romacs-agent-submit-admission-tests--intent "late quit")) (once t)
           (commit (symbol-function 'romacs-agent--draft-written)))
       (cl-letf (((symbol-function 'romacs-agent--draft-written)
                  (lambda (id) (funcall commit id) (when (and id once) (setq once nil) (signal 'quit nil)))))
         (should (eq (plist-get (car (romacs-agent-submit-admission-tests--quitting session intent)) :status)
                     'accepted)))
       (should-not (romacs-agent-queue session))
       (setf (romacs-agent-state session) 'idle)
       (romacs-agent--dispatch session)
       (should (equal (romacs-agent-submit-admission-tests--inputs session) '("late quit")))))))

(ert-deftest romacs-agent-submit-admission-dropped-as-it-is-chosen-is-not-sent ()
  ;; A queued draft let go while the dispatcher decides (a hold predicate
  ;; waiting, a timer dropping it) is not written; its retry is a fresh
  ;; attempt, written once.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (setf (romacs-agent-state session) 'running)
     (let* ((intent (romacs-agent-submit-admission-tests--intent "dropped" 'queue))
            (first (romacs-agent-submit-admission-tests--submit session intent))
            (dropped nil))
       (romacs-agent-submit-admission-tests--await (lambda () (car first)))
       (let ((romacs-agent-hold-functions
              (list (lambda (_)
                      (unless dropped
                        (setq dropped t)
                        (romacs-agent-queue-drop session (car (romacs-agent-queue session))))
                      nil))))
         (setf (romacs-agent-state session) 'idle)
         (romacs-agent--dispatch session))
       (should dropped)
       (should-not (romacs-agent-submit-admission-tests--inputs session))
       (let ((again (romacs-agent-submit-admission-tests--submit session intent)))
         (romacs-agent-submit-admission-tests--await (lambda () (car again))))
       (should (equal (romacs-agent-submit-admission-tests--inputs session) '("dropped")))))))

(ert-deftest romacs-agent-submit-admission-answers-go-out-before-hooks ()
  ;; Hooks held over an admission run after its submit has returned and
  ;; its answer has gone out, whatever they wait on.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (let* ((returned nil) (answers (list nil)) (at-hook 'none)
            (romacs-agent-input-functions
             (list (lambda (&rest _)
                     (when (eq at-hook 'none)
                       (setq at-hook (list returned (car answers)))
                       (accept-process-output nil 0.05))))))
       (romacs-agent-submit (romacs-agent-id session) (romacs-agent-submit-admission-tests--intent)
                            (lambda (answer) (push answer (car answers))))
       (setq returned t)
       (romacs-agent-submit-admission-tests--await (lambda () (not (eq at-hook 'none))))
       (should (car at-hook))
       (should (cadr at-hook))
       (should (= (length (car answers)) 1))))))

(ert-deftest romacs-agent-submit-admission-a-quit-finding-the-session-is-answered ()
  ;; A quit as its session is looked up, loaded from its directory: the
  ;; submit is still answered, once.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (cl-letf (((symbol-function 'romacs-agent-get) (lambda (&rest _) (signal 'quit nil))))
       (let ((answers (romacs-agent-submit-admission-tests--quitting session (romacs-agent-submit-admission-tests--intent))))
         (should (= (length answers) 1))
         (should (eq (plist-get (car answers) :status) 'refused)))))))

(ert-deftest romacs-agent-submit-admission-an-unknown-result-is-unconfirmed ()
  ;; A remote result that says no state the draft took (a command kept
  ;; before its outcome) is unconfirmed, never accepted.
  (let ((intent (romacs-agent-submit-admission-tests--intent)))
    (should (eq (plist-get (romacs-agent-remote--submit-answer intent (romacs-agent-rpc-object "state" "unknown") nil)
                           :status)
                'unconfirmed))
    (should (eq (plist-get (romacs-agent-remote--submit-answer intent (romacs-agent-rpc-object "state" "written") nil)
                           :status)
                'accepted))))

;;; Each answer its own timer, unconfirmed on the wire, one hook runner

(ert-deftest romacs-agent-submit-admission-each-answer-goes-alone ()
  ;; Two drafts answered by one release: the first one's callback throws
  ;; out, and the second is still answered, and its composer unlocks.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (setf (romacs-agent-state session) 'running)
     (romacs-agent-submit-admission-tests--chat
      session "composer draft"
      (lambda ()
        (let ((tag (make-symbol "first-answer")) (first nil))
          (romacs-agent-submit (romacs-agent-id session) (romacs-agent-submit-admission-tests--intent "a" 'queue)
                               (lambda (answer) (throw tag answer)))
          (romacs-agent-log-queue)
          (should romacs-agent-log--sending)
          ;; Its caller waits until the throw comes.
          (setq first (catch tag
                        (let ((deadline (+ (float-time) 3)))
                          (while (< (float-time) deadline) (accept-process-output nil 0.02)))))
          (should (eq (plist-get first :status) 'accepted))
          (romacs-agent-submit-admission-tests--answered)
          (should (equal (romacs-agent-submit-admission-tests--draft) ""))
          (should (equal (mapcar #'romacs-agent-item-text (romacs-agent-queue session)) '("a" "composer draft")))))))))

(ert-deftest romacs-agent-submit-admission-a-cut-send-is-unconfirmed-over-the-wire ()
  ;; A draft whose line is cut short by a quit, sent through the service:
  ;; its result is unconfirmed, which the remote chat keeps, never taken.
  (romacs-agent-submit-admission-tests--serving
   (lambda (session connect _ask)
     (let* ((client (funcall connect "cut"))
            (source (romacs-agent-remote--make :client client))
            (intent (romacs-agent-submit-admission-tests--intent "cut short"))
            (got nil))
       (cl-letf (((symbol-function 'romacs-agent--line) (lambda (&rest _) (signal 'quit nil))))
         (romacs-agent-log-source-submit source session intent (lambda (answer) (setq got answer)))
         (romacs-agent-submit-admission-tests--await (lambda () got)))
       (should (eq (plist-get got :status) 'unconfirmed))
       (should-not (romacs-agent-queue session))))))

(ert-deftest romacs-agent-submit-admission-a-room-hook-waits-for-the-answer ()
  ;; An instance's room coming back while a draft is admitted (its
  ;; harness's output read as the draft's line is written) tells its
  ;; hook once the draft is answered, never while it is pending.
  (romacs-agent-submit-admission-tests--with
   (lambda (session)
     (let* ((now (float-time))
            (romacs-agent--room (make-hash-table :test #'equal))
            (romacs-agent--room-loaded t)
            (romacs-agent-room-check-interval nil)
            (intent (romacs-agent-submit-admission-tests--intent "pending"))
            (seen 'none) (noted nil)
            (romacs-agent-room-functions
             (list (lambda (_)
                     (setq seen (if (romacs-agent-submit-admission-tests--pending-p (plist-get intent :draft-id))
                                    'pending 'answered))))))
       (puthash "room-test" (list :status 'rejected :time (1- now) :resets (+ now 600)) romacs-agent--room)
       (cl-letf (((symbol-function 'romacs-agent--keep-room) #'ignore))
         (romacs-agent-submit-admission-tests--on-write
          (lambda (_)
            (unless noted
              (setq noted t)
              (romacs-agent-note-room "room-test" (list :type 'rate-limit :status 'allowed :window "five_hour"
                                                         :used 0.3 :resets (+ now 600) :time now))))
          (lambda ()
            (let ((answers (romacs-agent-submit-admission-tests--submit session intent)))
              (romacs-agent-submit-admission-tests--await (lambda () (car answers))))
            (romacs-agent-submit-admission-tests--await (lambda () (not (eq seen 'none)))))))
       (should noted)
       (should (eq seen 'answered))))))

(defconst romacs-agent-submit-admission-tests--direct-hooks
  '(;; They decide what is dispatched, or their value is needed at once.
    romacs-agent-hold-functions romacs-agent-requested-functions romacs-agent-writer-held-live-functions
    ;; Machinery whose work its caller goes on from: a session loaded
    ;; whole (its kept queue), a restore's setup, the store's tables.
    romacs-agent-loaded-functions romacs-agent-restored-hook romacs-agent-store-made-functions
    ;; Run below the agents module, which they do not load: the store's
    ;; writer process, the protocol client, the MCP server, the question
    ;; popup and the glyph table (a user's customization).
    romacs-agent-writer-start-functions romacs-agent-writer-held-start-functions
    romacs-agent-writer-gave-up-functions romacs-agent-client-follow-functions
    romacs-agent-client-rows-functions romacs-agent-client-session-functions romacs-mcp-peer-close-functions
    romacs-agent-ask-sent-functions romacs-agent-glyph-alist-changed-hook
    ;; A composer's own buffer-local hooks, run with its own edit.
    romacs-agent-log-draft-replaced-hook romacs-agent-log-attach-tray-functions)
  "The hooks of lisp/agents/ run directly, not through `romacs-agent--run-hook'.")

(ert-deftest romacs-agent-submit-admission-every-hook-goes-through-the-runner ()
  ;; Every hook lisp/agents/ runs goes through the one runner, which holds
  ;; while drafts are admitted, but those named as run directly.
  (let* ((dir (file-name-directory (locate-library "romacs-agent.el")))
         (stray nil))
    (dolist (file (directory-files dir t "\\`romacs-\\(?:agent\\|mcp\\).*\\.el\\'"))
      (with-temp-buffer
        (insert-file-contents file)
        (goto-char (point-min))
        (while (re-search-forward "\\(#'\\|(\\)run-hook\\(?:s\\|-with-args\\(?:-until-\\(?:success\\|failure\\)\\)?\\)\\_>" nil t)
          (let ((quoted (equal (match-string 1) "#'"))
                (line (line-number-at-pos))
                (hook (and (looking-at "[ \t\n]*'\\([^ \t\n()]+\\)") (intern (match-string 1))))
                (in (save-excursion (and (re-search-backward "^(def[a-z*-]+ \\([^ \t\n]+\\)" nil t) (match-string 1)))))
            (unless (if quoted (equal in "romacs-agent--run-hook")
                      (memq hook romacs-agent-submit-admission-tests--direct-hooks))
              (push (format "%s:%d %s" (file-name-nondirectory file) line (or hook in)) stray))))))
    (should-not stray)))

(provide 'romacs-agent-submit-admission-tests)
;;; romacs-agent-submit-admission-tests.el ends here
