;;; romacs-agent-eye-tests.el --- A reply reads as it renders -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-log)
  (require 'romacs-agent-ui))

(defun romacs-agent-eye-tests--shown (beg end)
  "BEG..END as the window shows it: invisible text left out."
  (let ((shown ""))
    (while (< beg end)
      (let ((next (next-single-property-change beg 'invisible nil end)))
        (unless (invisible-p beg) (setq shown (concat shown (buffer-substring-no-properties beg next))))
        (setq beg next)))
    shown))

(ert-deftest romacs-agent-eye-a-reply-hides-its-inline-marks ()
  (let ((session (romacs-agent--make :id "E" :harness 'claude :state 'running
                                     :tools (make-hash-table :test #'equal)
                                     :streamed (make-hash-table :test #'equal))))
    (with-temp-buffer
      (romacs-agent-log-mode)
      (setq romacs-agent-log--session session
            romacs-agent-log--end (point-min-marker)
            romacs-agent-log--input (point-min-marker))
      (set-marker-insertion-type romacs-agent-log--end t)
      (romacs-agent-log-draw session '(:type delta :text "## Done\n\nSome **bold**, _em_ and `code`.\n\n```sh\nmake\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |"))
      (romacs-agent-log-draw session '(:type message-end))
      (let ((shown (romacs-agent-eye-tests--shown (point-min) romacs-agent-log--end)))
        ;; Inline marks and the heading's are gone; its words keep their faces.
        (should (string-search "Done\n" shown))
        (should-not (string-search "##" shown))
        (should (string-search "Some bold, em and code." shown))
        ;; Blocks keep their marks: a fence, a table.
        (should (string-search "```sh" shown))
        (should (string-search "| 1 | 2 |" shown)))
      ;; The text itself is whole: copied, it is the Markdown.
      (should (string-search "**bold**" (buffer-substring-no-properties (point-min) romacs-agent-log--end))))))

(ert-deftest romacs-agent-eye-a-mark-no-more-shows-again ()
  (with-temp-buffer
    (insert "a **b**")
    (romacs-markdown-fontify-region (point-min) (point-max))
    (romacs-markdown-hide-markup (point-min) (point-max))
    (should (eq (get-text-property 3 'invisible) 'romacs-markdown-markup))
    ;; Faced again as no mark (its delimiter now text): shown.
    (with-silent-modifications (remove-text-properties (point-min) (point-max) '(face nil)))
    (romacs-markdown-hide-markup (point-min) (point-max))
    (should-not (get-text-property 3 'invisible))
    ;; Another property's invisibility is not ours to take.
    (with-silent-modifications (put-text-property 1 2 'invisible 'fold))
    (romacs-markdown-hide-markup (point-min) (point-max))
    (should (eq (get-text-property 1 'invisible) 'fold))))

(ert-deftest romacs-agent-eye-the-user-stands-behind-a-bar ()
  ;; Each line of the user's message behind a bar in the colour that means
  ;; them; a message from elsewhere behind a thin one; the bar is never
  ;; part of what is copied.
  (let ((session (romacs-agent--make :id "E" :harness 'claude :state 'running
                                     :tools (make-hash-table :test #'equal)
                                     :streamed (make-hash-table :test #'equal))))
    (with-temp-buffer
      (romacs-agent-log-mode)
      (setq romacs-agent-log--session session
            romacs-agent-log--end (point-min-marker)
            romacs-agent-log--input (point-min-marker))
      (set-marker-insertion-type romacs-agent-log--end t)
      (romacs-agent-log-draw session '(:type input :text "first line\nsecond line" :request 1))
      (goto-char (point-min))
      (dolist (words '("first line" "second line"))
        (search-forward words)
        (let ((bol (line-beginning-position)))
          (should (eq (char-after bol) ?▌))
          (should (get-text-property bol 'romacs-agent-decor))
          (should (memq 'romacs-agent-log-you-bar (ensure-list (get-text-property bol 'face))))))
      (should (equal (string-trim (romacs-agent-log--undecorated (buffer-substring (point-min) romacs-agent-log--end)))
                     "❯ first line\nsecond line")))))

(ert-deftest romacs-agent-eye-a-message-from-elsewhere-previews-as-it-reads ()
  ;; Its first lines that say something, without their Markdown marks.
  (let ((session (romacs-agent--make :id "E" :harness 'claude :state 'running
                                     :tools (make-hash-table :test #'equal)
                                     :streamed (make-hash-table :test #'equal))))
    (with-temp-buffer
      (romacs-agent-log-mode)
      (setq romacs-agent-log--session session
            romacs-agent-log--end (point-min-marker)
            romacs-agent-log--input (point-min-marker))
      (set-marker-insertion-type romacs-agent-log--end t)
      (romacs-agent-log-draw session (list :type 'input
                                           :text (concat "<romacs-message id=\"m-1\" kind=\"child\" from=\"lane x\">\n"
                                                         "**Done:** the `fix` landed.\n\n- next\n</romacs-message>\n"
                                                         "Reply to this message with romacs_task_reply(messageId=\"m-1\", text=...).")))
      (let ((text (substring-no-properties (romacs-agent-log--undecorated (buffer-substring (point-min) romacs-agent-log--end)))))
        (should (string-search "← lane x · child\nDone: the fix landed.\n- next" text))
        ;; Its Markdown faces, dimmed over: bold reads bold.
        (goto-char (point-min))
        (search-forward "Done:")
        (let ((faces (ensure-list (get-text-property (match-beginning 0) 'face))))
          (should (memq 'romacs-markdown-strong faces))
          (should (memq 'romacs-agent-log-other faces)))
        (should-not (string-search "**" text))))))

(ert-deftest romacs-agent-eye-a-sender-is-cut-at-a-word ()
  (should (equal (romacs-agent-log-rounds--words "lane chat (claude3): agents ADE showcase: visible work" 30)
                 (concat "lane chat (claude3): agents" (truncate-string-ellipsis))))
  (should (equal (romacs-agent-log-rounds--words "short" 30) "short"))
  ;; One word longer than the room: cut inside it, as nothing else can.
  (should (<= (string-width (romacs-agent-log-rounds--words (make-string 50 ?x) 30)) 30)))

(provide 'romacs-agent-eye-tests)
;;; romacs-agent-eye-tests.el ends here
