(progn
;;; minibuf.el --- VM2: the Lisp half of read-from-minibuffer  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; Evaluated once per VM, on the first prompt, by prims/minibuf.rs. GNU's
;; read_minibuf (minibuf.c:573) runs a recursive command loop, which calls
;; Lisp per key, so that loop is Lisp here (PRIMITIVE-DOCTRINE section 3).
;; Every value it keeps across a command is a register of a traced frame.
;; GNU minibuf.c `minibuf_level'.

;;; Code:

(defvar romacs--minibuf-level 0)
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

(defun romacs--minibuf-read-event ()
  "Next event from `unread-command-events', or nil at the end of the input.
keyboard.c read_char: an unread (t . EVENT) or (no-record . EVENT) is EVENT.
With no unread event and no keyboard macro, read_char redisplays and waits
for the keyboard: here the host's, when one is attached (host_input.rs)."
  (if unread-command-events
      (let ((event (car unread-command-events)))
        (setq unread-command-events (cdr unread-command-events))
        (if (and (consp event) (memq (car event) '(t no-record))) (cdr event) event))
    (and (not executing-kbd-macro) (romacs--minibuf-host-input-p)
         (romacs--read-host-event))))

(defun romacs--minibuf-host-input-p ()
  "Non-nil when the embedder attached a keyboard (Vm::set_host_input)."
  (and (boundp 'romacs--host-input) romacs--host-input))

(defun romacs--minibuf-read-key-sequence ()
  "A complete key sequence, or nil when the input (the macro) has ended.
keyboard.c read_key_sequence returns 0 at the end of a keyboard macro."
  (let ((event (romacs--minibuf-read-event)) keys)
    (when event
      (setq keys (vector event))
      (while (and keys (keymapp (key-binding keys t)))
        (setq event (romacs--minibuf-read-event))
        (setq keys (and event (apply #'vector (append keys (list event))))))
      keys)))

(defun romacs--minibuf-command-loop ()
  "keyboard.c command_loop_2/command_loop_1, for one minibuffer.
An `error' inside a command is reported and the loop continues (cmd_error);
any other signal, such as `quit', leaves the minibuffer."
  (let (keys)
    (setq last-command this-command real-last-command real-this-command)
    (while (setq keys (romacs--minibuf-read-key-sequence))
      (let ((command (key-binding keys t)))
        (setq last-command-event (aref keys (1- (length keys))))
        (if (null command)
            (message "%S is undefined" keys)
          (setq this-command command real-this-command command
                current-prefix-arg prefix-arg prefix-arg nil)
          (condition-case err
              (unless (and (fboundp 'romacs-delete-selection-before-command)
                           (romacs-delete-selection-before-command command))
                (call-interactively command))
            (error (setq prefix-arg nil) (message "%S" err)))
          (setq last-command this-command real-last-command real-this-command))))
    nil))

(defun romacs--minibuf-recursive-edit ()
  "keyboard.c recursive_edit_1: catch `exit'; t quits, a string is an
error, a function is called (minibuffer-quit-recursive-edit throws one)."
  (let ((val (catch 'exit (romacs--minibuf-command-loop))))
    (cond ((eq val t) (signal 'quit nil))
          ((stringp val) (signal 'error (list val)))
          ((functionp val) (funcall val)))
    nil))

(defun romacs--read-minibuf (prompt keymap initial pos histvar default)
  "minibuf.c read_minibuf (minibuf.c:573) after its argument checks."
  (when (and (> romacs--minibuf-level 0) (not enable-recursive-minibuffers))
    (if (eq (selected-window) (minibuffer-window))
        (signal 'user-error '("Command attempted to use minibuffer while in minibuffer"))
      (throw 'exit "Command attempted to use minibuffer while in minibuffer")))
  ;; minibuf.c:653: outside a keyboard macro batch GNU reads stdin; VM2
  ;; without a host keyboard has no terminal input, which batch GNU reports
  ;; at end of file. With a host keyboard (the GUI) it is interactive GNU.
  (unless (or executing-kbd-macro (romacs--minibuf-host-input-p))
    (signal 'end-of-file '("Error reading from stdin")))
  ;; read_minibuf specbinds `minibuffer-default' and increments
  ;; minibuf_level; both are restored by the unwind below. (setq, not let:
  ;; this whole file is compiled as one form, before its defvars run.)
  (let* ((outer-prefix current-prefix-arg)
         (outer-default minibuffer-default)
         (outer-level romacs--minibuf-level)
         ;; window.c:8428: the entry's window configuration saves the caller
         ;; only inside a minibuffer, and its restore puts it back.
         (outer-selected (and (> outer-level 0) romacs--minibuf-selected-window))
         (level (1+ outer-level))
         (buffer (get-buffer-create (format " *Minibuf-%d*" level)))
         (calling-buffer (current-buffer))
         (calling-window (selected-window))
         (mini (minibuffer-window))
         (mini-buffer (window-buffer mini))
         (prompt-end 1)
         val)
    ;; minibuf.c:794: an unbound history variable starts as nil.
    (unless (boundp histvar) (set histvar nil))
    (setq minibuffer-default default romacs--minibuf-level level)
    (unwind-protect
        (progn
          (set-buffer buffer)
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
          (use-local-map keymap)
          (when (and minibuffer-setup-hook (fboundp 'run-hooks))
            (run-hooks 'minibuffer-setup-hook))
          (romacs--minibuf-recursive-edit)
          (set-buffer buffer)
          (setq val (if minibuffer-allow-text-properties
                        (buffer-substring prompt-end (point-max))
                      (buffer-substring-no-properties prompt-end (point-max)))))
      ;; read_minibuf_unwind (minibuf.c:1111). The level and the default are
      ;; restored FIRST and unconditionally: if a later step signalled, a
      ;; stale level would refuse every future prompt as recursive. Then
      ;; empty this level's buffer, give the window back its idle buffer and
      ;; reselect the caller when it is still live (a command at the prompt
      ;; may have deleted it; GNU's record_unwind_current_buffer likewise
      ;; restores the buffer only if it is live).
      (setq current-prefix-arg outer-prefix prefix-arg nil
            minibuffer-default outer-default romacs--minibuf-level outer-level
            romacs--minibuf-selected-window outer-selected)
      (set-buffer buffer)
      (erase-buffer)
      (set-window-buffer mini mini-buffer)
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
)
