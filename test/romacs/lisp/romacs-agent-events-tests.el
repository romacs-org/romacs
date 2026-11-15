;;; romacs-agent-events-tests.el --- A session's records in SQLite -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-events))

(require 'romacs-agent-claude)

(defun romacs-agent-events-tests--line (seq key value)
  "Record SEQ's line as the supervisor writes it: keys sorted."
  (concat (json-serialize (romacs-mcp--object key value "seq" seq "t" (+ 1794600000.25 seq))) "\n"))

(defun romacs-agent-events-tests--turn (n seq)
  "Turn N's lines from SEQ: its prompt, its answer, its result."
  (concat (romacs-agent-events-tests--line
           seq "in" (romacs-mcp--object "message" (romacs-mcp--object "content" (format "question %d" n) "role" "user")
                                        "romacsRequest" n "type" "user"))
          (romacs-agent-events-tests--line
           (+ seq 1) "out" (romacs-mcp--object
                            "message" (romacs-mcp--object
                                       "content" (vector (romacs-mcp--object "text" (format "answer %d" n) "type" "text"))
                                       "id" (format "m%d" n))
                            "parent_tool_use_id" :null "type" "assistant"))
          (romacs-agent-events-tests--line
           (+ seq 2) "out" (romacs-mcp--object "result" (format "answer %d" n) "type" "result"))))

(defun romacs-agent-events-tests--with-session (turns body)
  "Call BODY with a session whose journal holds a run note and TURNS
turns, and its journal's file; a state root of its own."
  (let* ((root (make-temp-file "romacs-events-" t))
         (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state")
                                            (concat "XDG_RUNTIME_DIR=" root "/run"))
                                      process-environment))
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (id "e7e70000-2222-4333-8444-555566667777")
         (dir (expand-file-name id (romacs-agent--state-root)))
         (journal (expand-file-name "journal.jsonl" dir)))
    (unwind-protect
        (progn
          (make-directory dir t)
          (romacs-agent--write-json (expand-file-name "manifest.json" dir)
                                    (romacs-mcp--object "id" id "harness" "claude" "provider" "claude" "cwd" root
                                                        "title" "events" "titleAuto" :false "argv" []
                                                        "options" (romacs-mcp--object)))
          (let ((coding-system-for-write 'utf-8-unix))
            (write-region (concat (romacs-agent-events-tests--line 1 "note" (romacs-mcp--object "type" "run"))
                                  (mapconcat (lambda (n) (romacs-agent-events-tests--turn n (+ 2 (* 3 (1- n)))))
                                             (number-sequence 1 turns) ""))
                          nil journal nil 'silent))
          (romacs-agent--load dir)
          (funcall body (romacs-agent-get id) journal))
      (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
      (delete-directory root t))))

(defun romacs-agent-events-tests--seqs (session &optional from end)
  "The seqs of SESSION's records, as `romacs-agent--read-journal' reads them."
  (let ((seqs nil))
    (romacs-agent--read-journal session (lambda (record) (push (gethash "seq" record) seqs)) from end)
    (nreverse seqs)))

(defun romacs-agent-events-tests--at (session place)
  "The seq of the record SESSION's records hold at PLACE."
  (car (romacs-agent-events-tests--seqs session place)))

(ert-deftest romacs-agent-events-read-as-the-journal-was ()
  ;; Imported: the same records, the same tail, notes and places, read
  ;; from SQLite; the journal moved to the backup directory; a note
  ;; written with no supervisor goes to SQLite.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal)))
    (romacs-agent-events-tests--with-session
     12
     (lambda (session journal)
       (let* ((seqs (romacs-agent-events-tests--seqs session))
              (tail (romacs-agent--journal-tail session 3))
              (tail-seq (romacs-agent-events-tests--at session (car tail)))
              (after (romacs-agent-events-tests--seqs session (romacs-agent--journal-after session 20)))
              (notes (length (romacs-agent--journal-notes session "run")))
              (recorded (romacs-agent--recorded-p session)))
         (should (romacs-agent-events-import session))
         (should (romacs-agent-events-p session))
         (should-not (file-exists-p journal))
         (should (directory-files (romacs-agent--state-root) nil "\\`journal-backup-"))
         (should (equal (romacs-agent-events-tests--seqs session) seqs))
         (let ((tail2 (romacs-agent--journal-tail session 3)))
           (should (equal (cdr tail2) (cdr tail)))
           (should (equal (romacs-agent-events-tests--at session (car tail2)) tail-seq)))
         (should (equal (romacs-agent-events-tests--seqs session (romacs-agent--journal-after session 20)) after))
         (should-not (romacs-agent--journal-after session 999))
         (should (= (length (romacs-agent--journal-notes session "run")) notes))
         (should (eq (romacs-agent--recorded-p session) recorded))
         ;; With no supervisor, Romacs's own note goes to SQLite.
         (romacs-agent--note session (romacs-mcp--object "type" "selection"))
         (should (= (length (romacs-agent--journal-notes session "selection")) 1))
         (should (= (car (last (romacs-agent-events-tests--seqs session))) (1+ (car (last seqs)))))
         ;; Importing again: nothing.
         (should (romacs-agent-events-import session))
         (should (= (length (romacs-agent-events-tests--seqs session)) (1+ (length seqs)))))))))

