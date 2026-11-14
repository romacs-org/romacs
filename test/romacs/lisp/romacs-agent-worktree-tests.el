;;; romacs-agent-worktree-tests.el --- A delegated task's own checkout -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-tools))

;; These tests start no supervisor or harness: the build's own (beside the
;; binary) are not needed to make a session.
(advice-add 'romacs-agent--check-companions :override #'ignore)

(defun romacs-agent-worktree-tests--git (directory &rest args)
  "Git ARGS's output in DIRECTORY, trimmed; an error when it fails."
  (with-temp-buffer
    (let ((default-directory (file-name-as-directory directory))
          (process-environment (append romacs-agent-git-variables process-environment)))
      (unless (zerop (apply #'call-process "git" nil t nil args))
        (error "git %S: %s" args (buffer-string)))
      (string-trim (buffer-string)))))

(defmacro romacs-agent-worktree-tests--with (&rest body)
  "BODY with `repo', a repository with one commit and a subdirectory sub,
and `romacs-agent-worktree-directory' a private directory, `home'."
  (declare (indent 0))
  `(let* ((root (make-temp-file "romacs-worktree-" t))
          (repo (expand-file-name "project" root))
          (home (expand-file-name "trees/" root))
          (romacs-agent-worktree-directory home))
     (unwind-protect
         (progn
           (make-directory (expand-file-name "sub" repo) t)
           (write-region "a\n" nil (expand-file-name "sub/a.txt" repo) nil 'silent)
           (romacs-agent-worktree-tests--git repo "init" "-q")
           (romacs-agent-worktree-tests--git repo "add" ".")
           (romacs-agent-worktree-tests--git repo "-c" "commit.gpgsign=false" "-c" "user.name=t" "-c" "user.email=t@t"
                                             "commit" "-q" "-m" "first")
           ,@body)
       (delete-directory root t))))

(defun romacs-agent-worktree-tests--made (cwd)
  "What `romacs-agent-worktree-create' gives for CWD, waited for."
  (let ((result nil) (end (+ (float-time) 30)))
    (romacs-agent-worktree-create cwd (lambda (made) (setq result made)))
    (while (and (not result) (< (float-time) end)) (accept-process-output nil 0.05))
    result))

(ert-deftest romacs-agent-worktree-at-head-on-a-new-branch ()
  (romacs-agent-worktree-tests--with
    ;; A caller's GIT_DIR pointing elsewhere does not reach git.
    (let* ((process-environment (cons "GIT_DIR=/nonexistent" process-environment))
           (made (romacs-agent-worktree-tests--made (expand-file-name "sub" repo)))
           (path (plist-get made :path)))
      (should-not (plist-get made :error))
      (should (string-prefix-p (expand-file-name "project/agent-" home) path))
      (should (string-match-p "\\`agent/[0-9a-f]\\{8\\}\\'" (plist-get made :branch)))
      (should (equal (plist-get made :cwd) (expand-file-name "sub" path)))
      (should (file-exists-p (expand-file-name "sub/a.txt" path)))
      (should (equal (romacs-agent-worktree-tests--git path "rev-parse" "HEAD")
                     (romacs-agent-worktree-tests--git repo "rev-parse" "HEAD")))
      (should (equal (plist-get made :base) (romacs-agent-worktree-tests--git repo "rev-parse" "HEAD")))
      (should (equal (romacs-agent-worktree-tests--git path "branch" "--show-current") (plist-get made :branch)))
      ;; The caller's checkout stays on its branch.
      (should-not (equal (romacs-agent-worktree-tests--git repo "branch" "--show-current") (plist-get made :branch))))))

(ert-deftest romacs-agent-worktree-of-a-worktree-is-named-by-its-repository ()
  ;; A caller in a worktree: the new one goes beside the repository's.
  (romacs-agent-worktree-tests--with
    (let* ((first (plist-get (romacs-agent-worktree-tests--made repo) :path))
           (second (plist-get (romacs-agent-worktree-tests--made first) :path)))
      (should (equal (file-name-directory second) (file-name-directory first))))))

(ert-deftest romacs-agent-worktree-names-taken-are-passed-over ()
  (romacs-agent-worktree-tests--with
    (let* ((trees (expand-file-name "project/" home))
           (ids (list "dup" "dir" "ref" "free"))
           (romacs-agent-worktree--next-id (lambda () (pop ids))))
      ;; A worktree, a directory and a branch with the first names.
      (romacs-agent-worktree-tests--git repo "worktree" "add" "-q" "-b" "agent/dup" (concat trees "agent-dup"))
      (make-directory (expand-file-name "agent-dir" trees) t)
      (write-region "mine\n" nil (expand-file-name "agent-dir/keep.txt" trees) nil 'silent)
      (romacs-agent-worktree-tests--git repo "branch" "agent/ref")
      (write-region "work\n" nil (expand-file-name "agent-dup/sub/a.txt" trees) nil 'silent)
      (let ((made (romacs-agent-worktree-tests--made repo)))
        (should (equal (plist-get made :path) (concat trees "agent-free")))
        (should (equal (plist-get made :branch) "agent/free")))
      ;; Each left as it was.
      (should (equal (with-temp-buffer (insert-file-contents (expand-file-name "agent-dup/sub/a.txt" trees)) (buffer-string))
                     "work\n"))
      (should (equal (romacs-agent-worktree-tests--git (concat trees "agent-dup") "branch" "--show-current") "agent/dup"))
      (should (equal (directory-files (expand-file-name "agent-dir" trees) nil "\\`[^.]") '("keep.txt")))
      (should (equal (romacs-agent-worktree-tests--git repo "rev-parse" "agent/ref")
                     (romacs-agent-worktree-tests--git repo "rev-parse" "HEAD"))))))

(ert-deftest romacs-agent-worktree-needs-a-repository ()
  (let ((plain (make-temp-file "romacs-worktree-plain-" t)))
    (unwind-protect
        (let ((made (romacs-agent-worktree-tests--made plain)))
          (should (string-match-p "is not in a git repository" (plist-get made :error)))
          (should (string-match-p (regexp-quote plain) (plist-get made :error))))
      (delete-directory plain t))))

(defmacro romacs-agent-worktree-tests--starting (&rest body)
  "BODY with `romacs-agent-start' recording each child it starts as (CWD
OPTIONS) in `started', newest first, and one instance configured."
  (declare (indent 0))
  `(let ((started nil) (romacs-agent-providers '(("claude"))))
     (cl-letf (((symbol-function 'romacs-agent-start)
                (lambda (cwd _prompt _title _provider options &rest _)
                  (push (list cwd options) started)
                  (romacs-agent--make :id "c" :harness 'claude :state 'running :cwd cwd :options options))))
       ,@body)))

(defun romacs-agent-worktree-tests--poll (deferred)
  "DEFERRED's value once it has one, or its error's message."
  (let ((end (+ (float-time) 30)) (result nil))
    (should (eq (car deferred) 'romacs-mcp--deferred))
    (condition-case err
        (while (and (not (setq result (funcall (cadr deferred)))) (< (float-time) end))
          (accept-process-output nil 0.05))
      (user-error (setq result (list (error-message-string err)))))
    (car result)))

(defun romacs-agent-worktree-tests--delegate (cwd &rest fields)
  "romacs_delegate_task's answer for an isolated task in CWD with FIELDS,
polled to its end, as (VALUE . STARTED): VALUE the record, or the error's
message; STARTED each (CWD OPTIONS) a child was started with."
  (romacs-agent-worktree-tests--starting
    (let ((deferred (romacs-agent-tools--delegate
                     (apply #'romacs-mcp--object "task" "edit it" "cwd" cwd "isolation" "worktree" fields))))
      ;; Nothing starts before the worktree is there.
      (should-not started)
      (cons (romacs-agent-worktree-tests--poll deferred) started))))

(ert-deftest romacs-agent-worktree-delegated-child-runs-there ()
  (romacs-agent-worktree-tests--with
    (pcase-let* ((`(,task . ,started) (romacs-agent-worktree-tests--delegate repo))
                 (path (gethash "worktree" task)) (branch (gethash "branch" task)))
      (should (= (length started) 1))
      (should (equal (car (car started)) path))
      (should (equal (plist-get (cadr (car started)) :worktree) path))
      (should (equal (plist-get (cadr (car started)) :branch) branch))
      (should (equal (romacs-agent-worktree-tests--git path "branch" "--show-current") branch))
      ;; Its record keeps them across a restart.
      (let ((kept (romacs-agent--options-of (romacs-agent--options-object (cadr (car started))))))
        (should (equal (plist-get kept :worktree) path))
        (should (equal (plist-get kept :branch) branch))))))

(ert-deftest romacs-agent-worktree-delegation-outside-a-repository-starts-nothing ()
  (let ((plain (make-temp-file "romacs-worktree-plain-" t)))
    (unwind-protect
        (pcase-let ((`(,message . ,started) (romacs-agent-worktree-tests--delegate plain)))
          (should (string-match-p "Isolation \"worktree\": .* is not in a git repository" message))
          (should-not started))
      (delete-directory plain t))))

(ert-deftest romacs-agent-worktree-description-says-who-merges ()
  (let ((text (romacs-agent-tools--delegate-description)))
    (should (string-match-p "isolation \"worktree\"" text))
    (should (string-match-p "you merge" text))
    (should (string-match-p "never removes" text))))

(ert-deftest romacs-agent-worktree-wait-counts-from-the-request ()
  ;; Its deadline is the relay's, from when the call came: the worktree's
  ;; making is part of the wait.
  (romacs-agent-worktree-tests--with
    (romacs-agent-worktree-tests--starting
      (let* ((romacs-agent-reply-timeout 120)
             (deferred (let ((romacs-mcp--active (list :arrived (- (float-time) 300))))
                         (romacs-agent-tools--delegate
                          (romacs-mcp--object "task" "edit it" "cwd" repo "isolation" "worktree" "mode" "wait"))))
             (task (romacs-agent-worktree-tests--poll deferred)))
        (should (eq (gethash "waitTimedOut" task) t))
        (should (equal (gethash "status" task) "running"))))))

(ert-deftest romacs-agent-worktree-child-starts-with-nobody-waiting ()
  ;; The call withdrawn (its poll never runs again): the worktree made
  ;; still gets its child, recorded with its request.
  (romacs-agent-worktree-tests--with
    (romacs-agent-worktree-tests--starting
      (romacs-agent-tools--delegate
       (romacs-mcp--object "task" "edit it" "cwd" repo "isolation" "worktree" "clientRequestId" "r1"))
      (let ((end (+ (float-time) 30)))
        (while (and (not started) (< (float-time) end)) (accept-process-output nil 0.05)))
      (should (= (length started) 1))
      (let ((options (cadr (car started))))
        (should (equal (plist-get options :request) "r1"))
        (should (file-directory-p (plist-get options :worktree)))
        (should (equal (car (car started)) (plist-get options :worktree))))
      (should (= (hash-table-count romacs-agent-tools--making) 0)))))

(ert-deftest romacs-agent-worktree-bad-provider-makes-none ()
  (romacs-agent-worktree-tests--with
    (romacs-agent-worktree-tests--starting
      (should-error (romacs-agent-tools--delegate
                     (romacs-mcp--object "task" "edit it" "cwd" repo "isolation" "worktree"
                                         "target" (romacs-mcp--object "provider" "nosuch")))
                    :type 'user-error)
      (accept-process-output nil 0.2)
      (should-not started)
      (should-not (file-exists-p home))
      (should (= (length (split-string (romacs-agent-worktree-tests--git repo "worktree" "list") "\n" t)) 1)))))

(ert-deftest romacs-agent-worktree-from-a-directory-not-checked-out ()
  ;; An untracked directory has no place in the new checkout: its root then.
  (romacs-agent-worktree-tests--with
    (make-directory (expand-file-name "scratch" repo))
    (let ((made (romacs-agent-worktree-tests--made (expand-file-name "scratch" repo))))
      (should (equal (plist-get made :cwd) (plist-get made :path))))))

(ert-deftest romacs-agent-worktree-reads-only-what-git-said ()
  (should-not (romacs-agent-worktree--repository "warning: something\n/r\n\n/r/.git\nabc"))
  (should (equal (plist-get (romacs-agent-worktree--repository
                             (concat "/x/romacs\nsub/\n/x/romacs/.git\n" (make-string 40 ?a)))
                            :repo)
                 "romacs")))

(ert-deftest romacs-agent-worktree-harnesses-get-no-git-pointers ()
  (dolist (name '("GIT_DIR" "GIT_WORK_TREE" "GIT_INDEX_FILE"))
    (should (member name (romacs-agent--dropped-variables)))))

(provide 'romacs-agent-worktree-tests)
;;; romacs-agent-worktree-tests.el ends here
