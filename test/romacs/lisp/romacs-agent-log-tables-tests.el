;;; romacs-agent-log-tables-tests.el --- A chat reply's tables as grids -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-log))

(defmacro romacs-agent-log-tables-tests--with (width &rest body)
  "BODY in a chat whose windows leave WIDTH columns (a variable `width'
BODY may set), its events drawn by `draw', a reply by `reply'."
  (declare (indent 1))
  `(let ((width ,width)
         (actual-width (symbol-function 'romacs-agent-log-tables--window-width))
         (session (romacs-agent--make :id "T" :harness 'claude :state 'running
                                      :tools (make-hash-table :test #'equal)
                                      :streamed (make-hash-table :test #'equal))))
     (with-temp-buffer
       (romacs-agent-log-mode)
       (setq romacs-agent-log--session session
             romacs-agent-log--end (point-min-marker)
             romacs-agent-log--input (point-min-marker))
       (set-marker-insertion-type romacs-agent-log--end t)
       (cl-letf (((symbol-function 'romacs-agent-log-tables--window-width) (lambda () (or width (funcall actual-width)))))
         (cl-flet* ((draw (&rest event) (romacs-agent-log-draw session event))
                    (reply (text) (draw :type 'delta :text text) (draw :type 'message-end))
                    (transcript () (buffer-substring-no-properties (point-min) romacs-agent-log--end)))
           ,@body)))))

(defconst romacs-agent-log-tables-tests--table
  "| Name | Cost |\n|---|---|\n| `ls` | **low** |\n")

(ert-deftest romacs-agent-log-tables-a-reply-draws-its-table-as-a-grid ()
  (romacs-agent-log-tables-tests--with 70
    (reply (concat "Here it is:\n\n" romacs-agent-log-tables-tests--table "\nThat's all."))
    (let ((text (transcript)))
      (should-not (string-match-p "|---|" text))
      (should-not (string-match-p "[*`]" text))
      (should (string-match-p "^\\+-" text))
      (should (string-match-p "That's all\\." text)))
    ;; The grid keeps its cells' Markdown faces, its source on RET's side.
    (goto-char (point-min))
    (search-forward "ls")
    (should (memq 'romacs-markdown-code (ensure-list (get-text-property (1- (point)) 'face))))
    (search-forward "low")
    (should (memq 'romacs-markdown-strong (ensure-list (get-text-property (1- (point)) 'face))))
    (should (equal (car (get-text-property (1- (point)) 'romacs-agent-log-table))
                   romacs-agent-log-tables-tests--table))))

(ert-deftest romacs-agent-log-tables-a-table-refits-as-the-window-resizes ()
  (romacs-agent-log-tables-tests--with 70
    (let ((table "| A | B |\n|---|---|\n| one | a rather long cell of text that has to wrap somewhere |\n"))
      (reply table)
      (reply (concat "Again:\n\n" table))
      (let ((wide (transcript)))
        (setq width 30)
        (romacs-agent-log-tables-refit)
        (let ((narrow (transcript)))
          (should-not (equal narrow wide))
          ;; Every line of both grids within the narrower width.
          (dolist (line (split-string narrow "\n"))
            (when (string-match-p "\\`[+|]" line) (should (<= (string-width line) 28)))))
        (setq width 70)
        (romacs-agent-log-tables-refit)
        (should (equal (transcript) wide))))))

(ert-deftest romacs-agent-log-tables-a-table-drawn-hidden-fits-the-window-showing-it ()
  (romacs-agent-log-tables-tests--with nil
    (ignore #'transcript)
    (reply (concat "| Name | Cost |\n|---|---|\n| " (make-string 120 ?a) " | low |\n"))
    (let ((drawn (lambda () (nth 1 (get-text-property (text-property-not-all (point-min) (point-max) 'romacs-agent-log-table nil)
                                                       'romacs-agent-log-table)))))
      (should (= (funcall drawn) 78))
      (save-window-excursion
        (delete-other-windows)
        (let ((window (split-window-right)))
          (set-window-buffer window (current-buffer))
          (romacs--run-window-change-functions)
          (should (= (funcall drawn) (- (window-body-width window) 2))))))))

(ert-deftest romacs-agent-log-tables-a-table-keeps-its-reply-s-marks ()
  ;; A reply that opens with its table: its entry still starts there, on
  ;; its first character alone, so n and p find it; it stays sealed.
  (romacs-agent-log-tables-tests--with 70
    (reply romacs-agent-log-tables-tests--table)
    (let ((start (text-property-any (point-min) romacs-agent-log--end 'romacs-agent-log-entry 'text)))
      (should start)
      (should (get-text-property start 'romacs-agent-log-table))
      (should-not (get-text-property (+ start 5) 'romacs-agent-log-entry))
      (should (get-text-property (+ start 5) 'read-only)))))

(ert-deftest romacs-agent-log-tables-no-table-no-change ()
  (romacs-agent-log-tables-tests--with 70
    (reply "Plain words | with a pipe, but no table.")
    (should (string-match-p "with a pipe" (transcript)))
    (should-not (text-property-not-all (point-min) romacs-agent-log--end 'romacs-agent-log-table nil))))

(ert-deftest romacs-agent-log-tables-resize-preserves-selection-and-copy ()
  (romacs-agent-log-tables-tests--with nil
    (save-window-excursion
      (let ((chat (current-buffer)))
        (delete-other-windows)
        (switch-to-buffer chat))
      (romacs-agent-log--separator t)
      (let ((table "| Old | New |\n|---|---|\n| selected | a long description of the changes made in this revision |"))
        (reply table)
        (romacs--run-window-change-functions)
        (goto-char (point-min))
        (search-forward "selected")
        (set-mark (- (point) 8))
        (let ((wide (transcript)))
          (split-window-right)
          (romacs--run-window-change-functions)
          (should-not (equal (transcript) wide))
          (should (equal (filter-buffer-substring (mark) (point)) "selected"))
          (romacs-agent-log-copy)
          (should (equal (current-kill 0) table)))))))

(ert-deftest romacs-agent-log-tables-refit-preserves-narrowed-rolled-back-table ()
  (romacs-agent-log-tables-tests--with 70
    (reply romacs-agent-log-tables-tests--table)
    (let ((wide (transcript)))
      (with-silent-modifications (romacs-agent-log-turns--dim (point-min) romacs-agent-log--end))
      (save-restriction
        (narrow-to-region (+ (point-min) 6) (- romacs-agent-log--end 5))
        (setq width 25)
        (romacs-agent-log-tables-refit))
      (setq width 70)
      (romacs-agent-log-tables-refit)
      (should (equal (transcript) wide))
      (should (memq 'romacs-agent-log-note (ensure-list (get-text-property (point-min) 'face)))))))

(provide 'romacs-agent-log-tables-tests)
;;; romacs-agent-log-tables-tests.el ends here
