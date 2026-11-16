;;; romacs-agent-kept-tests.el --- A session restored from its kept state -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(require 'romacs-test-socket)

;; What `romacs-startup-progress' keeps: bound here around each restore.
(defvar romacs-progress-state)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-codex))

(defconst romacs-agent-kept-tests--id "c0ffee00-1111-4222-8333-444455556666")

(defun romacs-agent-kept-tests--record (seq key value)
  "Record SEQ of a journal, its keys in the supervisor's order (sorted)."
  (romacs-mcp--object key value "seq" seq "t" (+ 1794600000.25 seq)))

(defun romacs-agent-kept-tests--claude ()
  "A Claude Code run as its supervisor journals it: started up, a turn
with a streamed reply, a tool call and its approval, a limit report, a
second turn, then its exit.  Its first turn ends at seq 12."
  (let ((seq 0))
    (cl-flet ((out (object) (romacs-agent-kept-tests--record (cl-incf seq) "out" object))
              (in (object) (romacs-agent-kept-tests--record (cl-incf seq) "in" object))
              (o (&rest fields) (apply #'romacs-mcp--object fields)))
      (list
       (romacs-agent-kept-tests--record (cl-incf seq) "note" (o "type" "run"))
       (in (o "type" "control_request" "request_id" "romacs-init-1" "request" (o "subtype" "initialize")))
       (out (o "type" "control_response" "response" (o "subtype" "success" "request_id" "romacs-init-1")))
       (out (o "type" "system" "subtype" "init" "session_id" romacs-agent-kept-tests--id
               "model" "claude-opus-5-5" "permissionMode" "default"))
       (in (o "type" "user" "romacsRequest" 1 "message" (o "role" "user" "content" "first, café ✓")))
       (out (o "type" "stream_event" "parent_tool_use_id" :null
               "event" (o "type" "message_start" "message" (o "id" "m1"))))
       (out (o "type" "stream_event" "parent_tool_use_id" :null
               "event" (o "type" "content_block_delta" "delta" (o "type" "text_delta" "text" "hel"))))
       (out (o "type" "assistant" "parent_tool_use_id" :null
               "message" (o "id" "m1" "content" (vector (o "type" "text" "text" "hello")
                                                        (o "type" "tool_use" "id" "t1" "name" "Bash"
                                                           "input" (o "command" "ls"))))))
       (out (o "type" "control_request" "request_id" "r1"
               "request" (o "subtype" "can_use_tool" "tool_name" "Bash" "input" (o "command" "ls"))))
       (in (o "type" "control_response"
              "response" (o "subtype" "success" "request_id" "r1" "response" (o "behavior" "allow"))))
       (out (o "type" "user" "parent_tool_use_id" :null
               "message" (o "content" (vector (o "type" "tool_result" "tool_use_id" "t1" "content" "a\nb")))))
       (out (o "type" "result" "result" "hello" "total_cost_usd" 0.5 "duration_ms" 1500
               "modelUsage" (o "claude-opus-5-5" (o "contextWindow" 200000))))
       (out (o "type" "rate_limit_event" "rate_limit_info" (o "status" "allowed" "utilization" 0.5)))
       (in (o "type" "user" "romacsRequest" 2 "message" (o "role" "user" "content" "second")))
       (out (o "type" "assistant" "parent_tool_use_id" :null
               "message" (o "id" "m2" "content" (vector (o "type" "text" "text" "bye")))))
       (out (o "type" "result" "result" "bye" "total_cost_usd" 0.75))
       (romacs-agent-kept-tests--record (cl-incf seq) "exit" 0)))))

(defun romacs-agent-kept-tests--write (dir records &optional append)
  (let ((coding-system-for-write 'utf-8-unix))
    (write-region (mapconcat (lambda (record) (concat (romacs-agent--json record) "\n")) records "")
                  nil (expand-file-name "journal.jsonl" dir) append 'silent)))

(defmacro romacs-agent-kept-tests--with (&rest body)
  "BODY with a private state root under `root', a private runtime root
`run' short enough for its sockets (`romacs-test-socket-directory'), and
`dir' and `twin', two recorded sessions of one id under two state roots:
`dir' the one under the current state root."
  (declare (indent 0))
  `(let* ((root (make-temp-file "romacs-kept-" t))
          (run (romacs-test-socket-directory "kept-"))
          (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state")
                                             (concat "XDG_RUNTIME_DIR=" run))
                                       process-environment))
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent--keeping nil)
          (dir (expand-file-name romacs-agent-kept-tests--id (romacs-agent--state-root)))
          (twin (expand-file-name (concat "twin/" romacs-agent-kept-tests--id) root)))
     (dolist (d (list dir twin))
       (make-directory d t)
       (romacs-agent--write-json
        (expand-file-name "manifest.json" d)
        (romacs-mcp--object "id" romacs-agent-kept-tests--id "harness" "claude" "provider" "claude"
                            "cwd" root "title" "kept" "titleAuto" :false "argv" [] "options" (romacs-mcp--object)
                            "seenSeq" 0)))
     (cl-letf (((symbol-function 'romacs-agent--keep-models) #'ignore))
       (unwind-protect (progn ,@body) (delete-directory root t) (delete-directory run t)))))

