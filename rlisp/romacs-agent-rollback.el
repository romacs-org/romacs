;;; romacs-agent-rollback.el --- A session back to before one of its turns  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A session goes back to just before one of its prompts, as T3 Code's
;; rollback does: its harness's conversation (each driver's :rollback) and,
;; when asked, its project's files.  The journal keeps everything: a
;; rollback note marks the turns gone (`romacs-agent--live-records'), and
;; its chat dims them.
;;
;; Files come back from the prompt's start checkpoint (romacs-agent-diff.el),
;; through a private index: the user's index, stash and refs are never
;; touched.  Only the files the turns rolled back changed come back, and a
;; file something else changed meanwhile (between those turns, or since
;; the last) is restored only when the caller says so.

;;; Code:

(require 'romacs-agent)
(require 'romacs-agent-diff)

(defun romacs-agent-rollback--git (root index &rest args)
  "Git ARGS's output in ROOT, through the private INDEX when non-nil;
an error when git fails."
  (with-temp-buffer
    (let ((default-directory (file-name-as-directory root))
          (process-environment (append (and index (list (concat "GIT_INDEX_FILE=" index)))
                                       romacs-agent-git-variables process-environment)))
      (unless (zerop (apply #'call-process "git" nil t nil "--no-pager" args))
        (error "git %s: %s" (car args) (string-trim (buffer-string))))
      (buffer-string))))

(defun romacs-agent-rollback--changed (root from to)
  "The files that differ between trees FROM and TO of the project at ROOT."
  (if (equal from to) nil
    (split-string (romacs-agent-rollback--git root nil "diff" "--name-only" "--no-renames" "-z" from to) "\0" t)))

(defun romacs-agent-rollback--files (checkpoints turns now)
  "What going back to before the first of TURNS (request ids, oldest
first) does to the files, from CHECKPOINTS (`romacs-agent-diff--checkpoints')
and NOW, the project's tree now: (:root ROOT :start TREE :touched NAMES
:clobbered NAMES), the files the turns changed and those of them changed
by anything else, between them or since.  Nil when the first turn has
no start checkpoint."
  (let ((edges (make-hash-table :test #'equal)))
    ;; Each turn's edges by (TURN . KIND), once: no scan per turn.
    (dolist (c checkpoints) (unless (gethash (cons (car c) (nth 1 c)) edges)
                              (puthash (cons (car c) (nth 1 c)) c edges)))
   (cl-flet ((edge (turn kind) (nth 2 (gethash (cons turn kind) edges))))
    (when-let* ((first (gethash (cons (car turns) 'start) edges)))
      (let ((root (nth 3 first)) (touched nil) (outside nil) (tail turns))
        (while tail
          (let* ((start (edge (car tail) 'start))
                 (next (and (cdr tail) (edge (cadr tail) 'start)))
                 ;; A turn its harness cut short has no end: the next start's, else now.
                 (end (or (edge (car tail) 'end) next now)))
            (when start (setq touched (append (romacs-agent-rollback--changed root start end) touched)))
            (setq outside (append (romacs-agent-rollback--changed root end (or next now)) outside))
            (setq tail (cdr tail))))
        (setq touched (delete-dups touched))
        (let ((elsewhere (make-hash-table :test #'equal)))
          (dolist (name outside) (puthash name t elsewhere))
          (list :root root :start (nth 2 first) :touched touched
                :clobbered (seq-filter (lambda (name) (gethash name elsewhere)) touched))))))))

(defun romacs-agent-rollback--restore (session plan)
  "Put PLAN's touched files back as they were in its start tree: through a
private index in SESSION's directory; a file that tree has not is deleted."
  (let* ((root (plist-get plan :root)) (tree (plist-get plan :start))
         (names (plist-get plan :touched))
         (index (make-temp-name (expand-file-name "rollback-index-" (romacs-agent-dir session))))
         (kept (and names (split-string (apply #'romacs-agent-rollback--git root nil "ls-tree" "-r" "-z" "--name-only"
                                                tree "--" names)
                                        "\0" t)))
         (delete-by-moving-to-trash nil))
    (unwind-protect
        (when kept
          (romacs-agent-rollback--git root index "read-tree" tree)
          (apply #'romacs-agent-rollback--git root index "checkout-index" "-f" "--" kept))
      (when (file-exists-p index) (delete-file index)))
    (dolist (name names)
      (unless (member name kept)
        (let ((file (expand-file-name name root)))
          (when (or (file-exists-p file) (file-symlink-p file)) (delete-file file)))))
    names))

(defun romacs-agent-rollback--turns (records request)
  "The prompts from REQUEST's on in RECORDS, request ids oldest first, and
REQUEST's own record: (REQUESTS . RECORD)."
  (let ((tail (cl-member request records :key #'romacs-agent--record-request :test #'equal)))
    (cons (delq nil (mapcar #'romacs-agent--record-request tail)) (car tail))))

(defun romacs-agent--prompt-text (session record)
  "The text of RECORD, a prompt SESSION's harness was written."
  (plist-get (cl-find 'input (funcall (or (romacs-agent--harness-get session :input-events) #'ignore)
                                      session (gethash "in" record))
                      :key (lambda (e) (plist-get e :type)))
             :text))

(defun romacs-agent-rollback (session request then &rest options)
  "Take SESSION back to just before its prompt REQUEST: its harness's
conversation, and with OPTIONS' :files its project's files as they were
then.  Call THEN with the outcome: (:done N) N turns back; (:clobbered
NAMES), nothing done, when a file to restore was changed by something
else meanwhile and OPTIONS' :force is nil; (:error MESSAGE).  Refused at
once (`user-error') while the session is busy, or for a prompt that is no
longer its conversation's, or one with no checkpoint when :files."
  (unless (romacs-agent--harness-get session :rollback)
    (user-error "%s cannot roll a session back" (romacs-agent--harness-get session :display)))
  (unless (and (romacs-agent--harness-free-p session) (not (romacs-agent-interrupted session)))
    (user-error "Session %s is busy: roll it back when it is idle" (romacs-agent-title session)))
  (when (romacs-agent-rollback--holder session)
    (user-error "Session %s is rolling back already" (romacs-agent-title session)))
  (let* ((records (romacs-agent--live-records (romacs-agent--journal-records session)))
         (turns (romacs-agent-rollback--turns records request)))
    (unless (cdr turns) (user-error "That prompt is no longer this session's conversation"))
    ;; With no snapshot pending, the checkpoints are all there: refuse now.
    ;; Else the start may still be coming: it is looked for behind them.
    (when (and (plist-get options :files) (romacs-agent-diff--settled-p session)
               (not (seq-find (lambda (c) (and (equal (car c) request) (eq (nth 1 c) 'start)))
                              (romacs-agent-diff--checkpoints session))))
      (user-error "No checkpoint of this turn's files (no git project, or a turn from before checkpoints)"))
    ;; Held from here to the end: messages sent meanwhile wait in its queue
    ;; (`romacs-agent-send'), so none is accepted and then rolled back.
    (let* ((token (list request))
           (then (lambda (outcome)
                   (unless (eq (car outcome) :done) (romacs-agent-rollback--release session token))
                   (funcall then outcome)))
           (commit (lambda (plan) (romacs-agent-rollback--commit session token records request (car turns)
                                                                 (romacs-agent--prompt-text session (cdr turns))
                                                                 plan then))))
      (romacs-agent-rollback--hold session token)
      (if (not (plist-get options :files))
          (funcall commit nil)
        (romacs-agent-diff--snapshot
         session request 'now
         (lambda (now _root)
           (condition-case err
               ;; Read now, behind every snapshot queued before this one:
               ;; the last turn's end checkpoint among them.
               (let ((plan (and now (romacs-agent-rollback--files (romacs-agent-diff--checkpoints session)
                                                                  (car turns) now))))
                 (cond ((not now) (funcall then (list :error "Could not read the project now")))
                       ((not (equal (length (romacs-agent--live-records (romacs-agent--journal-records session)))
                                    (length records)))
                        (funcall then (list :error "The session moved on meanwhile: roll it back again")))
                       ((not plan) (funcall then (list :error "No checkpoint of this turn's files")))
                       ((and (plist-get plan :clobbered) (not (plist-get options :force)))
                        (funcall then (list :clobbered (plist-get plan :clobbered))))
                       (t (funcall commit plan))))
             (error (funcall then (list :error (error-message-string err)))))))))))

(defun romacs-agent-rollback--holder (session)
  "The token of the rollback holding SESSION, or nil."
  (plist-get (romacs-agent-driver-state session) :rolling-back))

(defun romacs-agent-rollback--hold (session token)
  "Hold SESSION for the rollback TOKEN names."
  (setf (romacs-agent-driver-state session)
        (plist-put (romacs-agent-driver-state session) :rolling-back token)))

(defun romacs-agent-rollback--release (session token &optional quiet)
  "End TOKEN's hold on SESSION, if it still holds it: what waited goes,
unless QUIET."
  (when (eq (romacs-agent-rollback--holder session) token)
    (romacs-agent-rollback--hold session nil)
    (when (and (not quiet) (romacs-agent-queue session)) (romacs-agent--send-next session nil))))

(defun romacs-agent-rollback--await (session token match then)
  "Call THEN with SESSION's first event MATCH accepts, once: while SESSION
is still rolling back under TOKEN, else never (its run ended, or the
rollback failed).  The waiter goes either way."
  (letrec ((waiter (lambda (seen event)
                     (cond ((not (eq (plist-get (romacs-agent-driver-state session) :rolling-back) token))
                            (remove-hook 'romacs-agent-event-functions waiter))
                           ((and (eq seen session) (funcall match event))
                            (remove-hook 'romacs-agent-event-functions waiter)
                            (funcall then event))))))
    (add-hook 'romacs-agent-event-functions waiter)))

(defun romacs-agent-rollback--commit (session token records request turns prompt plan then)
  "Roll SESSION back to before REQUEST, the first of TURNS, whose text is
PROMPT, and PLAN's files when given, under the hold TOKEN; THEN as
`romacs-agent-rollback' says.
In this order: the files (the step likeliest to fail, and safe to take
again); the harness's own rollback, answered; the journal's note, the
rollback's commit point; a relaunch, which reads the note.  A failure
before the note leaves the conversation as it was, to try again."
  (let* ((fail (lambda (why) (funcall then (list :error why))))
         (how nil))
    (cl-labels
        ((noted (_event)
           ;; A relaunch sends what waited once its harness is attached.
           (romacs-agent-rollback--release session token (eq (car-safe how) 'relaunch))
           (condition-case err
               (progn (when (eq (car-safe how) 'relaunch)
                        (when (cadr how) (funcall (cadr how)))
                        (romacs-agent-rollback--relaunch session))
                      (funcall then (list :done (length turns))))
             (error (funcall then (list :error (error-message-string err))))))
         (note ()
           (romacs-agent-rollback--await
            session token
            (lambda (event) (and (eq (plist-get event :type) 'rolled-back) (equal (plist-get event :to) request)))
            #'noted)
           (condition-case err
               (romacs-agent--note session (romacs-mcp--object "type" "rollback" "to" request "turns" (length turns)
                                                               "prompt" (or prompt "") "files" (if plan t :false)))
             (error (funcall fail (error-message-string err))))))
      (condition-case err
          (progn
            (setq how (funcall (romacs-agent--harness-get session :rollback) session records request))
            (when plan (romacs-agent-rollback--restore session plan))
            (if (eq (car-safe how) 'relaunch)
                (note)
              ;; The harness says when it has rolled back, or why not.
              (romacs-agent-rollback--await session token (lambda (event) (eq (plist-get event :type) 'reverted))
                                            (lambda (event)
                                              (if (plist-get event :error) (funcall fail (plist-get event :error)) (note))))
              (romacs-agent--line session how)))
        (error (funcall fail (error-message-string err)))))))

(defun romacs-agent-rollback--relaunch (session)
  "Run SESSION's harness anew, its conversation where its journal now has
it: stopped first when it runs."
  (if (not (process-live-p (romacs-agent-connection session)))
      (romacs-agent-resume session)
    (romacs-agent--stop-then session (lambda () (romacs-agent-rollback--resume-when-gone session 50)))))

(defun romacs-agent-rollback--resume-when-gone (session tries)
  "Resume SESSION once its connection to the stopped supervisor is gone,
looking again for TRIES tenths of a second."
  (if (and (process-live-p (romacs-agent-connection session)) (> tries 0))
      (run-at-time 0.1 nil #'romacs-agent-rollback--resume-when-gone session (1- tries))
    (romacs-agent-resume session)))

(provide 'romacs-agent-rollback)
;;; romacs-agent-rollback.el ends here
