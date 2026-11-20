;;; romacs-agent-log-faces.el --- A chat's reply faced as it streams  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A reply streaming into a chat (romacs-agent-log.el) gets its Markdown
;; faces as it comes, not at its end: once a batch of its text is in (a
;; timer at no delay, which every delta of one read shares), it is read
;; as a document from a top-level block before where it last ended, and
;; the blocks from there written again (`romacs-markdown-fontify-region'
;; with FROM): a batch costs what its last blocks hold, however long the
;; reply.  Its end refaces what is left.  Should faces fail, the reply keeps its text and says so
;; under it; it is not faced again.

;;; Code:

(require 'romacs-agent-log)
(require 'romacs-markdown)

(defvar-local romacs-agent-log-faces--current nil
  "Where the streaming reply's faces may next change, a marker: the start
of a top-level block, before which its faces are final; nil for its
start.")

(defvar-local romacs-agent-log-faces--end nil
  "A marker at where the reply's text ended when it was last faced, or
nil.")

(defvar-local romacs-agent-log-faces--whole nil
  "Whether the streaming reply defines a link reference, so it is read
whole each time: a reference anywhere in it may name that definition.")

(defconst romacs-agent-log-faces--definition "^ \\{0,3\\}\\[[^]\n]+\\]:"
  "A line that defines a Markdown link reference.")

(defvar-local romacs-agent-log-faces--failed nil
  "Why the streaming reply's faces failed, or nil.")

(defvar-local romacs-agent-log-faces--timer nil
  "The timer that faces the reply's next batch, while one waits.")

(defun romacs-agent-log-faces--reface (start end)
  "Face START..END, the streaming reply, from where its faces may change:
read as a document from the top-level block there, the blocks from the
one before where it last ended written again; keep why, should it fail."
  (unless romacs-agent-log-faces--failed
    (unless romacs-agent-log-faces--whole
      (save-excursion
        (goto-char (or romacs-agent-log-faces--current start))
        (setq romacs-agent-log-faces--whole (re-search-forward romacs-agent-log-faces--definition end t))))
    (condition-case err
        (romacs-agent-log--writing
          (let ((from (if romacs-agent-log-faces--whole
                          ;; Read whole: a definition reaches back as well.
                          (progn (romacs-markdown-fontify-region start end) start)
                        (romacs-markdown-fontify-region (or romacs-agent-log-faces--current start) end
                                                        romacs-agent-log-faces--end))))
            (setq romacs-agent-log-faces--current (set-marker (or romacs-agent-log-faces--current (make-marker)) from)
                  romacs-agent-log-faces--end (set-marker (or romacs-agent-log-faces--end (make-marker)) end))))
      (error (setq romacs-agent-log-faces--failed (error-message-string err))
             (message "The chat's reply is shown without its faces: %s" romacs-agent-log-faces--failed)))))

(defun romacs-agent-log-faces--batch (buffer)
  "Face what streamed into BUFFER's reply since its last batch."
  (when (buffer-live-p buffer)
    (with-current-buffer buffer
      ;; Called before its timer ran: that one is not needed.
      (when (timerp romacs-agent-log-faces--timer) (cancel-timer romacs-agent-log-faces--timer))
      (setq romacs-agent-log-faces--timer nil)
      (pcase romacs-agent-log--stream
        (`(text ,start ,at . ,_) (romacs-agent-log-faces--reface start at))))))

(defun romacs-agent-log-faces-soon ()
  "Face the streaming reply's text once the batch it came in is in."
  (unless romacs-agent-log-faces--timer
    (setq romacs-agent-log-faces--timer (run-at-time 0 nil #'romacs-agent-log-faces--batch (current-buffer)))))

(defun romacs-agent-log-faces-end (start at)
  "The reply START..AT ends: its faces for what is left, or, should they
have failed, a line under it that says why."
  (when (timerp romacs-agent-log-faces--timer) (cancel-timer romacs-agent-log-faces--timer))
  (setq romacs-agent-log-faces--timer nil)
  ;; Faced as far as it went already: nothing is left.
  (unless (and romacs-agent-log-faces--end (= romacs-agent-log-faces--end at))
    (romacs-agent-log-faces--reface start at))
  (when romacs-agent-log-faces--failed
    (romacs-agent-log--writing
      (save-excursion
        (goto-char at)
        (insert (romacs-agent-log--sealed
                 (propertize (concat "\n⚠ shown without its faces: " romacs-agent-log-faces--failed)
                             'face 'romacs-agent-log-denied))))))
  (romacs-agent-log-faces-drop))

(defun romacs-agent-log-faces-drop ()
  "Forget the streaming reply's faces: its timer, markers and state."
  (when (timerp romacs-agent-log-faces--timer) (cancel-timer romacs-agent-log-faces--timer))
  (when romacs-agent-log-faces--current (set-marker romacs-agent-log-faces--current nil))
  (when romacs-agent-log-faces--end (set-marker romacs-agent-log-faces--end nil))
  (setq romacs-agent-log-faces--timer nil romacs-agent-log-faces--current nil romacs-agent-log-faces--end nil
        romacs-agent-log-faces--failed nil romacs-agent-log-faces--whole nil))

;;; A command's output

(defcustom romacs-agent-log-faces-output-patterns
  '(("\\(?:^\\|[ (\"'`]\\)\\([~.]?[[:alnum:]_./-]*[[:alnum:]_-]\\.[[:alnum:]]+:[0-9]+\\(?::[0-9]+\\)?\\)" 1 link)
    ("\\_<\\(?:error\\|ERROR\\|failed\\|FAILED\\|FAIL\\|panicked\\|fatal\\)\\_>.*" 0 error)
    ("\\_<\\(?:warning\\|WARNING\\|warn\\)\\_>" 0 warning)
    ("\\_<\\(?:ok\\|OK\\|PASS\\|passed\\|done\\)\\_>" 0 success)
    ("\\_<[0-9a-f]\\{7,40\\}\\_>" 0 font-lock-constant-face))
  "What a command's output shown under its call is faced by: each
\(REGEXP GROUP FACE), the GROUP of each match of REGEXP in FACE, in front
of the output's own.  A file:line reference, an error line, a warning, a
success word and a commit hash, by default."
  :type '(repeat (list regexp integer face))
  :group 'romacs-agent)

(defun romacs-agent-log-faces-output (text)
  "TEXT, a command's output as shown, with the faces of
`romacs-agent-log-faces-output-patterns' in front of its own."
  (let ((text (copy-sequence text)))
    (pcase-dolist (`(,regexp ,group ,face) romacs-agent-log-faces-output-patterns)
      (let ((at 0))
        (while (and (< at (length text)) (string-match regexp text at))
          (when (match-beginning group)
            (add-face-text-property (match-beginning group) (match-end group) face nil text))
          (setq at (max (1+ at) (match-end 0))))))
    text))

(provide 'romacs-agent-log-faces)
;;; romacs-agent-log-faces.el ends here
