;;; romacs-agent-tail-tests.el --- A long chat draws its last turns -*- lexical-binding: t; -*-
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
  "SESSION's chat, drawn, point at its end."
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

(ert-deftest romacs-agent-chat-draws-its-last-turns ()
  ;; A chat draws its last turns, under a row that draws earlier ones.
  (romacs-agent-tail-tests--with-session
   5 0
   (lambda (session _journal)
     (let* ((romacs-agent-chat-turns 2) (buffer (romacs-agent-tail-tests--open session)))
       (unwind-protect
           (with-current-buffer buffer
             (should (equal (romacs-agent-tail-tests--questions buffer) '(4 5)))
             (should (string-prefix-p "▴ Earlier turns" (buffer-string)))
             ;; Two more: its first drawn before is at point.
             (goto-char (point-min))
             (romacs-agent-chat-earlier)
             (should (equal (romacs-agent-tail-tests--questions buffer) '(2 3 4 5)))
             (should (looking-at-p "> question 4"))
             ;; A draft written before paging is kept; its undo history,
             ;; whose places the redraw moved, goes.
             (goto-char (point-max))
             (insert "a draft")
             (romacs-agent-chat-earlier)
             (should (equal (romacs-agent-ui--composer-text) "a draft"))
             (should-not buffer-undo-list)
             ;; Then the rest: the row goes.
             (should (equal (romacs-agent-tail-tests--questions buffer) '(1 2 3 4 5)))
             (should-not (string-search "Earlier turns" (buffer-string)))
             (should (string-search "answer 1" (buffer-string))))
         (kill-buffer buffer))))))

(ert-deftest romacs-agent-chat-pages-out-its-oldest-turns ()
  ;; Past twice the turns it keeps, a turn's end drops the oldest, unless
  ;; point is among them.
  (romacs-agent-tail-tests--with-session
   2 0
   (lambda (session _journal)
     (let* ((romacs-agent-chat-turns 2) (buffer (romacs-agent-tail-tests--open session)) (seq 7)
            (turn (lambda (n)
                    (dolist (event (list (list :type 'input :text (format "question %d" n) :request n)
                                         (list :type 'delta :text (format "answer %d" n) :message (format "m%d" n))
                                         (list :type 'message-end :message (format "m%d" n))
                                         (list :type 'done)))
                      (romacs-agent-ui--render session (append event (list :seq (cl-incf seq) :last t)))))))
       (unwind-protect
           (with-current-buffer buffer
             (should (equal (romacs-agent-tail-tests--questions buffer) '(1 2)))
             (funcall turn 3)
             (funcall turn 4)
             (should (equal (romacs-agent-tail-tests--questions buffer) '(1 2 3 4)))
             (funcall turn 5)
             (should (equal (romacs-agent-tail-tests--questions buffer) '(4 5)))
             (should (string-prefix-p "▴ Earlier turns" (buffer-string)))
             (should (string-search "answer 5" (buffer-string)))
             ;; Its rules' fills are all in the buffer: a resize refits them.
             (should romacs-agent-ui--fills)
             (should (seq-every-p #'marker-position romacs-agent-ui--fills))
             (romacs-agent-ui--refit-rules)
             ;; Point among the oldest: they stay.
             (dolist (n '(6 7)) (funcall turn n))
             (goto-char (point-min))
             (funcall turn 8)
             (should (equal (romacs-agent-tail-tests--questions buffer) '(4 5 6 7 8))))
         (kill-buffer buffer))))))

;; Tasks' results arriving are inputs too, each a turn, drawn as cards
;; rather than prompts: the chat counts them as its journal's tail does.
(ert-deftest romacs-agent-chat-counts-arrivals-as-turns ()
  (romacs-agent-tail-tests--with-session
   5 0
   (lambda (session _journal)
     (cl-letf (((symbol-function 'romacs-agent-ui--wakes)
                (lambda (_session text) (and (not (equal text "question 1")) (list text))))
               ((symbol-function 'romacs-agent-ui--arrival)
                (lambda (session event wake)
                  (romacs-agent-ui--line session event (concat "⇠ " wake)))))
       (let* ((romacs-agent-chat-turns 2) (buffer (romacs-agent-tail-tests--open session)))
         (unwind-protect
             (with-current-buffer buffer
               (should (equal (romacs-agent-tail-tests--questions buffer) '(4 5)))
               (goto-char (point-min))
               (romacs-agent-chat-earlier)
               (should (equal (romacs-agent-tail-tests--questions buffer) '(2 3 4 5)))
               (should (looking-at-p "⇠ question 4")))
           (kill-buffer buffer)))))))

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

(ert-deftest romacs-agent-chat-replays-a-streamed-message-whole ()
  (romacs-agent-tail-tests--with-session
   1 0
   (lambda (session journal)
     (let ((coding-system-for-write 'utf-8-unix))
       (write-region (concat (romacs-agent-tail-tests--streamed 1 2) (romacs-agent-tail-tests--streamed 2 8 t))
                     nil journal nil 'silent))
     (let* ((streamed nil)
            (record (symbol-function 'romacs-agent--record))
            (buffer (cl-letf (((symbol-function 'romacs-agent--record)
                               (lambda (session line)
                                 (let ((out (gethash "out" line)))
                                   (when (and (hash-table-p out) (equal (gethash "type" out) "stream_event"))
                                     (push (gethash "api_message_id" out) streamed)))
                                 (funcall record session line))))
                      (romacs-agent-tail-tests--open session))))
       (unwind-protect
           (with-current-buffer buffer
             (let ((text (buffer-substring-no-properties (point-min) romacs-agent-ui--end)))
               (dolist (n '(1 2))
                 (let ((said (format "streamed answer %d" n)))
                   (should (string-search said text))
                   (should-not (string-search said text (1+ (string-search said text)))))))
             ;; The one said whole: its start alone read, none of its
             ;; deltas; the cut one's, all.
             (should (= (cl-count "msg_1" streamed :test #'equal) 1))
             (should (= (cl-count "msg_2" streamed :test #'equal) 3)))
         (kill-buffer buffer))))))

;; Claude writes a record as each block of a message ends: a block done
;; does not make the next one, cut short, whole.
(ert-deftest romacs-agent-chat-replays-a-block-cut-short ()
  (romacs-agent-tail-tests--with-session
   1 0
   (lambda (session journal)
     (let* ((stream (lambda (seq event &optional id)
                      (romacs-agent-tail-tests--line
                       seq "out" (romacs-mcp--object "api_message_id" (or id "msg_9") "event" event
                                                     "parent_tool_use_id" :null "session_id" "s"
                                                     "type" "stream_event" "uuid" "u"))))
            (start (lambda (seq id)
                     (funcall stream seq (romacs-mcp--object "message" (romacs-mcp--object "id" id) "type" "message_start")
                              id)))
            (coding-system-for-write 'utf-8-unix))
       (write-region
        (concat (romacs-agent-tail-tests--line
                 2 "in" (romacs-mcp--object "message" (romacs-mcp--object "content" "question 1" "role" "user")
                                            "romacsRequest" 1 "type" "user"))
                (funcall start 3 "msg_9")
                (funcall stream 4 (romacs-mcp--object "delta" (romacs-mcp--object "thinking" "pondering" "type" "thinking_delta")
                                                      "index" 0 "type" "content_block_delta"))
                (romacs-agent-tail-tests--line
                 5 "out" (romacs-mcp--object
                          "message" (romacs-mcp--object
                                     "content" (vector (romacs-mcp--object "thinking" "pondering" "type" "thinking"))
                                     "id" "msg_9")
                          "parent_tool_use_id" :null "type" "assistant" "uuid" "u"))
                (funcall stream 6 (romacs-mcp--object "delta" (romacs-mcp--object "text" "unfinished reply" "type" "text_delta")
                                                      "index" 1 "type" "content_block_delta"))
                (romacs-agent-tail-tests--line 7 "out" (romacs-mcp--object "result" "cut" "type" "result"))
                ;; Then a reply said whole: its text, once, under its own id.
                (romacs-agent-tail-tests--line
                 8 "in" (romacs-mcp--object "message" (romacs-mcp--object "content" "question 2" "role" "user")
                                            "romacsRequest" 2 "type" "user"))
                (funcall start 9 "msg_10")
                (funcall stream 10 (romacs-mcp--object "delta" (romacs-mcp--object "text" "complete reply" "type" "text_delta")
                                                      "index" 0 "type" "content_block_delta")
                         "msg_10")
                (romacs-agent-tail-tests--line
                 11 "out" (romacs-mcp--object
                           "message" (romacs-mcp--object
                                      "content" (vector (romacs-mcp--object "text" "complete reply" "type" "text"))
                                      "id" "msg_10")
                           "parent_tool_use_id" :null "type" "assistant" "uuid" "u"))
                (romacs-agent-tail-tests--line 12 "out" (romacs-mcp--object "result" "ok" "type" "result")))
        nil journal nil 'silent))
     (let ((buffer (romacs-agent-tail-tests--open session)))
       (unwind-protect
           (with-current-buffer buffer
             (let ((text (buffer-string)))
               (should (string-search "unfinished reply" text))
               (should (string-search "complete reply" text))
               (should-not (string-search "complete reply" text (1+ (string-search "complete reply" text))))))
         (kill-buffer buffer))))))

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

;; A chat made before its supervisor wrote any journal: empty, and no row.
(ert-deftest romacs-agent-chat-without-a-journal ()
  (romacs-agent-tail-tests--with-session
   1 0
   (lambda (session journal)
     (delete-file journal)
     (should (equal (romacs-agent--journal-tail session 2) '(nil . 0)))
     (let* ((romacs-agent-chat-turns 2) (buffer (romacs-agent-tail-tests--open session)))
       (unwind-protect
           (with-current-buffer buffer
             (should-not (string-search "Earlier turns" (buffer-string))))
         (kill-buffer buffer))))))

;; A turn rolled back to, before the turns drawn: all of them dim.
(ert-deftest romacs-agent-chat-dims-a-rollback-past-its-start ()
  (romacs-agent-tail-tests--with-session
   5 0
   (lambda (session _journal)
     (let* ((romacs-agent-chat-turns 2) (buffer (romacs-agent-tail-tests--open session)))
       (unwind-protect
           (with-current-buffer buffer
             (romacs-agent-ui--render session (list :type 'rolled-back :to 1 :turns 5 :prompt "question 1"
                                                    :seq 20 :last t))
             (goto-char (point-min))
             (search-forward "> question 4")
             (should (memq 'romacs-agent-dim (ensure-list (get-text-property (point) 'face)))))
         (kill-buffer buffer))))))

(ert-deftest romacs-agent-chat-undo-moves-with-the-transcript ()
  ;; Undo entries past where the transcript grew move with the text after
  ;; it; those before stay.
  (should (equal (romacs-agent-ui--undo-shifted '(nil (10 . 14) ("ab" . 12) ("cd" . -13) 15 (t . 0) (nil face x 11 . 13) (3 . 5))
                                                10 4)
                 '(nil (14 . 18) ("ab" . 16) ("cd" . -17) 19 (t . 0) (nil face x 15 . 17) (3 . 5)))))

(ert-deftest romacs-agent-chat-undo-shifts-in-place ()
  ;; The list and its entries stay the same conses.
  (let* ((entry (cons 10 14)) (entries (list nil entry 15)))
    (should (eq (romacs-agent-ui--undo-shifted entries 10 4) entries))
    (should (eq (cadr entries) entry))
    (should (equal entries '(nil (14 . 18) 19)))))

(ert-deftest romacs-agent-chat-undo-shift-applies-and-pending ()
  ;; A combined change's bounds move; one that cannot be placed ends the
  ;; list; a pending undo sharing the entries moves them once.
  (let ((entries (list nil (list 'apply 2 12 14 'ignore) 15)))
    (romacs-agent-ui--undo-shifted entries 10 4)
    (should (equal entries '(nil (apply 2 16 18 ignore) 19))))
  (let ((entries (list nil (cons 12 14) (list 'apply 'ignore 1) (cons 20 22))))
    (romacs-agent-ui--undo-shifted entries 10 4)
    (should (equal entries '(nil (16 . 18)))))
  (let* ((shared (cons 12 14)) (entries (list nil shared)) (pending (list shared))
         (seen (make-hash-table :test #'eq)))
    (romacs-agent-ui--undo-shifted entries 10 4 seen)
    (romacs-agent-ui--undo-shifted pending 10 4 seen)
    (should (equal shared '(16 . 18)))))

(ert-deftest romacs-agent-chat-undo-barrier-and-pending-owner ()
  ;; An entry that cannot be placed ends each list holding it, even one
  ;; whose entries were moved already through another.
  (let* ((barrier (list 'apply 'delete-region 12 14))
         (entries (list nil barrier (cons 24 26)))
         (pending (cdr entries))
         (seen (make-hash-table :test #'eq)))
    (setq entries (romacs-agent-ui--undo-shifted entries 10 4 seen))
    (setq pending (romacs-agent-ui--undo-shifted pending 10 4 seen))
    (should (equal entries '(nil)))
    (should-not pending))
  ;; Another buffer's pending undo is left as it is.
  (with-temp-buffer
    (setq-local romacs-agent-ui--end (point-marker))
    (let ((pending-undo-list (list (cons 12 14)))
          (last-command 'undo))
      (romacs-agent-ui--editing (insert "abcd"))
      (should (equal pending-undo-list '((12 . 14))))
      ;; The chat's own, an undo run where it is shown, moves.
      (cl-letf (((symbol-function 'window-buffer) (lambda (&rest _) (current-buffer))))
        (set-marker romacs-agent-ui--end 1)
        (romacs-agent-ui--editing (goto-char 1) (insert "abcd")))
      (should (equal pending-undo-list '((16 . 18)))))))

;; A journal whose last turns reach further back than the budget: the
;; tail starts at the earliest turn within it.
(ert-deftest romacs-agent-journal-tail-keeps-to-its-budget ()
  (romacs-agent-tail-tests--with-session
   5 100000
   (lambda (session journal)
     (let ((romacs-agent-journal-tail-bytes 250000))
       ;; Twelve turns asked, about two fit: the earliest within it.
       (let ((tail (romacs-agent--journal-tail session 12)))
         (should (car tail))
         (should (>= (car tail) (- (file-attribute-size (file-attributes journal)) 250000)))
         (should (memq (cdr tail) '(2 3)))
         ;; Asked for by hand, earlier turns are past it.
         (should (equal (romacs-agent--journal-tail session 12 nil nil 'none) '(nil . 5))))
       ;; In a chat: its first show keeps to it, its earlier row pages past.
       (let* ((romacs-agent-chat-turns 12) (buffer (romacs-agent-tail-tests--open session)))
         (unwind-protect
             (with-current-buffer buffer
               (should (memq (length (romacs-agent-tail-tests--questions buffer)) '(2 3)))
               (romacs-agent-chat-earlier)
               (should (equal (romacs-agent-tail-tests--questions buffer) '(1 2 3 4 5))))
           (kill-buffer buffer)))))))

;; What comes before a session's first prompt (its fork notes): only that.
(ert-deftest romacs-agent-journal-head-is-before-the-first-prompt ()
  (romacs-agent-tail-tests--with-session
   3 0
   (lambda (session journal)
     (should (= (romacs-agent--journal-head session) (romacs-agent-tail-tests--byte-of journal "\"question 1\"")))
     (delete-file journal)
     (write-region "{\"note\":{\"type\":\"run\"},\"seq\":1,\"t\":1}\n" nil journal nil 'silent)
     (should (= (romacs-agent--journal-head session) (file-attribute-size (file-attributes journal)))))))

;; A restored chat a window shows replays its tail, never the whole
;; journal.
(ert-deftest romacs-agent-chat-shown-again-replays-its-tail ()
  (romacs-agent-tail-tests--with-session
   5 0
   (lambda (session _journal)
     (let* ((romacs-agent-chat-turns 2) (froms nil)
            (read (symbol-function 'romacs-agent--read-journal))
            (buffer (romacs-agent-ui--make-chat session)))
       (unwind-protect
           (cl-letf (((symbol-function 'romacs-agent--read-journal)
                      (lambda (session function &optional from &rest rest)
                        (push from froms)
                        (apply read session function from rest))))
             (with-current-buffer buffer (romacs-agent-ui--shown-again))
             (should froms)
             (should (cl-every #'integerp froms))
             (should (equal (romacs-agent-tail-tests--questions buffer) '(4 5))))
         (kill-buffer buffer))))))

;;; romacs-agent-tail-tests.el ends here
