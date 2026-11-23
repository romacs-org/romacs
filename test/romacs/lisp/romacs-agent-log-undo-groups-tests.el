;;; romacs-agent-log-undo-groups-tests.el --- Undo across transcript writes -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

(require 'cl-lib)
(require 'ert)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent-log))

(ert-deftest romacs-agent-log-transcript-motion-splits-undo-group ()
  (dolist (automatic '(nil t))
    (with-temp-buffer
      (insert "Transcript\nad")
      (setq-local romacs-agent-log--input (copy-marker (- (point) 2)))
      (buffer-enable-undo)
      (backward-char)
      (insert "X")
      (when automatic
        (let ((undo-auto--undoably-changed-buffers (list (current-buffer))))
          (undo-auto--ensure-boundary 'amalgamate))
        (should (consp undo-auto--last-boundary-cause)))
      (romacs-agent-log--writing
        (save-excursion (delete-region (point-min) (1+ (point-min)))))
      (goto-char (point-max))
      (let ((this-command 'self-insert-command) (last-command 'self-insert-command)
            (undo-auto--this-command-amalgamating nil))
        (undo-auto-amalgamate)
        (insert "Y"))
      (undo-boundary)
      (let ((last-command 'self-insert-command)
            (this-command 'undo))
        (call-interactively #'undo)
        (should (equal (buffer-string) "ranscript\naXd"))
        (let ((last-command 'undo))
          (call-interactively #'undo)
          (should (equal (buffer-string) "ranscript\nad")))))))

(ert-deftest romacs-agent-log-undo-visits-history-once ()
  (with-temp-buffer
    (insert "\n")
    (setq-local romacs-agent-log--input (point-marker))
    (buffer-enable-undo)
    (dotimes (_ 1000)
      (insert "x")
      (undo-boundary)
      (romacs-agent-log--writing
        (save-excursion (goto-char (point-min)) (insert "y"))))
    (should (= (length romacs-agent-log--undo-shifts) 1000))
    (let ((history-size (length buffer-undo-list))
          (shift-entry (symbol-function 'romacs-agent-log--shift-undo-entry))
          (shift-list (symbol-function 'romacs-agent-log--shift-undo))
          (visits 0)
          (last-command 'self-insert-command)
          (this-command 'undo))
      (cl-letf (((symbol-function 'romacs-agent-log--shift-undo-entry)
                 (lambda (entry start delta)
                   (cl-incf visits)
                   (funcall shift-entry entry start delta)))
                ((symbol-function 'romacs-agent-log--shift-undo)
                 (lambda (undo start delta)
                   (cl-incf visits (length undo))
                   (funcall shift-list undo start delta))))
        (call-interactively #'undo))
      (should (<= 1000 visits history-size))
      (should-not romacs-agent-log--undo-shifts)
      (should (= (- (point-max) romacs-agent-log--input) 999)))))

(ert-deftest romacs-agent-log-timer-write-preserves-another-buffers-undo-point ()
  (with-temp-buffer
    (insert "Transcript\n")
    (setq-local romacs-agent-log--input (point-marker))
    (buffer-enable-undo)
    (insert "draft")
    (let ((chat (current-buffer))
          (timer-list nil)
          (timer-idle-list nil))
      (with-temp-buffer
        (buffer-enable-undo)
        (insert "abc")
        (goto-char 2)
        (undo-boundary)
        (run-at-time 0 nil
                     (lambda ()
                       (with-current-buffer chat
                         (romacs-agent-log--writing
                           (save-excursion (goto-char (point-min)) (insert "reply"))))))
        (romacs--timer-check)
        (should (buffer-local-value 'romacs-agent-log--undo-shifts chat))
        (goto-char 4)
        (insert "Y")
        (undo-boundary)
        (let ((last-command 'self-insert-command) (this-command 'undo))
          (call-interactively #'undo))
        (should (equal (buffer-string) "abc"))
        (should (= (point) 2))))))

;;; romacs-agent-log-undo-groups-tests.el ends here
