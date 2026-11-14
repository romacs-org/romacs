;;; romacs-agent-diff-tests.el --- What an agent's turn changed -*- lexical-binding: t; -*-
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
  (require 'romacs-agent-ui))

(defun romacs-agent-diff-tests--git (dir &rest args)
  "ARGS run as git in DIR, its output."
  (with-temp-buffer
    (let ((default-directory (file-name-as-directory dir))
          (process-environment (append '("GIT_AUTHOR_NAME=t" "GIT_AUTHOR_EMAIL=t@t" "GIT_COMMITTER_NAME=t"
                                         "GIT_COMMITTER_EMAIL=t@t" "GIT_CONFIG_NOSYSTEM=1")
                                       process-environment)))
      (apply #'call-process "git" nil t nil "-c" "commit.gpgsign=false" args)
      (buffer-string))))

(defun romacs-agent-diff-tests--wait (session count)
  "Wait until SESSION has COUNT checkpoints."
  (let ((deadline (+ (float-time) 20)))
    (while (and (< (length (romacs-agent-diff--checkpoints session)) count) (< (float-time) deadline))
      (accept-process-output nil 0.05))
    (should (= (length (romacs-agent-diff--checkpoints session)) count))))

(defmacro romacs-agent-diff-tests--with (&rest body)
  "BODY with `repo', a git project of one commit, and `session', live in it."
  (declare (indent 0))
  `(let* ((repo (make-temp-file "romacs-diff-repo-" t))
          (dir (make-temp-file "romacs-diff-session-" t))
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (session (romacs-agent--make :id "s" :harness 'claude :dir dir :cwd repo :caught-up t
                                       :title "t")))
     (puthash "s" session romacs-agent--sessions)
     (unwind-protect
         (progn
           (romacs-agent-diff-tests--git repo "init" "-q")
           (write-region "one\n" nil (expand-file-name "a.txt" repo) nil 'silent)
           (romacs-agent-diff-tests--git repo "add" "a.txt")
           (romacs-agent-diff-tests--git repo "commit" "-q" "-m" "first")
           ,@body)
       (delete-directory repo t)
       (delete-directory dir t))))

(ert-deftest romacs-agent-diff-a-turn-s-change ()
  (romacs-agent-diff-tests--with
    (let ((status (romacs-agent-diff-tests--git repo "status" "--porcelain")))
      (romacs-agent-diff--on-input session 7)
      (romacs-agent-diff--on-event session '(:type input :seq 40 :request 7))
      (romacs-agent-diff-tests--wait session 1)
      ;; The turn edits a tracked file and makes a new one.
      (write-region "two\n" nil (expand-file-name "a.txt" repo) nil 'silent)
      (write-region "new\n" nil (expand-file-name "b.txt" repo) nil 'silent)
      (romacs-agent-diff--on-event session '(:type done :seq 49))
      (romacs-agent-diff-tests--wait session 2)
      (pcase-let ((`(,start ,end) (romacs-agent-diff--span (romacs-agent-diff--checkpoints session) 7)))
        (should (equal (nth 1 start) 'start))
        (should (equal (nth 1 end) 'end))
        (should (equal (file-truename (nth 3 start)) (file-truename repo)))
        (let ((diff (romacs-agent-diff-tests--git repo "diff" "--no-color" (nth 2 start) (nth 2 end))))
          (should (string-match-p "^-one$" diff))
          (should (string-match-p "^\\+two$" diff))
          (should (string-match-p "b\\.txt" diff))))
      ;; The user's index, stash and files are as they were.
      (should (equal (romacs-agent-diff-tests--git repo "diff" "--cached" "--name-only") ""))
      (should (equal (romacs-agent-diff-tests--git repo "stash" "list") ""))
      (should-not (equal (romacs-agent-diff-tests--git repo "status" "--porcelain") status))
      (should (string-match-p "\\?\\? b\\.txt" (romacs-agent-diff-tests--git repo "status" "--porcelain"))))))

