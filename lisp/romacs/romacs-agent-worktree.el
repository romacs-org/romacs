;;; romacs-agent-worktree.el --- A delegated task's own checkout  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A task delegated with isolation "worktree" runs in a git worktree of its
;; own: one writer per checkout, so children working at once never share
;; one.  The worktree is a new branch agent/ID at the caller's HEAD, under
;; `romacs-agent-worktree-directory'.  It is made by git processes, never
;; on the command loop's time: a large repository's checkout takes seconds.
;;
;; Romacs never removes one: the child commits there, and the caller (or
;; the user) merges the branch and removes the worktree.  An existing
;; worktree, branch or directory is never reused or touched: a name taken
;; is passed over.

;;; Code:

(require 'cl-lib)

;; romacs-agent.el's, which loads this through romacs-agent-tools.el.
(defvar romacs-agent-git-variables)

(defcustom romacs-agent-worktree-directory "~/Worktrees/"
  "Where delegated tasks' worktrees go: DIRECTORY/REPO/agent-ID, REPO
the name of the directory holding the repository (its main checkout's,
for a caller in a worktree), ID the branch agent/ID's."
  :type 'directory :group 'romacs-agent)

(defvar romacs-agent-worktree--next-id
  (lambda () (format "%08x" (random #x100000000)))
  "A function returning a new worktree's candidate id.")

(defun romacs-agent-worktree--git (directory args then fail)
  "Run git ARGS in DIRECTORY; call THEN with its exit status and output
\(its standard error too, when it failed).  An error THEN signals is
FAIL's, with its message: the chain it continues always ends."
  (let* ((default-directory (file-name-as-directory directory))
         (process-environment (append romacs-agent-git-variables process-environment))
         (output (generate-new-buffer " *romacs-agent-worktree*"))
         (errors (make-pipe-process :name "romacs-agent-worktree-stderr" :noquery t
                                    :buffer (generate-new-buffer " *romacs-agent-worktree-stderr*"))))
    (make-process
     :name "romacs-agent-worktree" :buffer output :stderr errors :noquery t :connection-type 'pipe
     :command (append (list "git" "--no-pager") args)
     :sentinel (lambda (process _)
                 (unless (process-live-p process)
                   (let ((status (process-exit-status process))
                         (text (with-current-buffer output (buffer-string)))
                         (said (with-current-buffer (process-buffer errors) (buffer-string))))
                     (kill-buffer output)
                     (kill-buffer (process-buffer errors))
                     (delete-process errors)
                     (condition-case err
                         (funcall then status (string-trim (if (= status 0) text (concat said text))))
                       (error (funcall fail (error-message-string err))))))))))

(defun romacs-agent-worktree--repository (text)
  "`git rev-parse --show-toplevel --show-prefix --git-common-dir HEAD''s
TEXT as (:top TOP :prefix PREFIX :repo NAME :base COMMIT), or nil when
it reads otherwise."
  (pcase (split-string text "\n")
    (`(,top ,prefix ,common ,base)
     (when (string-match-p "\\`[0-9a-f]\\{40,64\\}\\'" base)
       (let ((common (directory-file-name common)))
         (list :top top :prefix prefix :base base
               :repo (file-name-nondirectory
                      (directory-file-name (if (equal (file-name-nondirectory common) ".git")
                                               (file-name-directory common)
                                             common)))))))))

(defun romacs-agent-worktree--taken (porcelain refs)
  "The paths and branches git's PORCELAIN worktree list and REFS (branch
names, one a line) hold, as (PATHS . BRANCHES)."
  (let ((paths nil) (branches (split-string refs "\n" t)))
    (dolist (line (split-string porcelain "\n" t))
      (cond ((string-prefix-p "worktree " line)
             (push (directory-file-name (expand-file-name (substring line 9))) paths))
            ((string-prefix-p "branch refs/heads/" line) (push (substring line 18) branches))))
    (cons paths branches)))

(defun romacs-agent-worktree--place (repo taken)
  "A new worktree of REPO: (PATH . BRANCH) none of TAKEN (PATHS .
BRANCHES) holds and no file is at.  Nil after a few tries."
  (cl-loop repeat 16
           for id = (funcall romacs-agent-worktree--next-id)
           for path = (directory-file-name
                       (expand-file-name (concat "agent-" id)
                                         (expand-file-name repo romacs-agent-worktree-directory)))
           for branch = (concat "agent/" id)
           unless (or (member path (car taken)) (member branch (cdr taken))
                      (file-exists-p path) (file-symlink-p path))
           return (cons path branch)))

(defun romacs-agent-worktree-create (cwd then)
  "Make a worktree of CWD's repository at its HEAD, on a new branch, in
the background; call THEN once with (:path PATH :branch BRANCH :base
COMMIT :cwd DIRECTORY), DIRECTORY CWD's place in it (its root, when CWD
is not in the commit), or (:error MESSAGE)."
  (let* ((answered nil)
         (finish (lambda (result) (unless answered (setq answered t) (funcall then result))))
         (fail (lambda (format &rest args) (funcall finish (list :error (apply #'format format args)))))
         (broken (lambda (why) (funcall fail "Isolation \"worktree\": %s" why))))
    (if (not (file-directory-p cwd))
        (funcall fail "Isolation \"worktree\": %s is no directory" cwd)
     ;; Its first process may not start (no git): that fails the call too.
     (condition-case err
      (romacs-agent-worktree--git
       cwd '("rev-parse" "--path-format=absolute" "--show-toplevel" "--show-prefix" "--git-common-dir" "HEAD")
       (lambda (status text)
         (let ((repository (and (= status 0) (romacs-agent-worktree--repository text))))
           (if (not repository)
               (funcall fail "Isolation \"worktree\": %s is not in a git repository with a commit (%s)" cwd text)
             (romacs-agent-worktree--git
              cwd '("worktree" "list" "--porcelain")
              (lambda (status porcelain)
                (if (/= status 0) (funcall fail "git worktree list failed: %s" porcelain)
                  (romacs-agent-worktree--git
                   cwd '("for-each-ref" "--format=%(refname:short)" "refs/heads/")
                   (lambda (status refs)
                     (let ((place (and (= status 0)
                                       (romacs-agent-worktree--place (plist-get repository :repo)
                                                                     (romacs-agent-worktree--taken porcelain refs)))))
                       (if (not place) (funcall fail "No free worktree name under %s" romacs-agent-worktree-directory)
                         (romacs-agent-worktree--add cwd repository place finish broken))))
                   broken)))
              broken))))
       broken)
      (error (funcall broken (error-message-string err)))))))

(defun romacs-agent-worktree--add (cwd repository place then fail)
  "Add the worktree PLACE (PATH . BRANCH) of REPOSITORY, for THEN."
  (romacs-agent-worktree--git
   cwd (list "worktree" "add" "-b" (cdr place) (car place) (plist-get repository :base))
   (lambda (status text)
     (funcall then (if (/= status 0)
                       (list :error (format "git worktree add %s failed: %s" (car place) text))
                     (let ((within (expand-file-name (plist-get repository :prefix) (car place))))
                       (list :path (car place) :branch (cdr place) :base (plist-get repository :base)
                             ;; An untracked or ignored directory is not checked out.
                             :cwd (directory-file-name (if (file-directory-p within) within (car place))))))))
   fail))

(provide 'romacs-agent-worktree)
;;; romacs-agent-worktree.el ends here
