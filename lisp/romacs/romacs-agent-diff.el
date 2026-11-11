;;; romacs-agent-diff.el --- What an agent's turn changed  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Each turn of a live session is bracketed by checkpoints: its project's
;; working tree, untracked files included and ignored ones left out, written
;; as a git tree object through a private index.  The user's index, stash,
;; refs and files are never touched; the objects are loose, and git's
;; garbage collection takes them in its time.  A turn's diff is the diff of
;; its two trees, in `diff-mode': its hunks go to their source, and revert
;; (\\[universal-argument] \\[diff-apply-hunk]) as any diff's do.
;;
;; A turn is named by its message's request id, journaled with it.  Its
;; start is taken as the message is written, not when its record is read
;; back, so an editor busy meanwhile still sees the project as it was.  A
;; session's snapshots run one at a time.  The message is not held for its
;; snapshot: an edit the harness makes within the snapshot's own time
;; (a fraction of a second; a model takes seconds to its first edit)
;; counts as the turn before's.

;;; Code:

(require 'romacs-agent)
(require 'diff-mode)

(defcustom romacs-agent-checkpoints t
  "Whether a live session's turns are bracketed by checkpoints of its
project, for `romacs-agent-diff-turn'."
  :type 'boolean :group 'romacs-agent)

(defconst romacs-agent-diff--script
  "unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE
cd \"$1\" && root=$(git rev-parse --show-toplevel) && cd \"$root\" || exit 1
real=$(git rev-parse --path-format=absolute --git-path index)
cp -p \"$real\" \"$2\" 2>/dev/null || rm -f \"$2\"
GIT_INDEX_FILE=\"$2\" git add -A >/dev/null 2>&1 && tree=$(GIT_INDEX_FILE=\"$2\" git write-tree)
status=$?
rm -f \"$2\"
[ $status -eq 0 ] && printf '%s\\n%s\\n' \"$tree\" \"$root\""
  "A checkpoint of the project at $1, through the private index $2: its
tree and root, a line each.  The index starts as a copy of the user's,
only for its cached file stats, keeping its time: git trusts a cached stat
only when the file is older than the index, and a copy made now would
make an edit of the same size, just made, look unchanged.")

(defun romacs-agent-diff--file (session)
  (expand-file-name "checkpoints" (romacs-agent-dir session)))

(defun romacs-agent-diff--checkpoints (session)
  "SESSION's checkpoints, oldest first: each (TURN EDGE TREE ROOT), TURN
its message's request id and EDGE `start' or `end'."
  (let ((file (romacs-agent-diff--file session)))
    (when (file-readable-p file)
      (with-temp-buffer
        (insert-file-contents file)
        ;; In the order of the turns, whatever order the snapshots ended in.
        (sort (cl-loop for line in (split-string (buffer-string) "\n" t)
                       for (turn edge tree root) = (split-string line "\t")
                       when root collect (list (string-to-number turn) (intern edge) tree root))
              (lambda (a b) (if (= (car a) (car b)) (eq (nth 1 a) 'start) (< (car a) (car b)))))))))

(defvar romacs-agent-diff--queues (make-hash-table :test #'equal)
  "A session's id to its snapshots waiting while one runs, as (HEAD . LAST):
HEAD the jobs oldest first, LAST its last cons, so a job joins in constant
time.  One at a time, each sees the project after the one before.")

(defun romacs-agent-diff--snapshot (session turn edge &optional then)
  "Checkpoint SESSION's project as TURN's EDGE, in the background and
after its snapshots before, then call THEN, when given, with the tree and
root; nil when it is no git project or git fails.  Only a `start' or
`end' is kept."
  (let* ((id (romacs-agent-id session))
         (queued (gethash id romacs-agent-diff--queues :none))
         (job (list session turn edge then)))
    (if (eq queued :none)
        (progn (puthash id (cons nil nil) romacs-agent-diff--queues) (apply #'romacs-agent-diff--run job))
      (let ((cell (list job)))
        (if (car queued) (setcdr (cdr queued) cell) (setcar queued cell))
        (setcdr queued cell)))))

(defun romacs-agent-diff--settled-p (session)
  "Whether SESSION has no snapshot running or waiting."
  (eq (gethash (romacs-agent-id session) romacs-agent-diff--queues :none) :none))

(defun romacs-agent-diff--next (session)
  "SESSION's snapshot ended: run the next one waiting, if any."
  (let* ((id (romacs-agent-id session)) (queued (gethash id romacs-agent-diff--queues))
         (head (car queued)))
    (if head
        (progn (setcar queued (cdr head))
               (unless (cdr head) (setcdr queued nil))
               (apply #'romacs-agent-diff--run (car head)))
      (remhash id romacs-agent-diff--queues))))

(defun romacs-agent-diff--run (session turn edge then)
  "Take SESSION's snapshot now (`romacs-agent-diff--snapshot')."
  (let* ((directory (romacs-agent-dir session))
         (index (make-temp-name (expand-file-name "checkpoint-index-" directory)))
         (output (generate-new-buffer " *romacs-agent-checkpoint*")))
    (make-process
     :name "romacs-agent-checkpoint" :buffer output :noquery t :connection-type 'pipe
     :command (list "sh" "-c" romacs-agent-diff--script "sh" (expand-file-name (romacs-agent-cwd session)) index)
     :sentinel
     (lambda (process _)
       (unless (process-live-p process)
         (let ((lines (and (buffer-live-p output) (eq (process-exit-status process) 0)
                           (split-string (with-current-buffer output (buffer-string)) "\n" t))))
           (kill-buffer output)
           (when (and lines (memq edge '(start end)) (file-directory-p directory))
             (write-region (format "%d\t%s\t%s\t%s\n" turn edge (car lines) (cadr lines))
                           nil (romacs-agent-diff--file session) t 'silent))
           (unwind-protect (when then (funcall then (car lines) (cadr lines)))
             (romacs-agent-diff--next session))))))))

(defun romacs-agent-diff--live-p (session)
  "Whether SESSION's events now happen now: the registered session, its
journal's replay over."
  (and (eq (romacs-agent-get (romacs-agent-id session)) session) (romacs-agent-caught-up session)))

(defvar romacs-agent-diff--turns (make-hash-table :test #'equal)
  "A session's id to the request id of its last message read back: the
turn its end ends.")

(defun romacs-agent-diff-current (session)
  "The request id naming SESSION's last turn, or nil."
  (gethash (romacs-agent-id session) romacs-agent-diff--turns))

(defun romacs-agent-diff--on-input (session request)
  "A message was just written: checkpoint its turn's start, before the
harness has acted on it."
  (when (and romacs-agent-checkpoints (romacs-agent-cwd session) (romacs-agent-diff--live-p session))
    (romacs-agent-diff--snapshot session request 'start)))

(defun romacs-agent-diff--on-event (session event)
  "Note which turn runs; checkpoint a live turn's end."
  (pcase (plist-get event :type)
    ('input (when-let* ((request (plist-get event :request)))
              (puthash (romacs-agent-id session) request romacs-agent-diff--turns)))
    ;; A turn ends done, or cut by its harness's exit.
    ((or 'done (and 'exit (guard (romacs-agent-interrupted session))))
     (when-let* (((and romacs-agent-checkpoints (romacs-agent-cwd session) (romacs-agent-diff--live-p session)))
                 (request (romacs-agent-diff-current session)))
       (romacs-agent-diff--snapshot session request 'end)))))

(add-hook 'romacs-agent-input-functions #'romacs-agent-diff--on-input)
(add-hook 'romacs-agent-event-functions #'romacs-agent-diff--on-event)

(defun romacs-agent-diff--span (checkpoints turn)
  "TURN's (START END) checkpoints among CHECKPOINTS, END nil while none
follows it; TURN nil spans every turn."
  (let* ((starts (seq-filter (lambda (c) (eq (nth 1 c) 'start)) checkpoints))
         (start (if turn (seq-find (lambda (c) (= (car c) turn)) starts) (car starts)))
         (later (and start (seq-filter (lambda (c) (> (car c) (car start))) starts))))
    (when start
      (list start
            (if turn
                (or (seq-find (lambda (c) (and (= (car c) turn) (eq (nth 1 c) 'end))) checkpoints)
                    (car later))
              ;; Every turn: to the last one's end; while it runs, none.
              (let ((last (car (last checkpoints))))
                (and (eq (nth 1 last) 'end) last)))))))

(defun romacs-agent-diff--show (name root from to)
  "Show the diff of trees FROM and TO, of the project at ROOT, in buffer
NAME, in another window than this one."
  (let ((buffer (get-buffer-create name)))
    (with-current-buffer buffer
      (let ((inhibit-read-only t)) (erase-buffer))
      (setq default-directory (file-name-as-directory root))
      (diff-mode)
      (setq buffer-read-only t))
    (let ((default-directory (file-name-as-directory root)))
     (make-process
     :name "romacs-agent-diff" :buffer buffer :noquery t :connection-type 'pipe
     :command (list "git" "--no-pager" "diff" "--no-color" "--no-ext-diff" "-M" from to)
     :sentinel (lambda (process _)
                 (when (and (not (process-live-p process)) (buffer-live-p buffer))
                   (with-current-buffer buffer
                     (goto-char (point-min))
                     (when (= (point-min) (point-max))
                       (let ((inhibit-read-only t)) (insert "No change.\n")))
                     (set-buffer-modified-p nil))))
     :filter (lambda (process text)
               (when (buffer-live-p (process-buffer process))
                 (with-current-buffer (process-buffer process)
                   (let ((inhibit-read-only t))
                     (save-excursion (goto-char (point-max)) (insert text))))))))
    (pop-to-buffer buffer '((display-buffer-reuse-window display-buffer-pop-up-window) (inhibit-same-window . t)))))

(defun romacs-agent-diff-turn (session turn)
  "Show what SESSION's TURN (its message's request id) changed in its
project; TURN nil, what all its turns did.  A turn still running is
compared with the project now."
  (let* ((span (or (romacs-agent-diff--span (romacs-agent-diff--checkpoints session) turn)
                   (user-error (if turn "No checkpoint of this turn" "No checkpoints in this session"))))
         (start (car span)) (end (cadr span))
         (name (format "*agent diff: %s %s*" (romacs-agent-title session) (if turn (format "#%d" turn) "all"))))
    (if end
        (romacs-agent-diff--show name (nth 3 start) (nth 2 start) (nth 2 end))
      (romacs-agent-diff--snapshot
       session (car start) 'now
       (lambda (tree root)
         (if tree (romacs-agent-diff--show name root (nth 2 start) tree)
           (message "Could not read the project now")))))))

(provide 'romacs-agent-diff)
;;; romacs-agent-diff.el ends here
