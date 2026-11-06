(progn
;;; keyboard.el --- VM2: the Lisp half of the command loop  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

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
          (setq times (cons (romacs--timer-time (car timers)) times)
                timers (cdr timers))))
      (setq times (cons 'idle times) lists (cdr lists)))
    (cons (romacs--unread-p) times)))

(defun romacs--read-host-char (seconds &optional auto-save)
  "keyboard.c kbd_buffer_get_event's wait for the host keyboard: the next
host event, running timers as they fall due, or nil once SECONDS pass or a
timer queues input in `unread-command-events' (which read_char reads next).
Without SECONDS Emacs is idle meanwhile, so idle timers run too (read_char's
timer_start_idle and timer_stop_idle), and it collects garbage as the host's
idle pass does (`romacs--idle-collect'), once what it ran is on screen.
After AUTO-SAVE idle seconds with no input, it auto-saves once and waits
on.  A VM without the process prelude runs no timers."
  (let ((end (and seconds (+ (float-time) seconds)))
        (save-at (and auto-save (+ (float-time) auto-save)))
        (timers (fboundp 'romacs--timer-check))
        (c nil) (waiting t) (shown nil) (collect-ok t))
    (and timers (null end) (romacs--timer-start-idle))
    (unwind-protect
        (while waiting
          ;; Each round, as wait_reading_process_output's: completed
          ;; background commands' callbacks (before every block, not only
          ;; after a completion's wake, which a minibuffer read may have
          ;; used), the processes' ready output, connections and status
          ;; changes, then one timer_check pass, which runs each ripe timer at
          ;; most once.  Requeued input and the time limit come next, then
          ;; the deadline, from the timers as they are now: one that a
          ;; callback or timer added and is ripe already gives 0 and another
          ;; round, so a timer that keeps rescheduling itself never starves
          ;; the keyboard or the limit.  The host redisplays only when that
          ;; changed something or it has not yet, and a redisplay that
          ;; schedules a timer or requeues input ends the wait
          ;; (`romacs--wait-state').
          (let* ((background (romacs--dispatch-background))
                 ;; read_char's special events: file notification events
                 ;; run here, never input; an error ends the read.
                 (notified (and (fboundp 'romacs--file-notify-dispatch) (romacs--file-notify-dispatch)))
                 ;; Process output and worker messages both, every round:
                 ;; a busy connection must not starve the workers.
                 (processes (and timers (romacs--process-idle)))
                 (isolates (and (fboundp (quote romacs--isolate-idle)) (romacs--isolate-idle)))
                 (dispatched (or processes isolates background notified))
                 left)
            (and timers (romacs--timer-check))
            (when (and save-at (<= save-at (float-time)) (not (romacs--unread-p)) (not (input-pending-p)))
              (setq save-at nil shown nil)
              (romacs--auto-save))
            (setq left (and end (- end (float-time))))
            (if (or (romacs--unread-p) (and left (<= left 0)))
                (setq waiting nil)
              ;; A VM without the process prelude runs no timers.
              (let* ((ran (if timers romacs--timers-run 0))
                     (current (and shown (not dispatched) (= shown ran)))
                     ;; read_char's idle collection: a wait with no limit
                     ;; collects as the top level's does, once its screen is
                     ;; current; one held or abandoned for a key is not
                     ;; tried again in this wait.
                     (collect (and (null end) collect-ok (romacs--idle-collect t)))
                     (delay (and timers (romacs--timer-delay)))
                     (delay (if save-at (min (or delay save-at) (max 0 (- save-at (float-time)))) delay))
                     (delay (cond ((numberp collect) (min (or delay collect) collect))
                                  ((and collect (not current)) 0)
                                  (t delay))))
                (when (and (eq collect t) current (not (romacs--collect-idle)))
                  (setq collect-ok nil))
                ;; A user signal (SIGUSR1, SIGUSR2) is an event before the
                ;; host's (kbd_buffer_get_event).
                (setq c (or (romacs--take-user-signal)
                            (romacs--host-event
                             (if (and delay left) (min delay left) (or delay left))
                             current))
                      shown ran)
                ;; read_char: an event `special-event-map' binds (focus, a
                ;; user signal) runs its binding at once and is never input;
                ;; the read goes on, and redisplays first what the binding
                ;; changed.
                (when (and (or (consp c) (and c (symbolp c)))
                           (lookup-key special-event-map (vector (if (consp c) (car c) c))))
                  (romacs--special-event c)
                  (setq c nil shown nil))
                (setq waiting (null c))))))
      (and timers (null end) (romacs--timer-stop-idle)))
    c))

