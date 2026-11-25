;;; romacs-agent-watch-bytes-tests.el --- Bytes that are not UTF-8 on their way to a task -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A unit's log can hold bytes that are not UTF-8 (a test printing raw
;; bytes).  A watch notice made of it is text that JSON encodes: each bad
;; byte becomes U+FFFD, the text around it kept.  Text that cannot be
;; encoded never makes a task unreachable: put in its queue by any route,
;; it is refused at the call or dropped and reported as failed, and the
;; next message goes.  Writes are caught below the encoding, on the test
;; attachment, so the line written is the harness's line.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-tools)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-watch))

;;; Helpers

(defun romacs-agent-watch-bytes-tests--raw-p (text)
  "Whether TEXT holds a raw byte (an eight-bit character)."
  (and (cl-some (lambda (char) (>= char #x3FFF80)) text) t))

(defun romacs-agent-watch-bytes-tests--encodes-p (text)
  "Whether TEXT is a JSON string value."
  (condition-case nil (progn (json-serialize (vector text)) t) (error nil)))

(defun romacs-agent-watch-bytes-tests--file (file &rest parts)
  "Write PARTS (unibyte strings) to FILE as bytes; return FILE."
  (let ((coding-system-for-write 'no-conversion))
    (with-temp-file file
      (set-buffer-multibyte nil)
      (dolist (part parts) (insert part))))
  file)

(defun romacs-agent-watch-bytes-tests--notice (file)
  "The notice a task is told when a watched unit ends, its log FILE."
  (romacs-agent-watch--message (list :unit "check.service" :output file) "exited, status=0 (success)"))

;;; 1. A notice from a log that is not UTF-8 is text that encodes

(ert-deftest romacs-agent-watch-bytes-notice-replaces-a-raw-byte ()
  (let ((log (make-temp-file "romacs-watch-bytes-")))
    (unwind-protect
        (let ((text (romacs-agent-watch-bytes-tests--notice
                     (romacs-agent-watch-bytes-tests--file log "ok\n" (unibyte-string ?b ?a ?d ?\s #o351 ?x) " end\n"))))
          (should-not (romacs-agent-watch-bytes-tests--raw-p text))
          (should (romacs-agent-watch-bytes-tests--encodes-p text))
          ;; One bad byte, one U+FFFD; the readable text around it kept.
          (should (string-search (string ?b ?a ?d ?\s #xFFFD ?x ?\s ?e ?n ?d) text))
          (should (string-search "ok\n" text)))
      (delete-file log))))

(ert-deftest romacs-agent-watch-bytes-notice-replaces-a-lone-continuation-byte ()
  (let ((log (make-temp-file "romacs-watch-bytes-")))
    (unwind-protect
        (let ((text (romacs-agent-watch-bytes-tests--notice
                     (romacs-agent-watch-bytes-tests--file log (unibyte-string ?a #x80 ?b) "\n"))))
          (should-not (romacs-agent-watch-bytes-tests--raw-p text))
          (should (romacs-agent-watch-bytes-tests--encodes-p text))
          (should (string-suffix-p (string ?a #xFFFD ?b) text)))
      (delete-file log))))

(ert-deftest romacs-agent-watch-bytes-notice-replaces-a-character-cut-at-the-end ()
  ;; A writer killed in the middle of a character: its first bytes end the log.
  (let ((log (make-temp-file "romacs-watch-bytes-")))
    (unwind-protect
        (let ((text (romacs-agent-watch-bytes-tests--notice
                     (romacs-agent-watch-bytes-tests--file log "done\n" (unibyte-string ?t ?a ?i ?l ?\s #xE4 #xB8)))))
          (should-not (romacs-agent-watch-bytes-tests--raw-p text))
          (should (romacs-agent-watch-bytes-tests--encodes-p text))
          (should (string-search "done\ntail " text))
          (should (eq (aref text (1- (length text))) #xFFFD)))
      (delete-file log))))

(ert-deftest romacs-agent-watch-bytes-notice-replaces-a-character-cut-by-the-tail ()
  ;; The notice reads the log's last 16384 bytes: a cut inside a
  ;; character leaves its last bytes first, on a line the notice shows.
  (let* ((log (make-temp-file "romacs-watch-bytes-"))
         (line (encode-coding-string (make-string 6000 ?中) 'utf-8))
         (size (+ (length line) (length "\nend\n")))
         (cut (- size 16384)))
    ;; The cut falls inside a character, on the only line before the last.
    (should (/= (% cut 3) 0))
    (unwind-protect
        (let ((text (romacs-agent-watch-bytes-tests--notice
                     (romacs-agent-watch-bytes-tests--file log line "\nend\n"))))
          (should-not (romacs-agent-watch-bytes-tests--raw-p text))
          (should (romacs-agent-watch-bytes-tests--encodes-p text))
          (should (string-search (string #xFFFD ?中) text))
          (should (string-suffix-p "中中\nend" text)))
      (delete-file log))))

(ert-deftest romacs-agent-watch-bytes-file-entries-replace-raw-bytes ()
  ;; A watched file's new entries are told as they read.
  (let ((index (make-temp-file "romacs-watch-bytes-" nil ".md")))
    (unwind-protect
        (let ((entries (romacs-agent-watch--entries
                        (romacs-agent-watch-bytes-tests--file
                         index "# Held\n- 333 c " (unibyte-string #o351) " held\n")
                        "Held")))
          (should (= (length entries) 1))
          (let ((line (cdar entries)))
            (should-not (romacs-agent-watch-bytes-tests--raw-p line))
            (should (romacs-agent-watch-bytes-tests--encodes-p line))
            (should (equal line (string ?- ?\s ?3 ?3 ?3 ?\s ?c ?\s #xFFFD ?\s ?h ?e ?l ?d)))))
      (delete-file index))))

;;; 2. Text that cannot be encoded never makes a task unreachable

(defconst romacs-agent-watch-bytes-tests--bad
  (string-to-multibyte (unibyte-string ?b ?a ?d ?\s #o351 ?x))
  "Text no harness line can carry: a raw byte, as a log read as UTF-8 gives it.")

(defmacro romacs-agent-watch-bytes-tests--with (&rest body)
  "BODY with `lead' and its task `child', claude sessions attached to
pipes nothing reads, ready and idle, in a private directory `root'.
`caller' is the session calling tools (`lead').  `lines' collects each
line written on an attachment, `errors' each error event raised on a
session as (SESSION . TEXT)."
  (declare (indent 0))
  `(let* ((root (make-temp-file "romacs-watch-bytes-" t))
          (process-environment (copy-sequence process-environment))
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent--children (make-hash-table :test #'equal))
          (romacs-agent-watch--watches (make-hash-table :test #'equal))
          (romacs-agent-watch--timer nil)

          (romacs-agent-tools--copies (make-hash-table :test #'equal))
          (lead (romacs-agent--make :id "lead" :harness 'claude :state 'idle :title "lead" :cwd root
                                    :dir (expand-file-name "lead" root)))
          (child (romacs-agent--make :id "child" :harness 'claude :state 'idle :title "lane" :cwd root
                                     :dir (expand-file-name "child" root) :options '(:parent "lead")))
          (caller "lead")
          (lines nil) (errors nil)
          (real-send (symbol-function 'process-send-string))
          (romacs-agent-event-functions
           (cons (lambda (session event)
                   (when (eq (plist-get event :type) 'error)
                     (push (cons session (plist-get event :text)) errors)))
                 romacs-agent-event-functions)))
     (setenv "XDG_RUNTIME_DIR" root)
     (dolist (session (list lead child))
       (make-directory (romacs-agent-dir session))
       (puthash (romacs-agent-id session) session romacs-agent--sessions)
       ;; Attached and caught up, its driver ready, as a live harness leaves it
       ;; (`romacs-agent-test-attach' in romacs-agent-test-attached.el).
       (setf (romacs-agent-connection session) (make-pipe-process :name "romacs-agent-watch-bytes" :noquery t)
             (romacs-agent-caught-up session) t
             (romacs-agent-driver-state session) (list :ready t))
       (process-put (romacs-agent-connection session) 'romacs-agent-watch-bytes-tests t))
     (romacs-agent--index child)
     (cl-letf (((symbol-function 'romacs-agent-tools--caller) (lambda () caller))
               ((symbol-function 'romacs-agent--fold-now) #'ignore)
               ((symbol-function 'romacs-agent--continue) #'identity)
               ((symbol-function 'romacs-agent-interrupt) #'ignore)
               ((symbol-function 'run-at-time) (lambda (&rest _) (timer-create)))
               ((symbol-function 'process-send-string)
                (lambda (process string)
                  (if (process-get process 'romacs-agent-watch-bytes-tests)
                      (push (cons process string) lines)
                    (funcall real-send process string)))))
       (unwind-protect (progn ,@body)
         (dolist (session (list lead child))
           (delete-process (romacs-agent-connection session)))
         (delete-directory root t)))))

(defun romacs-agent-watch-bytes-tests--written (lines session)
  "What LINES wrote to SESSION's harness, the harness's lines joined."
  (mapconcat (lambda (line)
               (gethash "write" (json-parse-string (cdr line))))
             (cl-remove-if-not (lambda (line) (eq (car line) (romacs-agent-connection session))) lines)
             "\n"))

(defun romacs-agent-watch-bytes-tests--turn-ends (session)
  "SESSION's turn ends: what waits goes, as the dispatch after a turn's
end does, from a timer (an error there is the timer's, so caught here)."
  (setf (romacs-agent-state session) 'idle)
  (condition-case nil (romacs-agent--dispatch session) (error nil)))

(defun romacs-agent-watch-bytes-tests--send (task text)
  "romacs_task_send TEXT to TASK, queued, as its caller calls it."
  (romacs-agent-tools--send (romacs-mcp--object "taskId" (romacs-agent-id task) "message" text "mode" "queue")))

(defmacro romacs-agent-watch-bytes-tests--reachable (session lines errors &rest route)
  "Run ROUTE, which puts text no harness line carries in SESSION's queue,
then check SESSION is still reachable: ROUTE was refused (`user-error'),
or its item dropped and an error reported on SESSION (ERRORS); nothing
that cannot be encoded waits; the next romacs_task_send to it is written
(LINES), at once or at its turn's end."
  (declare (indent 3))
  `(let ((refused (condition-case err (progn ,@route nil)
                    (user-error err))))
     (romacs-agent-watch-bytes-tests--turn-ends ,session)
     (should (romacs-agent-watch-bytes-tests--send ,session "the next message"))
     (romacs-agent-watch-bytes-tests--turn-ends ,session)
     (should (string-search "the next message" (romacs-agent-watch-bytes-tests--written ,lines ,session)))
     (should-not (romacs-agent-queue ,session))
     (should (or refused (assq ,session ,errors)))))

(defun romacs-agent-watch-bytes-tests--log (root)
  "A unit's log in ROOT whose last line holds a raw byte; its file name."
  (romacs-agent-watch-bytes-tests--file (expand-file-name "check.log" root)
                                        "started\n" (unibyte-string ?b ?a ?d ?\s #o351 ?x) "\n"))

(ert-deftest romacs-agent-watch-bytes-notice-to-a-busy-task-is-delivered ()
  ;; The case reported: a unit-ended notice from a log holding a raw
  ;; byte, queued while the task's turn runs, goes at its end; the task
  ;; is told, and a message after it goes too.
  (romacs-agent-watch-bytes-tests--with
    (let ((watch (list :unit "check.service" :output (romacs-agent-watch-bytes-tests--log root))))
      (puthash "child" (list watch) romacs-agent-watch--watches)
      (setf (romacs-agent-state child) 'running)
      (romacs-agent-watch--report "child" watch)
      (romacs-agent-watch-bytes-tests--turn-ends child)
      (should (string-search "Unit check.service ended" (romacs-agent-watch-bytes-tests--written lines child)))
      (should-not (gethash "child" romacs-agent-watch--watches))
      (should (romacs-agent-watch-bytes-tests--send child "the next message"))
      (romacs-agent-watch-bytes-tests--turn-ends child)
      (should (string-search "the next message" (romacs-agent-watch-bytes-tests--written lines child)))
      (should-not (romacs-agent-queue child)))))

(ert-deftest romacs-agent-watch-bytes-notice-to-an-idle-task-is-delivered ()
  ;; Idle, the task is told at once, and the watch retired.
  (romacs-agent-watch-bytes-tests--with
    (let ((watch (list :unit "check.service" :output (romacs-agent-watch-bytes-tests--log root))))
      (puthash "child" (list watch) romacs-agent-watch--watches)
      (condition-case nil (romacs-agent-watch--report "child" watch) (error nil))
      (should (string-search "Unit check.service ended" (romacs-agent-watch-bytes-tests--written lines child)))
      (should-not (gethash "child" romacs-agent-watch--watches))
      (should-not (romacs-agent-queue child)))))

(ert-deftest romacs-agent-watch-bytes-file-notice-to-a-busy-task-is-delivered ()
  ;; A watched file's new entry holding a raw byte: told at the turn's end.
  (romacs-agent-watch-bytes-tests--with
    (let* ((index (romacs-agent-watch-bytes-tests--file (expand-file-name "INDEX.md" root) "# Held\n"))
           (watch (list :file index :section "Held" :seen nil)))
      (puthash "child" (list watch) romacs-agent-watch--watches)
      (romacs-agent-watch-bytes-tests--file index "# Held\n- 333 c " (unibyte-string #o351) " held\n")
      (setf (romacs-agent-state child) 'running)
      (romacs-agent-watch--tick)
      (romacs-agent-watch-bytes-tests--turn-ends child)
      (should (string-search "333 c" (romacs-agent-watch-bytes-tests--written lines child)))
      (should (romacs-agent-watch-bytes-tests--send child "the next message"))
      (romacs-agent-watch-bytes-tests--turn-ends child)
      (should (string-search "the next message" (romacs-agent-watch-bytes-tests--written lines child)))
      (should-not (romacs-agent-queue child)))))

(ert-deftest romacs-agent-watch-bytes-task-send-to-a-busy-task-keeps-it-reachable ()
  ;; romacs_task_send's text from a Lisp caller (MCP's JSON carries none:
  ;; its parser refuses such bytes), queued behind a running turn.
  (romacs-agent-watch-bytes-tests--with
    (setf (romacs-agent-state child) 'running)
    (romacs-agent-watch-bytes-tests--reachable child lines errors
      (romacs-agent-watch-bytes-tests--send child romacs-agent-watch-bytes-tests--bad))))

(ert-deftest romacs-agent-watch-bytes-task-send-to-an-idle-task-is-refused-or-reported ()
  ;; Sent at once: refused at the call, or reported failed; never the
  ;; encoder's own error at the caller.
  (romacs-agent-watch-bytes-tests--with
    (romacs-agent-watch-bytes-tests--reachable child lines errors
      (romacs-agent-watch-bytes-tests--send child romacs-agent-watch-bytes-tests--bad))))

(ert-deftest romacs-agent-watch-bytes-message-waiting-behind-it-still-goes ()
  ;; Messages from other sessions waiting for a turn's end go in one
  ;; turn: the bad one is let go, the good one behind it written.
  (romacs-agent-watch-bytes-tests--with
    (setf (romacs-agent-state child) 'running)
    (romacs-agent-watch-bytes-tests--send child romacs-agent-watch-bytes-tests--bad)
    (romacs-agent-watch-bytes-tests--send child "the message behind it")
    (romacs-agent-watch-bytes-tests--turn-ends child)
    (romacs-agent-watch-bytes-tests--turn-ends child)
    (should (string-search "the message behind it" (romacs-agent-watch-bytes-tests--written lines child)))
    (should-not (romacs-agent-queue child))
    (should (assq child errors))))

(ert-deftest romacs-agent-watch-bytes-delivery-kept-for-a-resume-keeps-it-reachable ()
  ;; A tool's delivery no sender keeps (`romacs-agent-tools--deliver', as a
  ;; wake's results go to a parent): its failure is said on the session,
  ;; and what is sent after it goes.
  (romacs-agent-watch-bytes-tests--with
    (romacs-agent-watch-bytes-tests--reachable child lines errors
      (romacs-agent-tools--deliver child romacs-agent-watch-bytes-tests--bad nil nil nil 'next))))

(ert-deftest romacs-agent-watch-bytes-reply-to-a-busy-parent-keeps-it-reachable ()
  ;; A message relayed between sessions: the task answers its parent's
  ;; message (romacs_task_reply), its parent's turn running.
  (romacs-agent-watch-bytes-tests--with
    (let ((asked (romacs-agent-tools--message "parent" lead)))
      (romacs-agent--raise child (list :type 'input :text "which branch?" :messages (list asked) :seq 40))
      (setf (romacs-agent-state lead) 'running)
      (romacs-agent-watch-bytes-tests--reachable lead lines errors
        ;; The next message to it is its user's, from outside any session.
        (setq caller "child")
        (unwind-protect
            (romacs-agent-tools--reply (romacs-mcp--object "messageId" (plist-get asked :id)
                                                           "text" romacs-agent-watch-bytes-tests--bad))
          (setq caller nil))))))

(ert-deftest romacs-agent-watch-bytes-queued-by-lisp-keeps-it-reachable ()
  ;; Queued by Lisp, as a chat's user's message (text a buffer can hold).
  (romacs-agent-watch-bytes-tests--with
    (setf (romacs-agent-state child) 'running)
    (romacs-agent-watch-bytes-tests--reachable child lines errors
      (romacs-agent-send-later child romacs-agent-watch-bytes-tests--bad nil nil nil nil t))))

(provide 'romacs-agent-watch-bytes-tests)
;;; romacs-agent-watch-bytes-tests.el ends here
