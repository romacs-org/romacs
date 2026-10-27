(progn
;;; keyboard.el --- VM2: the Lisp half of the command loop  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; Evaluated once per VM, on first use, by prims/keyboard.rs. GNU's
;; command loop (keyboard.c command_loop_1), key reader (read_key_sequence,
;; read_char) and keyboard macros (macros.c Fexecute_kbd_macro) call Lisp
;; at every step, so they are Lisp here; the GNU primitives are Rust entry
;; points into these functions. The C globals they keep are variables:
;; `romacs--command-keys' (this_command_keys, a vector),
;; `romacs--single-command-key-start' and `romacs--raw-command-keys' are
;; declared by the VM, the rest below.
;;
;; Events come from `unread-command-events', then the executing keyboard
;; macro, then the host keyboard (host_input.rs) when one is attached.
;; Without one, a read that would wait signals end of file; see
;; DIVERGENCES.md VM2-KBD1 for this and the other limits of this port.
;; This whole file is compiled as one form before its defvars run, so its
;; own variables are only ever set, never let-bound.

;;; Code:

(defvar romacs--command-loop-level 0 "keyboard.c command_loop_level.")
(defvar romacs--executing-macro nil
  "macros.c executing_kbd_macro: the macro under way, for error messages.")
(defvar romacs--macro-iterations 0 "macros.c executing_kbd_macro_iterations.")
(defvar romacs--key-sequence-command nil "keyboard.c read_key_sequence_cmd.")
(defvar romacs--key-sequence-remapped nil "keyboard.c read_key_sequence_remapped.")

;; BOOTSTRAP STAND-INS (DIVERGENCES.md VM2-KBD1). The command loop calls
;; `command-execute' (simple.el) and `undefined' (subr.el), which a bare VM
;; has not loaded. These minimal copies are installed only where nothing is
;; defined; loading GNU's files replaces them.
(unless (fboundp 'command-execute)
  (defun command-execute (cmd &optional record-flag keys special)
    "Execute CMD as an editor command (simple.el:2794, without disabled
commands and autoloads)."
    (let ((prefixarg (unless special
                       (prog1 prefix-arg
                         (setq current-prefix-arg prefix-arg prefix-arg nil)))))
      (if (arrayp (indirect-function cmd))
          (execute-kbd-macro (indirect-function cmd) prefixarg)
        (call-interactively cmd record-flag keys)))))
(unless (fboundp 'undefined)
  (defun undefined ()
    "Beep to tell the user this binding is undefined (subr.el:1297)."
    (interactive)
    (ding)
    (message "%s is undefined" (key-description (this-single-command-keys)))
    (setq prefix-arg nil)))

(defun romacs--host-input-p ()
  "Non-nil when the embedder attached a keyboard (Vm::set_host_input)."
  (and (boundp 'romacs--host-input) romacs--host-input))

;;;; read_char

(defun romacs--at-end-of-macro-p ()
  "macros.c at_end_of_macro_p, while `executing-kbd-macro' is non-nil."
  (or (eq executing-kbd-macro t)
      (>= executing-kbd-macro-index (length executing-kbd-macro))))

(defun romacs--add-command-key (key)
  "keyboard.c add_command_key."
  (setq romacs--command-keys (vconcat romacs--command-keys (list key))))

(defun romacs--read-char (&optional seconds)
  "keyboard.c read_char: the next event, recorded as input.
It comes from `unread-command-events', else from the executing keyboard
macro unless that is at its end, else from the host keyboard. With
SECONDS and none of those, nil (VM2 does not wait on the host); with
none at all, `end-of-file' as batch GNU's minibuffer reports it, raised
from the innermost recursive edit when there is one."
  (let ((reread nil) (recorded nil) c)
    (cond
     (unread-command-events
      (setq c (car unread-command-events)
            unread-command-events (cdr unread-command-events))
      ;; (t . EVENT) is read as new input; (no-record . EVENT) was recorded.
      (if (and (consp c) (eq (car c) t))
          (setq c (cdr c))
        (when (and (consp c) (eq (car c) 'no-record))
          (setq c (cdr c) recorded t))
        (setq reread t))
      (when (and (consp c) (eq (cdr c) 'disabled)
                 (or (symbolp (car c)) (integerp (car c))))
        (setq c (car c))))
     ((and executing-kbd-macro (not (romacs--at-end-of-macro-p)))
      (setq c (aref executing-kbd-macro executing-kbd-macro-index))
      ;; A string's 0x80 bit is the meta modifier.
      (when (and (stringp executing-kbd-macro) (/= 0 (logand c #x80)) (<= c #xff))
        (setq c (logior #x8000000 (logand c #x7f))))
      (setq executing-kbd-macro-index (1+ executing-kbd-macro-index)))
     ((and (null seconds) (romacs--host-input-p))
      (setq c (romacs--read-host-event))
      (unless executing-kbd-macro
        (setq num-nonmacro-input-events (1+ num-nonmacro-input-events))))
     (seconds)
     ;; Batch GNU blocks for the terminal here. With no input left, VM2
     ;; leaves the innermost recursive edit or minibuffer instead, so a
     ;; command loop that reports errors cannot spin on this one.
     ((> (recursion-depth) 0)
      (throw 'exit (lambda () (signal 'end-of-file '("Error reading from stdin")))))
     (t (signal 'end-of-file '("Error reading from stdin"))))
    (when c
      ;; record_char: recent-keys leaves out keyboard-macro input.
      (unless (or recorded executing-kbd-macro
                  (and (boundp 'inhibit--record-char) inhibit--record-char))
        (romacs--record-input c))
      (when (or (not reread) (= (length romacs--command-keys) 0))
        (romacs--add-command-key c))
      (setq last-input-event c))
    c))

;; keyboard.c read_filtered_event, behind read-event, read-char and
;; read-char-exclusive (Fread_char's modifier resolution is Rust).
(defun romacs--read-filtered-event (prompt seconds ascii-required error-nonascii)
  (when inhibit-interaction (signal 'inhibited-interaction nil))
  (when prompt (message "%s" prompt))
  (let ((seconds (and (numberp seconds) seconds)) val)
    (while (progn
             (setq val (romacs--read-char seconds))
             (and ascii-required (not (and seconds (null val)))
                  (progn
                    ;; A function key with an ASCII equivalent is that
                    ;; character, with the symbol's modifiers.
                    (when (symbolp val)
                      (let* ((mask (get val 'event-symbol-element-mask))
                             (ascii (and mask (get (car mask) 'ascii-character))))
                        (when (and (integerp ascii) (integerp (car (cdr mask))))
                          (setq val (logior ascii (car (cdr mask)))))))
                    (and (not (integerp val))
                         (if error-nonascii
                             (progn (setq unread-command-events (list val))
                                    (error "Non-character input-event"))
                           t))))))
    val))

;;;; read_key_sequence

(defun romacs--unshifted (event)
  "EVENT without its shift: read_key_sequence's upper-case fallback.
Nil when EVENT is neither shifted nor an upper-case character."
  (when translate-upper-case-key-bindings
    (cond
     ((integerp event)
      (let ((char (logand event #x3fffff)))
        (cond ((/= 0 (logand event #x2000000)) (logand event (lognot #x2000000)))
              ((and (characterp char) (/= (downcase char) char))
               (logior (downcase char) (logand event (lognot #x3fffff)))))))
     ((symbolp event)
      (let ((name (symbol-name event)))
        ;; GNU spells modifiers in the order A-C-H-M-S-s-.
        (when (string-match "\\`\\(?:[ACHM]-\\)*S-" name)
          (intern (concat (substring name 0 (- (match-end 0) 2))
                          (substring name (match-end 0))))))))))

(defun romacs--subvector (keys from &optional to)
  "The events of KEYS from index FROM to TO (default its end), as a vector."
  (let ((i (or to (length keys))) (out nil))
    (while (> i from)
      (setq i (1- i))
      (push (aref keys i) out))
    (vconcat out)))

(defun romacs--undefined-p (binding)
  "keyboard.c test_undefined."
  (or (null binding) (eq binding 'undefined)
      (and (symbolp binding) (eq (command-remapping binding) 'undefined))))

(defun romacs--keyremap (keys start map doit)
  "keyboard.c keyremap_step: translate a suffix of KEYS through MAP.
Suffixes are tried from index START on. With DOIT, a suffix bound to an
array (or to a function returning one) is replaced by it. Return (KEYS
START PENDING): START is where later translation may begin, PENDING is
non-nil while KEYS from START is a prefix of a longer translation."
  (let ((pending nil) (searching (keymapp map)))
    (while (and searching (< start (length keys)))
      (let ((next (lookup-key map (romacs--subvector keys start) t)))
        (when (and doit (functionp next) (not (keymapp next)))
          (setq next (funcall next nil))
          (unless (or (null next) (vectorp next) (stringp next))
            (error "Function %s returns invalid key sequence" next)))
        (cond ((and doit (or (vectorp next) (stringp next)))
               (setq keys (vconcat (romacs--subvector keys 0 start) next)
                     start (length keys) searching nil))
              ((keymapp next) (setq pending t searching nil))
              (t (setq start (1+ start))))))
    (list keys start pending)))

(defun romacs--read-key-sequence (dont-downcase-last)
  "keyboard.c read_key_sequence: read a complete key sequence.
Return its events as a vector, or nil at the end of an executing
keyboard macro. Set `romacs--key-sequence-command' to its binding in the
active keymaps (defaults included, not remapped) and
`romacs--key-sequence-remapped' to that command's remapping.
As GNU does, `input-decode-map' translates every sequence,
`local-function-key-map' an undefined one and `key-translation-map' any
one; an unbound upper-case or shifted key is then looked up without its
shift, and `this-command-keys-shift-translated' says when that was used."
  (let* ((start (length romacs--command-keys))
         (decode (and (boundp 'input-decode-map) input-decode-map))
         (function-keys (if (boundp 'local-function-key-map) local-function-key-map function-key-map))
         (keys []) (raw []) (upper nil) (binding nil) (reading t)
         (indec 0) (fkey 0) (keytran 0) (keytran-pending nil))
    (setq romacs--single-command-key-start start)
    (while reading
      (if (and executing-kbd-macro (romacs--at-end-of-macro-p)
               (not unread-command-events))
          (setq keys nil reading nil)
        (let ((event (romacs--read-char)))
          (setq raw (vconcat raw (list event))
                last-nonmenu-event event)
          ;; A click in a mode line, header line or margin is looked up
          ;; after its area, a fake prefix key.
          (when (and (= (length keys) 0) (consp event) (consp (cdr event))
                     (consp (car (cdr event))))
            (let ((area (posn-area (event-start event))))
              (when (and area (symbolp area))
                (setq keys (vector area) indec 1 fkey 1 keytran 1))))
          (setq keys (vconcat keys (list event)))
          (let ((step (romacs--keyremap keys indec decode t)))
            (setq keys (nth 0 step) indec (nth 1 step)))
          (setq binding (key-binding keys t t))
          (let ((step (romacs--keyremap keys fkey function-keys (romacs--undefined-p binding))))
            (unless (eq (nth 0 step) keys)
              (setq binding (key-binding (nth 0 step) t t)))
            (setq keys (nth 0 step) fkey (nth 1 step)))
          (let ((step (romacs--keyremap keys keytran key-translation-map t)))
            (unless (eq (nth 0 step) keys)
              (setq binding (key-binding (nth 0 step) t t)))
            (setq keys (nth 0 step) keytran (nth 1 step) keytran-pending (nth 2 step)))
          (cond
           ((keymapp binding))
           (binding (setq reading nil))
           (keytran-pending)
           (t
            (let ((lower (romacs--unshifted (aref keys (1- (length keys))))))
              (if (null lower)
                  (setq reading nil)
                (setq upper (cons (1- (length keys)) (aref keys (1- (length keys)))))
                (aset keys (1- (length keys)) lower)
                (setq binding (key-binding keys t t))
                ;; S-backspace reaches DEL through function-key-map.
                (let ((step (romacs--keyremap keys (1- (length keys)) function-keys
                                              (romacs--undefined-p binding))))
                  (unless (eq (nth 0 step) keys)
                    (setq binding (key-binding (nth 0 step) t t)))
                  (setq keys (nth 0 step) fkey (nth 1 step)))
                (unless (keymapp binding) (setq reading nil)))))))))
    (when keys
      (let ((shifted (and upper (= (car upper) (1- (length keys))))))
        (when (and shifted (or dont-downcase-last (null binding)))
          (aset keys (car upper) (cdr upper))
          (setq shifted nil))
        (when shifted (setq this-command-keys-shift-translated t)))
      ;; The record holds the sequence as read, translations applied.
      (setq romacs--command-keys
            (vconcat (romacs--subvector romacs--command-keys 0 start) keys)))
    (setq romacs--raw-command-keys raw
          romacs--key-sequence-command (and keys binding)
          romacs--key-sequence-remapped
          (and keys (symbolp binding) binding (command-remapping binding)))
    keys))

;; keyboard.c read_key_sequence_vs; Fread_key_sequence's string form is Rust.
(defun romacs--read-key-sequence-vs (prompt continue-echo dont-downcase-last _cmd-loop)
  (and prompt (not (stringp prompt))
       (signal 'wrong-type-argument (list 'stringp prompt)))
  (unless continue-echo
    (setq romacs--command-keys [] romacs--single-command-key-start 0))
  (or (romacs--read-key-sequence dont-downcase-last) []))

;;;; command_loop_1

(defun romacs--safe-run-hooks (hook)
  "keyboard.c safe_run_hooks: run HOOK, each function under a handler that
reports an error and removes that function from HOOK."
  (when (and (boundp hook) (symbol-value hook))
    (let ((inhibit-quit t))
      (run-hook-wrapped hook #'romacs--safe-run-hook-function hook))))

(defun romacs--safe-run-hook-function (function hook)
  "keyboard.c safe_run_hook_funcall and safe_run_hooks_error."
  (condition-case err
      (funcall function)
    (t (message "Error in %s (%S): %S" hook function err)
       (let ((without (lambda (list)
                        (let ((kept nil))
                          (while (consp list)
                            (unless (eq (car list) function) (push (car list) kept))
                            (setq list (cdr list)))
                          (nreverse kept)))))
         (cond ((and (consp (symbol-value hook)) (memq function (symbol-value hook)))
                (set hook (funcall without (symbol-value hook))))
               ((and (default-boundp hook) (consp (default-value hook))
                     (memq function (default-value hook)))
                (set-default hook (funcall without (default-value hook))))))))
  nil)

(defun romacs--after-command-mark (previous-buffer previous-tick)
  "command_loop_1 after a command: deactivate the mark if the command asked
for it, else run `activate-mark-hook' after a change of buffer or text."
  (when mark-active
    (cond ((eq transient-mark-mode 'identity) (setq transient-mark-mode nil))
          ((eq transient-mark-mode 'only) (setq transient-mark-mode 'identity)))
    (if deactivate-mark
        (when (fboundp 'deactivate-mark) (deactivate-mark))
      (when (or (not (eq (current-buffer) previous-buffer))
                (/= (buffer-modified-tick) previous-tick))
        (run-hooks 'activate-mark-hook)))))

(defun romacs--command-loop-1 ()
  "keyboard.c command_loop_1: read and execute commands.
Return nil at the end of an executing keyboard macro."
  (setq prefix-arg nil last-prefix-arg nil deactivate-mark nil
        romacs--command-keys [] romacs--single-command-key-start 0)
  ;; After a command that signalled out of the loop.
  (romacs--safe-run-hooks 'post-command-hook)
  (setq last-command this-command real-last-command real-this-command)
  (unless (consp last-command-event)
    (setq last-repeatable-command real-this-command))
  (let ((reading t) keys cmd previous-buffer previous-tick)
    (while reading
      (set-buffer (window-buffer (selected-window)))
      (setq deactivate-mark nil this-command nil real-this-command nil
            this-original-command nil this-command-keys-shift-translated nil)
      (setq keys (romacs--read-key-sequence nil))
      (set-buffer (window-buffer (selected-window)))
      (setq num-input-keys (1+ num-input-keys))
      (if (null keys)
          (setq reading nil)
        (setq last-command-event (aref keys (1- (length keys)))
              cmd romacs--key-sequence-command
              previous-buffer (current-buffer)
              previous-tick (buffer-modified-tick)
              deactivate-mark nil
              this-original-command cmd)
        (when romacs--key-sequence-remapped
          (setq cmd romacs--key-sequence-remapped))
        (romacs--record-input cmd t)
        (setq this-command cmd real-this-command cmd)
        (romacs--safe-run-hooks 'pre-command-hook)
        (if (null this-command)
            (undefined)
          (when (fboundp 'undo-auto--add-boundary) (undo-auto--add-boundary))
          ;; The display's delete-selection policy (romacs-editing.el).
          (unless (and (fboundp 'romacs-delete-selection-before-command)
                       (romacs-delete-selection-before-command this-command))
            (command-execute this-command)))
        (setq last-prefix-arg current-prefix-arg)
        (romacs--safe-run-hooks 'post-command-hook)
        (setq last-command this-command real-last-command real-this-command)
        (unless (consp last-command-event)
          (setq last-repeatable-command real-this-command))
        (setq romacs--command-keys [] romacs--single-command-key-start 0)
        (romacs--after-command-mark previous-buffer previous-tick))))
  nil)

(defun romacs--cmd-error (data)
  "keyboard.c cmd_error: report DATA through `command-error-function' and
resume the command loop. Any error but `minibuffer-quit' ends the macro."
  (let ((context (cond ((null romacs--executing-macro) "")
                       ((= romacs--macro-iterations 1) "After 1 kbd macro iteration: ")
                       (t (format "After %d kbd macro iterations: "
                                  romacs--macro-iterations)))))
    (unless (memq 'minibuffer-quit (get (car data) 'error-conditions))
      (setq executing-kbd-macro nil romacs--executing-macro nil))
    (setq prefix-arg nil last-prefix-arg nil)
    (let ((standard-output t) (standard-input t)
          (print-level 10) (print-length 10) (inhibit-quit t))
      (when command-error-function
        (funcall command-error-function data context nil))))
  (setq quit-flag nil inhibit-quit nil)
  0)

(defun romacs--command-loop-2 (errors)
  "keyboard.c command_loop_2: run the command loop until it returns.
With ERRORS, `error' is reported and the loop resumes (a recursive edit);
else only `minibuffer-quit' is (a keyboard macro), so any other signal
leaves through the caller."
  (while (if errors
             (condition-case data (romacs--command-loop-1)
               (error (romacs--cmd-error data)))
           (condition-case data (romacs--command-loop-1)
             (minibuffer-quit (romacs--cmd-error data)))))
  nil)

;;;; Recursive edits

(defun romacs--recursive-edit-1 ()
  "keyboard.c recursive_edit_1: run a command loop until a throw to `exit'.
Its value t quits, a string is an error, a function is called."
  (let ((val (let ((standard-output (if (> romacs--command-loop-level 0) t standard-output))
                   (standard-input (if (> romacs--command-loop-level 0) t standard-input))
                   (inhibit-redisplay nil))
               (prog1 (catch 'exit (romacs--command-loop-2 t))
                 (setq romacs--executing-macro nil)))))
    (cond ((eq val t) (signal 'quit nil))
          ((stringp val) (signal 'error (list val)))
          ((functionp val) (funcall val)))
    nil))

(defun romacs--recursive-edit ()
  "keyboard.c Frecursive_edit."
  (let ((buffer (and (not (eq (current-buffer) (window-buffer (selected-window))))
                     (current-buffer))))
    (setq romacs--command-loop-level (1+ romacs--command-loop-level))
    (unwind-protect (romacs--recursive-edit-1)
      (setq romacs--command-loop-level (1- romacs--command-loop-level))
      (when buffer (set-buffer buffer))))
  nil)

(defun romacs--exit-recursive-edit (abort)
  "keyboard.c Fexit_recursive_edit, or with ABORT Fabort_recursive_edit."
  (if (or (> romacs--command-loop-level 0) (> (minibuffer-depth) 0))
      (throw 'exit abort)
    (signal 'user-error '("No recursive edit is in progress"))))

(defun romacs--top-level ()
  "keyboard.c Ftop_level."
  (throw 'top-level nil))

;;;; Keyboard macros

(defun romacs--execute-kbd-macro (macro count loopfunc)
  "macros.c Fexecute_kbd_macro."
  (let ((repeat 1) (success 0) (continue t) final)
    (setq romacs--macro-iterations 0)
    (when count (setq repeat (prefix-numeric-value count)))
    (setq final (indirect-function macro))
    (unless (or (stringp final) (vectorp final))
      (error "Keyboard macros must be strings or vectors"))
    (let ((outer-macro executing-kbd-macro)
          (outer-index executing-kbd-macro-index)
          (outer-command real-this-command))
      ;; pop_kbd_macro restores the outer macro on every exit.
      (unwind-protect
          (progn
            (while continue
              (setq executing-kbd-macro final romacs--executing-macro final
                    executing-kbd-macro-index 0 prefix-arg nil)
              (if (and loopfunc (null (funcall loopfunc)))
                  (setq continue nil)
                (romacs--command-loop-2 nil)
                (setq success (1+ success) romacs--macro-iterations success)
                (setq repeat (1- repeat))
                (setq continue (and (/= repeat 0)
                                    (or (stringp executing-kbd-macro)
                                        (vectorp executing-kbd-macro))))))
            (setq romacs--executing-macro nil
                  real-this-command executing-kbd-macro))
        (setq executing-kbd-macro outer-macro
              executing-kbd-macro-index outer-index
              real-this-command outer-command)
        (run-hooks 'kbd-macro-termination-hook))))
  nil)

(provide 'romacs-keyboard)
)
