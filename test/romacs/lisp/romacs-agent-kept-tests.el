;;; romacs-agent-kept-tests.el --- A session restored from its kept state -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(require 'romacs-test-socket)

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
        (should (file-exists-p (romacs-agent--kept-file first))))
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
        (let ((replayed (let ((romacs-agent--sessions (make-hash-table :test #'equal)))
                          (romacs-agent--load twin))))
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
        (let ((replayed (let ((romacs-agent--sessions (make-hash-table :test #'equal)))
                          (romacs-agent--load twin))))
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
        (let ((replayed (let ((romacs-agent--sessions (make-hash-table :test #'equal)))
                          (romacs-agent--load twin))))
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
        (let ((replayed (let ((romacs-agent--sessions (make-hash-table :test #'equal)))
                          (romacs-agent--load twin))))
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
             (file (romacs-agent--kept-file session))
             (good (with-temp-buffer (insert-file-contents file) (buffer-string))))
        (dolist (bad (list (replace-regexp-in-string ":layout (id " ":layout (old id " good t t)
                           (replace-regexp-in-string ":seq [0-9]+" ":seq 999" good)
                           (substring good 0 (/ (length good) 2))))
          (with-temp-file file (insert bad))
          (let* ((read 0)
                 (again (cl-letf* ((real (symbol-function 'romacs-agent--record))
                                   ((symbol-function 'romacs-agent--record)
                                    (lambda (session record) (cl-incf read) (funcall real session record))))
                          (romacs-agent-kept-tests--load dir))))
            (should (= read (length records)))
            (should (equal (romacs-agent-reply again) "bye"))))))))

(ert-deftest romacs-agent-kept-only-what-reads-back ()
  ;; A field holding what cannot be read back: nothing is kept.
  (romacs-agent-kept-tests--with
    (romacs-agent-kept-tests--write dir (romacs-agent-kept-tests--claude))
    (let ((session (romacs-agent-kept-tests--load dir)))
      (delete-file (romacs-agent--kept-file session))
      (remhash session romacs-agent--kept)
      (setf (romacs-agent-message session) (current-buffer))
      (should-not (romacs-agent--kept-text session))
      (romacs-agent--keep session)
      (should-not (file-exists-p (romacs-agent--kept-file session))))))

(ert-deftest romacs-agent-kept-a-copy-keeps-nothing ()
  ;; A copy replaying the journal (a chat drawn) never writes the session's state.
  (romacs-agent-kept-tests--with
    (romacs-agent-kept-tests--write dir (romacs-agent-kept-tests--claude))
    (let ((session (romacs-agent-kept-tests--load dir)))
      (delete-file (romacs-agent--kept-file session))
      (let ((scratch (romacs-agent--scratch session)))
        (setf (romacs-agent-caught-up scratch) t)
        (romacs-agent--read-journal session (lambda (record) (romacs-agent--record scratch record)))
        (romacs-agent--keep scratch))
      (should-not (file-exists-p (romacs-agent--kept-file session))))))

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
        (should (file-exists-p (romacs-agent--kept-file session)))
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

;;; romacs-agent-kept-tests.el ends here