(defun romacs-agent-kept-tests--replay (twin)
  "The session recorded in TWIN, loaded as a server that kept nothing of
it does: no state kept to go on from (its id's is the other's), so every
one of its records is read, which this checks."
  (let ((romacs-agent--sessions (make-hash-table :test #'equal)) (read 0)
        (records (with-temp-buffer (insert-file-contents (expand-file-name "journal.jsonl" twin))
                                   (count-lines (point-min) (point-max)))))
    (prog1 (cl-letf* ((real (symbol-function 'romacs-agent--record))
                      ((symbol-function 'romacs-agent--record)
                       (lambda (session record) (cl-incf read) (funcall real session record)))
                      ((symbol-function 'romacs-agent-turns-fold) #'ignore)
                      ((symbol-function 'romacs-agent-turns-fold-put) #'ignore))
             (romacs-agent--load twin))
      (cl-assert (= read records) t "the twin read %d of its %d records" read records))))

(defvar romacs-agent-kept-tests--due nil
  "The timers set within `romacs-agent-kept-tests--stepping', held, the
earliest first.")

(defmacro romacs-agent-kept-tests--stepping (&rest body)
  "BODY with the timers it sets held, not run (`romacs-agent-kept-tests--due'):
the restore's steps are taken as the command loop would take them
\(`romacs-agent-kept-tests--step-all'), with no wait on the clock."
  (declare (indent 0))
  `(let ((romacs-agent-kept-tests--due nil))
     (cl-letf (((symbol-function 'run-at-time)
                (lambda (_time _repeat function &rest args)
                  (setq romacs-agent-kept-tests--due (append romacs-agent-kept-tests--due (list (cons function args))))
                  nil)))
       ,@body)))

(defun romacs-agent-kept-tests--step-all ()
  "Run the held timers, the ones they set too, in order, until none is left."
  (while romacs-agent-kept-tests--due
    (let ((timer (pop romacs-agent-kept-tests--due))) (apply (car timer) (cdr timer)))))

(defun romacs-agent-kept-tests--load (dir)
  "The session recorded in DIR, loaded by a server knowing no session,
which then goes idle: what it keeps is written."
  (clrhash romacs-agent--sessions)
  (prog1 (romacs-agent--load dir)
    (romacs-agent--keep-pending)))

(defun romacs-agent-kept-tests--state (session)
  "SESSION's state as its kept form prints it: hash tables by their
contents, so two equal states print the same."
  (romacs-agent--kept-text session))

(ert-deftest romacs-agent-kept-restore-matches-a-whole-replay ()
  ;; Kept at its first turn's end, then restored with the rest of its
  ;; journal read on: the same state as a server reading it all.
  (romacs-agent-kept-tests--with
    (let* ((records (romacs-agent-kept-tests--claude))
           (head (cl-subseq records 0 12)) (tail (nthcdr 12 records)))
      (romacs-agent-kept-tests--write dir head)
      (let ((first (romacs-agent-kept-tests--load dir)))
        (should (= (romacs-agent-seq first) 12))
        (should (romacs-agent--kept-get first)))
      (romacs-agent-kept-tests--write dir tail t)
      (let* ((read nil)
             (restored (cl-letf* ((real (symbol-function 'romacs-agent--record))
                                  ((symbol-function 'romacs-agent--record)
                                   (lambda (session record) (push (gethash "seq" record) read)
                                     (funcall real session record))))
                         (romacs-agent-kept-tests--load dir))))
        ;; Only what came after its kept state was read.
        (should (equal (nreverse read) (number-sequence 13 (length records))))
        (romacs-agent-kept-tests--write twin records)
        (let ((replayed (romacs-agent-kept-tests--replay twin)))
          (should (= (romacs-agent-seq restored) (length records)))
          (should (equal (romacs-agent-reply restored) "bye"))
          (should (= (romacs-agent-turns restored) 2))
          (should (equal (romacs-agent-kept-tests--state restored) (romacs-agent-kept-tests--state replayed)))
          (should (eq (romacs-agent-state restored) (romacs-agent-state replayed))))))))

(ert-deftest romacs-agent-kept-restore-mid-turn-matches-a-whole-replay ()
  ;; Kept with an approval pending and a tool call open, as a server's
  ;; catching up keeps it: the rest of the turn is read on.
  (romacs-agent-kept-tests--with
    (let* ((records (romacs-agent-kept-tests--claude))
           (head (cl-subseq records 0 9)) (tail (nthcdr 9 records)))
      (romacs-agent-kept-tests--write dir head)
      (let ((first (romacs-agent-kept-tests--load dir)))
        (should (= (hash-table-count (romacs-agent-approvals first)) 1)))
      (romacs-agent-kept-tests--write dir tail t)
      (let ((restored (romacs-agent-kept-tests--load dir)))
        (romacs-agent-kept-tests--write twin records)
        (let ((replayed (romacs-agent-kept-tests--replay twin)))
          (should (equal (romacs-agent-kept-tests--state restored)
                         (romacs-agent-kept-tests--state replayed))))))))

(ert-deftest romacs-agent-kept-a-codex-run-matches-a-whole-replay ()
  ;; A per-turn Codex run, journaled as `codex exec' wrote its lines.
  (romacs-agent-kept-tests--with
    (let* ((seq 0)
           (o #'romacs-mcp--object)
           (records (list (romacs-agent-kept-tests--record (cl-incf seq) "in" (funcall o "prompt" "go"))
                          (romacs-agent-kept-tests--record (cl-incf seq) "out" (funcall o "type" "thread.started" "thread_id" "th"))
                          (romacs-agent-kept-tests--record (cl-incf seq) "out" (funcall o "type" "turn.started"))
                          (romacs-agent-kept-tests--record
                           (cl-incf seq) "out" (funcall o "type" "item.completed"
                                                        "item" (funcall o "type" "agent_message" "id" "i1" "text" "done")))
                          (romacs-agent-kept-tests--record (cl-incf seq) "out" (funcall o "type" "turn.completed"))
                          (romacs-agent-kept-tests--record (cl-incf seq) "exit" 0)
                          (romacs-agent-kept-tests--record (cl-incf seq) "in" (funcall o "prompt" "again"))
                          (romacs-agent-kept-tests--record
                           (cl-incf seq) "out" (funcall o "type" "item.completed"
                                                        "item" (funcall o "type" "agent_message" "id" "i2" "text" "twice")))
                          (romacs-agent-kept-tests--record (cl-incf seq) "out" (funcall o "type" "turn.completed"))
                          (romacs-agent-kept-tests--record (cl-incf seq) "exit" 0))))
      (dolist (d (list dir twin))
        (romacs-agent--manifest-put (romacs-agent--make :dir d) "harness" "codex" "provider" "codex"))
      (romacs-agent-kept-tests--write dir (cl-subseq records 0 6))
      (romacs-agent-kept-tests--load dir)
      (romacs-agent-kept-tests--write dir (nthcdr 6 records) t)
      (let ((restored (romacs-agent-kept-tests--load dir)))
        (romacs-agent-kept-tests--write twin records)
        (let ((replayed (romacs-agent-kept-tests--replay twin)))
          (should (equal (romacs-agent-reply restored) "twice"))
          (should (equal (romacs-agent-kept-tests--state restored)
                         (romacs-agent-kept-tests--state replayed))))))))

(ert-deftest romacs-agent-kept-live-turn-end-is-kept-when-idle ()
  ;; Its records the live ones: a turn's end is kept once the editor is
  ;; idle, as of that record; a record ending nothing waits for one.
  (romacs-agent-kept-tests--with
    (let* ((records (romacs-agent-kept-tests--claude))
           (session (progn (romacs-agent-kept-tests--write dir (cl-subseq records 0 10))
                           (romacs-agent-kept-tests--load dir)))
           (idle 0))
      (setf (romacs-agent-caught-up session) t)
      (cl-letf (((symbol-function 'run-with-idle-timer) (lambda (&rest _) (cl-incf idle))))
        (romacs-agent--record session (nth 10 records))
        (should-not romacs-agent--keeping)
        (romacs-agent--record session (nth 11 records))
        (romacs-agent--record session (nth 12 records))
        ;; Kept as of the last record handled before the editor is idle.
        (should (equal romacs-agent--keeping (list (list session 13 'idle))))
        (should (= idle 1)))
      (romacs-agent-kept-tests--write dir (cl-subseq records 10 13) t)
      (romacs-agent--keep-pending)
      (should (= (plist-get (romacs-agent--read-kept session) :seq) 13)))))

(ert-deftest romacs-agent-kept-an-end-for-a-busy-parent-survives-a-restart ()
  ;; A child's turn end handed to a parent at work waits for the parent's
  ;; turn: kept meanwhile and restarted, it is handed on again.
  (romacs-agent-kept-tests--with
    (let ((parent-id "a0a0a0a0-0000-4000-8000-000000000000"))
      (romacs-agent--manifest-put (romacs-agent--make :dir dir) "options" (romacs-mcp--object "parent" parent-id))
      (romacs-agent-kept-tests--write dir (cl-subseq (romacs-agent-kept-tests--claude) 0 12))
      (cl-flet ((restart (then)
                  (let ((romacs-agent-tools--handed (make-hash-table :test #'eq :weakness 'key))
                        (romacs-agent-tools--pending (make-hash-table :test #'equal))
                        (romacs-agent-tools--waking (make-hash-table :test #'equal)))
                    (let ((child (romacs-agent-kept-tests--load dir))
                          (parent (romacs-agent--make :id parent-id :harness 'claude :state 'running :dir root)))
                      (puthash parent-id parent romacs-agent--sessions)
                      ;; Its wake waits for its turn: none is scheduled here.
                      (puthash parent-id t romacs-agent-tools--waking)
                      (romacs-agent-tools--hand-on child)
                      (funcall then child parent-id)))))
        (restart (lambda (child _)
                   ;; Kept while the end waits for the parent.
                   (remhash child romacs-agent--kept)
                   (romacs-agent--keep child)))
        (restart (lambda (child parent-id)
                   (should (equal (mapcar #'cadr (gethash parent-id romacs-agent-tools--pending)) '(12)))
                   (should (equal (mapcar #'car (romacs-agent-ends child)) '(12)))))))))

(ert-deftest romacs-agent-kept-is-the-records-state-not-the-connections ()
  ;; Kept after a turn's end while no supervisor runs (the session shows
  ;; detached), then its run's exit read on: the turn stays a success.
  (romacs-agent-kept-tests--with
    (let* ((records (cl-subseq (romacs-agent-kept-tests--claude) 0 12))
           (exit (romacs-agent-kept-tests--record 13 "exit" 0)))
      (romacs-agent-kept-tests--write dir records)
      (should (eq (romacs-agent-state (romacs-agent-kept-tests--load dir)) 'detached))
      (romacs-agent-kept-tests--write dir (list exit) t)
      (let ((restored (romacs-agent-kept-tests--load dir)))
        (romacs-agent-kept-tests--write twin (append records (list exit)))
        (let ((replayed (romacs-agent-kept-tests--replay twin)))
          (should-not (romacs-agent-failed restored))
          (should (equal (romacs-agent-reply restored) "hello"))
          (should (equal (romacs-agent-kept-tests--state restored)
                         (romacs-agent-kept-tests--state replayed))))))))

(ert-deftest romacs-agent-kept-no-rollback-hold ()
  ;; A rollback under way holds the session in this process alone.
  (romacs-agent-kept-tests--with
    (romacs-agent-kept-tests--write dir (romacs-agent-kept-tests--claude))
    (let ((session (romacs-agent-kept-tests--load dir)))
      (setf (romacs-agent-driver-state session) (plist-put (romacs-agent-driver-state session) :rolling-back 'token))
      (remhash session romacs-agent--kept)
      (romacs-agent--keep session)
      (should-not (plist-member (cdr (assq 'driver-state (plist-get (romacs-agent--read-kept session) :fields)))
                                :rolling-back)))))

(ert-deftest romacs-agent-kept-a-span-starting-inside-a-record ()
  ;; The first span read from the journal's end starts inside a record,
  ;; at a nested object that looks like a record's start.
  (romacs-agent-kept-tests--with
    (let* ((session (romacs-agent--make :dir dir))
           (first "{\"note\":{\"type\":\"run\"},\"seq\":1,\"t\":1.5}\n")
           (nested "{\"seq\":1,\"y\":0}")
           (tail-head ",\"pad\":\"")
           (tail-end "\"},\"seq\":2,\"t\":2.5}\n")
           (pad (make-string (- 16384 (length nested) (length tail-head) (length tail-end)) ?b)))
      (with-temp-file (expand-file-name "journal.jsonl" dir)
        (set-buffer-multibyte nil)
        (insert first "{\"out\":{\"a\":\"" (make-string 100 ?a) "\",\"z\":" nested tail-head pad tail-end))
      (should (= (romacs-agent--journal-after session 1) (length first))))))

(ert-deftest romacs-agent-kept-is-read-only-as-written ()
  ;; Another session record's layout, a seq its journal lacks, or a form
  ;; that does not read: the whole journal is read instead.
  (romacs-agent-kept-tests--with
    (let ((records (romacs-agent-kept-tests--claude)))
      (romacs-agent-kept-tests--write dir records)
      (let* ((session (romacs-agent-kept-tests--load dir))
             (good (romacs-agent--kept-get session)))
        (dolist (bad (list (replace-regexp-in-string ":layout (id " ":layout (old id " good t t)
                           (replace-regexp-in-string ":seq [0-9]+" ":seq 999" good)
                           (substring good 0 (/ (length good) 2))))
          (romacs-agent--kept-put session bad)
          (let* ((read 0)
                 (again (cl-letf* ((real (symbol-function 'romacs-agent--record))
                                   ((symbol-function 'romacs-agent--record)
                                    (lambda (session record) (cl-incf read) (funcall real session record))))
                          (romacs-agent-kept-tests--load dir))))
            (should (= read (length records)))
            (should (equal (romacs-agent-reply again) "bye"))))))))

(ert-deftest romacs-agent-kept-survives-an-added-field ()
  ;; Kept by a build that lacked a field this one has: read back, no record
  ;; read again, the new field at its default.
  (romacs-agent-kept-tests--with
    (let ((dir (expand-file-name romacs-agent-kept-tests--id (romacs-agent--state-root))))
    (let ((records (romacs-agent-kept-tests--claude)))
      (romacs-agent-kept-tests--write dir records)
      (let* ((session (romacs-agent-kept-tests--load dir))
             (kept (car (read-from-string (romacs-agent--kept-get session))))
             (dropped (car (car (last (plist-get kept :fields))))))
        (romacs-agent--kept-put
         session (let ((print-circle t) (print-length nil) (print-level nil))
                   (prin1-to-string (plist-put (plist-put (copy-sequence kept) :layout (remq dropped (plist-get kept :layout)))
                                               :fields (assq-delete-all dropped (copy-alist (plist-get kept :fields)))))))
        (let* ((read 0)
               (again (cl-letf* ((real (symbol-function 'romacs-agent--record))
                                 ((symbol-function 'romacs-agent--record)
                                  (lambda (session record) (cl-incf read) (funcall real session record))))
                        (romacs-agent-kept-tests--load dir))))
          (should (= read 0))
          (should (equal (romacs-agent-reply again) "bye"))))))))
(ert-deftest romacs-agent-kept-only-what-reads-back ()
  ;; A field holding what cannot be read back: nothing is kept.
  (romacs-agent-kept-tests--with
    (romacs-agent-kept-tests--write dir (romacs-agent-kept-tests--claude))
    (let ((session (romacs-agent-kept-tests--load dir)))
      (romacs-agent--kept-put session nil)
      (remhash session romacs-agent--kept)
      (setf (romacs-agent-message session) (current-buffer))
      (should-not (romacs-agent--kept-text session))
      (romacs-agent--keep session)
      (should-not (romacs-agent--kept-get session)))))

(ert-deftest romacs-agent-kept-a-copy-keeps-nothing ()
  ;; A copy replaying the journal (a chat drawn) never writes the session's state.
  (romacs-agent-kept-tests--with
    (romacs-agent-kept-tests--write dir (romacs-agent-kept-tests--claude))
    (let ((session (romacs-agent-kept-tests--load dir)))
      (romacs-agent--kept-put session nil)
      (let ((scratch (romacs-agent--scratch session)))
        (setf (romacs-agent-caught-up scratch) t)
        (romacs-agent--read-journal session (lambda (record) (romacs-agent--record scratch record)))
        (romacs-agent--keep scratch))
      (should-not (romacs-agent--kept-get session)))))

(ert-deftest romacs-agent-kept-journal-after-either-key-order ()
  (romacs-agent-kept-tests--with
    (let ((session (romacs-agent--make :dir dir))
          (lines '("{\"out\":{\"seq\":7,\"t\":1},\"seq\":1,\"t\":1.5}"
                   "{\"seq\":2,\"t\":2.5,\"out\":{\"x\":\"\\\"seq\\\":2,\"}}"
                   "{\"note\":{\"type\":\"run\"},\"seq\":3,\"t\":3e2}")))
      (with-temp-file (expand-file-name "journal.jsonl" dir)
        (set-buffer-multibyte nil)
        (insert (mapconcat #'identity lines "\n") "\n"))
      (should (= (romacs-agent--journal-after session 1) (1+ (length (nth 0 lines)))))
      (should (= (romacs-agent--journal-after session 2) (+ 2 (length (nth 0 lines)) (length (nth 1 lines)))))
      (should (= (romacs-agent--journal-after session 3)
                 (file-attribute-size (file-attributes (expand-file-name "journal.jsonl" dir)))))
      ;; A seq only nested in a record names no line.
      (should-not (romacs-agent--journal-after session 7))
      (should-not (romacs-agent--journal-after session 4)))))

(ert-deftest romacs-agent-kept-attaches-from-its-kept-record ()
  ;; Its supervisor running, a session is asked from the record its kept
  ;; state is at, not from the journal's start.
  (romacs-agent-kept-tests--with
    (romacs-agent-kept-tests--write dir (cl-subseq (romacs-agent-kept-tests--claude) 0 12))
    (romacs-agent-kept-tests--load dir)
    (let* ((socket (romacs-agent--host-socket romacs-agent-kept-tests--id))
           (heard "")
           (server (progn (make-directory (file-name-directory socket) t)
                          (make-network-process :name "romacs-kept-host" :family 'local :service socket
                                                :server t :noquery t
                                                :filter (lambda (_process chunk) (setq heard (concat heard chunk)))))))
      (unwind-protect
          (let ((session (cl-letf (((symbol-function 'romacs-agent--endpoint) #'ignore))
                           (romacs-agent-kept-tests--load dir))))
            (with-timeout (10 (ert-fail "no attach line"))
              (while (not (string-search "\n" heard)) (accept-process-output nil 0.05)))
            (should (= (gethash "from" (gethash "attach" (json-parse-string heard))) 12))
            (should (= (romacs-agent-seq session) 12))
            (when (process-live-p (romacs-agent-connection session))
              (delete-process (romacs-agent-connection session))))
        (delete-process server)))))

(ert-deftest romacs-agent-kept-copied-state-reaches-no-other-supervisor ()
  ;; A recorded state copied under another runtime directory: its
  ;; manifest's socket path, a supervisor elsewhere, is never connected to.
  (romacs-agent-kept-tests--with
    (let* ((elsewhere (expand-file-name "elsewhere/host.sock" run))
           (connected nil)
           (server (progn (make-directory (file-name-directory elsewhere) t)
                          (make-network-process :name "romacs-kept-elsewhere" :family 'local :service elsewhere
                                                :server t :noquery t
                                                :log (lambda (&rest _) (setq connected t))))))
      (unwind-protect
          (progn
            (romacs-agent--manifest-put (romacs-agent--make :dir dir) "hostSocket" elsewhere)
            (romacs-agent-kept-tests--write dir (romacs-agent-kept-tests--claude))
            (let ((session (romacs-agent-kept-tests--load dir)))
              (accept-process-output nil 0.2)
              (should-not connected)
              (should (string-prefix-p (expand-file-name "romacs/" run) (romacs-agent-host-socket session)))
              (should (eq (romacs-agent-state session) 'ended))))
        (delete-process server)))))

(ert-deftest romacs-agent-kept-a-dead-socket-is-removed-and-the-session-kept ()
  ;; A supervisor its unit stopped leaves its socket: the session loads from
  ;; its journal as one with no socket does, never attaching there, keeps
  ;; what it folded, and the dead socket goes, so the next start reads from
  ;; the kept state.
  (romacs-agent-kept-tests--with
    (romacs-agent-kept-tests--write dir (romacs-agent-kept-tests--claude))
    (let ((socket (romacs-agent--host-socket romacs-agent-kept-tests--id)))
      (make-directory (file-name-directory socket) t)
      (delete-process (make-network-process :name "romacs-kept-dead" :family 'local :service socket :server t))
      (should (file-exists-p socket))
      (let ((session (cl-letf (((symbol-function 'romacs-agent--attach)
                                (lambda (&rest _) (ert-fail "attached where nothing answers"))))
                       (romacs-agent-kept-tests--load dir))))
        (should (eq (romacs-agent-state session) 'ended))
        (should (romacs-agent--kept-get session))
        (should-not (file-exists-p socket))))))

(ert-deftest romacs-agent-kept-a-live-socket-is-left-and-attached ()
  ;; A socket its supervisor answers at is never removed.
  (romacs-agent-kept-tests--with
    (let* ((socket (romacs-agent--host-socket romacs-agent-kept-tests--id))
           (server (progn (make-directory (file-name-directory socket) t)
                          (make-network-process :name "romacs-kept-live" :family 'local :service socket
                                                :server t :noquery t))))
      (unwind-protect
          (progn (should (romacs-agent--supervisor-listens-p (romacs-agent--make :id romacs-agent-kept-tests--id
                                                                                 :host-socket socket)))
                 (should (file-exists-p socket)))
        (delete-process server)))))

(ert-deftest romacs-agent-kept-one-copy-and-an-old-file-moved-in ()
  ;; Kept in its rows, one copy however often it is written and however
  ;; many turns it has; a state an earlier build kept in a file is read;
  ;; off the editor it is moved into its rows and the file removed, the
  ;; first time; the editor reads it and writes nothing.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (romacs-agent-kept-tests--with
    (let ((dir (expand-file-name romacs-agent-kept-tests--id (romacs-agent--state-root))))
    (romacs-agent-kept-tests--write dir (romacs-agent-kept-tests--claude))
    (let* ((session (romacs-agent-kept-tests--load dir)) (id (romacs-agent-id session))
           (db (romacs-agent-turns--db)))
      (sqlite-execute db "insert into turns (session, first_seq, last_seq) values (?, 5, 12), (?, 13, 20)" (list id id))
      (romacs-agent--kept-put session "(:a 1)")
      (romacs-agent--kept-put session "(:a 2)")
      (should (equal (sqlite-select db "select first_seq, fold from turns where session = ? and fold is not null" (list id))
                     '((13 "(:a 2)"))))
      ;; From a file, once.
      (romacs-agent--kept-put session nil)
      (write-region "(:a 3)" nil (romacs-agent--kept-file session) nil 'silent)
      ;; The editor reads it as it is, writing nothing.
      (let ((noninteractive nil))
        (cl-letf (((symbol-function 'isolate-main-p) (lambda () t)))
          (should (equal (romacs-agent--kept-get session) "(:a 3)"))))
      (should (file-exists-p (romacs-agent--kept-file session)))
      (should-not (romacs-agent-turns-fold id))
      ;; Moved in by another process between the look and the read: from the rows.
      (let ((noninteractive nil) (moved nil))
        (cl-letf* ((read (symbol-function 'insert-file-contents))
                   ((symbol-function 'isolate-main-p) (lambda () t))
                   ((symbol-function 'insert-file-contents)
                    (lambda (file &rest args)
                      (unless moved
                        (setq moved t)
                        (let ((noninteractive t)) (romacs-agent--kept-put session "(:a 3)"))
                        (rename-file file (concat file ".moved")))
                      (apply read file args))))
          (should (equal (romacs-agent--kept-get session) "(:a 3)")))
        (rename-file (concat (romacs-agent--kept-file session) ".moved") (romacs-agent--kept-file session))
        (romacs-agent--kept-put session nil))
      ;; Off the editor: moved in, its file removed.
      (should (equal (romacs-agent--kept-get session) "(:a 3)"))
      (should-not (file-exists-p (romacs-agent--kept-file session)))
      (should (equal (romacs-agent--kept-get session) "(:a 3)"))
      (should (= (caar (sqlite-select db "select count(*) from turns where session = ? and fold is not null" (list id))) 1))))))
(ert-deftest romacs-agent-kept-a-busy-database-is-tried-again ()
  ;; Its state not kept (the database held by another writer): tried again
  ;; at the next idle moment, no record since, and kept then.
  (romacs-agent-kept-tests--with
    (let ((dir (expand-file-name romacs-agent-kept-tests--id (romacs-agent--state-root))))
      (romacs-agent-kept-tests--write dir (romacs-agent-kept-tests--claude))
      (let* ((session (romacs-agent-kept-tests--load dir)) (tries 0) (romacs-agent--keep-tries nil))
        (remhash session romacs-agent--kept)
        (setq romacs-agent--keeping nil)
        (cl-letf* ((put (symbol-function 'romacs-agent--kept-put))
                   ((symbol-function 'romacs-agent--kept-put)
                    (lambda (s text) (and (> (cl-incf tries) 1) (funcall put s text))))
                   ((symbol-function 'run-with-idle-timer) (lambda (&rest _) nil)))
          ;; Kept as its records left it (idle), its connection's view
          ;; (detached) laid over it meanwhile.
          (romacs-agent--keep session 'idle)
          (setf (romacs-agent-state session) 'detached)
          (should-not (gethash session romacs-agent--kept))
          (should (equal romacs-agent--keeping (list (list session (romacs-agent-seq session) 'idle))))
          (romacs-agent--keep-pending))
        (should (= tries 2))
        (should (eql (gethash session romacs-agent--kept) (romacs-agent-seq session)))
        (should (eq (cdr (assq 'state (plist-get (romacs-agent--read-kept session) :fields))) 'idle))))))

(ert-deftest romacs-agent-kept-restore-soon-loads-every-session-in-turns ()
  ;; A server's start: sessions load from the command loop after it, the
  ;; `agents' step saying how far, and a restored chat's one loads alone.
  (romacs-agent-kept-tests--with
    (romacs-agent-kept-tests--write dir (romacs-agent-kept-tests--claude))
    (clrhash romacs-agent--sessions)
    (let* ((romacs-agent--restore-pending nil) (restored 0) (romacs-progress-state nil)
           (romacs-agent-restored-hook (list (lambda () (cl-incf restored)))))
      (cl-letf (((symbol-function 'romacs-agent--endpoint) #'ignore)
                ((symbol-function 'romacs-agent--sweep) #'ignore))
        (should (romacs-agent-load-session romacs-agent-kept-tests--id))
        (should (= restored 0))
        (clrhash romacs-agent--sessions)
        (romacs-agent-kept-tests--stepping
          (romacs-agent-restore-soon)
          (should romacs-agent--restore-pending)
          (should-not (gethash romacs-agent-kept-tests--id romacs-agent--sessions))
          (romacs-agent-kept-tests--step-all))
        (should-not romacs-agent--restore-pending)
        (should (romacs-agent-get romacs-agent-kept-tests--id))
        (should (= restored 1))
        (let ((agents (alist-get 'agents romacs-progress-state)))
          (should (eq (plist-get agents :status) 'done))
          (should (equal (plist-get agents :label) "Agent sessions 1/1")))))))

(ert-deftest romacs-agent-kept-a-session-asked-for-mid-restore-loads-at-once ()
  ;; While sessions load a turn at a time, one attached early may ask for
  ;; another not reached yet (its child): it is loaded then and there, as
  ;; any session asked for by its id is.
  (romacs-agent-kept-tests--with
    (romacs-agent-kept-tests--write dir (romacs-agent-kept-tests--claude))
    (clrhash romacs-agent--sessions)
    (let ((romacs-agent--restore-pending (list dir)))
      (should (romacs-agent-get romacs-agent-kept-tests--id))
      (should-not (romacs-agent-get "../escape")))
    (clrhash romacs-agent--sessions)
    ;; No restore running: asked for, it is loaded all the same.
    (let ((romacs-agent--restore-pending nil))
      (should-not (romacs-agent-loaded romacs-agent-kept-tests--id))
      (should (romacs-agent-get romacs-agent-kept-tests--id)))))

;; Worker VMs as these tests see them: a job is held until `--work' runs
;; it in a VM of its own (a private session table), then its callback.
(defmacro romacs-agent-kept-tests--with-workers (&rest body)
  "BODY with every journal folded aside, its job pushed on `jobs'."
  (declare (indent 0))
  `(let ((jobs nil) (romacs-agent-fold-bytes 0) (romacs-agent--folding (make-hash-table :test #'equal))
         (romacs-agent--fold-queue nil) (romacs-agent--fold-running 0) (romacs-agent--fold-loading nil)
         (romacs-agent--fold-load-timer nil) (romacs-agent--restoring nil))
     (cl-letf (((symbol-function 'isolate-main-p) (lambda () t))
               ((symbol-function 'isolate-spawn)
                (lambda (function dir &rest keys) (car (push (list 'future function dir nil nil keys) jobs))))
               ((symbol-function 'isolate-on-done) (lambda (future callback) (setf (nth 3 future) callback) future))
               ((symbol-function 'isolate-await) (lambda (future &optional _) (nth 4 future))))
       ,@body)))

(defun romacs-agent-kept-tests--drain ()
  "Load on now what the folds that ended left waiting."
  (when romacs-agent--fold-load-timer (cancel-timer romacs-agent--fold-load-timer))
  (while (or romacs-agent--fold-loading romacs-agent--fold-load-timer) (romacs-agent--fold-load-step)
         (when romacs-agent--fold-load-timer (cancel-timer romacs-agent--fold-load-timer)
               (setq romacs-agent--fold-load-timer nil))))

(defun romacs-agent-kept-tests--work (future)
  "Run FUTURE's job as its worker would, then its callback, then load on."
  (setf (nth 4 future) (let ((romacs-agent--sessions (make-hash-table :test #'equal))
                             (romacs-agent--kept (make-hash-table :test #'eq :weakness 'key)))
                         (funcall (nth 1 future) (nth 2 future))))
  (funcall (nth 3 future) future)
  (romacs-agent-kept-tests--drain))

(ert-deftest romacs-agent-kept-a-long-journal-folds-aside ()
  ;; No kept state, a journal too long to fold in the editor: a worker
  ;; folds it and keeps its state.  The session is known meanwhile, and
  ;; loads on from that state, reading no record again: the same state as
  ;; a whole replay here.
  (romacs-agent-kept-tests--with
    (let ((records (romacs-agent-kept-tests--claude)))
      (romacs-agent-kept-tests--write dir records)
      (romacs-agent-kept-tests--with-workers
        (clrhash romacs-agent--sessions)
        (let ((session (romacs-agent--load dir)) (read nil))
          (should (eq (romacs-agent-get romacs-agent-kept-tests--id) session))
          (should (= (romacs-agent-seq session) 0))
          (should (gethash romacs-agent-kept-tests--id romacs-agent--folding))
          (should (= (length jobs) 1))
          (cl-letf* ((real (symbol-function 'romacs-agent--record))
                     ((symbol-function 'romacs-agent--record)
                      (lambda (session record) (push (gethash "seq" record) read) (funcall real session record))))
            (let ((future (car jobs)))
              (setf (nth 4 future) (let ((romacs-agent--sessions (make-hash-table :test #'equal))
                                         (romacs-agent--kept (make-hash-table :test #'eq :weakness 'key)))
                                     (funcall (nth 1 future) (nth 2 future))))
              (should (equal (nth 4 future) (list (length records))))
              ;; The worker's fold kept in the session's rows.
              (should (romacs-agent--kept-get session))
              (setq read nil)
              (funcall (nth 3 future) future)
              (romacs-agent-kept-tests--drain)))
          (should-not read)
          (should-not (gethash romacs-agent-kept-tests--id romacs-agent--folding))
          (romacs-agent-kept-tests--write twin records)
          (let ((replayed (let ((romacs-agent-fold-bytes nil)) (romacs-agent-kept-tests--replay twin))))
            (should (= (romacs-agent-seq session) (length records)))
            (should (equal (romacs-agent-kept-tests--state session) (romacs-agent-kept-tests--state replayed)))
            (should (eq (romacs-agent-state session) (romacs-agent-state replayed)))))))))

(ert-deftest romacs-agent-kept-a-long-tail-past-its-kept-state-folds-aside ()
  ;; Kept at its first turn's end, then much more journaled (a long turn
  ;; under way): what is left past its kept state is folded aside too, on
  ;; from that state; the session shows its kept state meanwhile.
  (romacs-agent-kept-tests--with
    (let* ((records (romacs-agent-kept-tests--claude))
           (head (cl-subseq records 0 12)) (tail (nthcdr 12 records)))
      (romacs-agent-kept-tests--write dir head)
      (romacs-agent-kept-tests--load dir)
      (romacs-agent-kept-tests--write dir tail t)
      (romacs-agent-kept-tests--with-workers
        (setq romacs-agent-fold-bytes 10)
        (clrhash romacs-agent--sessions)
        (let ((session (romacs-agent--load dir)))
          (should (= (romacs-agent-seq session) 12))
          (should (= (length jobs) 1))
          (romacs-agent-kept-tests--work (car jobs))
          (should (= (romacs-agent-seq session) (length records)))
          (should (equal (romacs-agent-reply session) "bye")))))))

(ert-deftest romacs-agent-kept-a-kept-state-far-back-is-not-searched-for-here ()
  ;; Its kept record further back than a fold's worth of bytes: the editor
  ;; does not search the journal for it, the worker does.
  (romacs-agent-kept-tests--with
    (let* ((records (romacs-agent-kept-tests--claude))
           (head (cl-subseq records 0 12)) (tail (nthcdr 12 records)))
      (romacs-agent-kept-tests--write dir head)
      (romacs-agent-kept-tests--load dir)
      (romacs-agent-kept-tests--write
       dir (cl-loop for n from 100 to 400
                    collect (romacs-agent-kept-tests--record n "note" (romacs-mcp--object "type" "pad" "text" (make-string 200 ?p))))
       t)
      (let ((session (romacs-agent--make :id romacs-agent-kept-tests--id :dir dir)))
        (should (eq (romacs-agent--journal-after session 12 4096) 'beyond))
        (should (integerp (romacs-agent--journal-after session 12)))
        (should (integerp (romacs-agent--journal-after session 400 4096))))
      (romacs-agent-kept-tests--with-workers
        (setq romacs-agent-fold-bytes 4096)
        (clrhash romacs-agent--sessions)
        (let ((session (romacs-agent--load dir)))
          (should (= (romacs-agent-seq session) 0))
          (should (= (length jobs) 1))
          (romacs-agent-kept-tests--work (car jobs))
          (should (= (romacs-agent-seq session) 400)))))))

(ert-deftest romacs-agent-kept-folds-go-in-bounded-batches-loaded-by-one-timer ()
  ;; A batch takes journals while their bytes fit, one past the bound
  ;; alone; folds that end together load on from one timer.
  (romacs-agent-kept-tests--with
    (romacs-agent-kept-tests--with-workers
      (let* ((romacs-agent--fold-batch '(8 . 100)) (romacs-agent-fold-jobs 4)
             (made (mapcar (lambda (bytes)
                             (let ((d (expand-file-name (format "b%d" bytes) root)))
                               (make-directory d t)
                               (write-region (make-string bytes ?x) nil (expand-file-name "journal.jsonl" d) nil 'silent)
                               (cons (romacs-agent--make :id (format "b%d" bytes) :dir d) nil)))
                           '(60 80 30))))
        (setq romacs-agent--fold-queue made)
        (romacs-agent--fold-next)
        (should (equal (mapcar (lambda (job) (mapcar #'file-name-nondirectory (nth 2 job))) (reverse jobs))
                       '(("b60") ("b80") ("b30"))))
        (let ((timers (lambda () (cl-count #'romacs-agent--fold-load-step timer-list :key #'timer--function))))
          (dolist (job jobs) (setf (nth 4 job) nil) (funcall (nth 3 job) job))
          (should (= (funcall timers) 1))
          (romacs-agent-kept-tests--drain)
          (should (= (funcall timers) 0)))))))

(ert-deftest romacs-agent-kept-a-restore-ends-after-its-folds ()
  ;; The restore's tail (attaching, sweeping, the restored hook) waits for
  ;; the sessions folded aside; its progress says running until then.
  (romacs-agent-kept-tests--with
    (romacs-agent-kept-tests--write dir (romacs-agent-kept-tests--claude))
    (clrhash romacs-agent--sessions)
    (let* ((romacs-agent--restore-pending nil) (restored 0) (romacs-progress-state nil)
           (romacs-agent-restored-hook (list (lambda () (cl-incf restored)))))
      (romacs-agent-kept-tests--with-workers
        (cl-letf (((symbol-function 'romacs-agent--endpoint) #'ignore)
                  ((symbol-function 'romacs-agent--sweep) #'ignore))
          (romacs-agent-kept-tests--stepping
            (romacs-agent-restore-soon)
            (romacs-agent-kept-tests--step-all))
          (should-not romacs-agent--restore-pending)
          (should (= restored 0))
          (should (eq (plist-get (alist-get 'agents romacs-progress-state) :status) 'running))
          (romacs-agent-kept-tests--work (car jobs))
          (should (= restored 1))
          (should (eq (plist-get (alist-get 'agents romacs-progress-state) :status) 'done))
          (should (= (romacs-agent-seq (romacs-agent-get romacs-agent-kept-tests--id))
                     (length (romacs-agent-kept-tests--claude)))))))))

(ert-deftest romacs-agent-kept-a-folding-session-acted-on-folds-here ()
  ;; Attached (or resumed, launched) while a worker folds it: folded here
  ;; first, and the worker's late answer is let go.  The worker is given
  ;; the editor's options.
  (romacs-agent-kept-tests--with
    (let ((records (romacs-agent-kept-tests--claude)) (romacs-agent-approval-policy 'ask))
      (romacs-agent-kept-tests--write dir records)
      (romacs-agent-kept-tests--with-workers
        (clrhash romacs-agent--sessions)
        (let ((session (romacs-agent--load dir)))
          (should (eq (alist-get 'romacs-agent-approval-policy (plist-get (nth 5 (car jobs)) :bindings)) 'ask))
          (cl-letf (((symbol-function 'romacs-agent--connect) #'ignore))
            (romacs-agent--attach session 0))
          (should (= (romacs-agent-seq session) (length records)))
          (should-not (gethash romacs-agent-kept-tests--id romacs-agent--folding))
          (let ((read nil) (future (car jobs)))
            (setf (nth 4 future) (let ((romacs-agent--sessions (make-hash-table :test #'equal))
                                       (romacs-agent--kept (make-hash-table :test #'eq :weakness 'key)))
                                   (funcall (nth 1 future) (nth 2 future))))
            (cl-letf* ((real (symbol-function 'romacs-agent--record))
                       ((symbol-function 'romacs-agent--record)
                        (lambda (session record) (push record read) (funcall real session record))))
              (funcall (nth 3 future) future)
              (romacs-agent-kept-tests--drain))
            (should-not read))
          (should (= romacs-agent--fold-running 0)))))))

(ert-deftest romacs-agent-kept-a-message-to-a-folding-session-folds-it-first ()
  ;; A message sent while a worker folds the session: what it is sent as
  ;; is decided on its folded state, not the stub's.
  (romacs-agent-kept-tests--with
    (let ((records (romacs-agent-kept-tests--claude)) (seen nil))
      (romacs-agent-kept-tests--write dir records)
      (romacs-agent-kept-tests--with-workers
        (clrhash romacs-agent--sessions)
        (let ((session (romacs-agent--load dir)))
          (should (gethash romacs-agent-kept-tests--id romacs-agent--folding))
          (cl-letf (((symbol-function 'romacs-agent--input)
                     (lambda (session &rest _) (setq seen (romacs-agent-seq session))))
                    ((symbol-function 'romacs-agent--enqueue)
                     (lambda (session &rest _) (setq seen (romacs-agent-seq session))))
                    ((symbol-function 'romacs-agent--continue) #'ignore))
            (ignore-errors (romacs-agent-send session "again")))
          (should (eql seen (length records)))
          (should-not (gethash romacs-agent-kept-tests--id romacs-agent--folding)))))))

(ert-deftest romacs-agent-kept-a-whole-restore-folds-what-folds-aside ()
  ;; A restore that ends at once (a view's first look) folds here what a
  ;; worker folds still, before its tail; one under way stepwise is left to
  ;; go on, its sessions known so far shown.
  (romacs-agent-kept-tests--with
    (romacs-agent-kept-tests--write dir (romacs-agent-kept-tests--claude))
    (let ((restored 0) (romacs-agent--restored nil) (romacs-agent--restore-pending nil) (romacs-progress-state nil))
      (romacs-agent-kept-tests--with-workers
        (cl-letf (((symbol-function 'romacs-agent--endpoint) #'ignore)
                  ((symbol-function 'romacs-agent--sweep) #'ignore))
          (let ((romacs-agent-restored-hook (list (lambda () (cl-incf restored)))))
            (clrhash romacs-agent--sessions)
            (let ((session (romacs-agent--load dir)))
              (should (gethash romacs-agent-kept-tests--id romacs-agent--folding))
              ;; Stepwise under way: a view's look restores nothing at once.
              (setq romacs-agent--restoring t)
              (romacs-agent-know-sessions)
              (should (= restored 0))
              (should (gethash romacs-agent-kept-tests--id romacs-agent--folding))
              ;; With nothing new to load, its tail waits for the fold.
              (setq romacs-agent--restoring nil)
              (romacs-agent-restore-soon)
              (should (= restored 0))
              (romacs-agent-restore)
              (should (= restored 1))
              (should-not romacs-agent--restoring)
              (should-not (gethash romacs-agent-kept-tests--id romacs-agent--folding))
              (should (= (romacs-agent-seq session) (length (romacs-agent-kept-tests--claude)))))))))))

(ert-deftest romacs-agent-kept-a-fold-with-no-worker-is-done-here ()
  ;; No worker to fold it (none starts): the session still loads, from its
  ;; journal here.
  (romacs-agent-kept-tests--with
    (romacs-agent-kept-tests--write dir (romacs-agent-kept-tests--claude))
    (romacs-agent-kept-tests--with-workers
      (cl-letf (((symbol-function 'isolate-spawn) (lambda (&rest _) (error "No worker"))))
        (clrhash romacs-agent--sessions)
        (let ((session (romacs-agent--load dir)))
          (romacs-agent-kept-tests--drain)
          (should (= (romacs-agent-seq session) (length (romacs-agent-kept-tests--claude))))
          (should-not (gethash romacs-agent-kept-tests--id romacs-agent--folding))
          (should (= romacs-agent--fold-running 0)))))))

;;; romacs-agent-kept-tests.el ends here