(ert-deftest romacs-agent-events-replay-in-steps-from-sqlite ()
  ;; A replay between keystrokes reads its pages from one snapshot: a
  ;; record added meanwhile is not waited for.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal))
        (romacs-agent-events--page 4))
    (romacs-agent-events-tests--with-session
     5
     (lambda (session _journal)
       (romacs-agent-events-import session)
       (let ((result :none) (steps 0))
         (cl-letf (((symbol-function 'run-at-time)
                    (lambda (_time _repeat f &rest args)
                      (when (= (cl-incf steps) 1)
                        (romacs-agent-events-append session (romacs-mcp--object "seq" 999 "t" 1.0 "note" (romacs-mcp--object "type" "late"))))
                      (apply f args))))
           (romacs-agent-events-async session (lambda (events) (setq result events)) 3))
         (should (listp result))
         (should (equal (mapcar (lambda (e) (plist-get e :text))
                                (seq-filter (lambda (e) (eq (plist-get e :type) 'input)) result))
                        '("question 1" "question 2" "question 3" "question 4" "question 5"))))))))

(ert-deftest romacs-agent-events-thinking-counters-are-not-kept ()
  (should (romacs-agent-events--counter-p
           (romacs-mcp--object "seq" 1 "out" (romacs-mcp--object "type" "system" "subtype" "thinking_tokens"))))
  (should-not (romacs-agent-events--counter-p
               (romacs-mcp--object "seq" 1 "out" (romacs-mcp--object "type" "system" "subtype" "init")))))

(ert-deftest romacs-agent-events-a-launch-imports-first ()
  ;; With the store on, a supervisor started for a journal's session
  ;; writes to SQLite: the session is imported before it starts, and its
  ;; command names the database.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal))
        (romacs-agent-events-store t) (command nil))
    (romacs-agent-events-tests--with-session
     2
     (lambda (session journal)
       (cl-letf (((symbol-function 'call-process)
                  (lambda (_program &rest args) (setq command args) 0))
                 ((symbol-function 'romacs-agent--route-here) #'ignore))
         (romacs-agent--launch session '("harness")))
       (should (romacs-agent-events-p session))
       (should-not (file-exists-p journal))
       (should (member "--store" command))
       (should (member (romacs-agent-store--file) command))))))

(ert-deftest romacs-agent-events-two-records-of-one-seq-are-kept ()
  ;; A journal holding two records of one seq (a note written beside its
  ;; supervisor's record): both imported, read in their order, a page's
  ;; end never between them.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal))
        (romacs-agent-events--page 2))
    (romacs-agent-events-tests--with-session
     1
     (lambda (session journal)
       (write-region (romacs-agent-events-tests--line 4 "note" (romacs-mcp--object "type" "selection")) nil journal t 'silent)
       (write-region (romacs-agent-events-tests--line 4 "note" (romacs-mcp--object "type" "beside")) nil journal t 'silent)
       (should (romacs-agent-events-import session))
       (should (equal (romacs-agent-events-tests--seqs session) '(1 2 3 4 4 4)))
       (let ((pager (romacs-agent-events-pager session)) (seen nil) (at 0))
         (while at
           (pcase (funcall (car pager) at)
             (`(,lines . ,next) (setq seen (append seen lines) at next))
             (_ (setq at nil))))
         (funcall (cdr pager))
         (should (= (length seen) 6)))))))

(ert-deftest romacs-agent-events-a-per-turn-start-is-where-records-go ()
  ;; A per-turn harness's turn, the store on: imported first, so where its
  ;; run's records start is a place of SQLite's, after its last record.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal))
        (romacs-agent-events-store t))
    (romacs-agent-events-tests--with-session
     2
     (lambda (session _journal)
       (cl-letf (((symbol-function 'romacs-agent--launch) #'ignore)
                 ((symbol-function 'romacs-agent--attach) #'ignore)
                 ((symbol-function 'romacs-agent--check-config-dir) #'ignore)
                 ((symbol-function 'romacs-agent--argv) (lambda (&rest _) '("harness"))))
         (romacs-agent--turn session "next"))
       (should (romacs-agent-events-p session))
       (should (equal (romacs-agent-journal-start session) (romacs-agent-events-end session)))
       (should (= (romacs-agent-journal-start session) 8))))))

(ert-deftest romacs-agent-events-a-failed-replay-lets-its-pages-go ()
  ;; A replay that fails in a step lets go of its snapshot; one that is done
  ;; lets go of it too, once.
  (let ((closed 0))
    (cl-letf (((symbol-function 'romacs-agent--journal-pager)
               (lambda (_s) (cons (lambda (_from) (cons (list "{\"seq\":1,\"t\":1.0,\"in\":{\"prompt\":\"p\"}}") nil))
                                  (lambda () (cl-incf closed)))))
              ((symbol-function 'run-at-time) (lambda (_t _r f &rest args) (apply f args))))
      (let ((session (romacs-agent--make :id "f" :harness 'claude :tools (make-hash-table :test #'equal)
                                         :streamed (make-hash-table :test #'equal))))
        (romacs-agent-events-async session #'ignore)
        (should (= closed 1))
        (cl-letf (((symbol-function 'romacs-agent--record) (lambda (&rest _) (error "Broken"))))
          (should-error (romacs-agent-events-async session #'ignore)))
        (should (= closed 2))))))

(defun romacs-agent-events-tests--delta (seq id text)
  (romacs-agent-events-tests--line
   seq "out" (romacs-mcp--object "api_message_id" id "type" "stream_event"
                                 "event" (romacs-mcp--object "type" "content_block_delta" "index" 0
                                                             "delta" (romacs-mcp--object "type" "text_delta" "text" text)))))

(defun romacs-agent-events-tests--reply (session)
  "SESSION's reply as a fold of its records leaves it."
  (let ((scratch (romacs-agent--scratch session)) (romacs-agent-event-functions nil))
    (romacs-agent--read-journal session (lambda (record) (romacs-agent--record scratch record)))
    (romacs-agent-reply scratch)))

(ert-deftest romacs-agent-events-compaction-keeps-what-a-fold-needs ()
  ;; Compacted: a completed message's deltas are gone, an interrupted
  ;; one's kept, and a fold leaves the same reply.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal)))
    (romacs-agent-events-tests--with-session
     0
     (lambda (session journal)
       (write-region (concat (romacs-agent-events-tests--line 2 "in" (romacs-mcp--object "message" (romacs-mcp--object "content" "q" "role" "user")
                                                                                          "romacsRequest" 1 "type" "user"))
                             (romacs-agent-events-tests--delta 3 "m1" "Hel")
                             (romacs-agent-events-tests--delta 4 "m1" "lo")
                             (romacs-agent-events-tests--line
                              5 "out" (romacs-mcp--object "type" "assistant" "parent_tool_use_id" :null
                                                          "message" (romacs-mcp--object "id" "m1" "content"
                                                                                        (vector (romacs-mcp--object "type" "text" "text" "Hello")))))
                             (romacs-agent-events-tests--line 6 "out" (romacs-mcp--object "type" "result" "result" "Hello"))
                             (romacs-agent-events-tests--line 7 "in" (romacs-mcp--object "message" (romacs-mcp--object "content" "q2" "role" "user")
                                                                                          "romacsRequest" 2 "type" "user"))
                             (romacs-agent-events-tests--delta 8 "m2" "Interrupt")
                             (romacs-agent-events-tests--delta 9 "m2" "ed"))
                     nil journal t 'silent)
       (romacs-agent-events-import session)
       (let ((before (romacs-agent-events-tests--reply session)))
         (should (= (romacs-agent-events-compact (romacs-agent-id session)) 2))
         (should (equal (romacs-agent-events-tests--seqs session) '(1 2 5 6 7 8 9)))
         (should (equal (romacs-agent-events-tests--reply session) before))
         ;; Compacted to its last record: not compacted again until it has more.
         (should-not (member (romacs-agent-id session) (romacs-agent-events--compactable))))))))

(ert-deftest romacs-agent-events-a-running-session-is-not-compacted ()
  ;; Its supervisor's socket there when compacting holds the write lock:
  ;; nothing is removed.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let ((romacs-agent-store--db nil) (romacs-agent-events--made nil)
        (romacs-agent-events--imported (make-hash-table :test #'equal)))
    (romacs-agent-events-tests--with-session
     0
     (lambda (session journal)
       (write-region (concat (romacs-agent-events-tests--delta 2 "m1" "Hi")
                             (romacs-agent-events-tests--line
                              3 "out" (romacs-mcp--object "type" "assistant" "parent_tool_use_id" :null
                                                          "message" (romacs-mcp--object "id" "m1" "content" []))))
                     nil journal t 'silent)
       (romacs-agent-events-import session)
       (let ((socket (romacs-agent--host-socket (romacs-agent-id session))))
         (make-directory (file-name-directory socket) t)
         (write-region "" nil socket nil 'silent)
         (should-error (romacs-agent-events-compact (romacs-agent-id session)))
         (should (equal (romacs-agent-events-tests--seqs session) '(1 2 3)))
         (delete-file socket))))))

;;; romacs-agent-events-tests.el ends here
