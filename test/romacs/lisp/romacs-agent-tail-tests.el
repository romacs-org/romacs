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

(defun romacs-agent-tail-tests--turn (n seq &optional padding)
  "Turn N's lines from SEQ: its prompt, its answer (PADDING characters
more), its result."
  (concat (romacs-agent-tail-tests--line
           seq "in" (romacs-mcp--object "message" (romacs-mcp--object "content" (format "question %d" n) "role" "user")
                                        "romacsRequest" n "type" "user"))
          (romacs-agent-tail-tests--line
           (+ seq 1) "out" (romacs-mcp--object
                            "message" (romacs-mcp--object
                                       "content" (vector (romacs-mcp--object
                                                          "text" (concat (format "answer %d" n) (make-string (or padding 0) ?.))
                                                          "type" "text"))
                                       "id" (format "m%d" n))
                            "parent_tool_use_id" :null "type" "assistant"))
          (romacs-agent-tail-tests--line
           (+ seq 2) "out" (romacs-mcp--object "result" (format "answer %d" n) "type" "result"))))

(defun romacs-agent-tail-tests--with-session (turns padding body)
  "Call BODY with a session whose journal holds TURNS turns, each answer
PADDING characters long, and its journal's file."
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
                                  (mapconcat (lambda (n) (romacs-agent-tail-tests--turn n (+ 2 (* 3 (1- n))) padding))
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
  (dolist (padding '(0 100000))
    (romacs-agent-tail-tests--with-session
     5 padding
     (lambda (session journal)
       (should (= (romacs-agent--journal-tail session 2)
                  (romacs-agent-tail-tests--byte-of journal "\"question 4\"")))
       (should (= (romacs-agent--journal-tail session 4)
                  (romacs-agent-tail-tests--byte-of journal "\"question 2\"")))
       ;; No more than that many: the whole journal.
       (should-not (romacs-agent--journal-tail session 5))
       (should-not (romacs-agent--journal-tail session 9))))))

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
             ;; Then the rest: the row goes.
             (romacs-agent-chat-earlier)
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
             ;; Point among the oldest: they stay.
             (dolist (n '(6 7)) (funcall turn n))
             (goto-char (point-min))
             (funcall turn 8)
             (should (equal (romacs-agent-tail-tests--questions buffer) '(4 5 6 7 8))))
         (kill-buffer buffer))))))

;;; romacs-agent-tail-tests.el ends here
