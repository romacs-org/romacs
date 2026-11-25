(progn
;;; keyboard.el --- VM2: the Lisp half of the command loop  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Evaluated once per VM, on first use, by prims/keyboard.rs. GNU's
;; command loop (keyboard.c command_loop_1) and keyboard macros (macros.c
;; Fexecute_kbd_macro) call Lisp at every step, so they are Lisp here; the
;; GNU primitives are Rust entry points into these functions. The key
;; reader (read_key_sequence, read_char) is Rust
;; (crates/romacs-vm/src/command_loop/), as are this command's keys (the
;; input stream's); it calls the waits, echo and reader hooks below.
;;
;; Events come from `unread-command-events', then the executing keyboard
;; macro, then the host keyboard (host_input.rs) when one is attached.
;; Without one, a read that would wait signals end of file; see
;; ledger/divergences VM2-KBD1 for this and the other limits of this port.
;; This whole file is compiled as one form before its defvars run, so its
;; own variables are only ever set, never let-bound.

;;; Code:

(defvar romacs--command-loop-level 0 "keyboard.c command_loop_level.")
(defvar romacs--executing-macro nil
  "macros.c executing_kbd_macro: the macro under way, for error messages.")
(defvar romacs--macro-iterations 0 "macros.c executing_kbd_macro_iterations.")
(defvar romacs--key-sequence-command nil "keyboard.c read_key_sequence_cmd.")
(defvar romacs--key-sequence-remapped nil "keyboard.c read_key_sequence_remapped.")

;; BOOTSTRAP STAND-INS (ledger/divergences VM2-KBD1). The command loop calls
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

(defvar romacs--reading-command-keys nil
  "Non-nil while the command loop reads a command's key sequence, between
commands: completed background commands may run their callbacks then.")

(defun romacs--dispatch-background ()
  "Run completed background commands' callbacks while the command loop
waits for a command's keys outside any minibuffer read (lisp/vm/process.el
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
        (outer-inhibit inhibit-quit)
        (c nil) (waiting t) (shown nil) (collect-ok t))
    (and timers (null end) (romacs--timer-start-idle))
    (unwind-protect
        ;; The wait's own Lisp does not quit: as GNU's, a C-g typed meanwhile
        ;; is this read's key (keyboard.c quit_throw_to_read_char), not a
        ;; `quit' through the read's caller, an input method among them.
        (let ((inhibit-quit t))
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
                            ;; `debug-on-event' and a third C-g clear
                            ;; `inhibit-quit': those do quit.
                            (and inhibit-quit (eq quit-flag t) (progn (setq quit-flag nil) ?\C-g))
                            (let ((inhibit-quit outer-inhibit))
                              (romacs--host-event
                               (if (and delay left) (min delay left) (or delay left))
                               current)))
                      shown ran)
                ;; read_char: an event `special-event-map' binds (focus, a
                ;; user signal, a native model's) runs its binding at once
                ;; and is never input; the read goes on, and redisplays
                ;; first what the binding changed.  The binding quits as
                ;; the read's caller does: GNU runs it outside the wait.
                (when (and (or (consp c) (and c (symbolp c)))
                           (lookup-key special-event-map (vector (if (consp c) (car c) c))))
                  (let ((inhibit-quit outer-inhibit))
                    (romacs--special-event c))
                  (setq c nil shown nil))
                (setq waiting (null c)))))))
      (and timers (null end) (romacs--timer-stop-idle)))
    c))

(defvar romacs-batch-read-horizon 1.0
  "Seconds a batch read with nothing queued waits for a timer to queue input.
With no live process or file watch, the read ends in end of input once
no timer or idle timer is due within this many seconds, or once it has
waited this long since the last one went quiet: batch GNU would block
for the terminal forever.")

(defun romacs--batch-live-p ()
  "Non-nil while a file watch or a live process could still queue input."
  (or (and (fboundp 'romacs--file-notify-watching-p) (romacs--file-notify-watching-p))
      (and (fboundp 'process-list)
           (catch 'found
             (dolist (process (process-list))
               ;; `process-live-p' is subr.el's: a bare VM has not loaded it.
               (when (memq (process-status process) '(run stop open listen connect))
                 (throw 'found t)))))))

(defun romacs--batch-input-soon-p ()
  "Non-nil while something could queue input for a batch read soon.
That is a live file watch or process, or a timer or idle timer due within
`romacs-batch-read-horizon'.  Idle timers count once Emacs is idle."
  (or (romacs--batch-live-p)
      (let ((delay (romacs--timer-delay)))
        (and delay (<= delay romacs-batch-read-horizon)))))

(defun romacs--read-batch-wait (seconds)
  "kbd_buffer_get_event's wait with no terminal to read: run timers, idle
timers, process output and file notification events until input is
requeued, or SECONDS pass, or, without SECONDS, until no input may come
soon: none queued within `romacs-batch-read-horizon' of the last live
process or watch, and no timer due within it.  Without SECONDS it idles
for the wait, as GNU's read_char does before it blocks.  It waits in
short slices, so it sees a process end or a sentinel queue a key."
  (let* ((start (float-time))
         (end (and seconds (+ start seconds)))
         (deadline (+ start romacs-batch-read-horizon))
         (waiting t))
    (unwind-protect
        (progn
          (unless seconds (romacs--timer-start-idle))
          (while waiting
            (when (fboundp 'romacs--file-notify-dispatch) (romacs--file-notify-dispatch))
            (let ((now (float-time)))
              (when (romacs--batch-live-p) (setq deadline (+ now romacs-batch-read-horizon)))
              (if (or (romacs--unread-p)
                      (if end (>= now end)
                        (or (> now deadline) (not (romacs--batch-input-soon-p)))))
                  (setq waiting nil)
                (romacs--wait-reading-process-output
                 (if end (min 0.1 (- end now)) 0.1) nil 0 t))))
          ;; A slice may oversleep past the deadline: what fell due runs once
          ;; more before the read gives up.
          (unless (or seconds (romacs--unread-p)) (romacs--timer-check)))
      (unless seconds (romacs--timer-stop-idle)))))

(defun romacs--batch-end-of-input ()
  "Signal end of input as a batch read does with nothing left to read.
Batch GNU blocks for the terminal here.  With no input left, VM2 leaves
the innermost recursive edit or minibuffer instead, so a command loop
that reports errors cannot spin on this one."
  (if (> (recursion-depth) 0)
      (throw 'exit (lambda () (signal 'end-of-file '("Error reading from stdin"))))
    (signal 'end-of-file '("Error reading from stdin"))))

(defun romacs--host-event (seconds display)
  "`romacs--read-host-event', whose `quit' is one of two.  A C-g that quit the
wait's redisplay is the read's key, as one typed in the wait itself.  Once the
frame has closed (the quit's data says so) it leaves the innermost recursive
edit first, as end of input does: the command loop there reports every signal
(GNU's C `error' handler), so it would read, and quit, again.  Each level
quits in turn, down to the host's command."
  (condition-case err (romacs--read-host-event seconds display)
    ((debug quit)
     (cond ((null (cdr err)) ?\C-g)
           ((> (recursion-depth) 0) (throw 'exit (lambda () (signal 'quit nil))))
           (t (signal 'quit nil))))))

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
  "Whether read_char has a requeued event: in any of its unread queues, or
a `switch-frame' a focus change queued (`unread_switch_frame')."
  (or unread-post-input-method-events unread-command-events unread-input-method-events
      (romacs--unread-switch-frame-p)))

(defvar input-method-exit-on-first-char nil
  "keyboard.c `Fset's this: non-nil when the input method is to return its
first character at once, as a `read-key-sequence' outside the command loop
binds it.")

(defvar input-method-use-echo-area nil
  "keyboard.c `Fset's this: non-nil when the input method is to show its
guidance through the prompt of its own reads, as every read that is not a
command's binds it.")

(defun romacs--read-host-input (seconds)
  "read_char's host source, for the Rust reader: read_char's auto-saves
for a command's keys (after `auto-save-interval' events, and while idle),
then the host wait (`romacs--read-host-char') for at most SECONDS.  An
event a timer or filter queued in `unread-command-events' meanwhile is
the wait's own input, read as the keyboard's (kbd_buffer_get_event,
keyboard.c:4169), unless the wait was read_char's auto-save `sit_for'
\=(keyboard.c:2999), which leaves it requeued.  Value: (EVENT RECORDED
QUIT), RECORDED when it was queued as (no-record . EVENT) and QUIT when it
is the quit character from the keyboard; nil when the wait ended with
none."
  (when (and romacs--reading-command-keys (romacs--auto-save-due-p) (not (romacs--unread-p)) (not (input-pending-p)))
    (romacs--auto-save))
  (let* ((sit (and romacs--reading-command-keys (null seconds) (romacs--auto-save-delay)))
         (c (romacs--read-host-char seconds sit))
         (quit (eq c ?\C-g))
         (recorded nil))
    (when (and (null c) (not sit) (consp unread-command-events))
      (setq c (car unread-command-events)
            unread-command-events (cdr unread-command-events))
      (cond ((and (consp c) (eq (car c) t)) (setq c (cdr c)))
            ((and (consp c) (eq (car c) 'no-record)) (setq c (cdr c) recorded t))))
    (and c (list c recorded quit))))

(defun romacs--read-batch-input (seconds)
  "read_char's sources without a host keyboard, for the Rust reader: a user
signal is an event (kbd_buffer_get_event), a special one running its
binding; else batch GNU's wait in kbd_buffer_get_event, which runs
timers, process output and file notification events until SECONDS pass
or, without SECONDS, for as long as one of them could queue input
\=(`romacs--read-batch-wait'), then ends input.  Value: the event, t to
read the queues again, or nil when SECONDS passed."
  (let ((c (romacs--take-user-signal)))
    (cond (c (if (romacs--special-event c) t c))
          ((fboundp 'romacs--wait-reading-process-output)
           (romacs--read-batch-wait seconds)
           (cond ((romacs--unread-p) t)
                 ((null seconds) (romacs--batch-end-of-input))))
          (seconds nil)
          (t (romacs--batch-end-of-input)))))

(defun romacs--clear-echo ()
  "read_char's wipe of the echo area, `echo-area-clear-hook' first."
  (romacs--safe-run-hooks 'echo-area-clear-hook)
  (message nil))

;;;; read_key_sequence

(defvar romacs--sequence-start-functions nil
  "Functions run with no arguments once per key sequence, after its first
event is read and before it is looked up: a choice of keymaps made here
holds for the whole sequence, whatever arrives between its keys.")

(defvar romacs--key-read-functions nil
  "Functions called with each event a key sequence's read reads, as
it reads it.  They only observe: what the read reads, records and
returns is as without them (key hints hide on the next key).")

(defvar romacs--prefix-help-functions nil
  "Functions offered a prefix's unbound help key in a command read, each
called with the prefix (the sequence before it, translated) and the
event; one returning non-nil has taken it: the read goes on with the
prefix pending, as if the key had not come.  Offered only where taking
it changes nothing else (`romacs--read-key-sequence').")

(defvar romacs--sequence-end-functions nil
  "Functions run with no arguments when a read whose sequence started
ends, normally or not: a choice made at its start is undone here, so a
read nested in another's restores the outer one's.")

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
  (setq prefix-arg nil last-prefix-arg nil deactivate-mark nil)
  (romacs--set-command-keys [] 0)
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
  ;; command_loop_1 reads a key sequence that may be a `switch-frame'.
  (let ((keys (let ((romacs--reading-command-keys t)) (romacs--read-key-sequence nil t t)))
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
      (romacs--set-command-keys [] 0)
      (romacs--after-command-mark previous-buffer previous-tick)
      ;; A step of a session's flight recording (crate::flight).
      (romacs--flight-step this-command)
      ;; The keys of a command that ran, unless it set a prefix argument
      ;; for the next one, belong to the macro being defined.
      (when (and defining-kbd-macro (null prefix-arg))
        (romacs--finalize-kbd-macro-chars)))
    keys))

(defun romacs--typing-step ()
  "`romacs--command-loop-step' as one command step of the native keys it
reads: `romacs--typing-begin' before it, and `romacs--typing-end' after,
told whether it returned or was thrown out of (crates/romacs-vm/src/typing.rs).
Return its value."
  (let ((returned nil))
    (romacs--typing-begin)
    (unwind-protect (prog1 (romacs--command-loop-step) (setq returned t))
      (romacs--typing-end returned))))

(defun romacs--command-loop-1 ()
  "keyboard.c command_loop_1: read and execute commands.
Return nil at the end of an executing keyboard macro."
  (romacs--command-loop-resume)
  (while (romacs--typing-step))
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
          (romacs--note-signals #'romacs--typing-step)
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

(defun romacs--c-half-p (function)
  "Whether FUNCTION is a private `romacs--' function: the Lisp half of
something GNU writes in C, whose frames GNU never records."
  (and (symbolp function)
       (eq t (compare-strings "romacs--" nil nil (symbol-name function) 0 8))))

(defun romacs--note-signaling-function (_error)
  "eval.c signal_or_quit's Vsignaling_function, from the frames below this
handler: past the top one, and past `error' after it.  A `signal' frame
whose caller is `error' or `user-error' is dropped first: pinned GNU runs
subr.el native-compiled, whose call to `signal' records no frame.  A frame
called by a C half, and the run of C halves under it, are dropped too:
they are C in GNU, so its top frame is the primitive that called them."
  (let ((frames nil) (n 0))
    (mapbacktrace (lambda (_evald function _args _flags)
                    (when (< n 16) (setq frames (cons function frames) n (1+ n))))
                  #'romacs--note-signaling-function)
    (setq frames (cdr (nreverse frames)))
    (if (and (eq (car frames) 'signal) (memq (car (cdr frames)) '(error user-error)))
        (setq frames (cdr frames)))
    (when (romacs--c-half-p (car (cdr frames)))
      (setq frames (cdr frames))
      (while (romacs--c-half-p (car frames)) (setq frames (cdr frames))))
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
