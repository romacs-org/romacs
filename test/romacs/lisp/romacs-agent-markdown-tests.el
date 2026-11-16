;;; romacs-agent-markdown-tests.el --- Replies rendered as Markdown -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-mcp)
(require 'romacs-test-socket)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))
(require 'romacs-agent-ui)

(defmacro romacs-agent-markdown-tests--with-chat (var &rest body)
  "BODY in a chat of a session VAR, its directory temporary."
  (declare (indent 1))
  `(let* ((dir (make-temp-file "romacs-agent-markdown" t))
          (,var (romacs-agent--make :id "11111111-2222-4333-8444-777777777777" :harness 'claude :state 'idle
                                    :title "chat" :dir dir :tools (make-hash-table :test #'equal))))
     (unwind-protect
         (with-temp-buffer
           (romacs-agent-chat-mode)
           (setq romacs-agent-ui--session ,var
                 romacs-agent-ui--messages (make-hash-table :test #'equal)
                 romacs-agent-ui--end (point-marker)
                 romacs-agent-ui--composer (make-overlay (point) (point) nil nil nil))
           ,@body)
       (delete-directory dir t))))

(defun romacs-agent-markdown-tests--stream (session text &optional chunk)
  "Draw TEXT as message m1's deltas of CHUNK characters, not yet ended."
  (let ((at 0) (seq 0) (chunk (or chunk 5)))
    (while (< at (length text))
      (romacs-agent-ui--draw session (list :type 'delta :text (substring text at (min (length text) (+ at chunk)))
                                           :message "m1" :seq (cl-incf seq))
                             (current-buffer))
      (setq at (+ at chunk)))))

(defun romacs-agent-markdown-tests--face (needle)
  (save-excursion
    (goto-char (point-min))
    (search-forward needle)
    (get-text-property (match-beginning 0) 'face)))

(defconst romacs-agent-markdown-tests--reply
  "## Plan\n\nRun `make` then **check**.\n\n```sh\nmake check\n```\n\n- one\n- two\n")

(ert-deftest romacs-agent-markdown-reply-is-faced-when-it-ends ()
  (romacs-agent-markdown-tests--with-chat session
    (cl-letf (((symbol-function 'romacs-agent-ui--seen-p) #'ignore))
      (romacs-agent-markdown-tests--stream session romacs-agent-markdown-tests--reply)
      ;; Unseen, nothing is faced while it streams.
      (should-not (romacs-agent-markdown-tests--face "Plan"))
      (romacs-agent-ui--draw session (list :type 'message-end :message "m1" :seq 100) (current-buffer)))
    (should (equal (romacs-agent-markdown-tests--face "Plan") 'romacs-markdown-heading-2))
    (should (equal (romacs-agent-markdown-tests--face "## ") '(romacs-markdown-markup romacs-markdown-heading-2)))
    (should (equal (romacs-agent-markdown-tests--face "`make`") '(romacs-markdown-markup romacs-markdown-code)))
    (should (equal (romacs-agent-markdown-tests--face "check**") 'romacs-markdown-strong))
    (should (equal (romacs-agent-markdown-tests--face "sh\n") '(romacs-markdown-language romacs-markdown-code-block)))
    (should (eq (romacs-agent-markdown-tests--face "- one") 'romacs-markdown-list-marker))
    ;; The text is the reply's, as written, and stays sealed.
    (should (string-match-p (regexp-quote romacs-agent-markdown-tests--reply) (buffer-string)))
    (should (get-text-property (1+ (string-match "Run" (buffer-string))) 'read-only))))

(ert-deftest romacs-agent-markdown-reply-is-faced-live-when-seen ()
  "Seen, a streaming reply is faced as it arrives, and ends faced as a
reply faced whole."
  (let (live whole)
    (romacs-agent-markdown-tests--with-chat session
      (cl-letf (((symbol-function 'romacs-agent-ui--seen-p) (lambda () t)))
        (romacs-agent-markdown-tests--stream session "## Plan\n\nRun `make` then **ch")
        (should (equal (romacs-agent-markdown-tests--face "Plan") 'romacs-markdown-heading-2))
        (should (equal (romacs-agent-markdown-tests--face "make`") 'romacs-markdown-code))
        (romacs-agent-markdown-tests--stream session "eck**.\n\n```sh\nmake check\n```\n\n- one\n- two\n")
        (romacs-agent-ui--draw session (list :type 'message-end :message "m1" :seq 100) (current-buffer))
        (setq live (buffer-string))))
    (romacs-agent-markdown-tests--with-chat session
      (cl-letf (((symbol-function 'romacs-agent-ui--seen-p) #'ignore))
        (romacs-agent-markdown-tests--stream session romacs-agent-markdown-tests--reply 1000)
        (romacs-agent-ui--draw session (list :type 'message-end :message "m1" :seq 100) (current-buffer))
        (setq whole (buffer-string))))
    (should (equal-including-properties live whole))))

(ert-deftest romacs-agent-markdown-faces-only-the-reply ()
  (romacs-agent-markdown-tests--with-chat session
    (romacs-agent-ui--draw session (list :type 'input :text "say *hi*" :seq 1) (current-buffer))
    (cl-letf (((symbol-function 'romacs-agent-ui--seen-p) (lambda () t)))
      (romacs-agent-ui--draw session (list :type 'delta :text "*hi*" :message "m1" :seq 2) (current-buffer))
      (romacs-agent-ui--draw session (list :type 'message-end :message "m1" :seq 3) (current-buffer)))
    ;; The prompt keeps its own face; the reply's emphasis is Markdown's.
    (should (eq (get-text-property (1+ (string-match "say" (buffer-string))) 'face) 'romacs-agent-prompt))
    (goto-char (point-max))
    (search-backward "hi*")
    (should (eq (get-text-property (point) 'face) 'romacs-markdown-emphasis))))

(ert-deftest romacs-agent-markdown-table-grid-keeps-cell-faces ()
  (let ((grid (romacs-markdown-table-grid "| Name | Cost |\n|---|---|\n| `ls` | **low** |\n" 40)))
    (should (string-prefix-p "+-" grid))
    (should-not (string-match-p "|---" grid))
    (let ((name (string-match "Name" grid)) (code (string-match "`ls`" grid)) (low (string-match "low" grid)))
      (should (memq 'romacs-markdown-table-header (ensure-list (get-text-property name 'face grid))))
      (should (memq 'romacs-markdown-code (ensure-list (get-text-property (1+ code) 'face grid))))
      (should (memq 'romacs-markdown-strong (ensure-list (get-text-property low 'face grid))))
      (should (memq 'romacs-markdown-markup (ensure-list (get-text-property 0 'face grid)))))
    (should-not (romacs-markdown-table-grid "no table here" 40))))

(provide 'romacs-agent-markdown-tests)
;;; romacs-agent-markdown-tests.el ends here
