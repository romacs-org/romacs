(progn
;;; minibuf.el --- VM2: the Lisp half of read-from-minibuffer  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Evaluated once per VM, after keyboard.el, the Lisp half of keyboard.c,
;; by prims/keyboard.rs on the first use of either. GNU's read_minibuf
;; (minibuf.c:573) runs a recursive command loop, which calls Lisp per key,
;; so that loop is Lisp here (PRIMITIVE-DOCTRINE section 3): keyboard.el's.
;; Every value it keeps across a command is a register of a traced frame.
;; GNU minibuf.c `minibuf_level'.

;;; Code:

(defvar romacs--minibuf-level 0)
;; minibuf.c minibuf_prompt and, per active minibuffer, the command loop
;; level it was entered at (minibuf_c_loop_level); innermost first.
(defvar romacs--minibuf-prompts nil)
(defvar romacs--minibuf-loop-levels nil)
;; simple.el:2008; read_minibuf binds it while the prompt is active.
(defvar minibuffer-default nil)

;; BOOTSTRAP STAND-INS (DIVERGENCES.md VM2-MINIBUF1). GNU binds RET, C-j and
;; C-g in `minibuffer-local-map' from Lisp (minibuffer.el:3407-3415), and
;; `exit-minibuffer' / `minibuffer-quit-recursive-edit' are Lisp
;; (minibuffer.el:3077, 3104); `abort-minibuffers' is C (minibuf.c:472).
;; VM2 cannot load minibuffer.el yet, so these minimal copies are installed
;; only where nothing is defined or bound. Loading GNU's minibuffer.el
;; replaces the two Lisp functions and rebinds the keys; `abort-minibuffers'
;; is C in GNU, so this Lisp copy stays until VM2 has that primitive.
(unless (fboundp 'minibuffer-quit-recursive-edit)
  (defun minibuffer-quit-recursive-edit (&optional levels)
    "Quit the command that requested this minibuffer input (minibuffer.el:3104)."
    (unless levels (setq levels 1))
    (if (> levels 1)
        (throw 'exit (lambda () (minibuffer-quit-recursive-edit (1- levels))))
      (throw 'exit (lambda () (signal 'minibuffer-quit nil))))))
(unless (fboundp 'exit-minibuffer)
  (defun exit-minibuffer ()
    "Terminate this minibuffer argument (minibuffer.el:3077, without its
innermost-loop checks, which need recursive minibuffers)."
    (interactive)
    (throw 'exit nil)))
(unless (fboundp 'abort-minibuffers)
  (defun abort-minibuffers ()
    "Abort the current minibuffer (minibuf.c:472, single level only)."
    (interactive)
    (if (= romacs--minibuf-level 0) (error "Not in a minibuffer"))
    (minibuffer-quit-recursive-edit)))
(unless (lookup-key minibuffer-local-map "\C-g")
  (define-key minibuffer-local-map "\C-g" 'abort-minibuffers))
(unless (lookup-key minibuffer-local-map "\n")
  (define-key minibuffer-local-map "\n" 'exit-minibuffer))
(unless (lookup-key minibuffer-local-map "\r")
  (define-key minibuffer-local-map "\r" 'exit-minibuffer))

(defun romacs--read-minibuf (prompt keymap initial pos histvar default inherit)
  "minibuf.c read_minibuf (minibuf.c:573) after its argument checks.
INHERIT is INHERIT-INPUT-METHOD."
  (when (and (> romacs--minibuf-level 0) (not enable-recursive-minibuffers))
    (if (eq (selected-window) (minibuffer-window))
        (signal 'user-error '("Command attempted to use minibuffer while in minibuffer"))
      (throw 'exit "Command attempted to use minibuffer while in minibuffer")))
  ;; minibuf.c:653: outside a keyboard macro batch GNU reads stdin; VM2
  ;; without a host keyboard has no terminal input, which batch GNU reports
  ;; at end of file. With a host keyboard (the GUI) it is interactive GNU.
  (unless (or executing-kbd-macro (romacs--host-input-p))
    (signal 'end-of-file '("Error reading from stdin")))
  ;; read_minibuf specbinds `minibuffer-default' and
  ;; `current-minibuffer-command' and increments
  ;; minibuf_level; all are restored by the unwind below. (setq, not let:
  ;; this whole file is compiled as one form, before its defvars run.)
  (let* ((outer-prefix current-prefix-arg)
         (outer-default minibuffer-default)
         (outer-command current-minibuffer-command)
         (outer-level romacs--minibuf-level)
         ;; minibuf_save_list and the specbind of
         ;; `minibuffer-completing-file-name' (minibuf.c:602, 727).
         (outer-history-variable minibuffer-history-variable)
         (outer-history-position minibuffer-history-position)
         (outer-completing-file-name minibuffer-completing-file-name)
         (ambient-directory default-directory)
         ;; window.c:8428: the entry's window configuration saves the caller
         ;; only inside a minibuffer, and its restore puts it back.
         (outer-selected (and (> outer-level 0) romacs--minibuf-selected-window))
         (outer-prompts romacs--minibuf-prompts)
         (outer-keys romacs--command-keys)
         (outer-loop-levels romacs--minibuf-loop-levels)
         (level (1+ outer-level))
         (reused (get-buffer (format " *Minibuf-%d*" level)))
         (buffer (get-buffer-create (format " *Minibuf-%d*" level)))
         (calling-buffer (current-buffer))
         (calling-window (selected-window))
         ;; minibuf.c:783: `current-input-method' is buffer local.
         (input-method (and inherit (boundp 'current-input-method) current-input-method))
         (mini (minibuffer-window))
         (mini-buffer (window-buffer mini))
         (prompt-end 1)
         val)
    ;; minibuf.c:794: an unbound history variable starts as nil.
    (unless (boundp histvar) (set histvar nil))
    (setq minibuffer-default default current-minibuffer-command this-command
          romacs--minibuf-level level
          romacs--minibuf-prompts (cons (copy-sequence prompt) outer-prompts)
          romacs--minibuf-loop-levels (cons romacs--command-loop-level outer-loop-levels)
          minibuffer-history-variable histvar minibuffer-history-position 0)
    ;; minibuf.c:602, 765: a caller's file name reading is `lambda' here,
    ;; nil in a minibuffer this one opens.
    (when (eq minibuffer-completing-file-name 'lambda)
      (setq minibuffer-completing-file-name nil))
    (when minibuffer-completing-file-name
      (setq minibuffer-completing-file-name 'lambda))
    (unwind-protect
        (progn
          (set-buffer buffer)
          ;; minibuf.c get_minibuffer: a minibuffer used before starts
          ;; afresh, without the overlays its last use left.
          (when reused
            (let ((overlays (overlays-in (point-min) (point-max))))
              (while overlays
                (delete-overlay (car overlays))
                (setq overlays (cdr overlays)))))
          ;; set_minibuffer_mode: `minibuffer-mode' resets its local state.
          (when (fboundp 'minibuffer-mode) (minibuffer-mode))
          ;; minibuf.c:796-822: lines never truncate, and the directory is
          ;; the caller's, or the first live buffer's that has one.
          (set (make-local-variable 'truncate-lines) nil)
          (set (make-local-variable 'default-directory)
               (if (stringp ambient-directory) ambient-directory
                 (let ((buffers (buffer-list)) found)
                   (while (and buffers (not found))
                     (let ((directory (buffer-local-value 'default-directory (car buffers))))
                       (when (stringp directory) (setq found directory)))
                     (setq buffers (cdr buffers)))
                   found)))
          (set-window-buffer mini buffer)
          ;; minibuf.c:827.
          (when (or (= level 1) (not (eq mini calling-window)))
            (setq romacs--minibuf-selected-window calling-window))
          (select-window mini)
          (set-buffer buffer)
          (setq mark-active nil)
          ;; minibuf.c:274: the erase and the prompt bypass read-only text
          ;; and change hooks.
          (let ((inhibit-read-only t)
                (inhibit-modification-hooks t))
            (erase-buffer)
            (insert prompt)
            (when (> (point) 1)
              (put-text-property 1 (point) 'front-sticky t)
              (put-text-property 1 (point) 'rear-nonsticky t)
              (put-text-property 1 (point) 'field t)
              (let ((list minibuffer-prompt-properties))
                (while (consp list)
                  (let ((key (car list)))
                    (setq list (cdr list))
                    (when (consp list)
                      (let ((value (car list)))
                        (setq list (cdr list))
                        (if (eq key 'face)
                            (add-face-text-property 1 (point) value t)
                          (put-text-property 1 (point) key value)))))))))
          (setq prompt-end (point))
          (when initial
            (insert initial)
            (forward-char pos))
          ;; minibuf.c:905 clear_message: the minibuffer replaces the echo
          ;; area's message.
          (message nil)
          (use-local-map keymap)
          (when (and (stringp input-method) (fboundp 'activate-input-method))
            (activate-input-method input-method))
          (when (and minibuffer-setup-hook (fboundp 'run-hooks))
            (run-hooks 'minibuffer-setup-hook))
          ;; keyboard.el's command loop, reading the macro or the host.
          (romacs--recursive-edit-1)
          (set-buffer buffer)
          (setq val (if minibuffer-allow-text-properties
                        (buffer-substring prompt-end (point-max))
                      (buffer-substring-no-properties prompt-end (point-max)))))
      ;; run_exit_minibuf_hook, before read_minibuf_unwind, in the minibuffer.
      (when (buffer-live-p buffer)
        (set-buffer buffer)
        (romacs--safe-run-hooks 'minibuffer-exit-hook))
      ;; read_minibuf_unwind (minibuf.c:1111). The level and the default are
      ;; restored FIRST and unconditionally: if a later step signalled, a
      ;; stale level would refuse every future prompt as recursive. Then
      ;; empty this level's buffer, give the window back its idle buffer and
      ;; reselect the caller when it is still live (a command at the prompt
      ;; may have deleted it; GNU's record_unwind_current_buffer likewise
      ;; restores the buffer only if it is live).
      (setq current-prefix-arg outer-prefix prefix-arg nil
            minibuffer-default outer-default romacs--minibuf-level outer-level
            current-minibuffer-command outer-command
            romacs--minibuf-selected-window outer-selected
            romacs--minibuf-prompts outer-prompts romacs--minibuf-loop-levels outer-loop-levels
            romacs--command-keys outer-keys
            minibuffer-history-variable outer-history-variable
            minibuffer-history-position outer-history-position
            minibuffer-completing-file-name outer-completing-file-name)
      (set-buffer buffer)
      ;; minibuf.c:1186: past the read-only prompt, with no hooks, and
      ;; leaving deactivate-mark alone.
      (let ((inhibit-read-only t)
            (inhibit-modification-hooks t)
            (old-deactivate-mark deactivate-mark))
        (erase-buffer)
        (setq deactivate-mark old-deactivate-mark))
      (set-window-buffer mini mini-buffer)
      ;; minibuf.c:1197: leaving the outermost minibuffer resizes the
      ;; mini-window back to its display now, so a prompt the command
      ;; reads next grows from one line.
      (when (= romacs--minibuf-level 0)
        (romacs--resize-mini-window))
      (when (window-live-p calling-window)
        (select-window calling-window))
      (condition-case nil (set-buffer calling-buffer) (error nil)))
    ;; minibuf.c:1077-1096: history, through GNU's own subr.el function.
    (let ((histstring (cond ((> (length val) 0) val)
                            ((stringp default) default)
                            ((and (consp default) (stringp (car default))) (car default)))))
      (when (and history-add-new-input histstring (fboundp 'add-to-history))
        (add-to-history histvar histstring)))
    val))

;;;; minibuf.c's queries over the active minibuffers

(defun romacs--minibuffer-depth-of (buffer)
  "minibuf.c this_minibuffer_depth: BUFFER's level as an active minibuffer, or 0."
  (let ((level romacs--minibuf-level) (depth 0))
    (while (and (= depth 0) (> level 0))
      (if (eq buffer (get-buffer (format " *Minibuf-%d*" level)))
          (setq depth level)
        (setq level (1- level))))
    depth))

(defun romacs--innermost-minibuffer-p (buffer)
  "minibuf.c Finnermost_minibuffer_p."
  (and (> romacs--minibuf-level 0)
       (eq (or buffer (current-buffer))
           (get-buffer (format " *Minibuf-%d*" romacs--minibuf-level)))))

(defun romacs--minibuffer-innermost-command-loop-p (buffer)
  "minibuf.c Fminibuffer_innermost_command_loop_p."
  (let ((depth (romacs--minibuffer-depth-of (or buffer (current-buffer)))))
    (and (> depth 0)
         (eql (nth (- romacs--minibuf-level depth) romacs--minibuf-loop-levels)
              romacs--command-loop-level))))

(defun romacs--minibuffer-prompt ()
  "minibuf.c Fminibuffer_prompt."
  (copy-sequence (car romacs--minibuf-prompts)))

(defun romacs--minibuffer-prompt-end ()
  "minibuf.c Fminibuffer_prompt_end: in a minibuffer, the end of the
prompt's field; elsewhere `point-min'."
  (if (not (minibufferp (current-buffer)))
      (point-min)
    (let ((end (field-end (point-min))))
      (if (and (= end (point-max)) (null (get-char-property (point-min) 'field)))
          (point-min)
        end))))

(defun romacs--minibuffer-contents (properties)
  "minibuf.c Fminibuffer_contents, or without PROPERTIES its -no-properties."
  (if properties
      (buffer-substring (romacs--minibuffer-prompt-end) (point-max))
    (buffer-substring-no-properties (romacs--minibuffer-prompt-end) (point-max))))

(defun romacs--read-buffer (prompt def require-match predicate)
  "minibuf.c Fread_buffer: complete a buffer name, or call `read-buffer-function'."
  (when (bufferp def) (setq def (buffer-name def)))
  (let ((completion-ignore-case read-buffer-completion-ignore-case))
    (cond
     (read-buffer-function
      ;; Older reader functions take no PREDICATE.
      (if predicate
          (funcall read-buffer-function prompt def require-match predicate)
        (funcall read-buffer-function prompt def require-match)))
     (t
      ;; With a default, `format-prompt' puts it before the colon.
      (when def
        (when (stringp prompt)
          (let ((len (length prompt)))
            (setq prompt (substring prompt 0
                                    (cond ((and (>= len 2) (eq (aref prompt (- len 2)) ?:)
                                                (eq (aref prompt (1- len)) ?\s))
                                           (- len 2))
                                          ((and (>= len 1) (memq (aref prompt (1- len)) '(?: ?\s)))
                                           (1- len))
                                          (t len))))))
        (setq prompt (format-prompt prompt (if (consp def) (car def) def))))
      (completing-read prompt #'internal-complete-buffer predicate require-match
                       nil 'buffer-name-history def nil)))))

(defun romacs--read-coding-system (prompt default-coding-system)
  "coding.c Fread_coding_system: complete a coding system name, case-blind."
  (when (symbolp default-coding-system)
    (setq default-coding-system (symbol-name default-coding-system)))
  (let ((name (let ((completion-ignore-case t))
                (completing-read prompt coding-system-alist nil t nil
                                 'coding-system-history default-coding-system))))
    (if (= (length name) 0) nil (intern name))))

(defvar custom-variable-history nil
  "History of the options `read-variable' has read.")

(defun romacs--read-symbol (prompt predicate history default-value)
  "minibuf.c Fread_command, Fread_variable: complete a symbol PREDICATE accepts."
  (let ((name (completing-read prompt obarray predicate t nil history
                               (if (and default-value (symbolp default-value))
                                   (symbol-name default-value)
                                 default-value))))
    (and name (intern name))))

(defun romacs--read-command (prompt default-value)
  "minibuf.c Fread_command: complete the name of a command."
  (romacs--read-symbol prompt #'commandp nil default-value))

(defun romacs--read-variable (prompt default-value)
  "minibuf.c Fread_variable: complete the name of a user option."
  (romacs--read-symbol prompt #'custom-variable-p 'custom-variable-history default-value))

(defun romacs--internal-complete-buffer (string predicate flag)
  "minibuf.c Finternal_complete_buffer: complete STRING over the buffer names."
  (let ((alist (mapcar (lambda (buffer) (cons (buffer-name buffer) buffer)) (buffer-list))))
    (cond
     ((null flag) (try-completion string alist predicate))
     ((eq flag t)
      (let ((names (all-completions string alist predicate)))
        (if (> (length string) 0)
            names
          ;; Without input, leave out the internal buffers, unless every
          ;; buffer is one.
          (let ((visible (delq nil (mapcar (lambda (name) (unless (eq (aref name 0) ?\s) name))
                                           names))))
            (cond (visible visible)
                  ((= (length names) (length alist)) names))))))
     ((eq flag 'lambda) (test-completion string alist predicate))
     ((eq flag 'metadata)
      '(metadata (category . buffer) (cycle-sort-function . identity))))))

(defun romacs--yes-or-no-p (prompt)
  "fns.c Fyes_or_no_p, without dialog boxes: ask until the answer read
from the minibuffer is \"yes\" or \"no\"."
  (unless (stringp prompt) (signal 'wrong-type-argument (list 'stringp prompt)))
  (if use-short-answers
      (y-or-n-p prompt)
    (let* ((length (length prompt))
           (prompt (concat prompt
                           (if (and (> length 0)
                                    (not (string-match-p "\\`[[:blank:]]\\'" (string (aref prompt (1- length))))))
                               " " "")
                           yes-or-no-prompt))
           (enable-recursive-minibuffers t)
           ;; So `repeat' repeats the command that asked.
           (real-this-command real-this-command)
           answer)
      (catch 'romacs--answered
        (while t
          (setq answer (downcase (read-from-minibuffer prompt nil nil nil 'yes-or-no-p-history)))
          (cond ((equal answer "yes") (throw 'romacs--answered t))
                ((equal answer "no") (throw 'romacs--answered nil)))
          (ding)
          (discard-input)
          (message "Please answer yes or no.")
          (sleep-for 2))))))
)
