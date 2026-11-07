;;; perfbench-session.el --- perfbench.py's session rows, the same Lisp in every engine  -*- lexical-binding: t -*-

;;; Commentary:
;; A real command in a real session, for what a person feels: the time from
;; its key to its result, and how long the main VM kept the session from
;; running anything.  `perfbench-session-commands' names each command's
;; steps.  A repeating timer ticks for the whole session: a tick cannot run
;; while the main VM is busy, so the longest gap between ticks is how long
;; it blocked.  A command that only waits, as on a worker or a process,
;; leaves the ticks running however long its result takes.  A result counts
;; once it stands, the command has ended or reached a prompt, and it is on
;; the screen: the last redisplay is inside both figures.  A command with
;; :shown also reports when that first held, and one with :keys sends keys
;; while it runs and reports their median latency.

;;; Code:

(require 'cl-lib)

(defvar perfbench-session-repo nil "A private clone of the Emacs tree, with src/xdisp.c changed.")
(defvar perfbench-session-history nil "A clone of the Emacs tree at upstream master, with its history.")
(defvar perfbench-session-directory nil "The 5,000 files.")
(defvar perfbench-session-agenda nil "An Org file of scheduled items.")
(defvar perfbench-session-header nil "A large generated C header.")
(defvar perfbench-session-cycle nil "Themes to cycle through, each loaded before its row.")
(defvar perfbench-session-visits nil "Themes not yet loaded, one for each first visit.")
(defvar perfbench-session--theme nil "The theme the running row should end on.")
(defconst perfbench-session-commit "45b0f7699df36b3849edc3bdef37d4cb9e75b9ea")

(defun perfbench-session--file (name) (expand-file-name name perfbench-session-repo))

(defun perfbench-session--filled (buffer &optional text)
  "Whether BUFFER has text, TEXT among it if given, and no process is filling it."
  (let ((buffer (get-buffer buffer)))
    (and buffer (> (buffer-size buffer) 0) (not (get-buffer-process buffer))
         (or (null text)
             (with-current-buffer buffer
               (save-excursion (goto-char (point-min)) (search-forward text nil t)))))))

(defun perfbench-session--lines (buffer)
  (with-current-buffer buffer (count-lines (point-min) (point-max))))

(defun perfbench-session--xdisp ()
  (find-file (perfbench-session--file "src/xdisp.c"))
  (goto-char (point-min)))

(defun perfbench-session--kill (&rest buffers)
  (dolist (buffer buffers)
    (when (get-buffer buffer)
      (when (get-buffer-process buffer)
        (set-process-query-on-exit-flag (get-buffer-process buffer) nil))
      (with-current-buffer buffer (set-buffer-modified-p nil))
      (kill-buffer buffer))))

(defun perfbench-session--shown-p (name)
  "Whether the selected window shows the buffer NAME."
  (equal (buffer-name (window-buffer (selected-window))) name))

(defun perfbench-session--faces-p (name)
  "Whether the window shows NAME with all its visible text at its final faces:
every character `fontified', or a buffer jit-lock does not fontify.  The end
the last redisplay left costs nothing to read; updating it lays the window out,
so that comes second."
  (and (perfbench-session--shown-p name)
       (with-current-buffer name
         (let* ((window (selected-window))
                (start (window-start window))
                (unfontified (lambda (end) (text-property-any start (min end (point-max)) 'fontified nil))))
           (or (not (memq 'jit-lock-function fontification-functions))
               (and (not (funcall unfontified (window-end window)))
                    (not (funcall unfontified (window-end window t)))))))))

(defun perfbench-session--open (file name)
  "The steps of visiting FILE, whose buffer is NAME, to its final faces."
  (list :setup (lambda () (setq large-file-warning-threshold nil))
        :run (lambda () (find-file (funcall file)))
        :shown (lambda () (perfbench-session--shown-p name))
        :ready (lambda () (perfbench-session--faces-p name))
        :check (lambda () (buffer-size (get-buffer name)))
        :detail (lambda () (symbol-name (buffer-local-value 'major-mode (get-buffer name))))
        :cleanup (lambda () (perfbench-session--kill name))))

(defun perfbench-session--open-ts (file name)
  "As `perfbench-session--open', with C files in `c-ts-mode' in either engine."
  (let ((steps (perfbench-session--open file name)))
    (plist-put steps :setup
               (lambda ()
                 (setq large-file-warning-threshold nil)
                 (add-to-list 'major-mode-remap-alist '(c-mode . c-ts-mode))))
    (plist-put steps :cleanup
               (lambda ()
                 (perfbench-session--kill name)
                 (setq major-mode-remap-alist (delete '(c-mode . c-ts-mode) major-mode-remap-alist))))))

(defun perfbench-session--switch (theme load)
  "Make THEME the only enabled theme, first loading it when LOAD."
  (setq perfbench-session--theme theme)
  (mapc #'disable-theme custom-enabled-themes)
  (if load (load-theme theme t) (enable-theme theme)))

(defun perfbench-session--theme-steps (next load)
  "The steps of a switch to the theme NEXT returns, loading it when LOAD."
  (list :run (lambda () (perfbench-session--switch (funcall next) load))
        :ready (lambda () (eq (car custom-enabled-themes) perfbench-session--theme))
        :check (lambda () (if (equal custom-enabled-themes (list perfbench-session--theme)) 1 0))
        :cleanup #'ignore))

(defconst perfbench-session-log "*vc-change-log*")

(defconst perfbench-session-commands
  `((open-xdisp . ,(perfbench-session--open (lambda () (perfbench-session--file "src/xdisp.c")) "xdisp.c"))
    (open-header . ,(perfbench-session--open (lambda () perfbench-session-header) "generated.h"))
    (open-xdisp-ts . ,(perfbench-session--open-ts (lambda () (perfbench-session--file "src/xdisp.c")) "xdisp.c"))
    (open-header-ts . ,(perfbench-session--open-ts (lambda () perfbench-session-header) "generated.h"))
    (vc-history
     :setup ,(lambda () (require 'vc) (require 'vc-git) (require 'log-view))
     :run ,(lambda () (let ((default-directory (file-name-as-directory perfbench-session-history))
                            (vc-log-show-limit 0)
                            ;; The key runs from *scratch*, which has no backend of its own.
                            (vc-deduce-backend-nonvc-modes t))
                        (call-interactively #'vc-print-root-log)))
     :shown ,(lambda () (let ((buffer (get-buffer perfbench-session-log)))
                          (and buffer (> (buffer-size buffer) 0) (get-buffer-window buffer))))
     :keys ,(lambda () (let ((buffer (get-buffer perfbench-session-log)))
                         (and buffer (get-buffer-process buffer)
                              (eq (window-buffer (selected-window)) buffer))))
     :ready ,(lambda () (perfbench-session--filled perfbench-session-log))
     ;; Commits, not lines: a commit is the graph's `*', and connector lines
     ;; lie between them.  Plain text, so the count rests on no mode's regexp.
     :check ,(lambda () (with-current-buffer perfbench-session-log
                          (format "%d %s" (how-many "^[|/\\\\ ]*\\*" (point-min) (point-max))
                                  (substring (secure-hash 'sha256 (buffer-substring-no-properties (point-min) (point-max))) 0 12))))
     :cleanup ,(lambda () (perfbench-session--kill perfbench-session-log)))
    (theme-first . ,(perfbench-session--theme-steps (lambda () (pop perfbench-session-visits)) t))
    (theme-cycle
     :setup ,(lambda () (dolist (theme perfbench-session-cycle) (unless (custom-theme-p theme) (load-theme theme t t))))
     ,@(perfbench-session--theme-steps
        (lambda () (setq perfbench-session-cycle (append (cdr perfbench-session-cycle) (list (car perfbench-session-cycle))))
          (car perfbench-session-cycle))
        nil))
    (dired
     :run ,(lambda () (dired perfbench-session-directory))
     :ready ,(lambda () (derived-mode-p 'dired-mode))
     :check ,(lambda () (with-current-buffer (window-buffer (selected-window)) (count-lines (point-min) (point-max))))
     :cleanup ,(lambda () (kill-buffer (window-buffer (selected-window)))))
    (occur
     :setup ,#'perfbench-session--xdisp
     :run ,(lambda () (occur "glyph_row"))
     :ready ,(lambda () (perfbench-session--filled "*Occur*"))
     :check ,(lambda () (perfbench-session--lines "*Occur*"))
     :cleanup ,(lambda () (perfbench-session--kill "*Occur*" "xdisp.c")))
    (project-find-regexp
     :setup ,(lambda () (require 'project) (require 'xref)
               (setq project-list-file (expand-file-name "projects.eld" temporary-file-directory)))
     :run ,(lambda () (let ((default-directory (file-name-as-directory perfbench-session-repo)))
                        (project-find-regexp "\\<make_fixnum\\>")))
     :ready ,(lambda () (perfbench-session--filled "*xref*"))
     :check ,(lambda () (perfbench-session--lines "*xref*"))
     :cleanup ,(lambda () (perfbench-session--kill "*xref*")))
    (project-find-file
     :setup ,(lambda () (require 'project)
               (setq project-list-file (expand-file-name "projects.eld" temporary-file-directory)))
     :run ,(lambda () (let ((default-directory (file-name-as-directory perfbench-session-repo)))
                        (project-find-file)))
     :ready ,(lambda () (minibufferp (window-buffer (selected-window))))
     :check ,(lambda () 1)
     :cleanup ,#'ignore)
    (vc-log
     :setup ,(lambda () (require 'vc) (require 'vc-git))
     :run ,(lambda () (let ((default-directory (file-name-as-directory perfbench-session-repo)))
                        (vc-print-log-internal 'Git (list default-directory) perfbench-session-commit t 2000)))
     :ready ,(lambda () (perfbench-session--filled "*vc-change-log*"))
     :check ,(lambda () (perfbench-session--lines "*vc-change-log*"))
     :cleanup ,(lambda () (perfbench-session--kill "*vc-change-log*")))
    (vc-diff
     :setup ,(lambda () (require 'vc) (perfbench-session--xdisp))
     :run ,(lambda () (vc-diff))
     :ready ,(lambda () (perfbench-session--filled "*vc-diff*" "perfbench"))
     :check ,(lambda () (perfbench-session--lines "*vc-diff*"))
     :cleanup ,(lambda () (perfbench-session--kill "*vc-diff*" "xdisp.c")))
    (vc-dir
     :setup ,(lambda () (require 'vc-dir))
     :run ,(lambda () (vc-dir (file-name-as-directory perfbench-session-repo)))
     :ready ,(lambda () (let ((buffer (window-buffer (selected-window))))
                          (with-current-buffer buffer
                            (and (derived-mode-p 'vc-dir-mode) (not (vc-dir-busy))
                                 (save-excursion (goto-char (point-min)) (search-forward "xdisp.c" nil t))))))
     :check ,(lambda () 1)
     :cleanup ,(lambda () (perfbench-session--kill (window-buffer (selected-window)))))
    (shell
     :setup ,(lambda () (require 'shell) (setq shell-file-name "/bin/sh" explicit-shell-file-name "/bin/sh"))
     :run ,(lambda () (shell)
             (process-send-string (get-buffer-process "*shell*") "echo perfbench$((1+1))\n"))
     :ready ,(lambda () (let ((buffer (get-buffer "*shell*")))
                          (and buffer (with-current-buffer buffer
                                        (save-excursion (goto-char (point-min)) (search-forward "perfbench2" nil t))))))
     :check ,(lambda () 1)
     :cleanup ,(lambda () (perfbench-session--kill "*shell*")))
    (org-agenda
     :setup ,(lambda () (require 'org-agenda) (setq org-agenda-files (list perfbench-session-agenda)))
     :run ,(lambda () (org-agenda-list nil "2026-01-01" 'month))
     :ready ,(lambda () (perfbench-session--filled "*Org Agenda*"))
     :check ,(lambda () (perfbench-session--lines "*Org Agenda*"))
     :cleanup ,(lambda () (perfbench-session--kill "*Org Agenda*") (org-release-buffers org-agenda-new-buffers))))
  "Each session command: its :setup, untimed; :run, bound to F5 and timed
from that key; :ready, true once its result stands; :check, what it made,
compared across engines; :cleanup, untimed; :shown, true once it first shows
anything; :keys, true while keys should be sent into it; :detail, a fact the
row reports that may differ between engines.")

(defun perfbench-session-text ()
  "A fresh edit buffer of 600 lines, its point at the start."
  (perfbench-session--kill "perfbench-edit")
  (switch-to-buffer (get-buffer-create "perfbench-edit"))
  (dotimes (line 600) (insert (format "line %d of some ordinary prose text, about sixty bytes long\n" line)))
  (goto-char (point-min))
  (setq buffer-undo-list nil))

(defun perfbench-session-kills ()
  "The edit buffer with something to yank."
  (perfbench-session-text)
  (kill-new "line of some ordinary prose text, about sixty bytes long\n"))

(defun perfbench-session-undos ()
  "The edit buffer with forty changes to undo."
  (perfbench-session-text)
  (dotimes (_ 40) (insert "line\n") (undo-boundary)))

(defun perfbench-session-org ()
  "ORG-NEWS in Org mode, at its first heading."
  (perfbench-session--kill "ORG-NEWS")
  (find-file (perfbench-session--file "etc/ORG-NEWS"))
  (org-mode)
  (goto-char (point-min))
  (outline-next-heading))

(defun perfbench-session-visible ()
  "How many of the next 4,000 characters show, for what Org folding did."
  (let ((shown 0))
    (dotimes (offset (min 4000 (- (point-max) (point))))
      (unless (invisible-p (+ (point) offset)) (setq shown (1+ shown))))
    shown))

(defvar perfbench-session--state nil "The armed command's steps.")
(defvar perfbench-session--file nil "Where to write its result.")
(defvar perfbench-session--started nil "When its key came.")
(defvar perfbench-session--ended nil "Whether its command has ended.")
(defvar perfbench-session--first nil "Seconds until :shown first held.")
(defvar perfbench-session--key-sent nil "When the key in flight was sent.")
(defvar perfbench-session--key-ran nil "Whether the key in flight has run.")
(defvar perfbench-session--next-key 0 "When the next key may go.")
(defvar perfbench-session--latencies nil "Seconds each key took.")
(defvar perfbench-session--last nil "The last tick.")
(defvar perfbench-session-worst-gap 0.0 "The longest gap between ticks since the last reset.")

(defun perfbench-session--median (values)
  (when values (nth (/ (length values) 2) (sort (copy-sequence values) #'<))))

(defun perfbench-session--send-key (now)
  "Send a key while the command runs, once the last has run, 50 ms after it.
It takes from its sending to the first tick after its command ended."
  (when perfbench-session--key-ran
    (push (- now perfbench-session--key-sent) perfbench-session--latencies)
    (setq perfbench-session--key-sent nil perfbench-session--key-ran nil
          perfbench-session--next-key (+ now 0.05)))
  (let ((keys (plist-get perfbench-session--state :keys)))
    (when (and keys perfbench-session--first (not perfbench-session--key-sent) (not unread-command-events)
               (>= now perfbench-session--next-key) (funcall keys))
      (setq perfbench-session--key-sent now
            unread-command-events (list (if (zerop (% (length perfbench-session--latencies) 4)) ?p ?n))))))

(defun perfbench-session--finish ()
  "Record the armed command, its result on the screen: this redisplay counts."
  (redisplay t)
  (let ((now (float-time))
        (inhibit-message t))
    (setq perfbench-session-worst-gap (max perfbench-session-worst-gap (- now perfbench-session--last))
          perfbench-session--last now)
    (write-region (format "(%S %S %S %S %d)\n" (- now perfbench-session--started) perfbench-session-worst-gap
                          perfbench-session--first (perfbench-session--median perfbench-session--latencies)
                          (length perfbench-session--latencies))
                  nil perfbench-session--file nil 'silent))
  (setq perfbench-session--state nil perfbench-session--started nil)
  ;; After the harness stops counting, so the probe's polling is not counted.
  (run-with-timer 0.6 nil #'perfbench-gui-resume))

(defun perfbench-session--tick ()
  (let ((now (float-time)))
    (when perfbench-session--last
      (setq perfbench-session-worst-gap (max perfbench-session-worst-gap (- now perfbench-session--last))))
    (setq perfbench-session--last now)
    (when (and perfbench-session--state perfbench-session--started)
      (when (and (not perfbench-session--first) (plist-get perfbench-session--state :shown)
                 (ignore-errors (funcall (plist-get perfbench-session--state :shown))))
        (setq perfbench-session--first (- now perfbench-session--started)))
      (perfbench-session--send-key now)
      (when (and (or perfbench-session--ended (> (minibuffer-depth) 0))
                 (not perfbench-session--key-sent)
                 (ignore-errors (funcall (plist-get perfbench-session--state :ready))))
        (perfbench-session--finish)))))

(defun perfbench-session--begin ()
  (when (and perfbench-session--state (not perfbench-session--started))
    (setq perfbench-session--started (float-time)
          perfbench-session--ended nil
          perfbench-session--first nil
          perfbench-session--key-sent nil
          perfbench-session--key-ran nil
          perfbench-session--next-key 0
          perfbench-session--latencies nil
          perfbench-session--last perfbench-session--started
          perfbench-session-worst-gap 0.0)))

(defun perfbench-session--end ()
  (when perfbench-session--started (setq perfbench-session--ended t))
  (when perfbench-session--key-sent (setq perfbench-session--key-ran t)))

(defun perfbench-session-reset ()
  "Start a new longest gap."
  (setq perfbench-session--last (float-time) perfbench-session-worst-gap 0.0)
  t)

(defun perfbench-session-arm (name file)
  "Run NAME's setup and bind F5 to it; its result goes to FILE."
  (let ((steps (alist-get name perfbench-session-commands)))
    (when-let* ((setup (plist-get steps :setup))) (funcall setup))
    (global-set-key [f5] (lambda () (interactive) (funcall (plist-get steps :run))))
    (setq perfbench-session--state steps perfbench-session--file file perfbench-session--started nil))
  t)

(defun perfbench-session-finish (name)
  "NAME's check and detail, as a list, then its cleanup."
  (let* ((steps (alist-get name perfbench-session-commands))
         (check (funcall (plist-get steps :check)))
         (detail (when-let* ((detail (plist-get steps :detail))) (funcall detail))))
    (funcall (plist-get steps :cleanup))
    (perfbench-session--clear)
    (list check detail)))

(defun perfbench-session--clear ()
  "Disarm, and put the windows back, for the next row."
  (setq perfbench-session--state nil perfbench-session--started nil)
  (delete-other-windows)
  (switch-to-buffer "*scratch*"))

(defun perfbench-session-abort (name)
  "After a failed row: NAME's cleanup if it has one, whatever state it left."
  (ignore-errors (funcall (plist-get (alist-get name perfbench-session-commands) :cleanup)))
  (perfbench-session--clear)
  t)

(add-hook 'pre-command-hook #'perfbench-session--begin)
(add-hook 'post-command-hook #'perfbench-session--end)
(run-at-time 0 0.005 #'perfbench-session--tick)

;;; perfbench-session.el ends here
