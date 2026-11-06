;;; romacs-editing.el --- Shared editor interaction policy -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Shared editor interaction policy.

;;; Code:

;; Under --loadup GNU's Lisp has already defined most of the commands below
;; (simple.el, register.el, delsel.el): they are GNU's there, and Romacs's
;; versions stand in only where GNU's Lisp did not define the name.
(defmacro romacs-editing--stand-in (name arglist &rest body)
  "Define NAME with ARGLIST and BODY unless GNU's Lisp already defined it."
  (declare (indent defun) (doc-string 3))
  `(unless (and (fboundp 'symbol-file) (symbol-file ',name 'defun))
     (defun ,name ,arglist ,@body)))

(romacs-editing--stand-in substitute-command-keys (string &optional _no-face _include-menus)
  "STRING with each \\=\\[COMMAND] as its key and each \\=\\`KEYS' as KEYS, in
`help-key-binding'; a \\=\\<MAP> names the keymap commands after it are
looked up in.  The part of help.el's function Romacs's own hints use."
  (let ((map nil) (start 0) (parts nil))
    (while (string-match "\\\\\\(?:<\\([^>]+\\)>\\|\\[\\([^]]+\\)\\]\\|`\\([^']+\\)'\\)" string start)
      (let ((text (substring string start (match-beginning 0))))
        (cond ((match-beginning 1)
               (setq map (symbol-value (intern (substring string (match-beginning 1) (match-end 1))))
                     parts (cons text parts)))
              (t (let* ((command (and (match-beginning 2) (intern (substring string (match-beginning 2) (match-end 2)))))
                        (key (and command (where-is-internal command map t)))
                        (shown (cond (key (key-description key))
                                     (command (concat "M-x " (symbol-name command)))
                                     (t (substring string (match-beginning 3) (match-end 3))))))
                   (setq parts (cons (propertize shown 'face 'help-key-binding) (cons text parts)))))))
      (setq start (match-end 0)))
    (apply #'concat (reverse (cons (substring string start) parts)))))

(defvar values nil)
(defvar read-expression-history nil)
(defvar eval-expression-print-length 12)
(defvar eval-expression-print-level 4)
(defvar mark-ring nil)
(make-variable-buffer-local 'mark-ring)
(put 'mark-ring 'permanent-local t)
(defvar mark-ring-max 16)
(defvar global-mark-ring nil)
(defvar global-mark-ring-max 16)
(defvar interprogram-cut-function nil)
(defvar interprogram-paste-function nil)
(defvar save-interprogram-paste-before-kill nil)
(defvar kill-transform-function nil)
(defvar kill-do-not-save-duplicates nil)
(defvar yank-pop-change-selection nil)
(defvar romacs-clipboard-last-text nil)

(defun romacs-clipboard-selection-value ()
  "Return new desktop text, falling back to the kill ring on clipboard errors.
Empty text is no text, as in GNU's `gui-selection-value' (select.el)."
  (condition-case nil
      (let ((text (romacs--clipboard-text)))
        (when (equal text "") (setq text nil))
        (unless (equal text romacs-clipboard-last-text)
          (setq romacs-clipboard-last-text text)
          text))
    (error nil)))

(defun romacs-clipboard-select-text (text)
  "Publish TEXT when available; clipboard failure must not prevent an internal kill."
  (condition-case nil
      (progn (romacs--clipboard-set text)
             (setq romacs-clipboard-last-text text))
    (error nil)))

(defvar activate-mark-hook nil)
(defvar deactivate-mark-hook nil)
(defvar set-mark-command-repeat-pop nil)
(defvar use-empty-active-region nil)
(setq transient-mark-mode t)
(defvar delete-selection-mode t)

(romacs-editing--stand-in delete-selection-mode (&optional arg)
  "Toggle replacement of active selections by ordinary typing and delete keys.
With ARG, enable for positive values and disable otherwise."
  (interactive "P")
  (setq delete-selection-mode
        (if arg (> (prefix-numeric-value arg) 0) (not delete-selection-mode))))

(defun romacs-delete-selection-before-command (command)
  "Apply COMMAND's selection policy, returning non-nil when it replaces the command.
Plain deletion never adds text to the kill ring.  Programmatic edits are unchanged."
  (when (and delete-selection-mode (use-region-p))
    (let ((action (get command 'delete-selection)))
      (when (memq action '(t supersede))
        (let ((beg (region-beginning)) (end (region-end)))
          (when (and (minibufferp) (get-text-property beg 'read-only))
            (signal 'text-read-only '("Minibuffer prompt is read-only")))
          (delete-region beg end)
          (deactivate-mark t)
          (and (eq action 'supersede) (/= beg end)))))))

(put 'self-insert-command 'delete-selection t)
(put 'delete-char 'delete-selection 'supersede)
(put 'delete-backward-char 'delete-selection 'supersede)

(romacs-editing--stand-in region-active-p ()
  "Return non-nil when this buffer has an active transient region."
  (and transient-mark-mode mark-active (not (null (mark t)))))

(romacs-editing--stand-in use-region-p ()
  "Return non-nil for an active nonempty region, or an explicitly enabled empty one."
  (and (region-active-p)
       (or use-empty-active-region (/= (region-beginning) (region-end)))))

(romacs-editing--stand-in use-region-beginning ()
  "Return the active usable region's beginning, or nil."
  (and (use-region-p) (region-beginning)))

(romacs-editing--stand-in use-region-end ()
  "Return the active usable region's end, or nil."
  (and (use-region-p) (region-end)))

(romacs-editing--stand-in activate-mark (&optional no-tmm)
  "Activate this buffer's ordinary mark and run its activation hook."
  (when (mark t)
    (unless (region-active-p)
      (setq mark-active t)
      (unless (or transient-mark-mode no-tmm)
        (make-local-variable 'transient-mark-mode)
        (setq transient-mark-mode 'lambda))
      (run-hooks 'activate-mark-hook))))

(romacs-editing--stand-in deactivate-mark (&optional force)
  "Deactivate the ordinary mark without modifying text or clipboard ownership."
  (when (or force (region-active-p))
    (when (or (eq transient-mark-mode 'lambda) (eq (car-safe transient-mark-mode) 'only))
      (kill-local-variable 'transient-mark-mode))
    (setq mark-active nil)
    (run-hooks 'deactivate-mark-hook)))

(romacs-editing--stand-in exchange-point-and-mark (&optional arg)
  "Exchange point and mark, activating the region unless ARG is non-nil."
  (interactive "P")
  (let ((old (mark t)))
    (unless old (signal 'user-error '("No mark set in this buffer")))
    (set-mark (point))
    (goto-char old)
    (if arg (deactivate-mark t) (activate-mark)))
  nil)

(romacs-editing--stand-in keyboard-quit ()
  "Cancel editor interaction and deactivate the current region."
  (interactive)
  (deactivate-mark t)
  (signal 'quit nil))

(romacs-editing--stand-in end-of-buffer (&optional arg)
  "Move to the accessible end, preserving Romacs's minimal reveal policy.
A numeric ARG selects tenths from the end; a universal prefix suppresses mark pushing."
  (interactive "P")
  (or (consp arg) (region-active-p) (push-mark))
  (goto-char (if (and arg (not (consp arg)))
                 (- (point-max) (/ (* (- (point-max) (point-min)) (prefix-numeric-value arg)) 10))
               (point-max)))
  (when (and arg (not (consp arg))) (forward-line 1)))

;; The prompt state M-: and the workspace's readers
;; (romacs-workspace-completion.el) bind while they read.
(defvar romacs-prompt--input-start nil)
(defvar romacs-prompt--history-position nil)
(defvar romacs-prompt--history-input nil)
(defvar romacs-prompt--history-variable nil)

(defun romacs-prompt--replace-input (text)
  "Replace the active minibuffer input with TEXT, retaining its prompt."
  (delete-region romacs-prompt--input-start (point-max))
  (goto-char (point-max))
  (insert text))

(defun romacs-prompt-previous-history ()
  "Recall an older entry from the active prompt's history variable."
  (interactive nil minibuffer-mode)
  (when (null romacs-prompt--history-position)
    (setq romacs-prompt--history-input
          (buffer-substring-no-properties romacs-prompt--input-start (point-max))))
  (let* ((next (if romacs-prompt--history-position (1+ romacs-prompt--history-position) 0))
         (entry (nth next (symbol-value romacs-prompt--history-variable))))
    (if entry
        (progn (setq romacs-prompt--history-position next) (romacs-prompt--replace-input entry))
      (message "No older history entry"))))

(defun romacs-prompt-next-history ()
  "Recall a newer history entry, or restore the input preceding recall."
  (interactive nil minibuffer-mode)
  (when romacs-prompt--history-position
    (if (= romacs-prompt--history-position 0)
        (progn (setq romacs-prompt--history-position nil)
               (romacs-prompt--replace-input romacs-prompt--history-input))
      (setq romacs-prompt--history-position (1- romacs-prompt--history-position))
      (romacs-prompt--replace-input (nth romacs-prompt--history-position (symbol-value romacs-prompt--history-variable))))))

(defun romacs-read-expression ()
  "Read exactly one Lisp expression, with history and ordinary quit handling."
  (let* ((prompt "Eval: ")
         (romacs-prompt--input-start (1+ (length prompt)))
         (romacs-prompt--history-position nil)
         (romacs-prompt--history-input nil)
         (romacs-prompt--history-variable 'read-expression-history)
         (map (make-sparse-keymap))
         (initial nil) (done nil) result)
    (set-keymap-parent map minibuffer-local-map)
    (define-key map [13] 'exit-minibuffer)
    (define-key map [10] 'exit-minibuffer)
    (define-key map [7] 'abort-minibuffers)
    (define-key map [27 112] 'romacs-prompt-previous-history)
    (define-key map [27 110] 'romacs-prompt-next-history)
    (while (not done)
      (setq initial (let ((history-add-new-input nil))
                      (read-from-minibuffer prompt initial map nil 'read-expression-history)))
      (condition-case err
          (let* ((source (concat "(" initial "
)"))
                 (parsed (read-from-string source))
                 (forms (car parsed)))
            (unless (and (= (cdr parsed) (length source)) (= (length forms) 1))
              (signal 'user-error '("Enter exactly one complete expression")))
            (setq result (car forms) done t)
            (add-to-history 'read-expression-history initial))
        (error (setq prompt (concat (error-message-string err) "; Eval: ")
                     romacs-prompt--input-start (1+ (length prompt))))))
    result))

(romacs-editing--stand-in eval-expression (exp &optional insert-value no-truncate char-print-limit)
  "Evaluate EXP lexically in the calling buffer and retain its value in `values'.
Interactively read one expression with history; INSERT-VALUE inserts its printed value.
This Romacs adapter does not enter GNU's debugger or add integer character annotations."
  (interactive (list (romacs-read-expression) current-prefix-arg))
  (when char-print-limit (signal 'user-error '("Character result annotations are not supported")))
  (let* ((result (eval exp t))
         (print-length (unless no-truncate eval-expression-print-length))
         (print-level (unless no-truncate eval-expression-print-level))
         (text (prin1-to-string result)))
    (setq values (cons result values))
    (if insert-value (insert text) (romacs-gui-echo text))
    result))

(defvar romacs-yank-start nil)
(defvar romacs-yank-end nil)
(make-variable-buffer-local 'romacs-yank-start)
(make-variable-buffer-local 'romacs-yank-end)

(romacs-editing--stand-in kill-append (string before-p)
  "Append STRING to the latest kill, or prepend when BEFORE-P is non-nil."
  (kill-new (if before-p (concat string (car kill-ring))
              (concat (car kill-ring) string)) t))

(defun romacs-region-arguments ()
  "Return mark and point in kill direction, or explain the absent region."
  (let ((position (mark t)))
    (unless position (signal 'user-error '("The mark is not set now, so there is no region")))
    (list position (point))))

(romacs-editing--stand-in copy-region-as-kill (beg end &optional region)
  "Copy the ordinary region to the shared kill ring without changing text."
  (interactive (romacs-region-arguments))
  (let ((text (buffer-substring (min beg end) (max beg end))))
    (if (eq last-command 'kill-region) (kill-append text (< end beg)) (kill-new text)))
  (deactivate-mark t)
  nil)

(romacs-editing--stand-in kill-ring-save (beg end &optional region)
  "Copy BEG through END to the shared kill ring."
  (interactive (romacs-region-arguments))
  (copy-region-as-kill beg end region))

(romacs-editing--stand-in kill-region (beg end &optional region)
  "Kill BEG through END, concatenating consecutive kills in their direction.
Read-only text is copied before deletion signals its ordinary read-only error."
  (interactive (romacs-region-arguments))
  (copy-region-as-kill beg end region)
  (setq this-command 'kill-region)
  (delete-region beg end)
  nil)

(romacs-editing--stand-in append-next-kill (&optional arg)
  "Make the next kill append to the current kill-ring entry."
  (interactive "P")
  (setq this-command 'kill-region))

(romacs-editing--stand-in kill-line (&optional arg)
  "Kill to the line end, or ARG line beginnings, using the shared kill ring."
  (interactive "P")
  (let* ((beg (point))
         (end (save-excursion
               (if arg (forward-line (prefix-numeric-value arg))
                 (end-of-line)
                 (when (= beg (point)) (forward-char 1)))
               (point))))
    (kill-region beg end)))

(romacs-editing--stand-in yank (&optional arg)
  "Insert a shared kill-ring entry and remember its exact marker bounds."
  (interactive "*P")
  (let ((text (current-kill (if (or (null arg) (consp arg)) 0 (1- (prefix-numeric-value arg))))))
    (push-mark (point) t)
    (setq romacs-yank-start (copy-marker (point)))
    (insert-for-yank text)
    (setq romacs-yank-end (copy-marker (point)))
    (when (consp arg) (exchange-point-and-mark t))
    (setq this-command 'yank)
    (deactivate-mark t))
  nil)

(romacs-editing--stand-in yank-pop (&optional arg)
  "Replace the previous yank with another kill-ring entry."
  (interactive "*p")
  (unless (and (eq last-command 'yank)
               (markerp romacs-yank-start) (markerp romacs-yank-end)
               (eq (marker-buffer romacs-yank-start) (current-buffer))
               (eq (marker-buffer romacs-yank-end) (current-buffer)))
    (signal 'user-error '("Previous command was not a yank")))
  (let ((text (current-kill (or arg 1)))
        (beg (marker-position romacs-yank-start))
        (end (marker-position romacs-yank-end))
        (before (= (point) (marker-position romacs-yank-start))))
    (delete-region beg end)
    (goto-char beg)
    (set-marker (mark-marker) beg)
    (insert-for-yank text)
    (set-marker romacs-yank-end (point))
    (when before (exchange-point-and-mark t))
    (setq this-command 'yank)
    (deactivate-mark t))
  nil)

(romacs-editing--stand-in mark-whole-buffer ()
  "Select the entire accessible buffer."
  (interactive)
  (push-mark (point) t)
  (goto-char (point-min))
  (push-mark (point-max) t t))

(defvar register-alist nil)
(defvar romacs-register-key nil)

(romacs-editing--stand-in set-register (register value)
  "Store VALUE in character REGISTER."
  (unless (integerp register) (signal 'wrong-type-argument (list 'characterp register)))
  (let ((entry (assq register register-alist)))
    (if entry (setcdr entry value) (setq register-alist (cons (cons register value) register-alist))))
  value)

(romacs-editing--stand-in get-register (register)
  "Return the value in REGISTER, or nil."
  (cdr (assq register register-alist)))

(defun romacs-register-read-key ()
  "Accept one register character through the ordinary minibuffer command loop."
  (interactive nil minibuffer-mode)
  (unless (and (integerp last-command-event) (<= 0 last-command-event) (<= last-command-event 1114111))
    (signal 'user-error '("Use a character to name a register")))
  (setq romacs-register-key last-command-event)
  (exit-minibuffer))

(defun romacs-read-register (prompt)
  "Read one character naming a register; C-g cancels."
  (let ((romacs-register-key nil) (map (make-sparse-keymap)) (history-add-new-input nil))
    (define-key map [t] 'romacs-register-read-key)
    (define-key map [7] 'abort-minibuffers)
    (read-from-minibuffer prompt nil map)
    (unless romacs-register-key (signal 'quit nil))
    romacs-register-key))

(romacs-editing--stand-in copy-to-register (register start end &optional delete-flag)
  "Copy START through END to REGISTER, optionally deleting the source."
  (interactive (list (romacs-read-register "Copy to register: ") (region-beginning) (region-end) current-prefix-arg))
  (set-register register (buffer-substring (min start end) (max start end)))
  (when delete-flag (delete-region start end))
  (deactivate-mark t))

(romacs-editing--stand-in append-to-register (register start end &optional delete-flag)
  "Append the region to a text REGISTER, optionally deleting the source."
  (interactive (list (romacs-read-register "Append to register: ") (region-beginning) (region-end) current-prefix-arg))
  (set-register register (concat (or (get-register register) "") (buffer-substring (min start end) (max start end))))
  (when delete-flag (delete-region start end))
  (deactivate-mark t))

(romacs-editing--stand-in prepend-to-register (register start end &optional delete-flag)
  "Prepend the region to a text REGISTER, optionally deleting the source."
  (interactive (list (romacs-read-register "Prepend to register: ") (region-beginning) (region-end) current-prefix-arg))
  (set-register register (concat (buffer-substring (min start end) (max start end)) (or (get-register register) "")))
  (when delete-flag (delete-region start end))
  (deactivate-mark t))

(romacs-editing--stand-in insert-register (register &optional arg)
  "Insert a text or number REGISTER, leaving mark at its other boundary."
  (interactive (list (romacs-read-register "Insert register: ") current-prefix-arg))
  (let ((value (get-register register)))
    (cond ((stringp value))
          ((numberp value) (setq value (number-to-string value)))
          (t (signal 'user-error '("Register does not contain text or a number"))))
    (push-mark (point) t)
    (insert-for-yank value)
    (unless arg (exchange-point-and-mark t)))
  nil)

(romacs-editing--stand-in point-to-register (register &optional arg)
  "Save the current buffer position as a moving marker in REGISTER."
  (interactive (list (romacs-read-register "Point to register: ") current-prefix-arg))
  (when arg (signal 'user-error '("Frame and window registers are not supported")))
  (set-register register (copy-marker (point))))

(romacs-editing--stand-in jump-to-register (register &optional arg)
  "Jump to the live buffer and position saved in REGISTER."
  (interactive (list (romacs-read-register "Jump to register: ") current-prefix-arg))
  (let* ((marker (get-register register))
         (buffer (and (markerp marker) (marker-buffer marker))))
    (unless (and buffer (buffer-live-p buffer))
      (signal 'user-error '("Register does not point to a live buffer")))
    (romacs-select-buffer buffer)
    (let ((position (marker-position marker)))
      (unless (and (<= (point-min) position) (<= position (point-max))) (widen))
      (goto-char position)))
  nil)

;; The workspace's keys for these commands, which GNU's bindings.el and
;; simple.el bind the same way once they load.
(unless (featurep 'simple)
  (global-set-key [134217855] 'backward-kill-word)
  (global-set-key [27 127] 'backward-kill-word)
  (global-set-key [C-backspace] 'backward-kill-word)
  (global-set-key [134217830] 'forward-word)
  (global-set-key [27 102] 'forward-word)
  (global-set-key [134217826] 'backward-word)
  (global-set-key [27 98] 'backward-word)
  (global-set-key [134217847] 'kill-ring-save)
  (global-set-key [27 119] 'kill-ring-save)
  (global-set-key [23] 'kill-region)
  (global-set-key [11] 'kill-line)
  (global-set-key [25] 'yank)
  (global-set-key [134217849] 'yank-pop)
  (global-set-key [27 121] 'yank-pop)
  (global-set-key [24 104] 'mark-whole-buffer)
  (global-set-key [24 114 115] 'copy-to-register)
  (global-set-key [24 114 105] 'insert-register)
  (global-set-key [24 114 32] 'point-to-register)
  (global-set-key [24 114 106] 'jump-to-register)
  (global-set-key [134217786] 'eval-expression)
  (global-set-key [27 58] 'eval-expression)
  (global-set-key [134217788] 'beginning-of-buffer)
  (global-set-key [27 60] 'beginning-of-buffer)
  (global-set-key [134217790] 'end-of-buffer)
  (global-set-key [27 62] 'end-of-buffer)
  (global-set-key [0] 'set-mark-command)
  (global-set-key [24 24] 'exchange-point-and-mark)
  (global-set-key [7] 'keyboard-quit))
;; The host delivers Return as the function key `return', as GNU's X and
;; pgtk frames do. simple.el translates it to RET in `function-key-map' and
;; binds RET to `newline'; without simple.el this workspace does both
;; itself, its RET inserting a newline as `newline' does. The global
;; bindings above have installed the C keymaps.
(unless (lookup-key function-key-map [return])
  (put 'return 'ascii-character ?\C-m)
  (define-key function-key-map [return] [?\C-m]))
;; Escape too is a function key, `escape', made ESC the same way.
(unless (lookup-key function-key-map [escape])
  (put 'escape 'ascii-character ?\e)
  (define-key function-key-map [escape] [?\e]))
(defun romacs-newline (n)
  "Insert N newlines: RET in a workspace without simple.el's `newline'."
  (interactive "*p")
  ;; As `newline' does, so that the command's event stays RET.
  (let ((last-command-event ?\n))
    (self-insert-command n)))
(unless (fboundp 'newline) (global-set-key [13] 'romacs-newline))
(put 'romacs-newline 'completion-predicate #'ignore)