(ert-deftest romacs-agent-diff-only-live-turns ()
  (romacs-agent-diff-tests--with
    ;; A replay, or a copy drawn again, takes no checkpoint.
    (setf (romacs-agent-caught-up session) nil)
    (romacs-agent-diff--on-input session 3)
    (romacs-agent-diff--on-event session '(:type input :request 3))
    (romacs-agent-diff--on-event session '(:type done))
    (let ((copy (romacs-agent--make :id "s" :harness 'claude :dir dir :cwd repo :caught-up t)))
      (romacs-agent-diff--on-input copy 3)
      (romacs-agent-diff--on-event copy '(:type done)))
    (accept-process-output nil 0.5)
    (should-not (romacs-agent-diff--checkpoints session))))

(ert-deftest romacs-agent-diff-spans ()
  (let ((checkpoints '((3 start "a" "/r") (3 end "b" "/r") (8 start "c" "/r") (12 start "d" "/r") (12 end "e" "/r"))))
    (should (equal (mapcar #'caddr (romacs-agent-diff--span checkpoints 3)) '("a" "b")))
    ;; No end of its own: the next turn's start.
    (should (equal (mapcar #'caddr (romacs-agent-diff--span checkpoints 8)) '("c" "d")))
    ;; Every turn: the first start to the last end; to now while one runs.
    (should (equal (mapcar #'caddr (romacs-agent-diff--span checkpoints nil)) '("a" "e")))
    (should (equal (mapcar #'caddr (romacs-agent-diff--span (append checkpoints '((14 start "f" "/r"))) nil)) '("a" nil)))
    (should-not (romacs-agent-diff--span checkpoints 99))))

(ert-deftest romacs-agent-diff-no-git-project ()
  (let* ((plain (make-temp-file "romacs-diff-plain-" t))
         (dir (make-temp-file "romacs-diff-session-" t))
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (session (romacs-agent--make :id "p" :harness 'claude :dir dir :cwd plain :caught-up t)))
    (puthash "p" session romacs-agent--sessions)
    (unwind-protect
        (let ((process-environment (cons (concat "GIT_CEILING_DIRECTORIES=" (file-name-directory (directory-file-name plain)))
                                         process-environment)))
          (romacs-agent-diff--on-input session 1)
          (accept-process-output nil 1)
          (should-not (romacs-agent-diff--checkpoints session))
          (should-error (romacs-agent-diff-turn session 1) :type 'user-error))
      (delete-directory plain t) (delete-directory dir t))))

(ert-deftest romacs-agent-diff-in-order-one-at-a-time ()
  (romacs-agent-diff-tests--with
    ;; Asked for together: run in turn, read back in the turns' order.
    (romacs-agent-diff--snapshot session 5 'end)
    (romacs-agent-diff--snapshot session 3 'start)
    (romacs-agent-diff--snapshot session 5 'start)
    (should (= (length (car (gethash "s" romacs-agent-diff--queues))) 2))
    (romacs-agent-diff-tests--wait session 3)
    (should (equal (mapcar (lambda (c) (list (car c) (nth 1 c))) (romacs-agent-diff--checkpoints session))
                   '((3 start) (5 start) (5 end))))
    (should (eq (gethash "s" romacs-agent-diff--queues :gone) :gone))))

(ert-deftest romacs-agent-diff-runs-in-the-project ()
  (romacs-agent-diff-tests--with
    (romacs-agent-diff--on-input session 2)
    (romacs-agent-diff--on-event session '(:type input :request 2))
    (romacs-agent-diff-tests--wait session 1)
    (write-region "two\n" nil (expand-file-name "a.txt" repo) nil 'silent)
    (romacs-agent-diff--on-event session '(:type done))
    (romacs-agent-diff-tests--wait session 2)
    ;; Called from elsewhere, it still reads the project's trees.
    (let ((default-directory temporary-file-directory))
      (save-window-excursion
        (romacs-agent-diff-turn session 2)
        (let ((deadline (+ (float-time) 10)))
          (while (and (get-buffer-process (current-buffer)) (< (float-time) deadline))
            (accept-process-output nil 0.05)))
        (should (derived-mode-p 'diff-mode))
        (should (string-match-p "^\\+two$" (buffer-string)))
        (kill-buffer)))))

(ert-deftest romacs-agent-diff-a-busy-editor-still-sees-the-turn ()
  (romacs-agent-diff-tests--with
    ;; Written: the start is taken now.  The harness edits and ends while
    ;; the editor is busy; its records are read back afterwards, together.
    (romacs-agent-diff--on-input session 4)
    (romacs-agent-diff-tests--wait session 1)
    (write-region "two\n" nil (expand-file-name "a.txt" repo) nil 'silent)
    (romacs-agent-diff--on-event session '(:type input :request 4))
    (romacs-agent-diff--on-event session '(:type done))
    (romacs-agent-diff-tests--wait session 2)
    (pcase-let ((`(,start ,end) (romacs-agent-diff--span (romacs-agent-diff--checkpoints session) 4)))
      (should (string-match-p "^\\+two$" (romacs-agent-diff-tests--git repo "diff" (nth 2 start) (nth 2 end)))))))

(ert-deftest romacs-agent-diff-the-journal-names-the-turn ()
  ;; A message's record keeps its request id, the harness's line does not;
  ;; read back, its input event carries the id.
  (let* ((written nil) (heard nil) (raised nil)
         (session (romacs-agent--make :id "j" :harness 'claude :tools (make-hash-table :test #'equal)
                                      :streamed (make-hash-table :test #'equal)))
         (romacs-agent-input-functions (list (lambda (_s id) (push id heard)))))
    (cl-letf (((symbol-function 'romacs-agent--line)
               (lambda (_s object &optional journal) (push (list object journal) written))))
      (romacs-agent--input session "hi"))
    (pcase-let ((`(,line ,record) (car written)))
      (should heard)
      (should-not (gethash "romacsRequest" line))
      (should (equal (gethash "romacsRequest" record) (car heard)))
      (cl-letf (((symbol-function 'romacs-agent--raise) (lambda (_s event) (push event raised))))
        (romacs-agent--record session (romacs-mcp--object "seq" 1 "in" record)))
      (should (equal (plist-get (cl-find 'input raised :key (lambda (e) (plist-get e :type))) :request)
                     (car heard))))))

(provide 'romacs-agent-diff-tests)
;;; romacs-agent-diff-tests.el ends here
