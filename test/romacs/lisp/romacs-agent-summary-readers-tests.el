;;; romacs-agent-summary-readers-tests.el --- Summary-backed agent views -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

(require 'ert)
(require 'cl-lib)
(let ((macroexp-inhibit-compiler-macros t))
  (let ((load-path (cons (file-name-directory load-file-name) load-path)))
    (require 'romacs-agent-test-stored))
  (require 'romacs-agent-summary)
  (require 'romacs-agent-board)
  (require 'romacs-agent-menu)
  (require 'romacs-agent-remote)
  (require 'romacs-agent-switch)
  (require 'romacs-agent-watch))

(defconst romacs-agent-summary-readers--fixtures
  '((idle :last-time 100)
    (working :state running :activity "Read" :turn-start 90 :last-time 99)
    (approval :state waiting-approval :driver-state (:asks ("approve")) :last-time 98)
    (question :state waiting-approval :driver-state (:asks ("choose") :questions ("choose")) :last-time 97)
    (workers :children-rows ((:key working :title "Native worker" :subagent t)) :last-time 96)
    (watching :last-time 95)
    (failed :failed t :done-seq 9 :last-time 94)
    (cancelled :cancelled t :failed t :done-seq 9 :last-time 93)
    (parent :last-time 92)
    (child :options (:parent "parent") :done-seq 5 :last-time 91)
    (stored :last-time 89)
    (selection :options (:model "claude-selected") :last-time 88)))

(defmacro romacs-agent-summary-readers--with-fixtures (&rest body)
  "Run BODY with private loaded, stored and remote sessions."
  (declare (indent 0))
  `(romacs-agent-test-with-store
     (let ((romacs-agent--sessions (make-hash-table :test #'equal))
           (romacs-agent--children (make-hash-table :test #'equal))
           (romacs-agent-ui--unloaded-waiting (make-hash-table :test #'equal))
           (romacs-agent-watch--watches (make-hash-table :test #'equal))
           (romacs-agent-writer-queued nil)
           (romacs-agent-board-source romacs-agent-log-local)
           (romacs-agent-glyphs nil))
       (cl-letf (((symbol-function 'romacs-agent-history-items) #'ignore)
                 ((symbol-function 'float-time) (lambda (&optional _) 110)))
         (romacs-agent-summary-readers--install)
         ,@body))))

(defun romacs-agent-summary-readers--install ()
  (make-directory (romacs-agent--state-root) t)
  (pcase-dolist (`(,name . ,fields) romacs-agent-summary-readers--fixtures)
    (let* ((id (symbol-name name))
           (session (apply #'romacs-agent--make
                           (append fields (list :id id :title id :harness 'claude :provider "work"
                                                :model "claude-reported" :cwd "/p/one/" :state 'idle
                                                :dir (expand-file-name id (romacs-agent--state-root)))))))
      (make-directory (romacs-agent-dir session))
      (if (eq name 'stored)
          (should (romacs-agent-store--put session nil))
        (puthash id session romacs-agent--sessions)
        (romacs-agent--index session))
      (when-let* ((request (car (romacs-agent-asks session))))
        (puthash request t (romacs-agent-approvals session)))))
  (puthash "watching" '((:unit "fixture.service")) romacs-agent-watch--watches)
  (romacs-agent-ui--summarize-children (gethash "parent" romacs-agent--sessions))
  (setf (romacs-agent-driver-state (gethash "selection" romacs-agent--sessions))
        '(:reported-effort ("claude-reported" . "high"))))

(defun romacs-agent-summary-readers--remote ()
  (let* ((client (romacs-agent-client--make))
         (source (romacs-agent-remote--make :client client))
         (row (romacs-agent-rpc-object
               "id" "remote" "title" "remote" "state" "running" "key" "working"
               "harness" "codex" "provider" "remote-account" "model" "codex-remote"
               "activity" "Edit" "turn_start" 95 "last_time" 100 "cwd" "/remote/project/")))
    (puthash "remote" row (romacs-agent-client-rows client))
    (cons source (romacs-agent-remote--mirror source row))))

(cl-defun romacs-agent-summary-readers--board (&optional folded (project "/p/one"))
  (with-temp-buffer
    (romacs-agent-board-mode)
    (setq romacs-agent-board--project project)
    (setq romacs-agent-board--folds (make-hash-table :test #'equal))
    (puthash "parent" (if folded 'closed 'open) romacs-agent-board--folds)
    (romacs-agent-board--render (current-buffer) 100)
    (mapcar
     (lambda (row)
       (let ((id (plist-get row :id)))
         (goto-char (romacs-agent-board--find id))
         (list id (plist-get row :key)
               (string-trim-right (buffer-substring-no-properties
                                   (line-beginning-position) (line-end-position))))))
     (romacs-agent-board--shown-rows))))

(defun romacs-agent-summary-readers--labels ()
  (mapcar (lambda (label) (substring-no-properties (funcall label)))
          '(romacs-agent-menu--title romacs-agent-menu--model-label
            romacs-agent-menu--effort-label romacs-agent-menu--plan-label)))

(defun romacs-agent-summary-readers--annotation ()
  (let ((chat (current-buffer)) annotation)
    (cl-letf (((symbol-function 'buffer-list) (lambda (&optional _) (list chat)))
              ((symbol-function 'switch-to-buffer) #'ignore)
              ((symbol-function 'completing-read)
               (lambda (_prompt collection &rest _)
                 (let ((annotate (completion-metadata-get (completion-metadata "" collection nil)
                                                          'annotation-function)))
                   (setq annotation (funcall annotate (buffer-name chat))))
                 (buffer-name chat))))
      (romacs-agent-switch))
    (substring-no-properties annotation)))

(defun romacs-agent-summary-readers--views (session source read)
  (with-temp-buffer
    (romacs-agent-log-mode)
    (setq romacs-agent-log--source source)
    (romacs-agent-log--set-session session)
    (funcall read)))

(defconst romacs-agent-summary-readers--golden-board
  '(("/p/one" nil "▌ one  1 needs approval · 1 asks you · 1 failed · 1 working · 1 waiting on its workers · 1 waiting …")
    ("approval" approval "  Needs ap… approval                                                         claude · reported  now")
    ("question" question "  Asks you  question                                                         claude · reported  now")
    ("failed" failed "  Failed    failed                                                           claude · reported  now")
    ("working" working "  Working   working                                        Read              claude · reported  20s")
    ("workers" waiting "  Waiting … workers                                                          claude · reported  now")
    ("watching" watching "  Waiting … watching                                                         claude · reported  now")
    ("idle" idle "            idle                                                             claude · reported  now")
    ("parent" idle "˅           parent                                                           claude · reported  now")
    ("child" completed "    Completed child                                                          claude · reported  now")
    ("stored" idle "            stored                                                           claude · reported  now")
    ("selection" idle "            selection                                                        claude · reported  now")
    ("cancelled" ended "  Ended     cancelled                                                        claude · reported  now")))
(defconst romacs-agent-summary-readers--golden-folded
  '(("/p/one" nil "▌ one  1 needs approval · 1 asks you · 1 failed · 1 working · 1 waiting on its workers · 1 waiting …")
    ("approval" approval "  Needs ap… approval                                                         claude · reported  now")
    ("question" question "  Asks you  question                                                         claude · reported  now")
    ("failed" failed "  Failed    failed                                                           claude · reported  now")
    ("working" working "  Working   working                                        Read              claude · reported  20s")
    ("workers" waiting "  Waiting … workers                                                          claude · reported  now")
    ("watching" watching "  Waiting … watching                                                         claude · reported  now")
    ("idle" idle "            idle                                                             claude · reported  now")
    ("parent" idle "›           parent                                         1 completed       claude · reported  now")
    ("stored" idle "            stored                                                           claude · reported  now")
    ("selection" idle "            selection                                                        claude · reported  now")
    ("cancelled" ended "  Ended     cancelled                                                        claude · reported  now")))
(defconst romacs-agent-summary-readers--golden-remote
  '(("/remote/project" nil "▌ project  /remote/project  1 working")
    ("remote" working "  Working   remote                                         Edit              codex · remote     15s")))
(defconst romacs-agent-summary-readers--golden-annotations
  '("  Idle claude · reported one"
    "  Working claude · reported one"
    "  Needs approval claude · reported one"
    "  Asks you claude · reported one"
    "  Waiting on its workers claude · reported one"
    "  Waiting on job: fixture.service claude · reported one"
    "  Failed claude · reported one"
    "  Ended claude · reported one"
    "  Idle claude · reported one"
    "  Completed claude · reported one"
    "  Idle claude · reported one"
    "  Idle claude · reported one"
    "  Working codex · remote project"))
(defconst romacs-agent-summary-readers--golden-labels
  '(("Agent · idle" "Model: reported" "Effort: default" "Plan mode: off")
    ("Agent · working" "Model: reported" "Effort: default" "Plan mode: off")
    ("Agent · approval" "Model: reported" "Effort: default" "Plan mode: off")
    ("Agent · question" "Model: reported" "Effort: default" "Plan mode: off")
    ("Agent · workers" "Model: reported" "Effort: default" "Plan mode: off")
    ("Agent · watching" "Model: reported" "Effort: default" "Plan mode: off")
    ("Agent · failed" "Model: reported" "Effort: default" "Plan mode: off")
    ("Agent · cancelled" "Model: reported" "Effort: default" "Plan mode: off")
    ("Agent · parent" "Model: reported" "Effort: default" "Plan mode: off")
    ("Agent · child" "Model: reported" "Effort: default" "Plan mode: off")
    ("Agent · stored" "Model: reported" "Effort: default" "Plan mode: off")
    ("Agent · selection" "Model: selected" "Effort: high (default)" "Plan mode: off")
    ("Agent · remote" "Model: remote" "Effort: default" "Plan mode: off")))

(defun romacs-agent-summary-readers--each-view (read)
  (let ((remote (romacs-agent-summary-readers--remote)))
    (append
     (mapcar (lambda (fixture)
               (let ((id (symbol-name (car fixture))))
                 (romacs-agent-summary-readers--views
                  (romacs-agent-get id t) romacs-agent-log-local read)))
             romacs-agent-summary-readers--fixtures)
     (list (romacs-agent-summary-readers--views (cdr remote) (car remote) read)))))

(ert-deftest romacs-agent-summary-readers-golden-board ()
  (romacs-agent-summary-readers--with-fixtures
    (should (equal (romacs-agent-summary-readers--board) romacs-agent-summary-readers--golden-board))
    (should (equal (romacs-agent-summary-readers--board t) romacs-agent-summary-readers--golden-folded))
    (let ((romacs-agent-board-source (car (romacs-agent-summary-readers--remote))))
      (should (equal (romacs-agent-summary-readers--board nil "/remote/project")
                     romacs-agent-summary-readers--golden-remote)))
    (should-not (gethash "stored" romacs-agent--sessions))))

(ert-deftest romacs-agent-summary-readers-golden-labels ()
  (romacs-agent-summary-readers--with-fixtures
    (should (equal (romacs-agent-summary-readers--each-view #'romacs-agent-summary-readers--labels)
                   romacs-agent-summary-readers--golden-labels))))

(ert-deftest romacs-agent-summary-readers-golden-switcher ()
  (romacs-agent-summary-readers--with-fixtures
    (should (equal (romacs-agent-summary-readers--each-view #'romacs-agent-summary-readers--annotation)
                   romacs-agent-summary-readers--golden-annotations))))

(define-error 'romacs-agent-summary-readers-record-read "Reader touched the session record")

(defvar romacs-agent-summary-readers--inside-summary nil)

(defun romacs-agent-summary-readers--trapped-functions ()
  (append '(romacs-agent-get romacs-agent-loaded)
          (mapcar (lambda (slot) (intern (format "romacs-agent-%s" slot)))
                  (romacs-agent--slot-names 'romacs-agent))))

(defun romacs-agent-summary-readers--with-trap (read)
  (let ((registry romacs-agent--sessions)
        (summary (symbol-function 'romacs-agent-summary))
        (originals (mapcar (lambda (name) (cons name (symbol-function name)))
                           (romacs-agent-summary-readers--trapped-functions))))
    (unwind-protect
        (progn
          (pcase-dolist (`(,name . ,original) originals)
            (fset name (lambda (&rest args)
                         (unless romacs-agent-summary-readers--inside-summary
                           (signal 'romacs-agent-summary-readers-record-read (list name)))
                         (apply original args))))
          (cl-letf (((symbol-function 'romacs-agent-summary)
                     (lambda (&rest args)
                       (let ((romacs-agent--sessions registry)
                             (romacs-agent-summary-readers--inside-summary t))
                         (apply summary args)))))
            (makunbound 'romacs-agent--sessions)
            (funcall read)))
      (set 'romacs-agent--sessions registry)
      (pcase-dolist (`(,name . ,original) originals)
        (fset name original)))))

(ert-deftest romacs-agent-summary-readers-trap-catches-both-boundaries ()
  (romacs-agent-summary-readers--with-fixtures
    (let ((session (gethash "idle" romacs-agent--sessions)))
      (should-error
       (romacs-agent-summary-readers--with-trap
        (lambda () romacs-agent--sessions))
       :type 'void-variable)
      (should (memq 'romacs-agent-title (romacs-agent-summary-readers--trapped-functions)))
      (should-error
       (romacs-agent-summary-readers--with-trap
        (lambda () (funcall (symbol-function 'romacs-agent-title) session)))
       :type 'romacs-agent-summary-readers-record-read)
      (should-error
       (romacs-agent-summary-readers--with-trap
        (lambda ()
          (let ((macroexp-inhibit-compiler-macros t))
            (eval (list 'romacs-agent-title session) t))))
       :type 'romacs-agent-summary-readers-record-read)
      (should (equal (romacs-agent-title session) "idle"))
      (should (eq (gethash "idle" romacs-agent--sessions) session)))))

(ert-deftest romacs-agent-summary-readers-trap-board ()
  (romacs-agent-summary-readers--with-fixtures
    (should (equal (romacs-agent-summary-readers--with-trap #'romacs-agent-summary-readers--board)
                   romacs-agent-summary-readers--golden-board))
    (should (equal (romacs-agent-summary-readers--with-trap
                    (lambda () (romacs-agent-summary-readers--board t)))
                   romacs-agent-summary-readers--golden-folded))
    (let ((romacs-agent-board-source (car (romacs-agent-summary-readers--remote))))
      (should (equal (romacs-agent-summary-readers--with-trap
                      (lambda () (romacs-agent-summary-readers--board nil "/remote/project")))
                     romacs-agent-summary-readers--golden-remote)))))

(defun romacs-agent-summary-readers--check-trapped-views (read expected)
  (should (equal (romacs-agent-summary-readers--each-view
                  (lambda () (romacs-agent-summary-readers--with-trap read)))
                 expected)))

(ert-deftest romacs-agent-summary-readers-trap-menu ()
  (romacs-agent-summary-readers--with-fixtures
    (romacs-agent-summary-readers--check-trapped-views #'romacs-agent-summary-readers--labels
                                                      romacs-agent-summary-readers--golden-labels)
    (let ((parent (gethash "parent" romacs-agent--sessions))
          (worker (gethash "working" romacs-agent--sessions)))
      (push (romacs-agent-ui--child-row worker) (romacs-agent-children-rows parent))
      (romacs-agent--summary-stale "parent")
      (romacs-agent-summary-readers--views
       parent romacs-agent-log-local
       (lambda ()
         (let* ((label (lambda () (romacs-agent-menu--worker-label 0)))
                (expected (funcall label)))
           (should (string-match-p "working" expected))
           (should (equal (romacs-agent-summary-readers--with-trap label) expected))
           (should-not (romacs-agent-menu--worker-label 1))
           (should (equal (mapcar (lambda (row) (list (plist-get row :id) (plist-get row :key)))
                                  (romacs-agent-summary-readers--with-trap #'romacs-agent-menu--thread-rows))
                          '(("working" working) ("child" idle))))))))))

(ert-deftest romacs-agent-summary-readers-trap-switcher ()
  (romacs-agent-summary-readers--with-fixtures
    (romacs-agent-summary-readers--check-trapped-views #'romacs-agent-summary-readers--annotation
                                                      romacs-agent-summary-readers--golden-annotations)))

(ert-deftest romacs-agent-summary-readers-derived-slots ()
  (romacs-agent-summary-readers--with-fixtures
    (let ((working (romacs-agent-summary "working"))
          (approval (romacs-agent-summary "approval"))
          (question (romacs-agent-summary "question"))
          (workers (romacs-agent-summary "workers"))
          (watching (romacs-agent-summary "watching"))
          (selection (romacs-agent-summary "selection")))
      (should (equal (funcall 'romacs-agent-summary-active-tool working) "Read"))
      (should (eq (funcall 'romacs-agent-summary-state-key working) 'working))
      (should (eq (funcall 'romacs-agent-summary-user-wait-kind approval) 'approval))
      (should (eq (funcall 'romacs-agent-summary-user-wait-kind question) 'question))
      (should (= (funcall 'romacs-agent-summary-user-wait-count approval) 1))
      (should (= (funcall 'romacs-agent-summary-user-wait-count question) 1))
      (should (= (funcall 'romacs-agent-summary-user-wait-count working) 0))
      (should (eq (funcall 'romacs-agent-summary-state-key workers) 'waiting))
      (should (eq (funcall 'romacs-agent-summary-state-key watching) 'watching))
      (should (equal (funcall 'romacs-agent-summary-watch-unit watching) "fixture.service"))
      (should (equal (funcall 'romacs-agent-summary-selected-model selection) "claude-selected"))
      (should (equal (funcall 'romacs-agent-summary-default-effort selection) "high")))))

(ert-deftest romacs-agent-summary-readers-rename-by-id ()
  (romacs-agent-summary-readers--with-fixtures
    (let (called)
      (cl-letf (((symbol-function 'romacs-agent-rename)
                 (lambda (id title) (setq called (list id title)))))
        (romacs-agent-summary-readers--views
         (gethash "idle" romacs-agent--sessions) romacs-agent-log-local
         (lambda () (romacs-agent-chat-rename "New title"))))
      (should (equal called '("idle" "New title"))))))

(ert-deftest romacs-agent-summary-readers-worker-digit-survives-reorder ()
  (romacs-agent-summary-readers--with-fixtures
    (let* ((parent (gethash "parent" romacs-agent--sessions))
           (child (gethash "child" romacs-agent--sessions))
           (other (gethash "working" romacs-agent--sessions))
           (opened nil))
      (setf (romacs-agent-state child) 'running)
      (romacs-agent-ui--summarize-children parent)
      (romacs-agent-summary-readers--views
       parent romacs-agent-log-local
       (lambda ()
         (should (string-match-p "child" (romacs-agent-menu--worker-label 0)))
         (should (equal (aref (romacs-agent-menu--slots) 0) "child"))
         (push (romacs-agent-ui--child-row other) (romacs-agent-children-rows parent))
         (romacs-agent--summary-stale "parent")
         (should (equal (romacs-agent-menu--workers) '("working" "child")))
         (cl-letf (((symbol-function 'romacs-agent-show)
                    (lambda (session &rest _) (setq opened (romacs-agent-id session)))))
           (let ((last-command-event ?1)) (romacs-agent-menu-open-worker)))))
      (should (equal opened (romacs-agent-id child))))))

(defun romacs-agent-summary-readers--completion (expected choose action)
  (let (observed)
    (cl-letf (((symbol-function 'completing-read)
               (lambda (_prompt collection &rest _)
                 (setq observed t)
                 (should (listp collection))
                 (should (equal collection expected))
                 (dolist (property '(display-sort-function cycle-sort-function))
                   (let ((sorter (completion-metadata-get
                                  (completion-metadata "" collection nil) property)))
                     (should (functionp sorter))
                     (should (equal (funcall sorter (copy-sequence collection)) expected))))
                 choose)))
      (prog1 (funcall action) (should observed)))))

(ert-deftest romacs-agent-summary-readers-plain-worker-completion ()
  (romacs-agent-summary-readers--with-fixtures
    (let ((parent (gethash "parent" romacs-agent--sessions))
          (child (gethash "child" romacs-agent--sessions))
          (worker (gethash "working" romacs-agent--sessions)))
      (setf (romacs-agent-children-rows parent) (mapcar #'romacs-agent-ui--child-row (list worker child)))
      (romacs-agent-summary-readers--views
       parent romacs-agent-log-local
       (lambda ()
         (cl-letf (((symbol-function 'romacs-agent-show)
                    (lambda (session &rest _) (romacs-agent-id session))))
           (should (equal (romacs-agent-summary-readers--completion
                           '("W working  Working  working" "I child  Idle  child") "I child  Idle  child"
                           (lambda () (cl-letf (((symbol-function 'romacs-agent-glyph) (lambda (role) (if (eq role 'chip-working) "W" "I")))
                                                ((symbol-function 'romacs-agent--short) #'identity))
                                        (romacs-agent-menu-threads))))
                          "child"))))))))

(ert-deftest romacs-agent-summary-readers-thread-menu-opens-a-settled-thread ()
  (romacs-agent-summary-readers--with-fixtures
    (let ((parent (gethash "parent" romacs-agent--sessions))
          (child (gethash "child" romacs-agent--sessions))
          (worker (gethash "working" romacs-agent--sessions))
          shown)
      (setf (romacs-agent-settled-seq child) (romacs-agent-done-seq child)
            (romacs-agent-children-rows parent)
            (mapcar #'romacs-agent-ui--child-row (list worker child)))
      (romacs-agent--summary-stale "parent")
      (romacs-agent-summary-readers--views
       parent romacs-agent-log-local
       (lambda ()
         (let ((live (concat (romacs-agent-glyph 'chip-working) " working  Working  working"))
               (settled (concat (romacs-agent-glyph 'completed) " child  Done  child"))
               (command (lookup-key romacs-agent-menu-map (kbd "t"))))
           (should (eq command 'romacs-agent-menu-threads))
           (cl-letf (((symbol-function 'romacs-agent-show)
                      (lambda (session action) (push (list session action) shown))))
             (romacs-agent-summary-readers--completion
              (list live settled) settled (lambda () (call-interactively command)))))))
      (should (equal shown (list (list child '(display-buffer-same-window))))))))

(ert-deftest romacs-agent-summary-readers-plain-effort-completion ()
  (should (equal (romacs-agent-summary-readers--completion
                  '("high" "low") "low" (lambda () (romacs-agent--read-effort '("high" "low") "high")))
                 "low")))

(ert-deftest romacs-agent-summary-readers-plain-model-completion ()
  (cl-letf (((symbol-function 'romacs-agent--want-models) #'ignore))
    (should (equal (romacs-agent-summary-readers--completion
                    '("Z model" "A model") "A model"
                    (lambda () (romacs-agent--pick-row
                                (lambda () '(("Z model" :model "z") ("A model" :model "a"))) "Model")))
                   '("A model" :model "a")))))

(ert-deftest romacs-agent-summary-readers-plain-switcher-completion ()
  (romacs-agent-summary-readers--with-fixtures
    (let ((recent (generate-new-buffer "Recent chat"))
          (older (generate-new-buffer "Older chat"))
          (switched nil))
      (unwind-protect
          (progn
            (with-current-buffer recent
              (romacs-agent-log-mode)
              (romacs-agent-log--set-session (gethash "working" romacs-agent--sessions)))
            (with-current-buffer older
              (romacs-agent-log-mode)
              (romacs-agent-log--set-session (gethash "idle" romacs-agent--sessions)))
            (cl-letf (((symbol-function 'buffer-list) (lambda (&optional _) (list recent older)))
                      ((symbol-function 'switch-to-buffer) (lambda (buffer &rest _) (setq switched buffer))))
              (romacs-agent-summary-readers--completion
               (mapcar #'buffer-name (list recent older)) (buffer-name older) #'romacs-agent-switch))
            (should (eq switched older)))
        (kill-buffer recent)
        (kill-buffer older)))))

(provide 'romacs-agent-summary-readers-tests)
;;; romacs-agent-summary-readers-tests.el ends here
