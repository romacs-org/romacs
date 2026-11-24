;;; romacs-term.el --- Native Alacritty terminal for Romacs -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Native Alacritty terminal for Romacs.  Every key goes through the
;; command loop.  Each window showing a terminal is in one of two modes:
;;
;; - Input mode: the window follows the cursor and keys reach the program
;;   through generated passthrough keymaps, except the keys in
;;   `romacs-term-keymap-exceptions' (vterm's `vterm-keymap-exceptions'),
;;   or on the alternate screen (a full-screen program)
;;   `romacs-term-alternate-exceptions', which keep their Emacs bindings.
;;   C-x and C-c stay Emacs prefixes on every screen; C-q gives the
;;   program the next key as it is (C-q C-x for nano's C-x), and C-c C-c
;;   sends C-c.
;; - Free mode: the window is a plain read-only Emacs buffer that output
;;   does not move, to search, select and copy from.  C-c C-j enters it on
;;   any screen; moving point off the cursor or scrolling back enters it
;;   too.  C-c C-j, typing, or a switch of screens returns to input mode.
;;
;; The keymap is chosen once per key sequence, when its first event is
;; read (`romacs--sequence-start-functions'), and is the buffer's local map
;; for that sequence alone, as vterm's mode map is local: minor modes and
;; the overriding maps (isearch, transient maps) come before it.  Output
;; arriving between a prefix and its next key never splits the sequence
;; between two routes, and a sequence that leaves the terminal buffer is
;; looked up with Emacs maps alone.
;;
;; The editable prompt line, bash's integration and completion come later;
;; this is the passthrough part.

;;; Code:

(defface romacs-term-dim '((t :inherit shadow))
  "What a terminal's mode line says around its state and keys.")

(defface romacs-term-running '((t :inherit success))
  "A terminal's mode in its mode line while its window takes input.")

(defface romacs-term-free '((t :inherit font-lock-keyword-face))
  "A terminal's mode in its mode line while its window is free.")

(defface romacs-term-exited '((t :inherit warning))
  "A terminal's state in its mode line once its program has exited.")

