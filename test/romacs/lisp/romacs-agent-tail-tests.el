;;; romacs-agent-tail-tests.el --- A long chat draws its last turns -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'romacs-agent-test-stored)
(require 'cl-lib)
(require 'romacs-mcp)
(require 'romacs-test-socket)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-ui))

(defconst romacs-agent-tail-tests--id "7e570000-2222-4333-8444-555566667777")

(defun romacs-agent-tail-tests--line (seq key value)
  "Record SEQ's line as the supervisor writes it: keys sorted."
  (concat (json-serialize (romacs-mcp--object key value "seq" seq "t" (+ 1794600000.25 seq))) "\n"))

(defun romacs-agent-tail-tests--turn (n seq &optional padding unended)
  "Turn N's lines from SEQ: its prompt, its answer (PADDING characters
more, or 100001 of its character when it is (char . C)), its result, unless UNENDED
\(the next prompt steers it)."
  (concat (romacs-agent-tail-tests--line
           seq "in" (romacs-mcp--object "message" (romacs-mcp--object "content" (format "question %d" n) "role" "user")
                                        "romacsRequest" n "type" "user"))
          (romacs-agent-tail-tests--line
           (+ seq 1) "out" (romacs-mcp--object
                            "message" (romacs-mcp--object
                                       "content" (vector (romacs-mcp--object
                                                          "text" (concat (format "answer %d" n)
                                                                         (if (consp padding) (make-string 100001 (cdr padding))
                                                                           (make-string (or padding 0) ?.)))
                                                          "type" "text"))
                                       "id" (format "m%d" n))
                            "parent_tool_use_id" :null "type" "assistant"))
          (if unended ""
            (romacs-agent-tail-tests--line
             (+ seq 2) "out" (romacs-mcp--object "result" (format "answer %d" n) "type" "result")))))

(defun romacs-agent-tail-tests--with-session (turns padding body &optional unended)
  "Call BODY with a session whose journal holds TURNS turns, each answer
PADDING characters long, and its journal's file; turn UNENDED has no
result."
  (let* ((root (make-temp-file "romacs-tail-" t))
         (run (romacs-test-socket-directory "tail-"))
         (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state")
                                            (concat "XDG_RUNTIME_DIR=" run))
                                      process-environment))
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (dir (expand-file-name romacs-agent-tail-tests--id (romacs-agent--state-root)))
         (journal (expand-file-name "journal.jsonl" dir)))
    ;; Its id is every test's: no other's session is taken as moved.
    (remhash romacs-agent-tail-tests--id romacs-agent-events--imported)
    (unwind-protect
        (progn
          (make-directory dir t)
          (romacs-agent--write-json (expand-file-name "manifest.json" dir)
                                    (romacs-mcp--object "id" romacs-agent-tail-tests--id "harness" "claude"
                                                        "provider" "claude" "cwd" root "title" "tail" "titleAuto" :false
                                                        "argv" [] "options" (romacs-mcp--object) "seenSeq" 0
                                                        "hostSocket" (expand-file-name "none/host.sock" root)))
          (let ((coding-system-for-write 'utf-8-unix))
            (write-region (concat (romacs-agent-tail-tests--line 1 "note" (romacs-mcp--object "type" "run"))
                                  (mapconcat (lambda (n) (romacs-agent-tail-tests--turn n (+ 2 (* 3 (1- n))) padding (eql n unended)))
                                             (number-sequence 1 turns) ""))
                          nil journal nil 'silent))
          (cl-letf (((symbol-function 'romacs-agent--keep-models) #'ignore))
            (romacs-agent-restore)
            (funcall body (romacs-agent-get romacs-agent-tail-tests--id) journal)))
      (remhash romacs-agent-tail-tests--id romacs-agent-events--imported)
      (delete-directory root t)
      (delete-directory run t))))

(defun romacs-agent-tail-tests--byte-of (journal text)
  "The byte at which JOURNAL's line holding TEXT starts."
  (with-temp-buffer
    (set-buffer-multibyte nil)
    (insert-file-contents-literally journal)
    (search-forward text)
    (1- (line-beginning-position))))

(ert-deftest romacs-agent-journal-tail-finds-the-last-prompts ()
  ;; From the journal's end, as far back as its last prompts go: a span
  ;; begun inside a record (answers longer than the first span) is no
  ;; prompt's line.
  ;; Answers of two- and three-byte characters put spans' starts inside
  ;; them: a byte is still a byte.
  (dolist (padding '(0 100000 (char . ?é) (char . ?漢)))
    (romacs-agent-tail-tests--with-session
     5 padding
     (lambda (session journal)
       (should (equal (romacs-agent--journal-tail session 2)
                      (cons (romacs-agent-tail-tests--byte-of journal "\"question 4\"") 2)))
       (should (equal (romacs-agent--journal-tail session 4)
                      (cons (romacs-agent-tail-tests--byte-of journal "\"question 2\"") 4)))
       ;; No more than that many: the whole journal, and how many it holds.
       (should (equal (romacs-agent--journal-tail session 5) '(nil . 5)))
       (should (equal (romacs-agent--journal-tail session 9) '(nil . 5)))))))

(ert-deftest romacs-agent-journal-tail-starts-where-no-turn-is-under-way ()
  ;; Prompt 4 came while turn 3 went on: a replay from it would parse
  ;; turn 3's end without its start, so the tail starts at prompt 3.
  (romacs-agent-tail-tests--with-session
   5 0
   (lambda (session journal)
     (should (equal (romacs-agent--journal-tail session 2)
                    (cons (romacs-agent-tail-tests--byte-of journal "\"question 3\"") 3))))
   3))

(ert-deftest romacs-agent-prompt-line-end-fits-every-harness ()
  ;; Claude's prompt, Codex's turn start and its steering: the request id
  ;; Romacs sent it under; a tool's output quoting one is no prompt.
  (dolist (line '("{\"in\":{\"message\":{\"content\":\"hi\",\"role\":\"user\"},\"romacsRequest\":7,\"type\":\"user\"},\"seq\":12,\"t\":1794662969.3}"
                  "{\"in\":{\"id\":\"turn/start:4\",\"method\":\"turn/start\",\"params\":{\"input\":[]},\"romacsRequest\":4},\"seq\":21,\"t\":1794662969.3056054}"))
    (should (string-match-p romacs-agent--prompt-line-end line)))
  (should-not (string-match-p romacs-agent--prompt-line-end
                              "{\"out\":{\"text\":\"\\\"romacsRequest\\\":7},\"},\"seq\":13,\"t\":1.5}")))

(defun romacs-agent-tail-tests--open (session)
  "SESSION's chat, drawn as a chat finds it (`romacs-agent-test-stored'),
point at its end."
  (romacs-agent-test-stored session)
  (let ((buffer (romacs-agent-ui--make-chat session)))
    (with-current-buffer buffer
      (romacs-agent-ui--fill buffer)
      (goto-char (point-max)))
    buffer))

(defun romacs-agent-tail-tests--questions (buffer)
  "The questions BUFFER's chat draws, in order."
  (with-current-buffer buffer
    (let ((found nil) (at (point-min)))
      (while (string-match "question \\([0-9]+\\)" (buffer-string) at)
        (push (string-to-number (match-string 1 (buffer-string))) found)
        (setq at (match-end 0)))
      (nreverse found))))

;; A message streamed, then said whole: the replay passes over its
;; deltas, unparsed, and draws it once.  One cut short has no whole
;; record: its deltas are what it said, and are read.
(defun romacs-agent-tail-tests--streamed (n seq &optional cut)
  "Turn N from SEQ, its answer streamed in two deltas, then its record
unless CUT, then its turn's end."
  (let* ((id (format "msg_%d" n))
         (stream (lambda (seq event)
                   (romacs-agent-tail-tests--line
                    seq "out" (romacs-mcp--object "api_message_id" id "event" event
                                                  "parent_tool_use_id" :null "session_id" "s" "type" "stream_event"
                                                  "uuid" "u"))))
         (delta (lambda (seq text)
                  (funcall stream seq (romacs-mcp--object "delta" (romacs-mcp--object "text" text "type" "text_delta")
                                                          "index" 0 "type" "content_block_delta")))))
    (concat (romacs-agent-tail-tests--line
             seq "in" (romacs-mcp--object "message" (romacs-mcp--object "content" (format "question %d" n) "role" "user")
                                          "romacsRequest" n "type" "user"))
            (funcall stream (+ seq 1) (romacs-mcp--object "message" (romacs-mcp--object "id" id) "type" "message_start"))
            (funcall delta (+ seq 2) "streamed ")
            (funcall delta (+ seq 3) (format "answer %d" n))
            (if cut ""
              (romacs-agent-tail-tests--line
               (+ seq 4) "out" (romacs-mcp--object
                                "message" (romacs-mcp--object
                                           "content" (vector (romacs-mcp--object "text" (format "streamed answer %d" n)
                                                                                 "type" "text"))
                                           "id" id)
                                "parent_tool_use_id" :null "type" "assistant" "uuid" "u")))
            (romacs-agent-tail-tests--line
             (+ seq 5) "out" (romacs-mcp--object "result" "ok" "type" "result")))))

(ert-deftest romacs-agent-journal-tail-bytes-past-odd-lines ()
  ;; A line ending in a carriage return, one with a byte that is no
  ;; UTF-8: the tail's byte is still the prompt's.
  (romacs-agent-tail-tests--with-session
   3 0
   (lambda (session journal)
     (let ((coding-system-for-write 'no-conversion))
       (write-region (concat "{\"note\":{\"type\":\"x\"},\"seq\":90,\"t\":1}\r\n" (unibyte-string #xff) "\n")
                     nil journal t 'silent))
     (let ((coding-system-for-write 'utf-8-unix))
       (write-region (mapconcat (lambda (n) (romacs-agent-tail-tests--turn n (+ 100 (* 3 n)))) '(4 5) "")
                     nil journal t 'silent))
     (should (equal (romacs-agent--journal-tail session 2)
                    (cons (romacs-agent-tail-tests--byte-of journal "\"question 4\"") 2))))))

;; What comes before a session's first prompt (its fork notes): only that.
(ert-deftest romacs-agent-journal-head-is-before-the-first-prompt ()
  (romacs-agent-tail-tests--with-session
   3 0
   (lambda (session journal)
     (should (= (romacs-agent--journal-head session) (romacs-agent-tail-tests--byte-of journal "\"question 1\"")))
     (delete-file journal)
     (write-region "{\"note\":{\"type\":\"run\"},\"seq\":1,\"t\":1}\n" nil journal nil 'silent)
     (should (= (romacs-agent--journal-head session) (file-attribute-size (file-attributes journal)))))))

;;; romacs-agent-tail-tests.el ends here