(defun romacs--read-batch-wait (seconds)
  "kbd_buffer_get_event's wait with no terminal to read: run timers,
process output and file notification events until SECONDS pass, input is
requeued, or, without SECONDS, no watch is left."
  (let ((end (and seconds (+ (float-time) seconds))) (waiting t))
    (while waiting
      (when (fboundp 'romacs--file-notify-dispatch) (romacs--file-notify-dispatch))
      (let ((left (and end (- end (float-time)))))
        (if (or (romacs--unread-p) (and left (<= left 0))
                (and (null end) (not (romacs--file-notify-watching-p))))
            (setq waiting nil)
          (romacs--wait-reading-process-output (or left 'forever) nil 0 t))))))

(defun romacs--host-event (seconds display)
  "`romacs--read-host-event', whose `quit' once the frame has closed leaves
the innermost recursive edit first, as end of input does: the command loop
there reports every signal (GNU's C `error' handler), so it would read, and
quit, again.  Each level quits in turn, down to the host's command."
  (if (> (recursion-depth) 0)
      (condition-case nil (romacs--read-host-event seconds display)
        (quit (throw 'exit (lambda () (signal 'quit nil)))))
    (romacs--read-host-event seconds display)))

(defvar romacs--last-auto-save 0
  "keyboard.c last_auto_save: `num-nonmacro-input-events' at the command
loop's last auto-save.")
(defvar romacs--last-non-minibuffer-size 0
  "keyboard.c last_non_minibuf_size: the size of the last buffer read_char
saw outside the minibuffer, which slows idle auto-saves.")

(defun romacs--auto-save ()
  "read_char's auto-save: `do-auto-save', quietly with
`auto-save-no-message', then record_auto_save.  The subr is fileio.c's.
Value: whether it ran."
  (prog1 (when (fboundp 'do-auto-save)
           (do-auto-save auto-save-no-message nil)
           t)
    (setq romacs--last-auto-save num-nonmacro-input-events)))

(defun romacs--auto-save-delay ()
  "read_char's idle auto-save delay: `auto-save-timeout' seconds, slowed
logarithmically by the size of the last buffer outside the minibuffer
(four quarters up to about 50k), or nil when there is nothing new to save."
  (unless (window-minibuffer-p (selected-window))
    (setq romacs--last-non-minibuffer-size (buffer-size)))
  (when (and (not noninteractive) (null executing-kbd-macro)
             (> num-nonmacro-input-events romacs--last-auto-save)
             (integerp auto-save-timeout) (> auto-save-timeout 0))
    (let ((size (1+ (ash romacs--last-non-minibuffer-size -8))) (level 0))
      (while (> size 64)
        (setq level (1+ level) size (- size (ash size -2))))
      (/ (* (max level 4) auto-save-timeout) 4))))

(defun romacs--auto-save-due-p ()
  "Whether more than `auto-save-interval' input events, and at least 20,
came since the command loop's last auto-save."
  (and (> auto-save-interval 0)
       (> (- num-nonmacro-input-events romacs--last-auto-save) (max auto-save-interval 20))))

(defun romacs--idle-auto-save ()
  "read_char's auto-saves before a command's first key, for a host that
waits for that key itself (the GUI), idle since `current-idle-time' and
with no input pending, as timers and process output left it: by event
count at once, else once the idle delay has passed.  Value: t when it
saved, else the seconds until it is due, or nil when none is."
  (cond ((or (romacs--unread-p) (input-pending-p)) nil)
        ((romacs--auto-save-due-p) (romacs--auto-save))
        (t (let ((delay (romacs--auto-save-delay)) (idle (current-idle-time)))
             (when (and delay idle)
               (let ((left (- delay (float-time idle))))
                 (if (> left 0) left (romacs--auto-save))))))))

(defvar romacs-gc-idle-delay 2
  "Seconds of idleness after which the command loop collects garbage,
once `romacs-gc-idle-factor' says enough was allocated since the last
collection; nil never collects at idle.  Collecting while idle keeps the
pauses off commands; the automatic trigger, `gc-cons-threshold' and
`gc-cons-percentage' with the request code compiled, still governs
collection while commands run.")

(defvar romacs-gc-idle-factor 4
  "The idle collection waits for more than 1/FACTOR of what makes an
automatic collection due, as `garbage-collect-maybe' with FACTOR does.")

(defun romacs--idle-collect (&optional in-lisp)
  "When the command loop's host, waiting for keys itself (the GUI), should
collect: once idle for `romacs-gc-idle-delay' seconds with no input
pending, and more than 1/`romacs-gc-idle-factor' of the way to an
automatic collection (`romacs--collection-share').  Value: t to collect
now, the seconds until then, or nil: a quiet session never waits for it.
The host collects outside any Lisp, where request code retires; IN-LISP,
for a wait inside Lisp (a prompt's), counts only the objects allocated,
all a collection there reclaims."
  (let ((idle (current-idle-time)))
    (when (and (numberp romacs-gc-idle-delay) (natnump romacs-gc-idle-factor) idle
               (not (romacs--unread-p)) (not (input-pending-p))
               (> (* romacs-gc-idle-factor (romacs--collection-share in-lisp)) 1))
      (let ((left (- romacs-gc-idle-delay (float-time idle))))
        (if (> left 0) left t)))))

(defun romacs--unread-p ()
  "Whether read_char has a requeued event: in any of its unread queues."
  (or unread-post-input-method-events unread-command-events unread-input-method-events))

(defun romacs--unwrap-menu-event (c)
  "read_char's undoing of read_char_x_menu_prompt: (EVENT) is EVENT."
  (if (and (consp c) (or (symbolp (car c)) (integerp (car c))) (null (cdr c))) (car c) c))

(defun romacs--keyboard-translate (c)
  "keyboard.c read_char: host character C through `keyboard-translate-table'.
A nil entry, or a C beyond a string or vector table, is no translation."
  (let ((table keyboard-translate-table))
    (or (and (natnump c)
             (if (char-table-p table) (characterp c)
               (and (or (stringp table) (vectorp table)) (< c (length table))))
             (aref table c))
        c)))

(defvar romacs--keyboard-quit nil
  "Whether `romacs--read-char''s last event was the quit character typed on
the host keyboard, before `keyboard-translate-table': keyboard.c
handle_interrupt's `C-g', which sets `quit-flag' (quit.rs).")

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
   (setq romacs--keyboard-quit nil)
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
      ;; read_char's auto-saves, for a command's keys: after
      ;; `auto-save-interval' events, and while idle.
      (when (and romacs--reading-command-keys (romacs--auto-save-due-p) (not (romacs--unread-p)) (not (input-pending-p)))
        (romacs--auto-save))
      (let ((sit (and romacs--reading-command-keys (null seconds) (romacs--auto-save-delay))))
        (setq c (romacs--read-host-char seconds sit)
              romacs--keyboard-quit (eq c ?\C-g))
        ;; An event a timer or filter queued in `unread-command-events'
        ;; while the host waited: when the wait was read_char's auto-save
        ;; `sit_for', it is read next as a requeued event (keyboard.c:2999);
        ;; otherwise it is the wait's own input, read as the keyboard's is
        ;; (kbd_buffer_get_event, keyboard.c:4169).  Any other requeued
        ;; event is read next, as GNU's wait returns on
        ;; requeued_command_events_pending_p.
        (when (and (null c) (not sit) (consp unread-command-events))
          (setq c (car unread-command-events)
                unread-command-events (cdr unread-command-events))
          (cond ((and (consp c) (eq (car c) t)) (setq c (cdr c)))
                ((and (consp c) (eq (car c) 'no-record)) (setq c (cdr c) recorded t)))))
      (setq c (and c (romacs--keyboard-translate c)))
      (if (null c)
          (when (romacs--unread-p) (setq again t))
        ;; read_char wipes the echo area for each event from the keyboard,
        ;; other than help, frame and window selection events
        ;; (keyboard.c:3231): a prompt `read-event' showed goes once its
        ;; event comes.
        (when (and (current-message)
                   (not (and (consp c) (memq (car c) '(help-echo switch-frame select-window)))))
          (romacs--safe-run-hooks 'echo-area-clear-hook)
          (message nil))))
     ;; Without a host keyboard a user signal is still an event
     ;; (kbd_buffer_get_event); a special one runs its binding and the read
     ;; goes on, as read_char's do.
     ((setq c (romacs--take-user-signal))
      (when (romacs--special-event c) (setq c nil again t)))
     ((and (fboundp 'romacs--wait-reading-process-output)
           (or seconds (and (fboundp 'romacs--file-notify-watching-p) (romacs--file-notify-watching-p))))
      ;; Batch GNU built with file notification (GNU/Linux) waits here, in
      ;; kbd_buffer_get_event: timers, process output and file
      ;; notification events run until SECONDS pass or, without SECONDS,
      ;; for as long as a watch is live.  Input a timer requeued is read
      ;; next; with no watch left, end of input comes as below.
      (romacs--read-batch-wait seconds)
      (when (or (romacs--unread-p) (null seconds)) (setq again t)))
     (seconds)
     ;; Batch GNU blocks for the terminal here. With no input left, VM2
     ;; leaves the innermost recursive edit or minibuffer instead, so a
     ;; command loop that reports errors cannot spin on this one.
     ((> (recursion-depth) 0)
      (throw 'exit (lambda () (signal 'end-of-file '("Error reading from stdin")))))
     (t (signal 'end-of-file '("Error reading from stdin")))))
    (when c
      ;; record_char: recent-keys and the macro being defined leave out
      ;; keyboard-macro input.  It counts the event too, whichever input it
      ;; came from (keyboard.c:3583), for `auto-save-interval' and the idle
      ;; auto-save.
      (unless (or recorded executing-kbd-macro
                  (and (boundp 'inhibit--record-char) inhibit--record-char))
        (romacs--record-input c)
        (setq num-nonmacro-input-events (1+ num-nonmacro-input-events))
        (when defining-kbd-macro (romacs--store-kbd-macro-char c)))
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
  ;; message_with_string's echo, which is not logged (keyboard.c:10776).
  (when prompt (let ((message-log-max nil)) (message "%s" prompt)))
  (let ((seconds (and (numberp seconds) seconds)) val)
    (while (progn
             (setq val (romacs--read-char seconds))
             ;; read_char's quit: a C-g from the keyboard set `quit-flag',
             ;; or cleared it under `inhibit-quit' and is read as a key;
             ;; maybe_quit acts on it before the event is returned.
             (and romacs--keyboard-quit (not inhibit-quit) (signal 'quit nil))
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

(defvar romacs--sequence-start-functions nil
  "Functions run with no arguments once per key sequence, after its first
event is read and before it is looked up: a choice of keymaps made here
holds for the whole sequence, whatever arrives between its keys.")

(defvar romacs--key-read-functions nil
  "Functions called with each event `romacs--read-key-sequence' reads, as
it reads it.  They only observe: what the read reads, records and
returns is as without them (key hints hide on the next key).")

(defvar romacs--prefix-help-functions nil
  "Functions offered a prefix's unbound help key in a command read, each
called with the prefix (the sequence before it, translated) and the
event; one returning non-nil has taken it: the read goes on with the
prefix pending, as if the key had not come.  Offered only where taking
it changes nothing else (`romacs--read-key-sequence').")

(defun romacs--help-event-p (event)
  "keyboard.c help_char_p of EVENT_HEAD (EVENT): `help-char' or a member
of `help-event-list'."
  (let ((head (if (consp event) (car event) event)))
    (or (eql head help-char) (memq head help-event-list))))

(defvar romacs--sequence-end-functions nil
  "Functions run with no arguments when a read whose sequence started
ends, normally or not: a choice made at its start is undone here, so a
read nested in another's restores the outer one's.")

(defun romacs--read-key-sequence (dont-downcase-last &optional command)
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
`this-command-keys-shift-translated' says when that was used.
An unbound help key after a prefix ends the sequence with
`prefix-help-command' (keyboard.c's fallback), unless COMMAND, the
command loop's own read, offers it to `romacs--prefix-help-functions'
and one takes it."
  (let ((started nil) (checkpoint nil) (clean nil) (help-command nil))
   (unwind-protect
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
      ;; A replay rereads translated or shifted events: none of them is a
      ;; raw event this read took, so no checkpoint stands for them.
      (setq state 'read n 0 first-unbound most-positive-fixnum binding '(keymap) checkpoint nil
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
                     (setq key (aref keys n) checkpoint nil)
                     (romacs--add-command-key key))
                    ((and executing-kbd-macro (romacs--at-end-of-macro-p)
                          (not unread-command-events))
                     (setq state 'end))
                    ;; A C-g read here is a key: read_key_sequence clears
                    ;; the `quit-flag' it set (keyboard.c:11352).
                    (t ;; A command read keeps what an event after the first
                       ;; read would undo, should a prefix help key be taken.
                       (when (and command (> n 0) romacs--prefix-help-functions)
                         (setq checkpoint (list n mock binding first-unbound (copy-sequence indec)
                                                (copy-sequence fkey) (copy-sequence keytran) upper
                                                shift-translated fake-prefixed last-nonmenu-event
                                                romacs--command-keys romacs--single-command-key-start raw
                                                (and defining-kbd-macro romacs--kbd-macro-events))
                               clean (not (or unread-command-events executing-kbd-macro
                                              (and (boundp 'inhibit--record-char) inhibit--record-char)))))
                       (setq key (romacs--read-char)
                             quit-flag nil
                             raw (vconcat raw (list key))
                             keys (vconcat keys (list key)))
                       (when romacs--key-read-functions
                         (run-hook-with-args 'romacs--key-read-functions key))
                       ;; Once per sequence, after its first event is read
                       ;; and before it is looked up; replays of the
                       ;; sequence do not run it again.
                       (unless started
                         (setq started t)
                         (run-hooks 'romacs--sequence-start-functions))))
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
                 ;; keyboard.c's integer upcase/shift fallback comes first.
                 ((and (null binding) (>= (aref keytran 2) n) lower (integerp key))
                  (aset keys (1- n) lower)
                  (setq upper (cons (1- n) key) mock (max n mock)
                        shift-translated t state 'replay))
                 ;; `not_upcase:' an unbound help key after a prefix.
                 ((and (null binding) (> n 1) (romacs--help-event-p key))
                  ;; Taken only in the clean case: the raw event this read
                  ;; took, no translation part-way or producing it.
                  (if (and checkpoint clean (= (car checkpoint) (1- n)) (<= mock n)
                           (>= (aref indec 2) n) (>= (aref fkey 2) n) (>= (aref keytran 2) n)
                           (run-hook-with-args-until-success 'romacs--prefix-help-functions
                                                             (romacs--subvector keys 0 (1- n)) key))
                      ;; Taken: the read goes on as before the key came; the
                      ;; macro being defined forgets the entry this read stored.
                      (pcase-let ((`(,n0 ,mock0 ,binding0 ,unbound0 ,indec0 ,fkey0 ,keytran0 ,upper0 ,shift0
                                    ,fake0 ,nonmenu0 ,ckeys0 ,cstart0 ,raw0 ,macro0)
                                   checkpoint))
                        (when (and defining-kbd-macro (eq (cdr romacs--kbd-macro-events) macro0)
                                   (eq (car romacs--kbd-macro-events) key))
                          (setq romacs--kbd-macro-events macro0))
                        (setq n n0 mock mock0 binding binding0 first-unbound unbound0
                              keys (romacs--subvector keys 0 n0) upper upper0 shift-translated shift0
                              fake-prefixed fake0 last-nonmenu-event nonmenu0
                              romacs--command-keys ckeys0 romacs--single-command-key-start cstart0 raw raw0)
                        (dotimes (i 4) (aset indec i (aref indec0 i)) (aset fkey i (aref fkey0 i))
                                 (aset keytran i (aref keytran0 i))))
                    ;; GNU: the sequence ends, its command `prefix-help-command'
                    ;; (read_key_sequence_cmd); its binding stays nil, so the
                    ;; final shift restoration runs as GNU's does.
                    (setq help-command (list prefix-help-command) state 'help)))
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
      (when (eq state 'help) (setq state 'read))
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
          romacs--key-sequence-command (and keys (if help-command (car help-command) binding))
          romacs--key-sequence-remapped
          (let ((command (if help-command (car help-command) binding)))
            (and keys (symbolp command) command (command-remapping command))))
    keys)
     ;; A read whose sequence started ends it, normally or not.
     (when started (run-hooks 'romacs--sequence-end-functions)))))

;; keyboard.c read_key_sequence_vs; Fread_key_sequence's string form is Rust.
(defun romacs--read-key-sequence-vs (prompt continue-echo dont-downcase-last _cmd-loop)
  (and prompt (not (stringp prompt))
       (signal 'wrong-type-argument (list 'stringp prompt)))
  ;; read_key_sequence (keyboard.c:10993): an interactive read installs
  ;; PROMPT as the start of the echo and shows it now, unlogged (echo_now).
  (when (and prompt (not executing-kbd-macro) (not noninteractive))
    (let ((message-log-max nil)) (message "%s" prompt)))
  (unless continue-echo
    (setq romacs--command-keys [] romacs--single-command-key-start 0))
  (or (romacs--read-key-sequence dont-downcase-last) []))

;;;; command_loop_1

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

(defun romacs--command-loop-top ()
  "The top of keyboard.c command_loop_1's loop, before it waits for keys:
the selected window's buffer is current and no command is running, so
idle timers and redisplay see `this-command' nil."
  (set-buffer (window-buffer (selected-window)))
  (setq deactivate-mark nil this-command nil real-this-command nil
        this-original-command nil this-command-keys-shift-translated nil))

(defun romacs--command-loop-step ()
  "One pass of keyboard.c command_loop_1's loop: read a key sequence and
run its command between `pre-command-hook' and `post-command-hook'.
Return the keys read, nil at the end of an executing keyboard macro."
  (romacs--command-loop-top)
  (let ((keys (let ((romacs--reading-command-keys t)) (romacs--read-key-sequence nil t)))
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
      (romacs--after-command-mark previous-buffer previous-tick)
      ;; The keys of a command that ran, unless it set a prefix argument
      ;; for the next one, belong to the macro being defined.
      (when (and defining-kbd-macro (null prefix-arg))
        (romacs--finalize-kbd-macro-chars)))
    keys))

(defun romacs--command-loop-1 ()
  "keyboard.c command_loop_1: read and execute commands.
Return nil at the end of an executing keyboard macro."
  (romacs--command-loop-resume)
  (while (romacs--command-loop-step))
  nil)

(defun romacs--special-event (event)
  "keyboard.c read_char for a special EVENT such as (focus-in FRAME): run
its `special-event-map' binding at once, outside any key sequence, with
EVENT as its keys.  Value: whether EVENT ran a command.  A VM without
GNU's preloaded Lisp has the bindings but not frame.el's handlers."
  (let ((command (lookup-key special-event-map (vector (if (consp event) (car event) event)))))
    (when (commandp command)
      (setq last-input-event event)
      (command-execute command nil (vector event) t)
      t)))

(defun romacs--host-command ()
  "keyboard.c command_loop_2 one command at a time, for a host whose own
loop calls this whenever input waits (the GUI): read one key sequence,
from the unread queues, a keyboard macro, then the host, and run its
command as command_loop_1 does.  A signal goes to cmd_error, a throw to
`top-level' ends the command, and the loop then resumes, so
`post-command-hook' runs for that command too.  Value: (COMMAND . ERROR),
COMMAND `this-command' and ERROR nil or the (CONDITION . DATA) signalled.
The host waits for the next input at the top of the loop."
  (let ((error nil) (thrown t))
    (catch 'top-level
      (condition-case data
          (romacs--note-signals #'romacs--command-loop-step)
        ((debug t) (setq error data) (romacs--cmd-error data)))
      (setq thrown nil))
    (when (or error thrown)
      (when thrown (catch 'top-level (romacs--top-level-1)))
      (romacs--command-loop-resume))
    (prog1 (cons this-command error)
      (romacs--command-loop-top))))

(defun romacs--top-level-1 ()
  "keyboard.c top_level_1, which the command loop runs on reaching its outer
level: evaluate `top-level', reporting an error as cmd_error does.  After a
throw to `top-level', startup.el's `normal-top-level' says so."
  (when top-level
    (condition-case data (eval top-level t)
      ((debug t) (romacs--cmd-error data)))))

(defvar romacs--signaling-function nil
  "eval.c Vsignaling_function: the function whose call signalled the error
the command loop handles, for `command-error-function'.")

(defun romacs--note-signals (function)
  "Call FUNCTION, noting for cmd_error which function signals out of it."
  (handler-bind-1 function '(t) #'romacs--note-signaling-function))

(defun romacs--note-signaling-function (_error)
  "eval.c signal_or_quit's Vsignaling_function, from the frames below this
handler: past the top one, and past `error' after it.  A `signal' frame
whose caller is `error' or `user-error' is dropped first: pinned GNU runs
subr.el native-compiled, whose call to `signal' records no frame."
  (let ((frames nil) (n 0))
    (mapbacktrace (lambda (_evald function _args _flags)
                    (when (< n 5) (setq frames (cons function frames) n (1+ n))))
                  #'romacs--note-signaling-function)
    (setq frames (cdr (nreverse frames)))
    (if (and (eq (car frames) 'signal) (memq (car (cdr frames)) '(error user-error)))
        (setq frames (cdr frames)))
    (setq frames (cdr frames))
    (if (eq (car frames) 'error) (setq frames (cdr frames)))
    (setq romacs--signaling-function (car frames))))

(defun romacs--cmd-error (data)
  "keyboard.c cmd_error and cmd_error_internal: report DATA through
`command-error-function' and resume the command loop.  Any error but
`minibuffer-quit' ends the macro."
  (let ((context (cond ((null romacs--executing-macro) "")
                       ((= romacs--macro-iterations 1) "After 1 kbd macro iteration: ")
                       (t (format "After %d kbd macro iterations: "
                                  romacs--macro-iterations))))
        (signal (if (memq 'quit (get (car data) 'error-conditions)) nil romacs--signaling-function)))
    (setq romacs--signaling-function nil)
    (if (not (memq 'minibuffer-quit (get (car data) 'error-conditions)))
        (setq executing-kbd-macro nil romacs--executing-macro nil)
      ;; An M-x that quit its minibuffer is part of the macro being defined.
      (when defining-kbd-macro (romacs--finalize-kbd-macro-chars)))
    (setq prefix-arg nil last-prefix-arg nil quit-flag nil)
    (let ((standard-output t) (standard-input t)
          (print-level 10) (print-length 10) (inhibit-quit t))
      (when command-error-function
        (funcall command-error-function data context signal))))
  (setq quit-flag nil inhibit-quit nil)
  0)

;; keyboard.c Fcommand_error_default_function, entered from Rust
;; (prims/keyboard.rs) so that batch reports reach it before the rest of
;; the command loop is in use.
(defun romacs--command-error-default-function (data context signal)
  "Produce default output for unhandled error message.
In batch, print it to stderr and exit; a `minibuffer-quit' rings the bell
and is printed to standard output instead.  Interactively, ring the bell
and show it in the echo area; *Messages* logs it with SIGNAL, the function
that signalled, as print_error_message logs its caller."
  (or (stringp context) (signal 'wrong-type-argument (list 'stringp context)))
  (let ((text (concat context (error-message-string data)))
        (minibuffer-quit (memq 'minibuffer-quit (get (car-safe data) 'error-conditions))))
    (cond ((and noninteractive (not minibuffer-quit))
           (message "%s" text) (kill-emacs -1))
          (noninteractive (ding t) (princ text t))
          ((and signal (symbolp signal))
           (if minibuffer-quit (ding t) (discard-input) (ding))
           (let ((inhibit-message t))
             (message "%s" (concat context (symbol-name signal) ": " (error-message-string data))))
           (let ((message-log-max nil)) (message "%s" text)))
          (t (if minibuffer-quit (ding t) (discard-input) (ding))
             (message "%s" text))))
  nil)

(defun romacs--command-loop-2 (errors)
  "keyboard.c command_loop_2: run the command loop until it returns.
With ERRORS, any signal is reported and the loop resumes (a recursive
edit, whose handler is GNU's C `error', which also lets the debugger in);
else only `minibuffer-quit' is (a keyboard macro), so any other signal
leaves through the caller."
  (while (if errors
             (condition-case data (romacs--note-signals #'romacs--command-loop-1)
               ((debug t) (romacs--cmd-error data)))
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

(defvar romacs--kbd-macro-events nil
  "macros.c kbd_macro_buffer up to kbd_macro_ptr: the events stored while
a macro is defined, newest first.")
(defvar romacs--kbd-macro-end 0
  "macros.c kbd_macro_end: how many of `romacs--kbd-macro-events', oldest
first, belong to the macro; later ones are the current command's.")

(defun romacs--store-kbd-macro-char (event)
  "macros.c store_kbd_macro_char: store EVENT in the macro being defined."
  (when defining-kbd-macro
    (setq romacs--kbd-macro-events (cons event romacs--kbd-macro-events)))
  nil)

(defun romacs--finalize-kbd-macro-chars ()
  "macros.c finalize_kbd_macro_chars: the events stored so far belong to
the macro."
  (setq romacs--kbd-macro-end (length romacs--kbd-macro-events)))

(defun romacs--cancel-kbd-macro-events ()
  "macros.c Fcancel_kbd_macro_events: drop this command's stored events."
  (setq romacs--kbd-macro-events
        (nthcdr (- (length romacs--kbd-macro-events) romacs--kbd-macro-end)
                romacs--kbd-macro-events))
  nil)

(defun romacs--end-kbd-macro ()
  "macros.c end_kbd_macro: stop defining; the events that belong to the
macro become `last-kbd-macro'."
  (romacs--cancel-kbd-macro-events)
  (setq defining-kbd-macro nil
        last-kbd-macro (vconcat (reverse romacs--kbd-macro-events))))

(defun romacs--start-kbd-macro (append no-exec)
  "macros.c Fstart_kbd_macro."
  (if defining-kbd-macro (error "Already defining kbd macro"))
  (if (null append)
      (progn (setq romacs--kbd-macro-events nil romacs--kbd-macro-end 0)
             (message "Defining kbd macro..."))
    (let ((macro last-kbd-macro) (events nil) (i 0))
      (or (stringp macro) (vectorp macro) (signal 'wrong-type-argument (list 'arrayp macro)))
      ;; A string's 0x80 bit is the meta modifier.
      (while (< i (length macro))
        (let ((c (aref macro i)))
          (if (and (stringp macro) (natnump c) (/= 0 (logand c #x80)))
              (setq c (logior #x8000000 (logand c (lognot #x80)))))
          (setq events (cons c events) i (1+ i))))
      (setq romacs--kbd-macro-events events romacs--kbd-macro-end (length events))
      (unless no-exec (execute-kbd-macro macro 1))
      (message "Appending to kbd macro...")))
  (setq defining-kbd-macro t)
  nil)

(defun romacs--end-kbd-macro-command (repeat loopfunc)
  "macros.c Fend_kbd_macro."
  (unless defining-kbd-macro (error "Not defining kbd macro"))
  (if (null repeat) (setq repeat 1)
    (or (integerp repeat) (signal 'wrong-type-argument (list 'fixnump repeat))))
  (romacs--end-kbd-macro)
  (message "Keyboard macro defined")
  (cond ((= repeat 0) (execute-kbd-macro last-kbd-macro repeat loopfunc))
        ((> repeat 1) (execute-kbd-macro last-kbd-macro (1- repeat) loopfunc)))
  nil)

(defun romacs--call-last-kbd-macro (prefix loopfunc)
  "macros.c Fcall_last_kbd_macro: the macro runs as the commands before it
see it, and its last command stays `this-command'."
  (setq this-command last-command real-this-command last-kbd-macro)
  (cond (defining-kbd-macro (error "Can't execute anonymous macro while defining one"))
        ((null last-kbd-macro) (error "No kbd macro has been defined"))
        (t (execute-kbd-macro last-kbd-macro prefix loopfunc)))
  (setq this-command last-command)
  nil)

(defun romacs--discard-input ()
  "keyboard.c Fdiscard_input: end the macro being defined, without the
current command's keys, and drop requeued input."
  (when defining-kbd-macro
    (romacs--end-kbd-macro))
  (setq unread-command-events nil)
  nil)

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
