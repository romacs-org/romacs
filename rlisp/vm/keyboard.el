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

(defvar romacs--reading-command-keys nil
  "Non-nil while the command loop reads a command's key sequence, between
commands: completed background commands may run their callbacks then.")

(defun romacs--dispatch-background ()
  "Run completed background commands' callbacks while the command loop
waits for a command's keys outside any minibuffer read (rlisp/vm/process.el
runs them between commands, never inside a recursive minibuffer read).
Non-nil when any ran."
  (when (and romacs--reading-command-keys (= (minibuffer-depth) 0)
             (fboundp 'romacs-process-dispatch-ready) (romacs--process-pending-p))
    (romacs-process-dispatch-ready)
    t))

(defun romacs--wait-state ()
  "What a host wait's deadline depends on besides the clock: whether input
is requeued, and when each ordinary and idle timer fires.  A redisplay
inside the wait that changes it (a mode-line `:eval' that schedules a
timer) ends the wait, so the deadline is computed again (host_input.rs)."
  (let ((times nil)
        (lists (list (and (boundp 'timer-list) timer-list)
                     (and (boundp 'timer-idle-list) timer-idle-list))))
    (while lists
      (let ((timers (car lists)))
        (while timers
          (setq times (cons (and (fboundp 'romacs--timer-time) (romacs--timer-time (car timers))) times)
                timers (cdr timers))))
      (setq times (cons 'idle times) lists (cdr lists)))
    (cons (romacs--unread-p) times)))

(defun romacs--read-host-char (seconds)
  "keyboard.c kbd_buffer_get_event's wait for the host keyboard: the next
host event, running timers as they fall due, or nil once SECONDS pass or a
timer queues input in `unread-command-events' (which read_char reads next).
Without SECONDS Emacs is idle meanwhile, so idle timers run too (read_char's
timer_start_idle and timer_stop_idle).  A VM without the process prelude
runs no timers."
  (let ((end (and seconds (+ (float-time) seconds)))
        (timers (fboundp 'romacs--timer-check))
        (c nil) (waiting t) (shown nil))
    (and timers (null end) (romacs--timer-start-idle))
    (unwind-protect
        (while waiting
          ;; Each round, as wait_reading_process_output's: completed
          ;; background commands' callbacks (before every block, not only
          ;; after a completion's wake, which a minibuffer read may have
          ;; used), then one timer_check pass, which runs each ripe timer at
          ;; most once.  Requeued input and the time limit come next, then
          ;; the deadline, from the timers as they are now: one that a
          ;; callback or timer added and is ripe already gives 0 and another
          ;; round, so a timer that keeps rescheduling itself never starves
          ;; the keyboard or the limit.  The host redisplays only when that
          ;; changed something or it has not yet, and a redisplay that
          ;; schedules a timer or requeues input ends the wait
          ;; (`romacs--wait-state').
          (let ((dispatched (romacs--dispatch-background)) left)
            (and timers (romacs--timer-check))
            (setq left (and end (- end (float-time))))
            (if (or (romacs--unread-p) (and left (<= left 0)))
                (setq waiting nil)
              ;; A VM without the process prelude runs no timers.
              (let ((ran (if timers romacs--timers-run 0))
                    (delay (and timers (romacs--timer-delay))))
                (setq c (romacs--read-host-event
                         (if (and delay left) (min delay left) (or delay left))
                         (and shown (not dispatched) (= shown ran)))
                      shown ran
                      waiting (null c))))))
      (and timers (null end) (romacs--timer-stop-idle)))
    c))

(defun romacs--unread-p ()
  "Whether read_char has a requeued event: in any of its unread queues."
  (or unread-post-input-method-events unread-command-events unread-input-method-events))

(defun romacs--unwrap-menu-event (c)
  "read_char's undoing of read_char_x_menu_prompt: (EVENT) is EVENT."
  (if (and (consp c) (or (symbolp (car c)) (integerp (car c))) (null (cdr c))) (car c) c))

(defun romacs--read-char (&optional seconds)
  "keyboard.c read_char: the next event, recorded as input.
It comes from `unread-post-input-method-events', `unread-command-events'
or `unread-input-method-events', in that order, else from the executing
keyboard macro unless that is at its end, else from the host keyboard,
waiting at most SECONDS when given (then nil).  No input method runs:
`input-method-function' is never called (DIVERGENCES.md VM2-KBD1).  With none of those and no host,
SECONDS gives nil and its absence `end-of-file', as batch GNU's minibuffer
reports it, raised from the innermost recursive edit when there is one."
  (let ((reread nil) (recorded nil) c (again t))
   (while again
    (setq again nil)
    (cond
     (unread-post-input-method-events
      (setq c (romacs--unwrap-menu-event (car unread-post-input-method-events))
            unread-post-input-method-events (cdr unread-post-input-method-events)
            reread t))
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
     (unread-input-method-events
      (setq c (romacs--unwrap-menu-event (car unread-input-method-events))
            unread-input-method-events (cdr unread-input-method-events)
            reread t))
     ((and executing-kbd-macro (not (romacs--at-end-of-macro-p)))
      (setq c (aref executing-kbd-macro executing-kbd-macro-index))
      ;; A string's 0x80 bit is the meta modifier.
      (when (and (stringp executing-kbd-macro) (/= 0 (logand c #x80)) (<= c #xff))
        (setq c (logior #x8000000 (logand c #x7f))))
      (setq executing-kbd-macro-index (1+ executing-kbd-macro-index)))
     ((romacs--host-input-p)
      (setq c (romacs--read-host-char seconds))
      (cond (c (unless executing-kbd-macro
                 (setq num-nonmacro-input-events (1+ num-nonmacro-input-events))))
            ;; A timer queued input while the host waited: read it as GNU's
            ;; wait returns on requeued_command_events_pending_p.
            ((romacs--unread-p) (setq again t))))
     (seconds)
     ;; Batch GNU blocks for the terminal here. With no input left, VM2
     ;; leaves the innermost recursive edit or minibuffer instead, so a
     ;; command loop that reports errors cannot spin on this one.
     ((> (recursion-depth) 0)
      (throw 'exit (lambda () (signal 'end-of-file '("Error reading from stdin")))))
     (t (signal 'end-of-file '("Error reading from stdin")))))
    (when c
      ;; record_char: recent-keys leaves out keyboard-macro input.
      (unless (or recorded executing-kbd-macro
                  (and (boundp 'inhibit--record-char) inhibit--record-char))
        (romacs--record-input c))
      ;; A read with SECONDS leaves its event out of the command's keys.
      (when (and (null seconds)
                 (or (not reread) (= (length romacs--command-keys) 0)))
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

(defun romacs--keyremap (map)
  "A keyboard.c keyremap scanning through MAP: [PARENT MAP START END].
MAP is the submap of PARENT the events from START to END lead to."
  (vector map map 0 0))

(defun romacs--keyremap-reset (remap start)
  "Restart REMAP's scan at START, from its whole map."
  (aset remap 1 (aref remap 0))
  (aset remap 2 start)
  (aset remap 3 start))

(defun romacs--keyremap-shift (remap diff)
  "Move REMAP's scan by DIFF events, after a translation before it."
  (aset remap 2 (+ (aref remap 2) diff))
  (aset remap 3 (+ (aref remap 3) diff)))

(defun romacs--keyremap-step (keys remap doit)
  "keyboard.c keyremap_step: extend REMAP's scan of KEYS by the event at its END.
With DOIT, when the events from START through that one are bound to an
array (or to a function returning one), return KEYS with them replaced
by it; the scan restarts after the replacement. Otherwise return nil:
a prefix of a longer translation keeps START, and anything else moves
START on by one event, where the scan restarts."
  (let* ((start (aref remap 2)) (end (aref remap 3))
         (next (and (keymapp (aref remap 0))
                    (lookup-key (aref remap 1) (vector (aref keys end)) t))))
    (aset remap 3 (1+ end))
    ;; access_keymap_keyremap: a symbol whose function is an array or keymap.
    (when (and next (symbolp next) (fboundp next)
               (or (arrayp (symbol-function next)) (keymapp (symbol-function next))))
      (setq next (symbol-function next)))
    (when (and doit (functionp next))
      (setq next (let ((current-key-remap-sequence (romacs--subvector keys start (1+ end))))
                   (funcall next nil)))
      (unless (or (null next) (vectorp next) (stringp next))
        (error "Function %s returns invalid key sequence" next)))
    (cond ((and doit (or (vectorp next) (stringp next)))
           (romacs--keyremap-reset remap (+ start (length next)))
           (vconcat (romacs--subvector keys 0 start) next (romacs--subvector keys (1+ end))))
          ((keymapp next) (aset remap 1 next) nil)
          (t (romacs--keyremap-reset remap (1+ start)) nil))))

(defun romacs--translate-keys (keys n binding indec fkey keytran)
  "read_key_sequence's translation scans once N events of KEYS are read.
Each map scans, one event at a time, only what the map before it has
finished with: `input-decode-map' (INDEC) up to N, the function key map
(FKEY) up to INDEC's start while BINDING is undefined or a prefix, and
`key-translation-map' (KEYTRAN) up to FKEY's start. The scans advance
in place. Return KEYS with the first translation made, the scans of
the maps before it moved by its change of length, or nil."
  (let ((new nil))
    (while (and (null new) (< (aref indec 3) n))
      (setq new (romacs--keyremap-step keys indec t)))
    (unless new
      (if (and (not (keymapp binding)) (not (romacs--undefined-p binding))
               (>= (aref indec 2) n))
          ;; A complete binding with no decoding pending holds no function key.
          (when (< (aref fkey 2) n) (romacs--keyremap-reset fkey n))
        (while (and (null new) (< (aref fkey 3) (aref indec 2)))
          (setq new (romacs--keyremap-step
                     keys fkey (and (= (1+ (aref fkey 3)) n) (romacs--undefined-p binding))))
          (when new (romacs--keyremap-shift indec (- (length new) (length keys)))))))
    (while (and (null new) (< (aref keytran 3) (aref fkey 2)))
      (setq new (romacs--keyremap-step keys keytran t))
      (when new
        (romacs--keyremap-shift indec (- (length new) (length keys)))
        (romacs--keyremap-shift fkey (- (length new) (length keys)))))
    new))

(defun romacs--read-key-sequence (dont-downcase-last)
  "keyboard.c read_key_sequence: read a complete key sequence.
Return its events as a vector, or nil at the end of an executing
keyboard macro. Set `romacs--key-sequence-command' to its binding in the
active keymaps (defaults included, not remapped) and
`romacs--key-sequence-remapped' to that command's remapping.
As GNU does, `input-decode-map' translates every sequence,
`local-function-key-map' an undefined one and `key-translation-map' any
one (`romacs--translate-keys'), and the sequence is then reread from its
start, so a translation's events pass through the maps after it. An
unbound upper-case or shifted key is looked up without its shift, and
`this-command-keys-shift-translated' says when that was used."
  (let* ((keys-start (length romacs--command-keys))
         (indec (romacs--keyremap (and (boundp 'input-decode-map) input-decode-map)))
         (fkey (romacs--keyremap (if (boundp 'local-function-key-map)
                                     local-function-key-map function-key-map)))
         (keytran (romacs--keyremap key-translation-map))
         ;; KEYS is keyboard.c's keybuf: N events of it are read, and those
         ;; up to MOCK (mock_input) are reread before new input. It always
         ;; holds (max N MOCK) events.
         (keys []) (n 0) (mock 0) (raw []) (binding nil) (first-unbound 0)
         (upper nil) (shift-translated nil) (fake-prefixed nil) (state 'replay))
    (setq romacs--single-command-key-start keys-start)
    (while (eq state 'replay)
      ;; replay_sequence: the record and the active maps start over.
      (setq state 'read n 0 first-unbound most-positive-fixnum binding '(keymap)
            last-nonmenu-event nil
            romacs--command-keys (romacs--subvector romacs--command-keys 0 keys-start))
      ;; Read while the sequence is a prefix, or unbound with a translation
      ;; still pending.
      (while (and (eq state 'read)
                  (if binding (keymapp binding) (< (aref keytran 2) n)))
        (if (< first-unbound (aref keytran 2))
            ;; The prefix up to FIRST-UNBOUND is unbound with no translation
            ;; left to try: drop it and go on after it.
            (let ((drop (1+ first-unbound)))
              (setq keys (romacs--subvector keys drop n) mock (- n drop) state 'replay)
              (dolist (remap (list indec fkey keytran))
                (romacs--keyremap-reset remap (- (aref remap 2) drop))))
          (let ((local-keys romacs--command-keys) (key nil) (again t))
            ;; replay_key, again after a fake prefix key.
            (while again
              (setq again nil romacs--command-keys local-keys)
              (cond ((< n mock)
                     (setq key (aref keys n))
                     (romacs--add-command-key key))
                    ((and executing-kbd-macro (romacs--at-end-of-macro-p)
                          (not unread-command-events))
                     (setq state 'end))
                    (t (setq key (romacs--read-char)
                             raw (vconcat raw (list key))
                             keys (vconcat keys (list key)))))
              ;; A click in a mode line, header line or margin is looked
              ;; up after its area, a fake prefix key.
              (when (and (eq state 'read) (consp key) (consp (cdr key))
                         (consp (car (cdr key))) (not (memq key fake-prefixed)))
                (let ((area (posn-area (event-start key))))
                  (when (and area (symbolp area))
                    (setq keys (vconcat (romacs--subvector keys 0 n) (vector area key))
                          mock (+ n 2) fake-prefixed (cons key fake-prefixed) again t)))))
            (when (eq state 'read)
              (setq binding (key-binding (romacs--subvector keys 0 (1+ n)) t t)
                    first-unbound (if binding (max (1+ n) first-unbound) (min n first-unbound))
                    n (1+ n)
                    last-nonmenu-event key
                    romacs--single-command-key-start
                    (max 0 (- (length romacs--command-keys) n)))
              (let ((new (romacs--translate-keys keys n binding indec fkey keytran))
                    (lower (romacs--unshifted key)))
                (cond
                 ;; mock_input = diff + max (t, mock_input): all of NEW.
                 (new (setq keys new mock (length new) state 'replay))
                 ((and (null binding) (>= (aref keytran 2) n) lower)
                  (aset keys (1- n) lower)
                  (setq upper (cons (1- n) key) mock (max n mock)
                        shift-translated t state 'replay)
                  ;; A shifted function key goes through the function key
                  ;; map again: S-backspace reaches DEL through backspace.
                  (unless (integerp key)
                    (aset fkey 2 0) (aset fkey 3 0)
                    (aset keytran 2 0) (aset keytran 3 0))))))))))
    (if (eq state 'end)
        (setq keys nil)
      (when (and upper (= (car upper) (1- n)) (or dont-downcase-last (null binding)))
        (aset keys (car upper) (cdr upper))
        (setq shift-translated nil))
      (when shift-translated (setq this-command-keys-shift-translated t))
      ;; A translation's events are all part of the sequence, even past
      ;; its binding.
      (while (< n mock)
        (romacs--add-command-key (aref keys n))
        (setq n (1+ n)))
      (setq keys (romacs--subvector keys 0 n)))
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

(defun romacs--command-loop-resume ()
  "The start of keyboard.c command_loop_1, which also resumes the loop after
a command signalled out of it: `post-command-hook' runs for that command."
  (setq prefix-arg nil last-prefix-arg nil deactivate-mark nil
        romacs--command-keys [] romacs--single-command-key-start 0)
  (romacs--safe-run-hooks 'post-command-hook)
  (setq last-command this-command real-last-command real-this-command)
  (unless (consp last-command-event)
    (setq last-repeatable-command real-this-command)))

(defun romacs--command-loop-step ()
  "One pass of keyboard.c command_loop_1's loop: read a key sequence and
run its command between `pre-command-hook' and `post-command-hook'.
Return the keys read, nil at the end of an executing keyboard macro."
  (set-buffer (window-buffer (selected-window)))
  (setq deactivate-mark nil this-command nil real-this-command nil
        this-original-command nil this-command-keys-shift-translated nil)
  (let ((keys (let ((romacs--reading-command-keys t)) (romacs--read-key-sequence nil)))
        cmd previous-buffer previous-tick)
    (set-buffer (window-buffer (selected-window)))
    (setq num-input-keys (1+ num-input-keys))
    (when keys
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
        ;; point_before_last_command_or_undo, before the command's edits.
        (romacs--note-command-point)
        ;; The display's delete-selection policy (romacs-editing.el). A
        ;; command the deletion supersedes runs as `ignore', as delsel.el's
        ;; pre-command-hook makes it, so it still consumes the prefix argument.
        (when (and (fboundp 'romacs-delete-selection-before-command)
                   (romacs-delete-selection-before-command this-command))
          (setq this-command 'ignore))
        (command-execute this-command))
      (setq last-prefix-arg current-prefix-arg)
      (romacs--safe-run-hooks 'post-command-hook)
      (setq last-command this-command real-last-command real-this-command)
      (unless (consp last-command-event)
        (setq last-repeatable-command real-this-command))
      (setq romacs--command-keys [] romacs--single-command-key-start 0)
      (romacs--after-command-mark previous-buffer previous-tick))
    keys))

(defun romacs--command-loop-1 ()
  "keyboard.c command_loop_1: read and execute commands.
Return nil at the end of an executing keyboard macro."
  (romacs--command-loop-resume)
  (while (romacs--command-loop-step))
  nil)

(defun romacs--host-command ()
  "keyboard.c command_loop_2 one command at a time, for a host whose own
loop calls this whenever input waits (the GUI): read one key sequence,
from the unread queues, a keyboard macro, then the host, and run its
command as command_loop_1 does.  Value: (COMMAND . ERROR), COMMAND
`this-command' and ERROR nil or the (CONDITION . DATA) it signalled,
which the host reports as cmd_error would.  The loop then resumes, so
`post-command-hook' runs for that command too."
  (condition-case data
      (progn (romacs--command-loop-step) (cons this-command nil))
    (t (prog1 (cons this-command data)
         (unless (memq 'minibuffer-quit (get (car data) 'error-conditions))
           (setq executing-kbd-macro nil romacs--executing-macro nil))
         (setq quit-flag nil inhibit-quit nil)
         (romacs--command-loop-resume)))))

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
