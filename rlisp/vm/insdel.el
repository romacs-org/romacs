(progn
;;; insdel.el --- VM2: the Lisp half of buffer text changes  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; Evaluated once per VM by prims/insdel.rs, the first time a text change
;; has Lisp to run.  The changes themselves are native (insdel.c); these
;; run the hooks GNU runs around one, called only when one can run, with
;; the change's region in BUFFER; and cmds.c's self-insertion, which calls
;; Lisp per character.

;;; Code:

(defun romacs--overlay-hooks (overlays after beg end &optional length)
  "Run report_overlay_modification's hooks OVERLAYS, (PROP . OVERLAY)
pairs, for the overlays still in the current buffer: each function of
PROP with the overlay, AFTER, BEG and END, and after a change LENGTH."
  (while overlays
    (let ((overlay (cdr (car overlays)))
          (functions (car (car overlays))))
      (if (eq (overlay-buffer overlay) (current-buffer))
          (while (consp functions)
            (if after
                (funcall (car functions) overlay t beg end length)
              (funcall (car functions) overlay nil beg end))
            (setq functions (cdr functions)))))
    (setq overlays (cdr overlays))))

(defun romacs--before-change (buffer beg end modification-hooks
                                     &optional undoable overlays)
  "Run what GNU runs before changing BEG..END of BUFFER.
prepare_to_modify_buffer first calls `undo-auto--undoable-change' when
UNDOABLE.  Then, unless `inhibit-modification-hooks':
verify_interval_modification calls each function of each
MODIFICATION-HOOKS value (the characters' `modification-hooks', in
order) with BEG and END; a visited file is locked on its first change;
`select-active-regions' saves the region; and signal_before_change runs
`first-change-hook' if BUFFER is unmodified, `before-change-functions'
with BEG and END and the OVERLAYS' hooks, all with
`inhibit-modification-hooks' bound to t.  A non-local exit from
`before-change-functions' resets it to nil, as reset_var_on_error does."
  (save-current-buffer
    (set-buffer buffer)
    (if undoable (undo-auto--undoable-change))
    (unless inhibit-modification-hooks
      (let ((tail modification-hooks))
        (while tail
          (let ((functions (car tail)))
            (while functions
              (funcall (car functions) beg end)
              (setq functions (cdr functions))))
          (setq tail (cdr tail))))
      (if (and buffer-file-truename buffer-file-name (not (buffer-modified-p))
               (fboundp 'lock-file))
          (lock-file buffer-file-truename))
      (if (and mark-active (boundp 'saved-region-selection)
               (not saved-region-selection) (boundp 'region-extract-function)
               (marker-buffer (mark-marker))
               (if (eq select-active-regions 'only)
                   (eq (car-safe transient-mark-mode) 'only)
                 (and select-active-regions transient-mark-mode)))
          (setq saved-region-selection (funcall region-extract-function nil)))
      (let ((inhibit-modification-hooks t))
        (if (and first-change-hook (not (buffer-modified-p)))
            (run-hooks 'first-change-hook))
        (if before-change-functions
            (let ((done nil))
              (unwind-protect
                  (progn (run-hook-with-args 'before-change-functions beg end)
                         (setq done t))
                (or done (setq before-change-functions nil)))))
        (romacs--overlay-hooks overlays nil beg end)))))

(defun romacs--after-change (buffer beg end length
                                    &optional behind in-front overlays)
  "Run signal_after_change for BUFFER: `after-change-functions' with BEG,
END and LENGTH, the length of the text the change replaced, then the
OVERLAYS' hooks and, after an insertion, the text's insert hooks BEHIND
and IN-FRONT with BEG and END, all with `inhibit-modification-hooks'
bound to t.  A non-local exit from `after-change-functions' resets it to nil."
  (save-current-buffer
    (set-buffer buffer)
    (unless inhibit-modification-hooks
      (let ((inhibit-modification-hooks t))
        (if after-change-functions
            (let ((done nil))
              (unwind-protect
                  (progn (run-hook-with-args 'after-change-functions beg end length)
                         (setq done t))
                (or done (setq after-change-functions nil)))))
        (romacs--overlay-hooks overlays t beg end length)
        (when (= length 0)
          (let ((functions behind))
            (while functions
              (funcall (car functions) beg end)
              (setq functions (cdr functions))))
          (unless (eq in-front behind)
            (let ((functions in-front))
              (while functions
                (funcall (car functions) beg end)
                (setq functions (cdr functions))))))))))

(defun romacs--self-insert (n c)
  "The rest of cmds.c `self-insert-command' once N and C are checked:
`undo-auto-amalgamate' for a single character, then a character C is
translated by `translation-table-for-input' and inserted N times, and a
non-character rings the bell (`ding'; VM2 has none without it)."
  (if (and (< n 2) (fboundp 'undo-auto-amalgamate)) (undo-auto-amalgamate))
  (if (not (characterp c))
      (if (fboundp 'ding) (ding))
    ;; translate_char: a character the table gives replaces C.
    (let* ((translated (if (char-table-p translation-table-for-input)
                           (aref translation-table-for-input c)))
           (character (if (characterp translated) translated c)))
      (if (eq (romacs--internal-self-insert character n) 2)
          (setq undo-auto--this-command-amalgamating nil))))
  nil)

(defun romacs--internal-self-insert (c n)
  "cmds.c internal_self_insert: insert N copies of C, overwriting under
`overwrite-mode', after expanding an abbrev, then auto-fill and
`post-self-insert-hook'.  The value is 2 when this did things that
call for an undo boundary, 1 when it might not have been simple."
  (let ((hairy (if (or before-change-functions after-change-functions) 1 0)))
    (if (and overwrite-mode (< (point) (point-max)))
        (error "VM2 overwrite-mode self-insert is not implemented"))
    (catch 'romacs--no-self-insert
      (if (and abbrev-mode (/= (char-syntax c) ?w) (not buffer-read-only)
               (> (point) (point-min)) (= (char-syntax (preceding-char)) ?w))
          (let* ((tick (buffer-modified-tick))
                 (sym (expand-abbrev)))
            (if (and (symbolp sym) sym (symbol-function sym)
                     (symbolp (symbol-function sym))
                     (get (symbol-function sym) 'no-self-insert))
                (throw 'romacs--no-self-insert (setq hairy 1)))
            (if (/= tick (buffer-modified-tick)) (setq hairy 2))))
      (if (> n 0) (insert-and-inherit (make-string n c)))
      (when (and (if (char-table-p auto-fill-chars)
                     (aref auto-fill-chars c)
                   (memq c '(?\s ?\n)))
                 auto-fill-function (> n 0))
        (if (= c ?\n) (forward-char -1))
        (let ((result (if (fboundp 'internal-auto-fill) (internal-auto-fill)
                        (funcall auto-fill-function))))
          (if (and (= c ?\n) (< (point) (point-max)) (eq (char-after) ?\n))
              (forward-char 1))
          (if result (setq hairy 2))))
      (run-hooks 'post-self-insert-hook)
      hairy)))
)
