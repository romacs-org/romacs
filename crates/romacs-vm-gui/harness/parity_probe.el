;;; parity_probe.el --- The same observer in GNU and Romacs  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; The parity harnesses (parity_live.py) load this file from each engine's
;; init.  A
;; repeating timer answers requests the harness writes to
;; $PARITY_PROBE_DIR/request.eld, a `(ID . FORM)' pair, by writing
;; `(ID ok VALUE)' or `(ID error CONDITION)' to response.eld.  Timers run
;; when the engine waits for input, so an answer also means the engine has
;; consumed the input before it.  The observer reads state; it does not
;; touch the match data, the current buffer, the echo area or *Messages*.
;; An init may set `parity-probe-variable-prefix', `parity-probe-variables'
;; and `parity-probe-secrets-function' before loading it.

;;; Code:

(defvar parity-probe-dir (file-name-as-directory (getenv "PARITY_PROBE_DIR")))
(defvar parity-probe-messages-start 1
  "*Messages* position where the current workflow began.")
(defvar parity-probe-text-limit 6000
  "Most characters of a window's text a snapshot records.")
(defvar parity-probe-variable-prefix "gnosis"
  "Prefix of the variables a snapshot records, unless `parity-probe-variables'.")
(defvar parity-probe-variables nil
  "The variables a snapshot records, if non-nil.
A package whose state holds credentials lists its own rather than
having every variable with its prefix recorded.")
(defvar parity-probe-secrets-function nil
  "A function of no arguments returning the credentials the session holds.
Every string the probe reports that contains one becomes \"<secret>\".")

(defun parity-probe--file (name) (expand-file-name name parity-probe-dir))

(defun parity-probe--scrub (string)
  "STRING, or \"<secret>\" when it holds a credential the session has."
  (if (and parity-probe-secrets-function
           (seq-some (lambda (secret) (and (stringp secret) (> (length secret) 7) (string-search secret string)))
                     (funcall parity-probe-secrets-function)))
      "<secret>"
    string))

(defun parity-probe--write (value)
  (let ((print-length nil) (print-level nil) (print-escape-newlines t)
        (print-escape-control-characters t) (print-circle nil)
        (coding-system-for-write 'utf-8-unix)
        (temporary (parity-probe--file "response.tmp")))
    (with-temp-file temporary (insert (prin1-to-string value)))
    (rename-file temporary (parity-probe--file "response.eld") t)))

(defun parity-probe--poll ()
  (let ((request (parity-probe--file "request.eld")))
    (when (file-exists-p request)
      (save-match-data
        (save-current-buffer
          (let* ((deactivate-mark deactivate-mark)
                 (inhibit-message t)
                 (message-log-max nil)
                 (pair (with-temp-buffer
                         (let ((coding-system-for-read 'utf-8-unix))
                           (insert-file-contents request))
                         (read (current-buffer)))))
            (delete-file request)
            (let ((answer (condition-case condition
                              (list 'ok (eval (cdr pair) t))
                            (error (list 'error (parity-probe--plain condition))))))
              ;; An answer the printer refuses still gets a reply.
              (condition-case condition
                  (parity-probe--write (cons (car pair) answer))
                (error (parity-probe--write
                        (list (car pair) 'error (list 'unprintable (parity-probe--plain condition)))))))))))))

(defvar parity-probe-event-ids nil
  "Ids gnosis-scheduler-event-id made, newest first.")

(with-eval-after-load 'gnosis-scheduler
  (advice-add 'gnosis-scheduler-event-id :filter-return
              (lambda (id) (push id parity-probe-event-ids) id)))

(defun parity-probe--plain (value &optional depth)
  "VALUE with objects the reader cannot read replaced by readable data.
An id gnosis made from the clock becomes <event-id-N>, N its order.
Below DEPTH 8 a structure is elided as `deep', so cycles end too."
  (setq depth (1+ (or depth 0)))
  (cond ((or (null value) (eq value t) (numberp value) (symbolp value)) value)
        ((> depth 8) 'deep)
        ((and (stringp value) (member value parity-probe-event-ids))
         (format "<event-id-%d>" (length (member value parity-probe-event-ids))))
        ((stringp value) (parity-probe--scrub (substring-no-properties value)))
        ((bufferp value) (list 'buffer (buffer-name value)))
        ((markerp value) (list 'marker (marker-position value)
                               (and (marker-buffer value) (buffer-name (marker-buffer value)))))
        ((windowp value) (list 'window (and (window-live-p value) (buffer-name (window-buffer value)))))
        ((hash-table-p value) (list 'hash-table (hash-table-count value)))
        ((functionp value) 'function)
        ((consp value)
         (let ((result nil) (n 0))
           (while (and (consp value) (< n 200))
             (push (parity-probe--plain (car value) depth) result)
             (setq value (cdr value) n (1+ n)))
           (nconc (nreverse result) (and value (list '\. (parity-probe--plain value depth))))))
        ((vectorp value) (vconcat (mapcar (lambda (v) (parity-probe--plain v depth)) (seq-take value 200))))
        ((or (timerp value)) 'timer)
        ((processp value) (list 'process (process-name value)))
        ((overlayp value) (list 'overlay (overlay-start value) (overlay-end value)))
        (t (list 'object (type-of value)))))

(defconst parity-probe-text-properties
  '(face font-lock-face invisible display mouse-face help-echo button category
    gnosis-id field read-only intangible cursor-intangible)
  "Text properties a snapshot records.")

(defun parity-probe--properties (position)
  (let (result)
    (dolist (name parity-probe-text-properties)
      (let ((value (get-text-property position name)))
        (when value (setq result (plist-put result name (parity-probe--plain value))))))
    result))

(defun parity-probe--runs (start end)
  "Property runs over START..END as (FROM TO PLIST), FROM relative to START."
  (let ((runs nil) (position start))
    (while (< position end)
      (let ((next (min end (next-property-change position nil end))))
        (push (list (- position start) (- next start) (parity-probe--properties position)) runs)
        (setq position next)))
    (let (merged)
      (dolist (run (nreverse runs))
        (if (and merged (equal (nth 2 (car merged)) (nth 2 run)))
            (setcar merged (list (car (car merged)) (nth 1 run) (nth 2 run)))
          (push run merged)))
      (seq-remove (lambda (run) (null (nth 2 run))) (nreverse merged)))))

(defun parity-probe--overlays (start end)
  (sort (delq nil
              (mapcar (lambda (overlay)
                        (let ((props (seq-filter #'identity
                                                 (mapcar (lambda (name)
                                                           (let ((value (overlay-get overlay name)))
                                                             (and value (list name (parity-probe--plain value)))))
                                                         '(face invisible display before-string after-string
                                                           priority line-prefix wrap-prefix)))))
                          (and props (list (- (overlay-start overlay) start) (- (overlay-end overlay) start)
                                           (apply #'append props)))))
                      (overlays-in start end)))
        (lambda (a b) (string< (prin1-to-string a) (prin1-to-string b)))))

(defun parity-probe--minor-modes ()
  (sort (seq-filter (lambda (mode) (and (boundp mode) (symbol-value mode)))
                    (copy-sequence (if (boundp 'local-minor-modes) local-minor-modes nil)))
        #'string<))

(defun parity-probe--variables (buffer)
  "The package's state, with BUFFER's local values: `parity-probe-variables'
that are bound, else every bound non-custom variable named with
`parity-probe-variable-prefix'."
  (with-current-buffer buffer
    (let (result)
      (if parity-probe-variables
          (dolist (symbol parity-probe-variables)
            (when (boundp symbol)
              (push (list symbol (parity-probe--plain (symbol-value symbol))) result)))
        (mapatoms
         (lambda (symbol)
           (let ((name (symbol-name symbol)))
             (when (and (boundp symbol) (string-prefix-p parity-probe-variable-prefix name)
                        (not (get symbol 'standard-value)) (not (keymapp (symbol-value symbol))))
               (push (list symbol (parity-probe--plain (symbol-value symbol))) result))))))
      (sort result (lambda (a b) (string< (car a) (car b)))))))

(defun parity-probe--format-line (format window)
  "FORMAT as WINDOW's mode or header line shows it, or the error it signals."
  (condition-case condition
      (parity-probe--scrub (substring-no-properties (format-mode-line format nil window)))
    (error (list 'error (parity-probe--plain condition)))))

(defun parity-probe--window (window)
  (let* ((buffer (window-buffer window))
         (start (window-start window))
         (end (with-current-buffer buffer (min (point-max) (+ start parity-probe-text-limit)))))
    (with-current-buffer buffer
      (list :buffer (buffer-name buffer)
            :start start :point (window-point window) :end (window-end window)
            ;; Widths only: Romacs's windows have no mode-line row (its status
            ;; bar is the mode line, DIVERGENCES.md VM2-STATUS-BAR), so their
            ;; heights and tops differ from GNU's by those rows.
            :body (list (window-body-width window))
            ;; Pixel edges, geometry by the harness's policy.  Not
            ;; `window-edges' or `window-left-column': Romacs gains their
            ;; frame-internal-border-width and window-left-column with
            ;; t3/daily-use-gui-fixes (0e58798).
            :edges (list (window-pixel-left window) (window-pixel-width window))
            :hscroll (window-hscroll window)
            :dedicated (window-dedicated-p window)
            :header (and header-line-format (parity-probe--format-line header-line-format window))
            :text (parity-probe--scrub (buffer-substring-no-properties start end))
            :runs (parity-probe--runs start end)
            :overlays (parity-probe--overlays start end)))))

(defun parity-probe--tree (window)
  "WINDOW's subtree: a buffer name, or (DIRECTION . CHILDREN) as `window-tree'."
  (if (window-live-p window)
      (buffer-name (window-buffer window))
    (let ((child (or (window-top-child window) (window-left-child window))) children)
      (while child
        (push (parity-probe--tree child) children)
        (setq child (window-next-sibling child)))
      (cons (if (window-top-child window) 'vertical 'horizontal) (nreverse children)))))

(defun parity-probe--messages ()
  (with-current-buffer (messages-buffer)
    (mapcar #'parity-probe--scrub
            (split-string (buffer-substring-no-properties (min parity-probe-messages-start (point-max)) (point-max))
                          "\n" t))))

(defun parity-probe-mark-messages ()
  "Start the *Messages* tail a snapshot returns here."
  (setq parity-probe-messages-start (with-current-buffer (messages-buffer) (point-max))))

(defun parity-probe-db (queries)
  "Rows for each SQL string in QUERIES, through gnosis's own connection."
  (and (bound-and-true-p gnosis-db) (sqlitep gnosis-db)
       (mapcar (lambda (query) (list query (parity-probe--plain (sqlite-select gnosis-db query)))) queries)))

(defun parity-probe-snapshot (&optional queries)
  "The observable state the harness compares."
  (let* ((window (selected-window))
         (buffer (window-buffer (if (minibufferp (window-buffer window)) (minibuffer-selected-window) window))))
    (list
     :selected-buffer (buffer-name (window-buffer window))
     :buffer (buffer-name buffer)
     :point (with-current-buffer buffer (point))
     :size (with-current-buffer buffer (buffer-size))
     :text-hash (with-current-buffer buffer (secure-hash 'sha1 (buffer-substring-no-properties (point-min) (point-max))))
     :narrowed (with-current-buffer buffer (buffer-narrowed-p))
     :major-mode (buffer-local-value 'major-mode buffer)
     :minor-modes (with-current-buffer buffer (parity-probe--minor-modes))
     :global-minor-modes (sort (seq-filter #'symbol-value (copy-sequence global-minor-modes)) #'string<)
     :read-only (buffer-local-value 'buffer-read-only buffer)
     :modified (buffer-modified-p buffer)
     :mode-line (with-current-buffer buffer
                  (parity-probe--format-line mode-line-format (get-buffer-window buffer)))
     :minibuffer (and (active-minibuffer-window)
                      (with-current-buffer (window-buffer (active-minibuffer-window))
                        (list (minibuffer-depth) (minibuffer-prompt) (minibuffer-contents-no-properties))))
     :echo (let ((message (current-message))) (and message (parity-probe--scrub (substring-no-properties message))))
     :messages (last (parity-probe--messages) 12)
     :buffers (seq-remove (lambda (name) (string-prefix-p " " name)) (mapcar #'buffer-name (buffer-list)))
     :tree (parity-probe--tree (frame-root-window))
     :windows (mapcar #'parity-probe--window (window-list nil 'nomini (frame-first-window)))
     ;; The frame's size: the status bar changes window bodies, not it.
     :frame (list (frame-width) (frame-height))
     :variables (parity-probe--variables buffer)
     :db (parity-probe-db queries))))

(defvar parity-probe--earlier-timers nil
  "The timers pending when the current step began (`parity-probe-step').")

(defun parity-probe-step ()
  "Note that a step begins: the one-shot timers pending now are not its work."
  (setq parity-probe--earlier-timers (copy-sequence timer-list))
  t)

(defun parity-probe-quiet (&optional horizon)
  "Whether no one-shot timer the step started is due within HORIZON
seconds (default 3).  Repeating timers (this probe's, cursor blinking,
autosave) never settle, and idle timers wait for the idleness the probe
preserves; a one-shot timer is work the step started (a deferred render,
a message to clear).  One pending when the step began is not: simple.el's
`undo-auto--boundary-timer' fires 10s after the first undoable change
since the last, and a package's heartbeat is re-armed on its own clock,
so waiting for them delayed whichever step came within HORIZON of them.
Timers are told apart by identity: one the step re-arms in place stays
ignored."
  (let ((limit (+ (float-time) (or horizon 3))))
    (not (seq-some (lambda (timer)
                     (and (not (timer--repeat-delay timer))
                          (not (memq timer parity-probe--earlier-timers))
                          (< (float-time (timer--time timer)) limit)))
                   timer-list))))

(defun parity-probe-ready (predicate)
  "A cheap answer to whether the step's PREDICATE holds, input is drained
and no one-shot timer is about to run."
  (list :time (float-time) :pending (input-pending-p)
        :ready (and (not (input-pending-p)) (parity-probe-quiet)
                    (with-current-buffer (window-buffer (selected-window)) (funcall predicate)) t)
        :buffer (buffer-name (window-buffer (selected-window)))
        :echo (let ((message (current-message))) (and message (parity-probe--scrub (substring-no-properties message))))))

(run-with-timer 0 0.025 #'parity-probe--poll)

(provide 'parity-probe)
;;; parity_probe.el ends here