(defvar romacs-term-shell nil
  "Shell override; nil uses the user's shell, $SHELL, or /bin/sh when that
is unset or empty.")
(defvar romacs-term-mode-hook nil
  "Normal hook run in a terminal buffer once it has its session.")
(defvar romacs-term--closing nil)
(defvar romacs-term-session nil
  "The terminal session a terminal buffer shows, local to it: nil in any
other buffer, in a terminal buffer until its session is attached, and
once it is killed.  It stays after the program exits: `romacs-term-running-p'
says whether the program runs.")
(defvar kill-ring nil)

(defvar romacs-term-keymap-exceptions
  '("C-c" "C-x" "C-u" "C-g" "C-h" "C-l" "M-x" "M-o" "C-y" "M-y" "C-v" "M-v")
  "Keys that keep their Emacs bindings in a terminal running a program.
Each is a `kbd' string.  While the window follows the cursor on the normal
screen, the other keys the terminal has bytes for (M-ESC aside), and not
bound in `romacs-term-mode-map', go to the program; the alternate screen
keeps `romacs-term-alternate-exceptions' and `romacs-term-alternate-map'
instead.  After changing this outside Customize, run
`romacs-term-update-keymaps'.")

(defvar romacs-term-alternate-exceptions '("C-c" "C-x" "M-x" "C-g")
  "Keys that keep their Emacs bindings while a program has the alternate
screen (a full-screen program such as an editor or pager).  Each is a
`kbd' string.  The keys of `romacs-term-mode-map' stay with Emacs there
too; C-q gives the program any key (`romacs-term-send-next-key').  After
changing this outside Customize, run `romacs-term-update-keymaps'.")

(defun romacs-term--set-exceptions (symbol value)
  "Customize's setter for the exception lists: set SYMBOL to VALUE and
rebuild the generated keymaps once they have been built; the first build
reads the lists as they are then."
  (set-default symbol value)
  (when romacs-term--passthrough-base (romacs-term-update-keymaps)))

(defvar romacs-term--passthrough-base nil
  "The generated passthrough bindings, rebuilt from
`romacs-term-keymap-exceptions'.")
(defvar romacs-term--alternate-base nil
  "The generated alternate-screen bindings, rebuilt from
`romacs-term-alternate-exceptions'; parent of `romacs-term-alternate-map'.")

(defvar romacs-term-mode-map
  (let ((map (make-sparse-keymap)))
    (define-key map [3 3] 'romacs-term-interrupt)
    (define-key map [3 11] 'romacs-term-interrupt)
    (define-key map [3 10] 'romacs-term-leave)
    (define-key map [17] 'romacs-term-send-next-key)
    map)
  "Keys of a terminal buffer on every screen: C-c C-c interrupts, C-c C-j
switches between input and free mode, and C-q sends the next key as it
is.  A binding made here wins over the keys sent to a program, and
survives `romacs-term-update-keymaps'.")

(defvar romacs-term-yank-map
  (let ((map (make-sparse-keymap)))
    (define-key map [25] 'romacs-term-yank)
    (define-key map [?\M-y] 'romacs-term-yank-pop)
    map)
  "Keys that send kills: C-y and M-y.  Bound on the normal screen and in a
free window; a full-screen program gets them.")

;; The key the mode line names, of the two that interrupt.
(put 'romacs-term-interrupt :advertised-binding [3 3])

(defvar romacs-term--passthrough (list 'keymap romacs-term-mode-map romacs-term-yank-map (make-sparse-keymap))
  "The passthrough route's keymap: `romacs-term-mode-map' and
`romacs-term-yank-map' composed over the generated bindings (its fourth
element, `romacs-term--passthrough-base').")
(defvar romacs-term-alternate-map (make-sparse-keymap)
  "Keys of a terminal on its alternate screen, above the generated ones
and below `romacs-term-mode-map'.")
(defvar romacs-term--alternate (list 'keymap romacs-term-mode-map romacs-term-alternate-map)
  "The alternate route's keymap: `romacs-term-mode-map' composed over
`romacs-term-alternate-map', whose parent is the generated bindings.")
(defvar romacs-term-away-map
  (let ((map (make-sparse-keymap)))
    (define-key map [remap self-insert-command] 'romacs-term-send-key)
    (define-key map [13] 'romacs-term-send-key)
    (define-key map [return] 'romacs-term-send-key)
    (define-key map [9] 'romacs-term-send-key)
    (define-key map [127] 'romacs-term-send-key)
    map)
  "Keys of a terminal window moved off its cursor: Emacs's, except that
typing puts the window back on the cursor and sends the key.")

(defvar romacs-term--away
  (list 'keymap romacs-term-away-map romacs-term-mode-map romacs-term-yank-map)
  "The free route's keymap: `romacs-term-away-map' and `romacs-term-mode-map'
composed over `romacs-term-yank-map'.")

(defun romacs-term--route-map (route)
  "The local keymap of a key sequence read on ROUTE."
  (cond ((eq route 'passthrough) romacs-term--passthrough)
        ((eq route 'alternate) romacs-term--alternate)
        ((eq route 'away) romacs-term--away)
        (t romacs-term-mode-map)))

(defvar romacs-term--latches nil
  "The key sequences being read, innermost first: (BUFFER . ROUTE) each.")

(defun romacs-term--add-hook (hook function)
  "Add FUNCTION to HOOK's global value, with `add-hook' where it exists."
  (if (fboundp 'add-hook)
      (add-hook hook function)
    (let ((value (if (boundp hook) (symbol-value hook))))
      (if (or (functionp value) (and (consp value) (eq (car value) 'lambda)))
          (setq value (list value)))
      (unless (memq function value)
        (set-default hook (cons function value))))))

(defun romacs-term-running-p ()
  "Return non-nil when the current buffer owns a running terminal."
  (and romacs-term-session (romacs--term-running-p romacs-term-session)))

(defun romacs-term-any-running-p ()
  "Return non-nil if a live buffer owns a running terminal, including hidden ones."
  (let ((buffers (buffer-list)) (running nil))
    (while (and buffers (not running))
      (with-current-buffer (car buffers)
        (when (and (boundp 'romacs-term-session) romacs-term-session
                   (romacs--term-running-p romacs-term-session))
          (setq running t)))
      (setq buffers (cdr buffers)))
    running))

;;;; Keymaps

(defconst romacs-term--function-keys
  '(return tab backtab backspace escape up down left right home end
    prior next insert delete deletechar
    f1 f2 f3 f4 f5 f6 f7 f8 f9 f10 f11 f12)
  "The function keys a terminal has bytes for.")

(declare-function romacs-gui-control-key-events "romacs-editing" ())

(defconst romacs-term--modifier-prefixes
  '("" "S-" "C-" "M-" "C-S-" "M-S-" "C-M-" "C-M-S-")
  "The modifier combinations bound for each function key.")

(defun romacs-term--descriptions (keys)
  "KEYS, `kbd' strings, as `key-description' gives them."
  (mapcar (lambda (key) (key-description (kbd key))) keys))

(defun romacs-term--excepted-p (key skip)
  "Whether KEY, a vector, is one of SKIP's descriptions: by its own, or by
the key the function key map makes it where it is unbound (Tab is TAB,
`C-m' RET), as the command loop would look it up."
  (let ((as (lookup-key (if (boundp 'local-function-key-map) local-function-key-map function-key-map) key)))
    (or (member (key-description key) skip)
        (and (arrayp as) (member (key-description as) skip)))))

(defun romacs-term--passthrough-keymap (exceptions)
  "A keymap that sends every key to the terminal except EXCEPTIONS: ASCII
and meta characters and the function keys with their modifiers
explicitly, and every other character the global map self-inserts
through a remapping, so non-ASCII and input-method characters are
covered.  An exception keeps a key's Emacs binding; a printable one, whose
binding would only self-insert into the read-only buffer, is still sent
through the remapping."
  ;; Full keymaps (a char-table each, the ESC map too), so a character's
  ;; lookup does not walk hundreds of bindings: the host's passthrough
  ;; check makes it on every key.
  (let ((map (make-keymap))
        (skip (romacs-term--descriptions exceptions))
        (code 0))
    (define-key map [27] (make-keymap))
    (while (< code 128)
      ;; Every ASCII character and its meta form, bound outright, so a
      ;; global binding of a printable key does not win; a bare ESC is the
      ;; `escape' function key, and M-ESC is left to Emacs.
      (let ((keys (list (vector code) (vector (logior code #x8000000)))))
        (when (= code 27) (setq keys nil))
        (while keys
          (unless (member (key-description (car keys)) skip)
            (define-key map (car keys) 'romacs-term-send-key))
          (setq keys (cdr keys))))
      (setq code (1+ code)))
    (let ((names romacs-term--function-keys))
      (while names
        (let ((prefixes romacs-term--modifier-prefixes))
          (while prefixes
            (let ((key (vector (intern (concat (car prefixes) (symbol-name (car names)))))))
              (unless (romacs-term--excepted-p key skip)
                (define-key map key 'romacs-term-send-key)))
            (setq prefixes (cdr prefixes))))
        (setq names (cdr names))))
    ;; The GUI's keys a terminal has only as control characters: `C-m' and
    ;; the rest (romacs-editing.el), and C-SPC and C-? (NUL and DEL), with
    ;; Meta.  Each stays with Emacs when it or the character it was is an
    ;; exception, and one with Super, Hyper or Alt, which a terminal has no
    ;; bytes for, is left.
    (dolist (pair (append (romacs-gui-control-key-events)
                          (list (cons (logior ?\s #x4000000) 0) (cons (logior ?\s #xc000000) #x8000000)
                                (cons (logior ?? #x4000000) 127) (cons (logior ?? #xc000000) #x800007f))))
      (unless (or (/= 0 (logand (cdr pair) #x1c00000))
                  (romacs-term--excepted-p (vector (car pair)) skip)
                  (member (key-description (vector (cdr pair))) skip))
        (define-key map (vector (car pair)) 'romacs-term-send-key)))
    ;; Last, so the front of the list: every lookup of a remapping finds it
    ;; first.
    (define-key map [remap self-insert-command] 'romacs-term-send-key)
    map))

(defun romacs-term-update-keymaps ()
  "Rebuild the generated terminal keymaps from the exception lists.
Bindings in `romacs-term-mode-map' and `romacs-term-alternate-map' are
kept."
  (interactive)
  (setq romacs-term--passthrough-base (romacs-term--passthrough-keymap romacs-term-keymap-exceptions)
        romacs-term--alternate-base (romacs-term--passthrough-keymap romacs-term-alternate-exceptions))
  (setcar (nthcdr 3 romacs-term--passthrough) romacs-term--passthrough-base)
  (set-keymap-parent romacs-term-alternate-map romacs-term--alternate-base))

(defun romacs-term--ensure-keymaps ()
  "Build the generated keymaps, once, and install the routes' hooks."
  (unless romacs-term--passthrough-base (romacs-term-update-keymaps))
  (romacs-term--add-hook 'romacs--sequence-start-functions 'romacs-term--sequence-start)
  (romacs-term--add-hook 'romacs--sequence-end-functions 'romacs-term--sequence-end)
  (romacs-term--add-hook 'post-command-hook 'romacs-term--track))

;;;; Routing

(defun romacs-term--route ()
  "The current buffer's route for a key sequence starting now: `away'
where the selected window is free (does not follow the cursor), on any
screen; else `alternate' or `passthrough' as the screen is a full-screen
program's or not; or nil: not a running
terminal in the selected window, or one whose local map another owns (a
confirmation's answers)."
  (when (and romacs-term-session (eq (window-buffer (selected-window)) (current-buffer))
             (eq (current-local-map) romacs-term-mode-map))
    (let ((state (romacs--term-state romacs-term-session)))
      (cond ((not (plist-get state :running)) nil)
            ((not (plist-get state :following)) 'away)
            ((plist-get state :alternate) 'alternate)
            (t 'passthrough)))))

(defun romacs-term--select (latch on)
  "Give LATCH's buffer its route's local map, ON, or its own again: only
over `romacs-term-mode-map', and undoing only that, so a local map another
installed meanwhile stays."
  (let ((buffer (car latch)) (route (cdr latch)))
    (when (and route (buffer-live-p buffer))
      (with-current-buffer buffer
        (let ((from (romacs-term--route-map (and (not on) route)))
              (to (romacs-term--route-map (and on route))))
          (when (eq (current-local-map) from) (use-local-map to)))))))

(defun romacs-term--sequence-start ()
  "Latch the route of the key sequence whose first event was just read."
  (condition-case nil
      (let ((latch (cons (current-buffer) (romacs-term--route))))
        (if romacs-term--latches (romacs-term--select (car romacs-term--latches) nil))
        (setq romacs-term--latches (cons latch romacs-term--latches))
        (romacs-term--select latch t))
    (error nil)))

(defun romacs-term--sequence-end ()
  "Release the latch of the key sequence just read; restore the outer one."
  (condition-case nil
      (when romacs-term--latches
        (romacs-term--select (car romacs-term--latches) nil)
        (setq romacs-term--latches (cdr romacs-term--latches))
        (if romacs-term--latches (romacs-term--select (car romacs-term--latches) t)))
    (error nil)))

(defvar romacs-term--motion-hinted nil
  "Whether this terminal has said how keys work away from its cursor.")
(make-variable-buffer-local 'romacs-term--motion-hinted)

(defun romacs-term--track ()
  "After each command: a terminal window the command moved off the
cursor, or scrolled back, becomes a plain Emacs buffer."
  (condition-case nil
      (progn (romacs--term-track) (romacs-term--motion-hint))
    (error nil)))

(defun romacs-term--away-keys ()
  "How a free window returns to input mode."
  (substitute-command-keys "type or \\<romacs-term-mode-map>\\[romacs-term-leave] to return to input"))

(defun romacs-term--motion-hint ()
  "The first time a command leaves a terminal's cursor, say once how keys
work there (`romacs-term--away-keys').  A command's own message (M-<'s
\"Mark set\") keeps the echo area; the hint waits for a later command."
  (when (and romacs-term-session (not romacs-term--motion-hinted)
             (eq (window-buffer (selected-window)) (current-buffer)))
    (let ((state (romacs--term-state romacs-term-session)))
      (when (and (plist-get state :running) (not (plist-get state :alternate))
                 (not (plist-get state :following)) (not (current-message)))
        (setq romacs-term--motion-hinted t)
        (message "Free mode: %s" (romacs-term--away-keys))))))

;;;; Commands

(defun romacs-term--session ()
  "The current buffer's terminal session, or a `user-error'."
  (or romacs-term-session (signal 'user-error '("Not a terminal buffer"))))

(defun romacs-term-input-mode ()
  "The selected window's mode in the current buffer's terminal: `input'
while it follows the cursor and keys go to the program, `free' while it
is a plain Emacs buffer, or nil where no terminal runs."
  (when (romacs-term-running-p)
    (if (plist-get (romacs--term-state romacs-term-session) :following) 'input 'free)))

(defun romacs-term--snap ()
  "Put the selected window back on the terminal's cursor."
  (romacs--term-follow (romacs-term--session) t))

(defun romacs-term-leave ()
  "Switch the selected window between input mode and free mode.
In input mode the window follows the cursor and keys go to the program.
In free mode point stays where it is and the buffer is an ordinary
read-only Emacs buffer, on the normal screen or a full-screen program's,
to move through, search, select and copy; output does not move it.
Typing, or this command again, returns to input mode."
  (interactive nil romacs-term-mode)
  (let* ((session (romacs-term--session))
         (state (romacs--term-state session)))
    (cond ((not (plist-get state :running)) (signal 'user-error '("The terminal has exited")))
          ((not (plist-get state :following)) (romacs-term--snap))
          (t (romacs--term-follow session nil)
             (setq romacs-term--motion-hinted t)
             (message "Free mode: %s" (romacs-term--away-keys))))))

(defun romacs-term-send-key ()
  "Send the key that ran this command to the terminal, as the terminal's
bytes for it: an ESC prefix before it (a meta key typed as ESC KEY)
makes it a meta key.  The selected window follows the cursor again."
  (interactive nil romacs-term-mode)
  (let* ((session (romacs-term--session))
         (keys (this-command-keys-vector))
         (event last-command-event))
    (when (and (> (length keys) 1) (eq (aref keys (- (length keys) 2)) 27))
      (setq event (if (integerp event)
                      (logior event #x8000000)
                    (intern (concat "M-" (symbol-name event))))))
    (romacs-term--snap)
    (romacs--term-send-event session event)))

(defun romacs-term-send-next-key ()
  "Read a key and send it to the terminal as it is, whatever Emacs binds
it to: \\[romacs-term-send-next-key] C-x gives a program C-x, and
\\[romacs-term-send-next-key] C-c gives it C-c.  As `quoted-insert', C-g
is read, not a quit.  The selected window follows the cursor again."
  (interactive nil romacs-term-mode)
  (unless (romacs-term-running-p) (signal 'user-error '("The terminal has exited")))
  (let ((session (romacs-term--session))
        (event (let ((inhibit-quit t)) (read-event "Send key: "))))
    (romacs-term--snap)
    (romacs--term-send-event session event)))

(defun romacs-term--paste (text)
  "Send TEXT to the terminal as a paste, after `yank-transform-functions'."
  (let ((session (romacs-term--session))
        (functions (and (boundp 'yank-transform-functions) yank-transform-functions)))
    (while functions
      (setq text (funcall (car functions) text)
            functions (cdr functions)))
    (romacs-term--snap)
    (romacs--term-paste session text)))

(defun romacs-term-yank (&optional arg)
  "Send the most recent kill to the terminal, as `yank' would insert it:
the clipboard first, through `interprogram-paste-function'.  With ARG,
the ARGth most recent kill.  Bracketed when the program asked for it."
  (interactive "P" romacs-term-mode)
  (romacs-term--paste (current-kill (cond ((listp arg) 0) ((eq arg '-) -2) (t (1- arg))))))

(defun romacs-term-yank-pop ()
  "Choose a kill from the kill ring and send it to the terminal.
Text sent to a program cannot be taken back, so nothing is replaced."
  (interactive nil romacs-term-mode)
  (romacs-term--paste
   (if (fboundp 'read-from-kill-ring)
       (read-from-kill-ring "Send from kill-ring: ")
     (completing-read "Send from kill-ring: " kill-ring nil t))))

(defun romacs-term-interrupt ()
  "Send Ctrl-C (ETX) to the terminal; its line discipline normally
delivers SIGINT."
  (interactive nil romacs-term-mode)
  (romacs--term-send (romacs-term--session) "\003"))

(defun romacs-term--kill-query ()
  "Ask before killing a buffer whose terminal still runs, as GNU's
`process-kill-buffer-query-function' asks for a buffer's process."
  (or (not (romacs-term-running-p)) romacs-term--closing
      (yes-or-no-p (format "Buffer %S has a running process; kill it? "
                           (buffer-name (current-buffer))))))

(defun romacs-term--close ()
  "Retire the native session when its terminal buffer is killed."
  (when romacs-term-session
    (romacs--term-close romacs-term-session)
    (setq romacs-term-session nil))
  ;; The session told the helper to end; the keeper lets go of it.
  (when romacs-term-handle
    (condition-case nil (romacs--keeper-call 'close romacs-term-handle nil) (error nil))
    (setq romacs-term-handle nil)))

(defun romacs-term--hint (&optional mode)
  "The mode line's keys in MODE, `input' or `free'."
  (romacs-key-hint (if (eq mode 'free)
                       "\\<romacs-term-mode-map>\\[romacs-term-leave] input"
                     "\\<romacs-term-mode-map>\\[romacs-term-interrupt] interrupt  \\[romacs-term-send-next-key] quote  \\[romacs-term-leave] free")
                   'romacs-term-dim))

(defconst romacs-term--mode-line
  (let ((separator (propertize "  |  " 'face 'romacs-term-dim))
        (input (propertize "input" 'face 'romacs-term-running))
        (free (propertize "free" 'face 'romacs-term-free))
        (exited (propertize "exited" 'face 'romacs-term-exited)))
    `("  " (:propertize "%b" face mode-line-buffer-id) ,separator mode-name "  "
      (:eval (let ((mode (romacs-term-input-mode)))
               (list (cond ((eq mode 'input) ,input) ((eq mode 'free) ,free) (t ,exited))
                     ,separator (romacs-term--hint mode))))))
  "A terminal's `mode-line-format': its buffer, its mode (input, free or
exited) and that mode's keys.")

(defun romacs-term--buffer-name (arg)
  "The terminal buffer's name for prefix ARG, as `eshell' chooses its
buffer: a number N names *romacs-term*<N>, any other non-nil ARG a new
buffer, and nil *romacs-term*."
  (cond ((numberp arg) (format "*romacs-term*<%d>" arg))
        (arg (generate-new-buffer-name "*romacs-term*"))
        (t "*romacs-term*")))

(defun romacs-term--retire (buffer)
  "Make way for a new terminal: kill BUFFER, a terminal whose program has
exited.  Any other buffer of that name is someone's text: a `user-error'."
  (when buffer
    (unless (buffer-local-value 'romacs-term-session buffer)
      (signal 'user-error (list (format "Buffer %s is not a terminal" (buffer-name buffer)))))
    (kill-buffer buffer)))

(defun romacs-term--running-buffer (name)
  "The buffer called NAME, if a terminal runs in it."
  (let ((buffer (get-buffer name)))
    (and buffer (with-current-buffer buffer (romacs-term-running-p)) buffer)))

(defun romacs-term (&optional shell arg)
  "Switch to the terminal *romacs-term*, or start it running SHELL in
`default-directory'.  With a numeric prefix ARG N, *romacs-term*<N>
instead; with \\[universal-argument] alone, a new terminal.  A terminal
whose program has exited starts afresh.
In input mode keys go to the program, except
`romacs-term-keymap-exceptions' and the keys of `romacs-term-mode-map';
the alternate screen keeps `romacs-term-alternate-exceptions'.  C-x and
C-c stay Emacs prefixes: C-c C-c sends Ctrl-C, and C-q sends the next key
as it is (C-q C-x for nano).  On the normal screen C-y and M-y send
kills.  C-c C-j, on any screen, makes the window free: a plain Emacs
buffer to search, select and copy from, until you type or press C-c C-j
again.  Moving point off the cursor or scrolling back frees it too."
  (interactive (list nil current-prefix-arg))
  (romacs-term--ensure-keymaps)
  (let* ((name (romacs-term--buffer-name arg))
         (running (romacs-term--running-buffer name))
         ;; Read first: the buffer retired may be the current one.
         (directory (or (and (boundp 'default-directory) default-directory) "/")))
    (if running
        (progn (romacs-select-buffer running) running)
      (romacs-term--retire (get-buffer name))
      (romacs-term--start name shell directory))))

(defvar romacs-term-handle nil
  "The keeper handle of this terminal's helper process.
Nil for a terminal whose helper belongs to this session alone, and so
ends with it.  Local to a terminal buffer.")

(defvar romacs-term-open-function nil
  "Function that starts a terminal in the current buffer, or nil.
Called with the shell and the directory; returns the session.  With nil
the terminal's helper process belongs to this session alone.
`romacs-term-keep' sets it, so terminals outlive the session.")

(defun romacs-term--open (shell directory)
  "Start SHELL in DIRECTORY as the current buffer's terminal; its session."
  (if romacs-term-open-function
      (funcall romacs-term-open-function shell directory)
    (romacs--term-create shell directory)))

(defun romacs-term--prepare (directory)
  "Make the current buffer a terminal buffer in DIRECTORY, with no session yet."
  (make-local-variable 'romacs-term-session)
  (make-local-variable 'romacs-term-handle)
  (make-local-variable 'default-directory)
  (make-local-variable 'mode-line-format)
  (make-local-variable 'kill-buffer-hook)
  (make-local-variable 'kill-buffer-query-functions)
  (setq default-directory directory
        major-mode 'romacs-term-mode mode-name "Term"
        buffer-file-name nil buffer-read-only t)
  (use-local-map romacs-term-mode-map))

(defun romacs-term--bound ()
  "Finish the current buffer once it has its session."
  (setq kill-buffer-hook '(romacs-term--close)
        kill-buffer-query-functions '(romacs-term--kill-query)
        mode-line-format romacs-term--mode-line)
  (run-hooks 'romacs-term-mode-hook))

(defun romacs-term--start (name shell directory)
  "Start a terminal running SHELL in DIRECTORY, in a new buffer called NAME,
and show it."
  (let* ((buffer (get-buffer-create name))
         (completed nil))
    (unwind-protect
        (progn
          (with-current-buffer buffer (romacs-term--prepare directory))
          ;; Shown first, so the session takes the size of the window that
          ;; shows it (with a one-line mini-window), not the caller's.
          (romacs-select-buffer buffer)
          (with-current-buffer buffer
            (setq romacs-term-session (romacs-term--open (or shell romacs-term-shell "") directory))
            (romacs-term--bound))
          (setq completed t)
          buffer)
      (unless completed
        (when (buffer-live-p buffer)
          (with-current-buffer buffer (romacs-term--close))
          (let ((kill-buffer-query-functions nil) (kill-buffer-hook nil)) (kill-buffer buffer)))))))

;; Customize knows the exception lists where it is loaded.
(when (fboundp 'custom-declare-variable)
  (custom-declare-variable 'romacs-term-keymap-exceptions (list 'quote romacs-term-keymap-exceptions)
                           (get 'romacs-term-keymap-exceptions 'variable-documentation)
                           :type '(repeat string) :set 'romacs-term--set-exceptions :group 'terminals)
  (custom-declare-variable 'romacs-term-alternate-exceptions (list 'quote romacs-term-alternate-exceptions)
                           (get 'romacs-term-alternate-exceptions 'variable-documentation)
                           :type '(repeat string) :set 'romacs-term--set-exceptions :group 'terminals))

(provide 'romacs-term)
