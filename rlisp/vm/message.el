(progn
;;; message.el --- VM2: the *Messages* log of `message'  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; Evaluated once per VM by prims/message.rs, the first time `message'
;; logs.  GNU's xdisp.c message_dolog and message_log_check_duplicate:
;; append the text to `messages-buffer-name', combining a repeated line
;; as " [N times]" and trimming to `message-log-max' lines, without undo,
;; read-only checks or modification hooks.

;;; Code:

(defun romacs--message-dolog (text)
  "Append TEXT and a newline to the buffer named `messages-buffer-name'.
A new buffer starts in `messages-buffer-mode' when that is defined.  Point
at the end of the buffer follows the new text; elsewhere it stays put, as
does a narrowing.  A line that repeats the previous one replaces it, as
\" [N times]\" when N > 1; past an integer `message-log-max', the oldest
lines are deleted."
  (if (not (stringp messages-buffer-name))
      (setq messages-buffer-name "*Messages*"))
  (let ((deactivate deactivate-mark))
    (save-current-buffer
      (let ((buffer (get-buffer messages-buffer-name)))
        (if buffer
            (set-buffer buffer)
          (set-buffer (get-buffer-create messages-buffer-name))
          (if (fboundp 'messages-buffer-mode) (messages-buffer-mode))))
      (setq buffer-undo-list t)
      (let* ((inhibit-read-only t)
             (inhibit-modification-hooks t)
             (z (1+ (buffer-size)))
             (point-at-end (= (point) z))
             (zv-at-end (= (point-max) z))
             (begv (copy-marker (point-min)))
             (zv (copy-marker (point-max)))
             (opoint (copy-marker (point))))
        (widen)
        (goto-char (point-max))
        (insert (substring-no-properties text) "\n")
        (forward-line -1)
        (let ((this-bol (point)))
          (if (> this-bol (point-min))
              (let* ((prev-bol (progn (forward-line -1) (point)))
                     (dups (romacs--message-log-duplicate prev-bol this-bol)))
                (if (> dups 0)
                    (progn
                      (delete-region prev-bol this-bol)
                      (if (> dups 1)
                          (progn
                            (goto-char (1- (point-max)))
                            (insert-before-markers (format " [%d times]" dups)))))))))
        (if (and (integerp message-log-max) (>= message-log-max 0))
            (progn
              (goto-char (point-max))
              (forward-line (- message-log-max))
              (delete-region (point-min) (point))))
        (if (or (> begv (point-min)) (not zv-at-end))
            (narrow-to-region begv (if zv-at-end (point-max) zv)))
        (goto-char (if point-at-end (point-max) opoint))
        (set-marker begv nil)
        (set-marker zv nil)
        (set-marker opoint nil)))
    (setq deactivate-mark deactivate)
    nil))

(defun romacs--message-log-duplicate (prev-bol this-bol)
  "Compare the last line, at THIS-BOL, with the previous one at PREV-BOL.
Return 0 if they differ, 1 if the last should just replace the previous
one (it extends a \"...\" prefix), or N > 1 if it should replace it with
\" [N times]\" appended."
  (let ((len (- (point-max) 1 this-bol))
        (i 0) (seen-dots nil) (result nil))
    (while (and (not result) (< i len))
      (if (and (>= i 3)
               (eq (char-after (+ prev-bol i -3)) ?.)
               (eq (char-after (+ prev-bol i -2)) ?.)
               (eq (char-after (+ prev-bol i -1)) ?.))
          (setq seen-dots t))
      (if (not (eq (char-after (+ prev-bol i)) (char-after (+ this-bol i))))
          (setq result (if seen-dots 1 0)))
      (setq i (1+ i)))
    (or result
        (let ((p (+ prev-bol len)))
          (cond
           ((eq (char-after p) ?\n) 2)
           ((and (eq (char-after p) ?\s) (eq (char-after (1+ p)) ?\[))
            ;; strtoimax, then exactly " times]\n".
            (let ((rest (buffer-substring-no-properties (+ p 2) (point-max))))
              (if (string-match "\\`[ \t\n\v\f\r]*\\([-+]?[0-9]+\\) times]\n" rest)
                  (let ((n (string-to-number (substring rest (match-beginning 1) (match-end 1)))))
                    (if (and (integerp n) (> n 0)) (1+ n) 0))
                0)))
           (t 0))))))

)
;;; message.el ends here
