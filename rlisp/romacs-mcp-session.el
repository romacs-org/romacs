;;; romacs-mcp-session.el --- Drive and observe the session over MCP -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; Optional input and observation tools for agents that use the editor as a
;; person does: keys go through the command loop, waiting lets it, timers
;; and processes settle, and an observation reports what the frame shows.
;; Plain Elisp, so GNU Emacs serves the same tools with the same shapes;
;; the paired-session bench (crates/romacs-vm-gui/harness/bench) compares
;; the two.  Explicit registration only; loading changes nothing.

;;; Code:
(require 'romacs-mcp)
(require 'cl-lib)
(require 'seq)

(defvar romacs-mcp-session-secrets-function nil
  "A function of no arguments returning the credentials the session holds.
Every string an observation reports that contains one becomes \"<secret>\".")

(defconst romacs-mcp-session-row-limit 400
  "Most characters of one observed row or *Messages* line.")

(defvar romacs-mcp-session--last-error nil
  "The last error the command loop reported, as a JSON object, or nil.")
(defvar romacs-mcp-session--error-count 0)
(defvar romacs-mcp-session--last-backtrace nil)
(defvar romacs-mcp-session--previous-command-error nil)

;;; Plain values

(defun romacs-mcp-session--bool (value) (if value t :false))

(defun romacs-mcp-session--string (string)
  "STRING without properties or raw bytes, bounded, credentials scrubbed."
  (let ((string (substring-no-properties string 0 (min (length string) romacs-mcp-session-row-limit))))
    (when (seq-find (lambda (char) (> char #x10FFFF)) string)
      (setq string (apply #'string (mapcar (lambda (char) (if (> char #x10FFFF) ?? char)) string))))
    (if (and romacs-mcp-session-secrets-function
             (seq-some (lambda (secret) (and (stringp secret) (> (length secret) 7) (string-search secret string)))
                       (funcall romacs-mcp-session-secrets-function)))
        "<secret>"
      string)))

(defun romacs-mcp-session--print (value)
  (romacs-mcp-session--string
   (let ((print-length 8) (print-level 3) (print-escape-newlines t)) (prin1-to-string value))))

(defun romacs-mcp-session--face (face)
  "FACE, a text or overlay property, as a stable string."
  (cond ((null face) nil)
        ((symbolp face) (symbol-name face))
        ((and (consp face) (seq-every-p #'symbolp face) (not (keywordp (car face))))
         (mapconcat #'symbol-name face "+"))
        (t (romacs-mcp-session--print face))))

;;; The displayed text of a window

(defun romacs-mcp-session--overlay-strings (position property start)
  "PROPERTY strings of overlays at boundary POSITION, display order.
An after-string belongs to an overlay ending at POSITION, a before-string
to one starting there; after-strings at START belong to earlier text."
  (let (strings)
    (dolist (overlay (overlays-in (max (point-min) (1- position)) (min (point-max) (1+ position))))
      (let ((value (overlay-get overlay property)))
        (when (and (stringp value)
                   (if (eq property 'after-string)
                       (and (= (overlay-end overlay) position)
                            (or (> position start) (= (overlay-start overlay) position)))
                     (= (overlay-start overlay) position)))
          (push (cons (or (overlay-get overlay 'priority) 0) value) strings))))
    (mapcar #'cdr (sort strings (lambda (a b) (< (car a) (car b)))))))

(defun romacs-mcp-session--display-text (start end faces)
  "The text from START to END as displayed, and its face runs if FACES.
Invisible text is skipped; display strings, spaces and images and overlay
before- and after-strings appear.  Returns (TEXT . RUNS), each run
\(FROM TO FACE) in TEXT's coordinates."
  (let ((position start) (pieces nil) (runs nil) (offset 0))
    (cl-flet ((emit (text &optional face from to)
                (when (> (length text) 0)
                  (push text pieces)
                  (when faces
                    (if from
                        (let ((at from))
                          (while (< at to)
                            (let ((next (min to (next-single-char-property-change at 'face nil to)))
                                  (name (romacs-mcp-session--face (get-char-property at 'face))))
                              (when name (push (list (+ offset (- at from)) (+ offset (- next from)) name) runs))
                              (setq at next))))
                      (let ((name (romacs-mcp-session--face (or face (get-text-property 0 'face text)))))
                        (when name (push (list offset (+ offset (length text)) name) runs)))))
                  (setq offset (+ offset (length text))))))
      (while (<= position end)
        (dolist (text (romacs-mcp-session--overlay-strings position 'after-string start)) (emit text))
        (when (< position end)
          (dolist (text (romacs-mcp-session--overlay-strings position 'before-string start)) (emit text)))
        (if (>= position end)
            (setq position (1+ end))
          (let ((next (min end
                           (next-single-char-property-change position 'invisible nil end)
                           (next-single-char-property-change position 'display nil end)
                           (next-overlay-change position)))
                (display (get-char-property position 'display)))
            (cond ((invisible-p position))
                  ((stringp display) (emit display))
                  ((eq (car-safe display) 'space) (emit " "))
                  ((eq (car-safe display) 'image) (emit "<image>"))
                  (t (emit (buffer-substring-no-properties position next) nil position next)))
            (setq position (max next (1+ position)))))))
    (cons (apply #'concat (nreverse pieces)) (nreverse runs))))

(defun romacs-mcp-session--rows (text runs limit)
  "TEXT split into at most LIMIT row objects, with RUNS clipped to each."
  (let ((rows nil) (start 0) (line 0))
    (while (and (<= start (length text)) (< line limit))
      (let* ((end (min (or (string-search "\n" text start) (length text))
                       (+ start romacs-mcp-session-row-limit)))
             (row (romacs-mcp--object
                   "text" (romacs-mcp-session--string (substring text start end)))))
        (when runs
          (puthash "faces"
                   (vconcat (delq nil (mapcar (lambda (run)
                                                (and (< (nth 0 run) end) (> (nth 1 run) start)
                                                     (vector (- (max start (nth 0 run)) start)
                                                             (- (min end (nth 1 run)) start) (nth 2 run))))
                                              runs)))
                   row))
        (push row rows)
        (setq start (1+ (or (string-search "\n" text start) (length text))) line (1+ line))))
    (vconcat (nreverse rows))))

(defun romacs-mcp-session--format-line (format window)
  "FORMAT as WINDOW's mode or header line shows it, or the error it signals."
  (condition-case condition
      (romacs-mcp-session--string (format-mode-line format nil window))
    (error (concat "<error: " (error-message-string condition) ">"))))

(defun romacs-mcp-session--window (window index faces limit)
  (let* ((buffer (window-buffer window))
         (start (window-start window))
         (end (or (ignore-errors (window-end window t)) (window-end window))))
    (with-current-buffer buffer
      (let ((shown (save-restriction (widen) (romacs-mcp-session--display-text start (min end (point-max)) faces))))
        (romacs-mcp--object
         "index" index "buffer" (buffer-name buffer)
         "selected" (romacs-mcp-session--bool (eq window (selected-window)))
         "start" start "end" end "point" (window-point window)
         "hscroll" (window-hscroll window)
         "body" (vector (window-body-width window) (window-body-height window))
         "dedicated" (romacs-mcp-session--bool (window-dedicated-p window))
         "modeLine" (let ((format (buffer-local-value 'mode-line-format buffer)))
                      (if format (romacs-mcp-session--format-line format window) :null))
         "headerLine" (let ((format (buffer-local-value 'header-line-format buffer)))
                        (if format (romacs-mcp-session--format-line format window) :null))
         "rows" (romacs-mcp-session--rows (car shown) (cdr shown) limit))))))

(defun romacs-mcp-session--layers (faces limit)
  "The visible child frames shown over the frame, oldest first: each one's
root window as `windows' reports it, with the frame's place in its parent
and its text size in columns and lines."
  (vconcat
   (cl-loop for frame in (visible-frame-list) for i from 0
            when (frame-parent frame)
            collect (let ((layer (romacs-mcp-session--window (frame-root-window frame) i faces limit)))
                      (puthash "position" (vector (car (frame-position frame)) (cdr (frame-position frame))) layer)
                      (puthash "size" (vector (frame-text-cols frame) (frame-text-lines frame)) layer)
                      layer))))

(defun romacs-mcp-session--tree (window windows)
  "WINDOW's subtree: a window index or a split with children."
  (if (window-live-p window)
      (romacs-mcp--object "window" (cl-position window windows))
    (let ((child (or (window-top-child window) (window-left-child window))) children)
      (while child
        (push (romacs-mcp-session--tree child windows) children)
        (setq child (window-next-sibling child)))
      (romacs-mcp--object
       "split" (if (window-top-child window) "vertical" "horizontal")
       "children" (vconcat (nreverse children))))))

(defun romacs-mcp-session--minor-modes (modes)
  (vconcat (sort (mapcar #'symbol-name
                         (seq-filter (lambda (mode) (and (boundp mode) (symbol-value mode))) (copy-sequence modes)))
                 #'string<)))

(defun romacs-mcp-session--messages (from count)
  "The last COUNT *Messages* lines after position FROM, or of its last 20
lines if FROM is nil."
  (with-current-buffer (messages-buffer)
    (let* ((start (if (and from (<= from (point-max))) from
                    (save-excursion (goto-char (point-max)) (forward-line -20) (point))))
           (lines (split-string (buffer-substring-no-properties start (point-max)) "\n" t)))
      (romacs-mcp--object
       "from" start "next" (point-max)
       "lines" (vconcat (mapcar #'romacs-mcp-session--string (last lines count)))))))

(defun romacs-mcp-session--observe (arguments)
  "The frame's observable state, the same shape in every engine.
Within the endpoint's output limit, measured as the MCP response encodes
it: without faces if they do not fit, then with fewer rows, then with
fewer *Messages* lines; `truncated' says which."
  (romacs-mcp--instance arguments)
  (let ((faces (eq (gethash "faces" arguments) t))
        (limit (or (gethash "maxRows" arguments) 60))
        (lines 60) (truncated :false) result)
    (while (progn
             (setq result (romacs-mcp-session--state arguments faces limit lines))
             (puthash "truncated" truncated result)
             (and (> (string-bytes (json-serialize (romacs-mcp--result result nil t)
                                                   :false-object :false :null-object :null))
                     (- romacs-mcp--output-limit 4096))
                  (or faces (> limit 0) (> lines 0))))
      (cond (faces (setq faces nil truncated "faces"))
            ((> limit 0) (setq limit (/ limit 2) truncated "rows"))
            (t (setq lines (/ lines 2) truncated "messages"))))
    result))

(defun romacs-mcp-session--state (arguments faces limit lines)
  "The observation for ARGUMENTS, with FACES, at most LIMIT rows a window
and at most LINES *Messages* lines."
  (let* ((window (selected-window))
         (mini (active-minibuffer-window))
         (buffer (window-buffer (if (and (minibufferp (window-buffer window)) (minibuffer-selected-window))
                                    (minibuffer-selected-window) window)))
         (windows (window-list nil 'nomini (frame-first-window))))
    (romacs-mcp--object
     "selectedBuffer" (buffer-name (window-buffer window))
     "selectedWindow" (or (cl-position window windows) :null)
     "buffer" (with-current-buffer buffer
                (romacs-mcp--object
                 "name" (buffer-name) "majorMode" (symbol-name major-mode)
                 "minorModes" (romacs-mcp-session--minor-modes (bound-and-true-p local-minor-modes))
                 "point" (point) "size" (buffer-size)
                 "narrowed" (romacs-mcp-session--bool (buffer-narrowed-p))
                 "modified" (romacs-mcp-session--bool (buffer-modified-p))
                 "readOnly" (romacs-mcp-session--bool buffer-read-only)
                 "file" (if buffer-file-name (abbreviate-file-name buffer-file-name) :null)
                 "directory" (abbreviate-file-name default-directory)))
     "globalMinorModes" (romacs-mcp-session--minor-modes (bound-and-true-p global-minor-modes))
     "tree" (romacs-mcp-session--tree (frame-root-window) windows)
     "windows" (vconcat (cl-loop for w in windows for i from 0
                                 collect (romacs-mcp-session--window w i faces limit)))
     "layers" (romacs-mcp-session--layers faces limit)
     "minibuffer" (if mini
                      (with-current-buffer (window-buffer mini)
                        (romacs-mcp--object
                         "active" t "depth" (minibuffer-depth)
                         "prompt" (romacs-mcp-session--string (or (minibuffer-prompt) ""))
                         "contents" (romacs-mcp-session--string (minibuffer-contents-no-properties))
                         "point" (- (point) (minibuffer-prompt-end))
                         "rows" (let ((shown (romacs-mcp-session--display-text (point-min) (point-max) faces)))
                                  (romacs-mcp-session--rows (car shown) (cdr shown) limit))))
                    (romacs-mcp--object "active" :false))
     "echo" (let ((message (current-message))) (if message (romacs-mcp-session--string message) :null))
     "messages" (romacs-mcp-session--messages (gethash "messagesFrom" arguments) lines)
     "lastError" (or romacs-mcp-session--last-error :null)
     "lastBacktrace" (or romacs-mcp-session--last-backtrace :null)
     "buffers" (vconcat (seq-remove (lambda (name) (string-prefix-p " " name))
                                    (mapcar #'buffer-name (buffer-list)))))))

;;; Errors

(defun romacs-mcp-session--command-error (data context caller)
  "Record DATA, the error the command loop reports, then report it as before."
  (setq romacs-mcp-session--last-error
        (romacs-mcp--object
         "serial" (cl-incf romacs-mcp-session--error-count)
         "message" (romacs-mcp-session--string (error-message-string data))
         "data" (romacs-mcp-session--print data)
         "context" (if context (romacs-mcp-session--string context) :null)
         "command" (romacs-mcp-session--print this-command)))
  (funcall (or romacs-mcp-session--previous-command-error #'command-error-default-function)
           data context caller))

(defun romacs-mcp-session--debugger (&rest arguments)
  "Record a backtrace for the signal ARGUMENTS describe; let it proceed."
  (when (and (eq (car arguments) 'error) (not (eq (car-safe (cadr arguments)) 'quit)))
    (setq romacs-mcp-session--last-backtrace
          (romacs-mcp--object
           "error" (romacs-mcp-session--print (cadr arguments))
           "command" (romacs-mcp-session--print this-command)
           "backtrace" (let ((text (with-output-to-string (backtrace))))
                         (substring text 0 (min (length text) 8000))))))
  nil)

(defun romacs-mcp-session-capture-backtraces (on)
  "Record a backtrace for each signal if ON; leave signals alone if not.
On sets `debug-on-signal', so a signal a handler catches is recorded too
\(the handler still runs afterwards), and `lastBacktrace' is the newest:
use it to rerun one failing step, not for a whole session.  Not
`debug-on-error' alone: a command loop that catches a command's error
without a `debug' handler never calls the debugger."
  (setq debug-on-signal (and on t)
        debug-on-error (and on t)
        debugger (if on #'romacs-mcp-session--debugger #'debug))
  on)

;;; Input and settling

(defun romacs-mcp-session--loop-idle-p (settle)
  "Whether input is drained and the command loop has waited SETTLE seconds."
  (let ((idle (current-idle-time)))
    (and (null unread-command-events) (not (input-pending-p)) (not executing-kbd-macro)
         idle (>= (float-time idle) settle))))

(defun romacs-mcp-session--events (arguments)
  "The events ARGUMENTS name: `keys' as `kbd' reads it, or `text' literally."
  (let ((keys (gethash "keys" arguments)) (text (gethash "text" arguments)))
    (cond ((and keys text) (user-error "Give keys or text, not both"))
          (keys (listify-key-sequence (kbd keys)))
          (text (string-to-list text))
          (t (user-error "Give keys or text")))))

(defun romacs-mcp-session--send-keys (arguments)
  "Feed the events ARGUMENTS name to the command loop.
Paced (the default), each event waits until the previous one is consumed
and the loop has been idle `gap' seconds, as typing does; a burst queues
them all at once.  Answers once all are consumed or `timeout' passes."
  (romacs-mcp--instance arguments)
  (let* ((events (romacs-mcp-session--events arguments))
         (count (length events))
         (paced (not (eq (gethash "burst" arguments) t)))
         (gap (or (gethash "gap" arguments) 0.05))
         (timeout (or (gethash "timeout" arguments) 30))
         (started (float-time)))
    (unless paced
      (setq unread-command-events (append unread-command-events events) events nil))
    (romacs-mcp-defer
     (lambda ()
       (when (and events (romacs-mcp-session--loop-idle-p gap))
         (setq unread-command-events (append unread-command-events (list (pop events)))))
       (let ((consumed (and (null events) (null unread-command-events)))
             (late (> (- (float-time) started) timeout)))
         (when (or consumed late)
           (list (romacs-mcp--object
                  "events" count "undelivered" (+ (length events) (length unread-command-events))
                  "paced" (romacs-mcp-session--bool paced)
                  "timedOut" (romacs-mcp-session--bool (and late (not consumed)))
                  "seconds" (- (float-time) started)))))))))

(defun romacs-mcp-session--own-timer-p (timer)
  (memq (timer--function timer) '(romacs-mcp--drain romacs-mcp--poll-deferred)))

(defun romacs-mcp-session--timer-name (timer)
  (let ((function (timer--function timer)))
    (if (symbolp function) (symbol-name function) "<lambda>")))

(defun romacs-mcp-session--due-timers (horizon)
  "One-shot timers due within HORIZON seconds, and idle ones not yet run."
  (let ((limit (+ (float-time) horizon)) (due nil))
    (dolist (timer timer-list)
      (when (and (not (timer--repeat-delay timer)) (not (romacs-mcp-session--own-timer-p timer))
                 (< (float-time (timer--time timer)) limit))
        (push (romacs-mcp-session--timer-name timer) due)))
    (dolist (timer timer-idle-list)
      (when (and (not (timer--repeat-delay timer))
                 (<= (- (float-time (timer--time timer)) (float-time (or (current-idle-time) 0))) horizon))
        (push (romacs-mcp-session--timer-name timer) due)))
    (nreverse due)))

(defun romacs-mcp-session--process-state ()
  "Each process's name, status and buffer tick, but the endpoint's own."
  (delq nil (mapcar (lambda (process)
                      (unless (string-prefix-p "romacs-mcp" (process-name process))
                        (list (process-name process) (process-status process)
                              (let ((buffer (process-buffer process)))
                                (and (buffer-live-p buffer) (buffer-modified-tick buffer))))))
                    (process-list))))

(defun romacs-mcp-session--wait-idle (arguments)
  "Answer once the command loop is idle and timers and processes settled.
Idle means input drained and `settle' seconds waiting for input; timers
settled means no one-shot timer due within `horizon' seconds; processes
settled means no process changed status or buffer for `settle' seconds.
At `timeout', answer with which of those had not happened."
  (romacs-mcp--instance arguments)
  (let* ((timeout (or (gethash "timeout" arguments) 10))
         (settle (or (gethash "settle" arguments) 0.3))
         (horizon (or (gethash "horizon" arguments) 1.0))
         (started (float-time))
         (processes (romacs-mcp-session--process-state))
         (changed started))
    (romacs-mcp-defer
     (lambda ()
       (let ((now (romacs-mcp-session--process-state)))
         (unless (equal now processes) (setq processes now changed (float-time))))
       (let* ((idle (romacs-mcp-session--loop-idle-p settle))
              (due (romacs-mcp-session--due-timers horizon))
              (quiet (>= (- (float-time) changed) settle))
              (late (> (- (float-time) started) timeout)))
         (when (or (and idle (null due) quiet) late)
           (list (romacs-mcp--object
                  "settled" (romacs-mcp-session--bool (and idle (null due) quiet))
                  "idle" (romacs-mcp-session--bool idle)
                  "timersSettled" (romacs-mcp-session--bool (null due))
                  "processesSettled" (romacs-mcp-session--bool quiet)
                  "timedOut" (romacs-mcp-session--bool (not (and idle (null due) quiet)))
                  "dueTimers" (vconcat (seq-uniq due))
                  "processes" (vconcat (mapcar (lambda (p) (format "%s:%s" (car p) (nth 1 p))) processes))
                  "seconds" (- (float-time) started)))))))))

;;; Registration

;;;###autoload
(defun romacs-mcp-enable-session-tools ()
  "Register the input, settling and observation tools; record command errors."
  (interactive)
  (unless (eq command-error-function #'romacs-mcp-session--command-error)
    (setq romacs-mcp-session--previous-command-error command-error-function
          command-error-function #'romacs-mcp-session--command-error))
  (let ((instance '("instance" . "string")))
    (romacs-mcp-register-tool
     "romacs_send_keys"
     "Type keys (a kbd string) or literal text through the command loop, paced like typing unless burst; answers when consumed."
     (romacs-mcp--schema (list instance '("keys" . "string") '("text" . "string") '("burst" . "boolean")
                               '("gap" . "number") '("timeout" . "number"))
                         '("instance"))
     #'romacs-mcp-session--send-keys)
    (romacs-mcp-register-tool
     "romacs_wait_idle"
     "Answer when the command loop is idle and timers and process output settled, or at timeout; says which."
     (romacs-mcp--schema (list instance '("timeout" . "number") '("settle" . "number") '("horizon" . "number"))
                         '("instance"))
     #'romacs-mcp-session--wait-idle (romacs-mcp--object "readOnlyHint" t))
    (romacs-mcp-register-tool
     "romacs_observe"
     "The frame as shown: selected window and buffer, window tree, rows, mode and header lines, minibuffer, echo, *Messages* since messagesFrom, last error, modes."
     (romacs-mcp--schema (list instance '("faces" . "boolean") '("maxRows" . "integer") '("messagesFrom" . "integer"))
                         '("instance"))
     #'romacs-mcp-session--observe (romacs-mcp--object "readOnlyHint" t))))

(provide 'romacs-mcp-session)
;;; romacs-mcp-session.el ends here
