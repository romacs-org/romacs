;;; romacs-agent-log-undo-tests.el --- Undo positions in the chat draft -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

(require 'ert)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent-log))

(ert-deftest romacs-agent-log-shift-undo-entries ()
  (with-temp-buffer
    (let* ((marker (point-marker))
           (unchanged `(nil (t . (1 2)) (,marker . -2) (apply ignore 10)
                         9 (7 . 10) ("before" . -9) (nil face bold 7 . 10)
                         (apply 1 (7 . 10) ignore) (apply 1 7 10 ignore)))
           (entries (append '(10 (10 . 13) ("text" . 10) ("back" . -13)
                              (nil face bold 10 . 13)
                              (apply -1 (10 . 13) ignore arg)
                              (apply -1 10 13 ignore arg))
                            unchanged))
           (shifted (append '(14 (14 . 17) ("text" . 14) ("back" . -17)
                              (nil face bold 14 . 17)
                              (apply -1 (14 . 17) ignore arg)
                              (apply -1 14 17 ignore arg))
                            unchanged))
           (original (copy-tree entries)))
      (should (equal (romacs-agent-log--shift-undo entries 10 4) shifted))
      (should (equal (romacs-agent-log--shift-undo shifted 14 -4) entries))
      (should (equal entries original))
      (should (eq (romacs-agent-log--shift-undo t 10 4) t))
      (should-not (romacs-agent-log--shift-undo nil 10 4)))))

(ert-deftest romacs-agent-log-writing-without-composer-movement ()
  (with-temp-buffer
    (insert "transcript")
    (setq-local romacs-agent-log--input (point-marker))
    (buffer-enable-undo)
    (insert "draft")
    (let ((undo buffer-undo-list)
          (start (marker-position romacs-agent-log--input)))
      (should (consp undo))
      (romacs-agent-log--writing (insert " after the draft"))
      (should (= start (marker-position romacs-agent-log--input)))
      (should (eq buffer-undo-list undo))
      (romacs-agent-log--writing (put-text-property 1 2 'face 'bold))
      (should (= start (marker-position romacs-agent-log--input)))
      (should (eq buffer-undo-list undo)))))

(ert-deftest romacs-agent-log-writing-preserves-redo ()
  (with-temp-buffer
    (insert "transcript\n")
    (setq-local romacs-agent-log--input (point-marker))
    (buffer-enable-undo)
    (insert "draft")
    (undo-boundary)
    (insert " tail")
    (undo-boundary)
    (let ((last-command 'self-insert-command)
          (this-command 'undo)
          (pending-undo-list nil))
      (call-interactively #'undo)
      (romacs-agent-log--writing
        (save-excursion (goto-char (point-min)) (insert "stream\n")))
      (undo-redo)
      (should (equal (buffer-string) "stream\ntranscript\ndraft tail")))))

;;; romacs-agent-log-undo-tests.el ends here
