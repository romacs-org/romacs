;;; warm-restart-tests.el --- A session image's windows laid out again -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'warm-restart)

;; What `romacs-startup-progress' keeps: bound here around each restore.
(defvar romacs-progress-state)
(defvar eshell-history-file-name)

(ert-deftest warm-restart-a-rebuilt-buffers-window-has-a-start ()
  ;; A window on a buffer its handler rebuilt (an agent chat, empty until
  ;; shown) is laid out from that buffer's own beginning and point, never
  ;; from no start: `window-state-put' would then leave the window's start
  ;; marker pointing nowhere, and the first `posn-at-point' on it (the
  ;; restore's own warning finding a window for itself) signalled,
  ;; cutting `normal-top-level' short.
  (let ((rebuilt (generate-new-buffer " *rebuilt*")))
    (unwind-protect
        (save-window-excursion
          (with-current-buffer rebuilt (insert "short") (goto-char 3))
          (delete-other-windows)
          (let* ((names (let ((table (make-hash-table :test #'equal)))
                          (puthash "old name" (cons rebuilt t) table)
                          table))
                 ;; The state as the image carried it: the old buffer's name,
                 ;; and the old text's start and point, far past this one's end.
                 (state (progn (set-window-buffer nil rebuilt) (window-state-get nil t)))
                 (carried (cons (car state)
                                (mapcar (lambda (item)
                                          (if (eq (car-safe item) 'buffer)
                                              `(buffer "old name" (start . 4000000) (point . 5859891)
                                                       ,@(cl-remove-if (lambda (p) (memq (car-safe p) '(start point)))
                                                                       (cddr item)))
                                            item))
                                        (cdr state))))
                 (live (cons (car carried) (warm-restart--live-tree (cdr carried) names)))
                 (props (cddr (assq 'buffer (cdr live)))))
            (should (equal (cadr (assq 'buffer (cdr live))) (buffer-name rebuilt)))
            (should (equal (alist-get 'start props) 1))
            (should (equal (alist-get 'point props) 3))
            (set-window-buffer nil (get-buffer-create "*scratch*"))
            (window-state-put live (selected-window) 'safe)
            (should (eq (window-buffer) rebuilt))
            (should (integerp (window-start)))
            (should-not (condition-case err (progn (posn-at-point 1) nil) (error err)))))
      (kill-buffer rebuilt))))

(ert-deftest warm-restart-a-start-restores-once-it-serves ()
  ;; `--restore' at a start applies nothing while the start runs: the
  ;; records come back from the command loop, a turn each, and the
  ;; `restore' step says how far it is, then that it is done.
  (let* ((warm-restart-directory (make-temp-file "romacs-wr-" t))
         (buffer (get-buffer-create "wr-carried"))
         (events nil) image)
    (unwind-protect
        (progn
          (with-current-buffer buffer (text-mode) (insert "carried\n"))
          (setq image (warm-restart-write))
          (kill-buffer buffer)
          (let ((romacs-progress-state nil)
                (status (lambda () (plist-get (alist-get 'restore romacs-progress-state) :status))))
            (should-not (warm-restart-restore-image image t))
            (should-not (get-buffer "wr-carried"))
            (with-timeout (20 (ert-fail "the restore did not end"))
              (while (not (memq 'done events))
                (accept-process-output nil 0.05)
                (cl-pushnew (funcall status) events)))
            (should (memq 'running events))
            (should (string-prefix-p "Session restored" (plist-get (alist-get 'restore romacs-progress-state) :label)))
            (should (equal (with-current-buffer "wr-carried" (buffer-string)) "carried\n"))
            (should (eq (plist-get (alist-get 'restore romacs-progress-state) :status) 'done))))
      (ignore-errors (kill-buffer "wr-carried"))
      (delete-directory warm-restart-directory t))))

(ert-deftest warm-restart-an-edit-made-before-its-turn-is-kept ()
  ;; A start serves while its restore goes on: a file edited here before
  ;; its record's turn keeps the edit, and the carried text goes in a
  ;; buffer beside it, named in the report.
  (let* ((warm-restart-directory (make-temp-file "romacs-wr-" t))
         (file (expand-file-name "edited.txt" warm-restart-directory))
         image)
    (unwind-protect
        (progn
          (with-temp-file file (insert "on disk\n"))
          (with-current-buffer (find-file-noselect file)
            (goto-char (point-max)) (insert "carried edit\n")
            (setq image (warm-restart-write))
            (set-buffer-modified-p nil) (kill-buffer))
          (with-current-buffer (find-file-noselect file)
            (goto-char (point-max)) (insert "edited here first\n"))
          (let ((report (warm-restart-restore-image image)))
            (should (equal (with-current-buffer (get-file-buffer file) (buffer-string)) "on disk\nedited here first\n"))
            (should (equal (with-current-buffer "edited.txt (restored)" (buffer-string)) "on disk\ncarried edit\n"))
            (should (cl-some (lambda (p) (string-match-p "before its turn" (plist-get p :reason)))
                             (plist-get report :problems)))))
      (dolist (name '("edited.txt (restored)"))
        (when (get-buffer name) (kill-buffer name)))
      (when-let* ((b (get-file-buffer file))) (with-current-buffer b (set-buffer-modified-p nil)) (kill-buffer b))
      (delete-directory warm-restart-directory t))))


(ert-deftest warm-restart-unseen-buffers-come-after-the-shown-and-get-their-modes-when-shown ()
  ;; A start's restore brings back the buffers its windows show first and
  ;; lays the windows out; the rest come after, in no mode of their own
  ;; until a window shows them.
  (let* ((warm-restart-directory (make-temp-file "romacs-wr-" t)) (order nil) image)
    (unwind-protect
        (save-window-excursion
          (delete-other-windows)
          (with-current-buffer (get-buffer-create "wr-shown") (text-mode) (insert "shown\n"))
          (with-current-buffer (get-buffer-create "wr-unseen") (emacs-lisp-mode) (insert "(unseen)\n"))
          (set-window-buffer nil "wr-shown")
          (setq image (warm-restart-write))
          (set-window-buffer nil (get-buffer-create "*scratch*"))
          (kill-buffer "wr-shown")
          (kill-buffer "wr-unseen")
          (cl-letf* ((real (symbol-function 'warm-restart--restore-one))
                     ((symbol-function 'warm-restart--restore-one)
                      (lambda (record &rest args)
                        (push (list (plist-get record :name) warm-restart--defer (and (get-buffer "wr-shown")
                                                                                      (eq (window-buffer) (get-buffer "wr-shown"))))
                              order)
                        (apply real record args))))
            (let ((romacs-progress-state nil))
              (warm-restart-restore-image image t)
              (with-timeout (20 (ert-fail "the restore did not end"))
                (while (not (eq (plist-get (alist-get 'restore romacs-progress-state) :status) 'done))
                  (accept-process-output nil 0.05)))))
          (setq order (nreverse order))
          (should (equal (assoc "wr-shown" order) '("wr-shown" nil nil)))
          ;; Restored after the windows were laid out, its modes waiting.
          (should (equal (assoc "wr-unseen" order) '("wr-unseen" t t)))
          (should (< (cl-position "wr-shown" order :key #'car :test #'equal)
                     (cl-position "wr-unseen" order :key #'car :test #'equal)))
          (with-current-buffer "wr-unseen"
            (should (eq major-mode 'fundamental-mode))
            (should (equal (buffer-string) "(unseen)\n")))
          ;; The host runs the window change functions before it redisplays.
          (should (memq #'warm-restart--wake-shown (default-value 'window-buffer-change-functions)))
          (set-window-buffer nil "wr-unseen")
          (run-hook-with-args 'window-buffer-change-functions (selected-frame))
          (with-current-buffer "wr-unseen" (should (eq major-mode 'emacs-lisp-mode)))
          (should-not warm-restart--deferred-buffers))
      (ignore-errors (kill-buffer "wr-shown"))
      (ignore-errors (kill-buffer "wr-unseen"))
      (delete-directory warm-restart-directory t))))

(ert-deftest warm-restart-an-unseen-record-waits-while-a-key-does ()
  ;; With a key waiting, an unseen record waits for the editor to be idle;
  ;; then it comes back, and a save gives it its modes first.
  (let ((record (list :id "w" :name "wr-wait" :strategy 'text :blob 0 :mode 'emacs-lisp-mode :point 1
                      :directory default-directory))
        (buffers (make-hash-table :test #'equal)) (idle nil) (next nil) (romacs-progress-state nil))
    (unwind-protect
        (cl-letf (((symbol-function 'run-with-idle-timer) (lambda (&rest args) (push args idle) nil))
                  ((symbol-function 'run-at-time) (lambda (&rest args) (push args next) nil)))
          (cl-letf (((symbol-function 'input-pending-p) (lambda (&rest _) t)))
            (warm-restart--step-unseen "image" (float-time) nil ["(wait)\n"] (list record) (list record) 0 buffers nil))
          (should-not (get-buffer "wr-wait"))
          (should (eq (nth 2 (car idle)) #'warm-restart--step-unseen))
          (cl-letf (((symbol-function 'input-pending-p) (lambda (&rest _) nil)))
            (warm-restart--step-unseen "image" (float-time) nil ["(wait)\n"] (list record) (list record) 0 buffers nil))
          (should (eq (nth 2 (car next)) #'warm-restart--step-unseen))
          (with-current-buffer "wr-wait"
            (should (eq major-mode 'fundamental-mode))
            (should (equal (buffer-string) "(wait)\n"))
            (run-hooks 'before-save-hook)
            (should (eq major-mode 'emacs-lisp-mode))))
      (ignore-errors (kill-buffer "wr-wait")))))

(ert-deftest warm-restart-an-unseen-file-visits-in-no-mode-until-shown ()
  ;; An unseen file is visited with its own mode, local variables and
  ;; `find-file-hook' waiting too; waking it runs them, then its record's.
  (let* ((dir (make-temp-file "romacs-wr-file-" t)) (file (expand-file-name "wake.el" dir)) (ran nil)
         (find-file-hook (list (lambda () (push (buffer-name) ran))))
         (record (list :id "f" :name "wake.el" :strategy 'file :file file :mode 'emacs-lisp-mode :point 1 :directory dir)))
    (unwind-protect
        (progn
          (write-region "(wake)\n" nil file nil 'silent)
          (let ((warm-restart--defer t) (warm-restart--problems nil))
            (warm-restart--restore-buffer record [] (make-hash-table :test #'equal)))
          (with-current-buffer (get-file-buffer file)
            (should (eq major-mode 'fundamental-mode))
            (should-not ran)
            (warm-restart--wake (current-buffer))
            (should (eq major-mode 'emacs-lisp-mode))
            (should (equal ran '("wake.el")))
            (should-not warm-restart--deferred)))
      (ignore-errors (kill-buffer (get-file-buffer file)))
      (delete-directory dir t))))

(ert-deftest warm-restart-an-unseen-file-wakes-whole-when-a-visit-finds-it ()
  ;; Found by a visit, as an agenda finds its files: woken first, its whole
  ;; text read for its local variables, its narrowing kept.
  (let* ((dir (make-temp-file "romacs-wr-file-" t)) (file (expand-file-name "local.txt" dir))
         (record (list :id "l" :name "local.txt" :strategy 'file :file file :mode 'text-mode :point 1
                       :narrowing '(1 . 5) :directory dir)))
    (unwind-protect
        (progn
          (write-region "body\n\n;; Local Variables:\n;; fill-column: 42\n;; End:\n" nil file nil 'silent)
          (let ((warm-restart--defer t) (warm-restart--problems nil))
            (warm-restart--restore-buffer record [] (make-hash-table :test #'equal)))
          (with-current-buffer (get-file-buffer file)
            (should (eq major-mode 'fundamental-mode))
            (should (= (point-max) 5))
            (should (eq (find-file-noselect file) (current-buffer)))
            (should (eq major-mode 'text-mode))
            (should (= fill-column 42))
            (should (= (point-max) 5))))
      (ignore-errors (kill-buffer (get-file-buffer file)))
      (delete-directory dir t))))

(ert-deftest warm-restart-a-modified-unseen-file-has-its-modes-and-asks-its-lock ()
  ;; A modified file, which a save may write, is not left without its
  ;; modes; its first edit locks its file, whenever it comes.
  (let* ((dir (make-temp-file "romacs-wr-file-" t)) (file (expand-file-name "mod.el" dir))
         (record (list :id "m" :name "mod.el" :strategy 'file :file file :mode 'emacs-lisp-mode :point 1
                       :blob 0 :modified t :directory dir)))
    (unwind-protect
        (progn
          (write-region "(old)\n" nil file nil 'silent)
          (let ((warm-restart--defer t) (warm-restart--problems nil))
            (warm-restart--restore-buffer record ["(new)\n"] (make-hash-table :test #'equal)))
          (with-current-buffer (get-file-buffer file)
            (should (eq major-mode 'emacs-lisp-mode))
            (should-not warm-restart--deferred)
            (should (buffer-modified-p))
            (should (equal (buffer-string) "(new)\n"))
            (should (local-variable-p 'warm-restart-lock-pending))))
      (ignore-errors (with-current-buffer (get-file-buffer file) (set-buffer-modified-p nil) (kill-buffer)))
      (delete-directory dir t))))

(ert-deftest warm-restart-a-window-laid-out-early-gets-its-history-back ()
  ;; Its windows laid out before every buffer was back: once they are, each
  ;; window's earlier buffers come back after those shown in it since.
  (let ((back (get-buffer-create "wr-history")) (names (make-hash-table :test #'equal)))
    (unwind-protect
        (save-window-excursion
          (with-current-buffer back (insert "history"))
          (puthash "old history" (cons back nil) names)
          (let ((warm-restart--histories (list (list (selected-window) '(("old history" 1 3) ("gone" 1 1)) nil))))
            (warm-restart--restore-histories names))
          (should (eq (car (car (window-prev-buffers))) back))
          (should (= (marker-position (nth 2 (car (window-prev-buffers)))) 3)))
      (kill-buffer back))))

(ert-deftest warm-restart-a-layout-put-again-keeps-its-histories ()
  ;; A layout put again (the frame resized before the first command) after
  ;; every buffer is back gives its new windows their histories at once.
  (let ((back (get-buffer-create "wr-again")) (names (make-hash-table :test #'equal)))
    (unwind-protect
        (save-window-excursion
          (delete-other-windows)
          (with-current-buffer back (insert "again"))
          (puthash "old again" (cons back nil) names)
          (let* ((state (window-state-get (frame-root-window) t))
                 (leaf (cdr state))
                 (saved (cons (car state) (cons 'leaf (cons '(prev-buffers ("old again" 1 4)) (cdr leaf)))))
                 (warm-restart--history-names names) (warm-restart--histories nil))
            (should (eq (car leaf) 'leaf))
            (warm-restart--put-noted (selected-frame) state saved)
            (should (eq (car (car (window-prev-buffers (frame-first-window)))) back))
            (should-not warm-restart--histories)))
      (kill-buffer back))))

(ert-deftest warm-restart-a-target-through-a-link-restarts-into-its-build ()
  ;; A restart into an installer's `current' link runs the build the link
  ;; names, its runtime root that build's true name: the spelling a
  ;; launcher that resolves the link prebuilt its worlds for, and one a
  ;; later install's move of the link leaves alone.
  (let* ((dir (file-truename (make-temp-file "warm-restart-target-" t)))
         (build (expand-file-name "build-1" dir))
         (link (expand-file-name "current" dir)))
    (unwind-protect
        (progn
          (make-directory (expand-file-name "etc" build) t)
          (let ((binary (expand-file-name "romacs-vm-gui" build)))
            (write-region "" nil binary nil 'silent)
            (set-file-modes binary #o755))
          (make-symbolic-link build link)
          (pcase-let ((`((,binary . ,_) ,root) (warm-restart--target link)))
            (should (equal binary (expand-file-name "romacs-vm-gui" build)))
            (should (equal root (concat "ROMACS_RUNTIME_ROOT=" build)))))
      (delete-directory dir t))))

(ert-deftest warm-restart-a-request-waits-for-what-its-obstacle-functions-name ()
  ;; Work a restart must not cut short (agent records still being
  ;; written) holds a request back without blocking: it is retried until
  ;; the function names nothing.
  (let ((pending 2) (warm-restart-obstacle-functions nil))
    (add-hook 'warm-restart-obstacle-functions
              (lambda () (and (> pending 0) (list (format "%d agent records not yet written" pending)))))
    (add-hook 'warm-restart-obstacle-functions #'ignore)
    (cl-letf (((symbol-function 'current-idle-time) (lambda () '(0 1 0 0)))
              ((symbol-function 'warm-restart-plan) (lambda () nil)))
      (should (equal (warm-restart--obstacles (list :force t)) '("2 agent records not yet written")))
      (setq pending 0)
      (should-not (warm-restart--obstacles (list :force t))))))

;; What the handover hooks do while they wait (the agents' writer drains,
;; timers and process output run) is in the plan the image is written from.
(defun warm-restart-test--restart-with-hook (hook &optional answer)
  "`romacs-restart' with HOOK on the handover hooks: the plan it writes, or
nil when it refused first.  ANSWER, when given, answers its questions
(called with each prompt) and the restart is not forced."
  (let ((warm-restart-before-handover-functions (list hook)) (written nil))
    (cl-letf (((symbol-function 'warm-restart-write)
               (lambda (plan &rest _) (setq written plan) (throw 'warm-restart-test-written nil)))
              ((symbol-function 'yes-or-no-p) (or answer (symbol-function 'yes-or-no-p))))
      (catch 'warm-restart-test-written (romacs-restart (not answer))))
    written))

(ert-deftest warm-restart-plan-is-made-after-the-handover-hooks ()
  (let ((name (generate-new-buffer-name "warm-restart-made-during-handover")))
    (unwind-protect
        (let ((plan (warm-restart-test--restart-with-hook
                     (lambda () (with-current-buffer (get-buffer-create name) (insert "made while draining"))))))
          (should (member name (mapcar (lambda (e) (plist-get e :name)) (plist-get plan :buffers)))))
      (when (get-buffer name) (kill-buffer name)))))

(ert-deftest warm-restart-refuses-a-buffer-the-handover-hooks-blocked ()
  (let ((name (generate-new-buffer-name "warm-restart-blocked-during-handover")))
    (unwind-protect
        (let ((err (should-error
                    (warm-restart-test--restart-with-hook
                     (lambda () (with-current-buffer (get-buffer-create name)
                                  (setq-local warm-restart-blocker-functions
                                              (list (lambda () "busy for the test"))))))
                    :type 'user-error)))
          (should (string-match-p "Cannot restart now: .*busy for the test" (cadr err))))
      (when (get-buffer name) (kill-buffer name)))))

;; A question waits for its answer, as the hooks do: what is made or
;; started meanwhile is asked about or carried too.
(ert-deftest warm-restart-asks-about-a-process-that-took-an-asked-name ()
  (let* ((name "warm-restart-test-renamed")
         (first (make-pipe-process :name name :noquery nil))
         (second nil) (prompts nil))
    (unwind-protect
        (progn
          (warm-restart-test--restart-with-hook
           (lambda () (delete-process first) (setq second (make-pipe-process :name name :noquery nil)))
           (lambda (prompt) (push prompt prompts) t))
          (should (equal (process-name second) name))
          (should (= 2 (cl-count-if (lambda (prompt) (string-search name prompt)) prompts))))
      (dolist (process (list first second))
        (when (and process (process-live-p process)) (delete-process process))))))

(ert-deftest warm-restart-plans-again-after-a-question-that-waited ()
  (let ((name "warm-restart-test-asked")
        (buffer-name (generate-new-buffer-name "warm-restart-made-while-asking"))
        (process nil))
    (unwind-protect
        (let ((plan (warm-restart-test--restart-with-hook
                     (lambda () (setq process (make-pipe-process :name name :noquery nil)))
                     (lambda (prompt) (when (string-search name prompt) (get-buffer-create buffer-name)) t))))
          (should (member buffer-name (mapcar (lambda (e) (plist-get e :name)) (plist-get plan :buffers)))))
      (when (and process (process-live-p process)) (delete-process process))
      (when (get-buffer buffer-name) (kill-buffer buffer-name)))))

;; An owner restart (11-18) named 24 dropped buffers "unsaved text not
;; carried" that held nothing typed: mail views and the agent board.
(ert-deftest warm-restart-unsaved-text-is-typed-text-not-a-filled-view ()
  "A read-only view its package fills, and a process's output, are
modified but hold nothing typed: an image lists them dropped, not as
unsaved text.  Input typed after a comint buffer's process mark, narrowed
or not, and typed text in a buffer left out of images, are named.  A
process whose mark is not placed yet does not stop the plan."
  (let* ((warm-restart-directory (make-temp-file "romacs-wr-" t))
         (view (generate-new-buffer "*wr-notmuch-search*"))
         (output (generate-new-buffer "wr-process-output"))
         (typing (generate-new-buffer "wr-process-typing"))
         (term (generate-new-buffer "wr-process-term"))
         (unplaced (generate-new-buffer "wr-process-unplaced"))
         (draft (generate-new-buffer "wr-draft-left-out"))
         (file (find-file-noselect (expand-file-name "kept.txt" warm-restart-directory)))
         (processes nil))
    (unwind-protect
        (progn
          ;; A mail search: read-only, filled by its package, rebuilt by it.
          (with-current-buffer view
            (special-mode)
            (setq-local warm-restart-regenerable "mail view; its package reopens it")
            (let ((inhibit-read-only t)) (insert "Thread one\nThread two\n"))
            (should (buffer-modified-p)))
          ;; Process buffers, output before the mark.  In a comint mode the
          ;; mark ends the output: one has input typed after it, not yet
          ;; sent, and is narrowed to the output.  A non-comint buffer's
          ;; text after its mark is the process's (a terminal's cursor).
          (dolist (buffer (list output typing term))
            (with-current-buffer buffer
              (unless (eq buffer term) (comint-mode))
              (let ((process (make-pipe-process :name "wr-output" :buffer buffer :noquery t)))
                (push process processes)
                (goto-char (process-mark process))
                (insert "output from the process\n$ ")
                (set-marker (process-mark process) (point)))))
          (with-current-buffer typing
            (goto-char (point-max))
            (insert "ls -l")
            (narrow-to-region (point-min) (process-mark (get-buffer-process typing))))
          (with-current-buffer term (goto-char (point-max)) (insert "prompt redrawn"))
          ;; A process whose mark is not placed, its writable buffer edited.
          (with-current-buffer unplaced
            (comint-mode)
            (let ((process (make-pipe-process :name "wr-unplaced" :buffer unplaced :noquery t)))
              (push process processes)
              (set-marker (process-mark process) nil))
            (insert "typed before any output"))
          ;; Typed text in a buffer that never goes into an image.
          (with-current-buffer draft
            (setq-local warm-restart-left-out "kept off the disk")
            (insert "typed by hand"))
          ;; A file left out the same way, edited, then made read-only.
          (with-current-buffer file
            (setq-local warm-restart-left-out "kept off the disk")
            (insert "typed into a file")
            (read-only-mode 1))
          (let* ((plan (warm-restart-plan))
                 (entry (lambda (buffer) (seq-find (lambda (e) (eq (plist-get e :buffer) buffer)) (plist-get plan :buffers)))))
            (should (plist-get (funcall entry view) :modified))
            (should-not (plist-get (funcall entry view) :unsaved))
            (should-not (plist-get (funcall entry output) :unsaved))
            (should (plist-get (funcall entry typing) :unsaved))
            (should-not (plist-get (funcall entry term) :unsaved))
            (should (plist-get (funcall entry unplaced) :unsaved))
            (should (plist-get (funcall entry draft) :unsaved))
            (should (plist-get (funcall entry file) :unsaved)))
          (let* ((image (warm-restart-write))
                 (lost (plist-get (car (warm-restart--read image)) :lost))
                 (named (lambda (buffer) (seq-find (lambda (e) (equal (plist-get e :buffer) (buffer-name buffer))) lost))))
            ;; Still listed as dropped; unsaved only where something was typed.
            (should (equal (funcall named view)
                           (list :buffer (buffer-name view) :reason "mail view; its package reopens it")))
            (should (funcall named output))
            (should-not (plist-get (funcall named output) :unsaved))
            (should (plist-get (funcall named typing) :unsaved))
            (should (equal (funcall named draft)
                           (list :buffer (buffer-name draft) :reason "kept off the disk" :unsaved t)))
            (should (equal (funcall named file)
                           (list :buffer (buffer-name file) :reason "kept off the disk" :unsaved t)))))
      (mapc #'delete-process processes)
      (dolist (buffer (list view output typing term unplaced draft file))
        (when (buffer-live-p buffer)
          (with-current-buffer buffer (set-buffer-modified-p nil))
          (kill-buffer buffer)))
      (delete-directory warm-restart-directory t))))

;; A restored eshell read its whole transcript as one command at RET: a
;; command line in it could run again (11-18).
(defvar warm-restart-tests--ran nil "Set by a command a restored shell must not run.")

(ert-deftest warm-restart-a-shell-comes-back-as-its-transcript ()
  "An eshell buffer's transcript, a command line in it and one typed and
not sent, comes back as inert text, eagerly or unseen and woken: RET in
it runs nothing, nor does a new eshell opened afterwards."
  (require 'eshell)
  (dolist (startup '(nil t))
    (let* ((warm-restart-directory (make-temp-file "romacs-wr-" t))
           (eshell-buffer-name (generate-new-buffer-name "*wr-eshell*"))
           (eshell-history-file-name nil)
           (shell (eshell))
           image)
      (unwind-protect
          (save-window-excursion
            (with-current-buffer shell
              (goto-char (point-max))
              (insert "(setq warm-restart-tests--ran 'transcript)")
              (eshell-send-input)
              (goto-char (point-max))
              (insert "(setq warm-restart-tests--ran 'pending)"))
            ;; The fixture works: the command it sent ran.
            (should (eq warm-restart-tests--ran 'transcript))
            (setq warm-restart-tests--ran nil)
            (set-window-buffer nil (get-buffer-create "*scratch*"))
            (setq image (warm-restart-write))
            ;; Written as its transcript: an older reader restoring a rollback
            ;; brings it back inert too.
            (let ((record (seq-find (lambda (r) (eq (plist-get r :transcript-of) 'eshell-mode))
                                    (plist-get (car (warm-restart--read image)) :buffers))))
              (should (equal (plist-get record :name) (concat eshell-buffer-name " (transcript)")))
              (should (eq (plist-get record :mode) 'fundamental-mode))
              (should-not (plist-get record :minor-modes))
              (should (equal (mapcar #'car (plist-get record :locals)) '(header-line-format))))
            (let ((kill-buffer-query-functions nil)) (kill-buffer shell))
            (let ((romacs-progress-state nil))
              (if (not startup)
                  (warm-restart-restore-image image)
                (warm-restart-restore-image image t)
                (with-timeout (20 (ert-fail "the restore did not end"))
                  (while (not (eq (plist-get (alist-get 'restore romacs-progress-state) :status) 'done))
                    (accept-process-output nil 0.05)))
                ;; Unseen, its modes wait; shown, it wakes as its transcript.
                (set-window-buffer nil (concat eshell-buffer-name " (transcript)"))
                (run-hook-with-args 'window-buffer-change-functions (selected-frame))))
            (with-current-buffer (concat eshell-buffer-name " (transcript)")
              (should (eq major-mode 'fundamental-mode))
              (should (string-match-p "eshell-mode" (format "%s" header-line-format)))
              (should (string-search "(setq warm-restart-tests--ran 'transcript)" (buffer-string)))
              (should (string-search "(setq warm-restart-tests--ran 'pending)" (buffer-string)))
              (goto-char (point-max))
              (call-interactively (key-binding (kbd "RET")))
              ;; Point before the line never sent, as a user may leave it.
              (goto-char (point-min))
              (search-forward "'pending)")
              (goto-char (match-beginning 0)))
            (should-not warm-restart-tests--ran)
            ;; A new eshell makes a buffer of its own, never one over the
            ;; transcript, and runs nothing it did not get.
            (let ((transcript (get-buffer (concat eshell-buffer-name " (transcript)"))))
              (with-current-buffer (eshell)
                (should-not (eq (current-buffer) transcript))
                (should-not (string-search "'pending)" (buffer-string)))
                (goto-char (point-max))
                (eshell-send-input)))
            (should-not warm-restart-tests--ran))
        (let ((kill-buffer-query-functions nil))
          (dolist (name (list eshell-buffer-name (concat eshell-buffer-name " (transcript)")))
            (when (get-buffer name) (kill-buffer name))))
        (setq warm-restart-tests--ran nil)
        (delete-directory warm-restart-directory t)))))

(defun warm-restart-tests--restore-older (records blobs &optional defer)
  "Restore RECORDS, as an image written before shells were carried as
transcripts holds them, with BLOBS, as the restore does: prepared first."
  (let ((warm-restart--defer defer) (buffers (make-hash-table :test #'equal)) problems)
    (dolist (record (warm-restart--prepare (list :schema warm-restart--schema :buffers records)) problems)
      (setq problems (append problems (warm-restart--restore-one record blobs buffers))))))

;; An image written before shells were carried as transcripts names a
;; shell's mode: the restore still brings it back inert, eagerly or woken,
;; and a shell mode whose library only the restore loads is known too.
;; A shell mode's desktop handler is never bypassed.
(ert-deftest warm-restart-an-older-image-s-shell-comes-back-as-its-transcript ()
  (let* ((dir (make-temp-file "romacs-wr-shell-" t))
         (library (expand-file-name "wr-test-repl.el" dir))
         (text "$ (setq warm-restart-tests--ran 'transcript)\n$ (setq warm-restart-tests--ran 'pending)")
         (blobs (vector text))
         (record (lambda (name mode &optional library)
                   (list :id name :name name :strategy 'text :mode mode :library library :blob 0
                         :point (1+ (length text)) :directory dir :minor-modes '(visual-line-mode)
                         :locals '((fill-column . 33))))))
    (with-temp-file library
      (insert "(require 'eshell)\n(define-derived-mode wr-test-repl-mode eshell-mode \"Repl\")\n(provide 'wr-test-repl)\n"))
    (unwind-protect
        (progn
          (dolist (defer '(nil t))
            (dolist (case `(("wr-old-eshell" eshell-mode nil) ("wr-old-repl" wr-test-repl-mode ,(file-name-sans-extension library))))
              (when (fboundp 'wr-test-repl-mode) (fmakunbound 'wr-test-repl-mode))
              (setq features (delq 'wr-test-repl features))
              (warm-restart-tests--restore-older (list (funcall record (car case) (nth 1 case) (nth 2 case))) blobs defer)
              ;; Under the transcript's name from the start, unseen or not: no
              ;; shell can take it before it wakes.  The derived mode's library,
              ;; not loaded before, is loaded to know it.
              (should-not (get-buffer (car case)))
              (let ((buffer (get-buffer (concat (car case) " (transcript)"))))
                (should buffer)
                (when defer (warm-restart--wake buffer)))
              (with-current-buffer (concat (car case) " (transcript)")
                (should (eq major-mode 'fundamental-mode))
                (should (string-match-p (symbol-name (nth 1 case)) (format "%s" header-line-format)))
                (should-not visual-line-mode)
                (should-not (local-variable-p 'fill-column))
                (should (equal (buffer-string) text))
                (goto-char (point-max))
                (call-interactively (key-binding (kbd "RET"))))
              (should-not warm-restart-tests--ran)
              (kill-buffer (concat (car case) " (transcript)"))))
          ;; A shell mode with its own desktop handler restores through it.
          (define-derived-mode warm-restart-tests--handled-shell-mode comint-mode "Handled")
          (let ((desktop-buffer-mode-handlers
                 (cons (cons 'warm-restart-tests--handled-shell-mode
                             (lambda (_file name misc)
                               (with-current-buffer (get-buffer-create name)
                                 (warm-restart-tests--handled-shell-mode)
                                 (insert misc)
                                 (current-buffer))))
                       desktop-buffer-mode-handlers))
                (warm-restart--problems nil))
            (warm-restart--restore-buffer
             (list :id "h" :name "wr-handled-shell" :strategy 'handler :mode 'warm-restart-tests--handled-shell-mode
                   :misc "draft kept by its handler" :point 1 :directory dir)
             [] (make-hash-table :test #'equal))
            (with-current-buffer "wr-handled-shell"
              (should (eq major-mode 'warm-restart-tests--handled-shell-mode))
              (should (equal (buffer-string) "draft kept by its handler")))))
      (dolist (name '("wr-old-eshell" "wr-old-repl" "wr-old-eshell (transcript)" "wr-old-repl (transcript)" "wr-handled-shell"))
        (when (get-buffer name) (kill-buffer name)))
      (setq warm-restart-tests--ran nil)
      (delete-directory dir t))))

(ert-deftest warm-restart-a-transcript-s-windows-follow-its-name ()
  ;; A window on a shell, and one that showed it before, show its
  ;; transcript, which the image names apart from the shell.
  (should (equal (warm-restart--renamed-in
                  '((leaf (buffer "*sh*" (point . 9) (start . 1)) (prev-buffers ("*sh*" 1 9) ("notes" 1 1)))
                    (leaf (buffer "notes" (point . 1)) (next-buffers ("*sh*" 1 3))))
                  '(("*sh*" . "*sh* (transcript)")))
                 '((leaf (buffer "*sh* (transcript)" (point . 9) (start . 1))
                         (prev-buffers ("*sh* (transcript)" 1 9) ("notes" 1 1)))
                   (leaf (buffer "notes" (point . 1)) (next-buffers ("*sh* (transcript)" 1 3))))))
  (should (equal (warm-restart--transcript-name "*sh*" (lambda (n) (member n '("*sh* (transcript)"))))
                 "*sh* (transcript)<2>")))

(ert-deftest warm-restart-an-older-shell-record-leaves-a-live-shell-be ()
  ;; A live, unmodified eshell of the name an older image's eshell record
  ;; names stays the user's shell: the transcript gets a buffer of its own.
  (require 'eshell)
  (let* ((eshell-buffer-name (generate-new-buffer-name "*wr-live-eshell*"))
         (eshell-history-file-name nil)
         (live (eshell))
         (text "$ (setq warm-restart-tests--ran 'pending)")
         (record (list :id "e" :name eshell-buffer-name :strategy 'text :mode 'eshell-mode :blob 0
                       :point (1+ (length text)) :directory default-directory)))
    (unwind-protect
        (progn
          (with-current-buffer live (set-buffer-modified-p nil))
          (warm-restart-tests--restore-older (list record) (vector text))
          (with-current-buffer live
            (should (eq major-mode 'eshell-mode))
            (should-not (string-search "'pending)" (buffer-string))))
          (with-current-buffer (concat eshell-buffer-name " (transcript)")
            (should (eq major-mode 'fundamental-mode))
            (should (equal (buffer-string) text))))
      (let ((kill-buffer-query-functions nil))
        (dolist (name (list eshell-buffer-name (concat eshell-buffer-name " (transcript)")))
          (when (get-buffer name) (kill-buffer name)))))))

(define-derived-mode warm-restart-tests--plain-shell-mode comint-mode "Plain shell")

(ert-deftest warm-restart-a-file-in-a-shell-mode-is-written-as-its-transcript ()
  (let* ((dir (make-temp-file "romacs-wr-shellfile-" t))
         (warm-restart-directory dir)
         (buffer (find-file-noselect (expand-file-name "session.log" dir)))
         (saved (progn (write-region "$ saved\n" nil (expand-file-name "saved.log" dir) nil 'silent)
                       (find-file-noselect (expand-file-name "saved.log" dir)))))
    (unwind-protect
        (progn
          (with-current-buffer buffer
            (warm-restart-tests--plain-shell-mode)
            (insert "$ typed and not saved"))
          ;; Unmodified on an existing file, its text is carried all the same.
          (with-current-buffer saved
            (warm-restart-tests--plain-shell-mode)
            (set-buffer-modified-p nil))
          (let ((record (seq-find (lambda (r) (equal (plist-get r :name) "saved.log (transcript)"))
                                  (plist-get (car (warm-restart--read (warm-restart-write))) :buffers))))
            (should (eq (plist-get record :strategy) 'text))
            (should-not (plist-get record :file))
            (should (plist-get record :blob)))
          ;; Detached text: no reader, an older one included, visits the file,
          ;; so no file mode or hook of it can make it a shell again.
          (let ((record (seq-find (lambda (r) (eq (plist-get r :transcript-of) 'warm-restart-tests--plain-shell-mode))
                                  (plist-get (car (warm-restart--read (warm-restart-write))) :buffers))))
            (should (eq (plist-get record :strategy) 'text))
            (should-not (plist-get record :file))
            (should (equal (plist-get record :name) "session.log (transcript)"))
            (should (eq (plist-get record :mode) 'fundamental-mode))
            (should (eq (plist-get record :transcript-of) 'warm-restart-tests--plain-shell-mode))
            (should (plist-get record :blob))))
      (dolist (b (list buffer saved)) (with-current-buffer b (set-buffer-modified-p nil)) (kill-buffer b))
      (delete-directory dir t))))

(ert-deftest warm-restart-an-older-image-s-shell-names-reserve-every-record-s ()
  ;; A transcript's name is free of every record the image holds, and an
  ;; older file record of a shell keeps it through the file's restore.
  (let* ((dir (make-temp-file "romacs-wr-names-" t))
         (file (expand-file-name "session.log" dir)))
    (unwind-protect
        (progn
          (write-region "$ typed" nil file nil 'silent)
          (warm-restart-tests--restore-older
           (list (list :id "s" :name "wr-sh" :strategy 'text :mode 'eshell-mode :blob 0 :point 1 :directory dir)
                 (list :id "n" :name "wr-sh (transcript)" :strategy 'text :mode 'text-mode :blob 1 :point 1 :directory dir)
                 (list :id "f" :name "session.log" :strategy 'file :file file :mode 'eshell-mode :point 1 :directory dir
                       :uniquify t))
           (vector "$ shell" "notes"))
          (with-current-buffer "wr-sh (transcript)" (should (equal (buffer-string) "notes")))
          (with-current-buffer "wr-sh (transcript)<2>"
            (should (eq major-mode 'fundamental-mode))
            (should (equal (buffer-string) "$ shell")))
          (with-current-buffer (get-file-buffer file)
            (should (equal (buffer-name) "session.log (transcript)"))
            (should (eq major-mode 'fundamental-mode))))
      (dolist (name '("wr-sh" "wr-sh (transcript)" "wr-sh (transcript)<2>"))
        (when (get-buffer name) (kill-buffer name)))
      (when (get-file-buffer file) (kill-buffer (get-file-buffer file)))
      (delete-directory dir t))))

(ert-deftest warm-restart-bare-buffer-names-in-a-window-follow-a-rename ()
  ;; A writable window state keeps its next buffers as bare names.
  (should (equal (warm-restart--renamed-in '((leaf (buffer "*sh*") (next-buffers "*sh*" "notes")))
                                           '(("*sh*" . "*sh* (transcript)")))
                 '((leaf (buffer "*sh* (transcript)") (next-buffers "*sh* (transcript)" "notes")))))
  (let ((names (make-hash-table :test #'equal)))
    (with-temp-buffer
      (rename-buffer "wr-bare-next" t)
      (puthash "old-next" (cons (current-buffer) nil) names)
      (should (equal (assq 'next-buffers (warm-restart--live-leaf '(leaf (next-buffers "old-next" "gone-buffer")) names))
                     (list 'next-buffers (buffer-name)))))))

(ert-deftest warm-restart-an-older-record-whose-mode-cannot-load-stops-no-other ()
  ;; A mode whose autoload names a library that is gone: its record comes
  ;; back inert, and the records after it are restored.
  (autoload 'warm-restart-tests--gone-mode "warm-restart-tests-no-such-library" nil t)
  (unwind-protect
      (progn
        (warm-restart-tests--restore-older
         (list (list :id "g" :name "wr-gone" :strategy 'text :mode 'warm-restart-tests--gone-mode :blob 0 :point 1
                     :directory default-directory)
               (list :id "k" :name "wr-kept" :strategy 'text :mode 'text-mode :blob 1 :point 1
                     :directory default-directory))
         (vector "gone mode's text" "kept text"))
        (with-current-buffer "wr-gone (transcript)"
          (should (eq major-mode 'fundamental-mode))
          (should (equal (buffer-string) "gone mode's text")))
        (with-current-buffer "wr-kept"
          (should (eq major-mode 'text-mode))
          (should (equal (buffer-string) "kept text"))))
    (dolist (name '("wr-gone" "wr-gone (transcript)" "wr-kept"))
      (when (get-buffer name) (kill-buffer name)))
    (fmakunbound 'warm-restart-tests--gone-mode)))

(ert-deftest warm-restart-a-transcript-is-named-by-the-image-not-uniquify ()
  (should-not (plist-get (warm-restart--inert-record (list :mode 'eshell-mode :uniquify t :name "x") 'eshell-mode) :uniquify)))

(ert-deftest warm-restart-an-older-record-s-mode-is-known-through-its-parents ()
  ;; A mode two libraries deep: its parent, an autoload into another
  ;; library, derives from eshell-mode.  A mode with no definition and no
  ;; library, and an unmodified file whose mode's autoload is broken, come
  ;; back inert too, the file with its text from the disk.
  (let* ((dir (make-temp-file "romacs-wr-chain-" t))
         (parent (expand-file-name "wr-test-shell-parent.el" dir))
         (child (expand-file-name "wr-test-shell-child.el" dir))
         (file (expand-file-name "notes.log" dir))
         (load-path (cons dir load-path)))
    (with-temp-file parent
      (insert "(require 'eshell)\n(define-derived-mode wr-test-shell-parent-mode eshell-mode \"P\")\n(provide 'wr-test-shell-parent)\n"))
    (with-temp-file child
      (insert "(autoload 'wr-test-shell-parent-mode \"wr-test-shell-parent\" nil t)\n"
              "(define-derived-mode wr-test-shell-child-mode wr-test-shell-parent-mode \"C\")\n(provide 'wr-test-shell-child)\n"))
    (write-region "text on the disk" nil file nil 'silent)
    (autoload 'warm-restart-tests--gone-file-mode "warm-restart-tests-no-such-library" nil t)
    (unwind-protect
        (progn
          (warm-restart-tests--restore-older
           (list (list :id "c" :name "wr-chain" :strategy 'text :mode 'wr-test-shell-child-mode
                       :library (file-name-sans-extension child) :blob 0 :point 1 :directory dir)
                 (list :id "n" :name "wr-nowhere" :strategy 'text :mode 'warm-restart-tests--nowhere-mode
                       :library (expand-file-name "no-such-library" dir) :blob 1 :point 1 :directory dir)
                 (list :id "f" :name "notes.log" :strategy 'file :file file :mode 'warm-restart-tests--gone-file-mode
                       :point 1 :directory dir))
           (vector "$ chained" "$ nowhere"))
          (dolist (case '(("wr-chain (transcript)" . "$ chained") ("wr-nowhere (transcript)" . "$ nowhere")
                          ("notes.log (transcript)" . "text on the disk")))
            (with-current-buffer (car case)
              (should (eq major-mode 'fundamental-mode))
              (should (equal (buffer-string) (cdr case)))))
          (should-not (get-buffer "wr-chain"))
          (should-not (get-buffer "wr-nowhere")))
      (dolist (name '("wr-chain (transcript)" "wr-nowhere (transcript)"))
        (when (get-buffer name) (kill-buffer name)))
      (when (get-file-buffer file) (kill-buffer (get-file-buffer file)))
      (fmakunbound 'warm-restart-tests--gone-file-mode)
      (delete-directory dir t))))

(ert-deftest warm-restart-a-transcript-s-file-wakes-as-text ()
  ;; Unseen and woken, a transcript's file takes no mode of its file, even
  ;; one auto-mode says is eshell's, and no file hook, even one that fails.
  ;; A mode two libraries deep through an alias of its parent is a shell's.
  (let* ((dir (make-temp-file "romacs-wr-wake-" t))
         (file (expand-file-name "session.eshlog" dir))
         (real (expand-file-name "wr-test-real-parent.el" dir))
         (child (expand-file-name "wr-test-alias-child.el" dir))
         (load-path (cons dir load-path))
         (auto-mode-alist (cons '("\\.eshlog\\'" . eshell-mode) auto-mode-alist))
         (find-file-hook (list (lambda () (error "A file hook that fails")))))
    (write-region "$ (setq warm-restart-tests--ran 'woken)" nil file nil 'silent)
    (with-temp-file real
      (insert "(require 'eshell)\n(define-derived-mode wr-test-real-parent-mode eshell-mode \"R\")\n(provide 'wr-test-real-parent)\n"))
    (with-temp-file child
      (insert "(autoload 'wr-test-real-parent-mode \"wr-test-real-parent\" nil t)\n"
              "(defalias 'wr-test-alias-parent-mode 'wr-test-real-parent-mode)\n"
              "(define-derived-mode wr-test-alias-child-mode wr-test-alias-parent-mode \"A\")\n(provide 'wr-test-alias-child)\n"))
    (unwind-protect
        (progn
          (warm-restart-tests--restore-older
           (list (list :id "f" :name "session.eshlog" :strategy 'file :file file :mode 'eshell-mode :point 1 :directory dir)
                 (list :id "a" :name "wr-aliased" :strategy 'text :mode 'wr-test-alias-child-mode
                       :library (file-name-sans-extension child) :blob 0 :point 1 :directory dir))
           (vector "$ aliased")
           t)
          (with-current-buffer "session.eshlog (transcript)"
            (warm-restart--wake (current-buffer))
            (should (eq major-mode 'fundamental-mode))
            (goto-char (point-max))
            (call-interactively (key-binding (kbd "RET"))))
          (should-not warm-restart-tests--ran)
          (with-current-buffer "wr-aliased (transcript)"
            (warm-restart--wake (current-buffer))
            (should (eq major-mode 'fundamental-mode))
            (should (equal (buffer-string) "$ aliased")))
          (should-not (get-buffer "wr-aliased")))
      (when (get-buffer "wr-aliased (transcript)") (kill-buffer "wr-aliased (transcript)"))
      (when (get-file-buffer file) (with-current-buffer (get-file-buffer file) (set-buffer-modified-p nil)) (kill-buffer (get-file-buffer file)))
      (setq warm-restart-tests--ran nil)
      (delete-directory dir t))))

(defun warm-restart-tests--restore-image (image startup)
  "Restore IMAGE, eagerly, or as at STARTUP, unseen buffers left asleep,
and wait for it to end."
  (let ((romacs-progress-state nil))
    (if (not startup)
        (warm-restart-restore-image image)
      (warm-restart-restore-image image t)
      (with-timeout (20 (ert-fail "the restore did not end"))
        (while (not (eq (plist-get (alist-get 'restore romacs-progress-state) :status) 'done))
          (accept-process-output nil 0.05))))))

(ert-deftest warm-restart-a-transcript-stays-one-across-restarts ()
  ;; Restored, a transcript is fundamental-mode text; at the next image it
  ;; is still written as its shell's transcript, under the same name,
  ;; restored eagerly or unseen, written asleep or woken.
  (require 'eshell)
  (dolist (startup '(nil t))
    (let* ((warm-restart-directory (make-temp-file "romacs-wr-twice-" t))
           (eshell-buffer-name (generate-new-buffer-name "*wr-twice*"))
           (eshell-history-file-name nil)
           (transcript (concat eshell-buffer-name " (transcript)"))
           (shell (eshell)))
      (unwind-protect
          (save-window-excursion
            (with-current-buffer shell (goto-char (point-max)) (insert "echo typed"))
            (set-window-buffer nil (get-buffer-create "*scratch*"))
            (dotimes (pass 3)
              (let ((image (warm-restart-write)))
                (let ((record (seq-find (lambda (r) (plist-get r :transcript-of))
                                        (plist-get (car (warm-restart--read image)) :buffers))))
                  (should (eq (plist-get record :transcript-of) 'eshell-mode))
                  (should (equal (plist-get record :name) transcript)))
                (let ((kill-buffer-query-functions nil))
                  (dolist (name (list eshell-buffer-name transcript)) (when (get-buffer name) (kill-buffer name))))
                (warm-restart-tests--restore-image image startup)
                (with-current-buffer transcript
                  (should (eq (and warm-restart--deferred t) startup))
                  (should (eq warm-restart--transcript-of 'eshell-mode)))
                ;; Woken before the last image, asleep before the one between.
                (when (and startup (/= pass 1))
                  (set-window-buffer nil transcript)
                  (run-hook-with-args 'window-buffer-change-functions (selected-frame))
                  (set-window-buffer nil "*scratch*"))
                (with-current-buffer transcript
                  (should (eq major-mode 'fundamental-mode))
                  (should (eq warm-restart--transcript-of 'eshell-mode))
                  (should (string-match-p "eshell-mode" (format "%s" header-line-format)))
                  (should (string-search "echo typed" (buffer-string)))))))
        (let ((kill-buffer-query-functions nil))
          (dolist (name (list eshell-buffer-name transcript)) (when (get-buffer name) (kill-buffer name))))
        (delete-directory warm-restart-directory t)))))

(defun warm-restart-tests--transcript-record (name)
  "The record the next image writes of the buffer NAME."
  (seq-find (lambda (r) (equal (plist-get r :name) name))
            (plist-get (car (warm-restart--read (warm-restart-write))) :buffers)))

(ert-deftest warm-restart-a-transcript-made-something-else-is-written-as-that ()
  ;; A transcript the user gives a file of its own, or another mode, is no
  ;; transcript any more: the next image writes it as what it became.
  (let* ((dir (make-temp-file "romacs-wr-repurposed-" t))
         (warm-restart-directory dir)
         (text-file (expand-file-name "kept.txt" dir))
         (plain-file (expand-file-name "kept-plain" dir)))
    (unwind-protect
        (progn
          ;; A file that changes no mode: the buffer keeps its locals.
          (with-current-buffer (get-buffer-create "wr-a (transcript)")
            (insert "$ ls")
            (warm-restart--become-transcript 'eshell-mode)
            (setq-local fill-column 33)
            (let ((change-major-mode-with-file-name nil)) (set-visited-file-name plain-file))
            (should (local-variable-p 'fill-column))
            (should-not (local-variable-p 'header-line-format)))
          (let ((record (warm-restart-tests--transcript-record "kept-plain")))
            (should (equal (plist-get record :file) plain-file))
            (should (eq (plist-get record :strategy) 'file))
            (should-not (plist-get record :transcript-of)))
          ;; A file whose name picks a mode.
          (with-current-buffer (get-buffer-create "wr-b (transcript)")
            (insert "$ ls")
            (warm-restart--become-transcript 'eshell-mode)
            (set-visited-file-name text-file))
          (let ((record (warm-restart-tests--transcript-record "kept.txt")))
            (should (equal (plist-get record :file) text-file))
            (should (eq (plist-get record :mode) 'text-mode))
            (should-not (plist-get record :transcript-of)))
          ;; Another mode.
          (with-current-buffer (get-buffer-create "wr-c (transcript)")
            (insert "$ ls")
            (warm-restart--become-transcript 'eshell-mode)
            (text-mode))
          (let ((record (warm-restart-tests--transcript-record "wr-c (transcript)")))
            (should (eq (plist-get record :mode) 'text-mode))
            (should-not (plist-get record :transcript-of))))
      (dolist (name '("kept-plain" "kept.txt" "wr-c (transcript)"))
        (when (get-buffer name)
          (with-current-buffer name (set-buffer-modified-p nil))
          (kill-buffer name)))
      (delete-directory dir t))))

(ert-deftest warm-restart-a-transcript-written-to-a-file-unseen-stays-the-file ()
  ;; Restored unseen and written to a file before it is shown: the save
  ;; wakes nothing that would make it a transcript again.
  (require 'eshell)
  (let* ((dir (make-temp-file "romacs-wr-unseen-" t))
         (warm-restart-directory dir)
         (eshell-buffer-name (generate-new-buffer-name "*wr-unseen*"))
         (eshell-history-file-name nil)
         (transcript (concat eshell-buffer-name " (transcript)"))
         (file (expand-file-name "kept-plain" dir))
         (shell (eshell)))
    (unwind-protect
        (save-window-excursion
          (with-current-buffer shell (goto-char (point-max)) (insert "echo typed"))
          (set-window-buffer nil (get-buffer-create "*scratch*"))
          (let ((image (warm-restart-write)))
            (let ((kill-buffer-query-functions nil)) (kill-buffer shell))
            (warm-restart-tests--restore-image image t))
          (with-current-buffer transcript
            (should warm-restart--deferred)
            (write-file file)
            (should-not warm-restart--transcript-of)
            (should-not header-line-format))
          (let ((record (warm-restart-tests--transcript-record "kept-plain")))
            (should (equal (plist-get record :file) file))
            (should-not (plist-get record :transcript-of))))
      (let ((kill-buffer-query-functions nil))
        (dolist (name (list eshell-buffer-name transcript "kept-plain")) (when (get-buffer name) (kill-buffer name))))
      (delete-directory dir t))))

(ert-deftest warm-restart-a-shell-named-like-a-transcript-is-named-apart ()
  ;; A live shell whose name looks like a transcript's still gets a name
  ;; of its own: a new shell of its name never lands in its transcript.
  (require 'eshell)
  (let* ((warm-restart-directory (make-temp-file "romacs-wr-lookalike-" t))
         (eshell-buffer-name (concat (generate-new-buffer-name "*wr-look*") " (transcript)"))
         (eshell-history-file-name nil)
         (transcript (concat eshell-buffer-name " (transcript)"))
         (shell (eshell)))
    (unwind-protect
        (save-window-excursion
          (with-current-buffer shell (goto-char (point-max)) (insert "echo typed"))
          (set-window-buffer nil (get-buffer-create "*scratch*"))
          (let ((image (warm-restart-write)))
            (should (seq-find (lambda (r) (and (plist-get r :transcript-of) (equal (plist-get r :name) transcript)))
                              (plist-get (car (warm-restart--read image)) :buffers)))
            (let ((kill-buffer-query-functions nil)) (kill-buffer shell))
            (warm-restart-tests--restore-image image nil))
          (with-current-buffer (eshell)
            (should-not (eq (current-buffer) (get-buffer transcript)))
            (should-not (string-search "echo typed" (buffer-string))))
          (with-current-buffer transcript
            (should (eq major-mode 'fundamental-mode))
            (should (string-search "echo typed" (buffer-string)))))
      (let ((kill-buffer-query-functions nil))
        (dolist (name (list eshell-buffer-name transcript)) (when (get-buffer name) (kill-buffer name))))
      (delete-directory warm-restart-directory t))))

(defmacro warm-restart-tests--failing-record (names &rest body)
  "BODY with `warm-restart--record' signalling for the buffers NAMES."
  (declare (indent 1))
  `(cl-letf* ((real (symbol-function 'warm-restart--record))
              ((symbol-function 'warm-restart--record)
               (lambda (entry blobs)
                 (if (member (plist-get entry :name) ,names) (error "A record that cannot be saved")
                   (funcall real entry blobs)))))
     ,@body))

(ert-deftest warm-restart-a-record-that-fails-sends-its-text-in-a-companion ()
  ;; Degrade per entry: a buffer whose record fails, its text typed, comes
  ;; back as a companion under its id; the next buffer's text is its own.
  (let* ((warm-restart-directory (make-temp-file "romacs-wr-companion-" t))
         (failing (get-buffer-create "wr-fails"))
         (draft (get-buffer-create "wr-draft"))
         image id)
    (unwind-protect
        (progn
          (with-current-buffer failing
            (insert "typed and at risk " (propertize "hunter2" 'romacs-secret t) " end")
            (setq id (warm-restart--buffer-id)))
          (with-current-buffer draft (insert "a draft of its own"))
          (warm-restart-tests--failing-record '("wr-fails")
            (setq image (warm-restart-write)))
          (should-not (cl-some (lambda (b) (string-prefix-p " *warm-restart companion*" (buffer-name b))) (buffer-list)))
          (with-temp-buffer
            (insert-file-contents-literally image)
            (should-not (search-forward "hunter2" nil t)))
          (let ((record (seq-find (lambda (r) (equal (plist-get r :companion) "wr-fails"))
                                  (plist-get (car (warm-restart--read image)) :buffers))))
            (should (equal (plist-get record :id) id))
            (should (equal (plist-get record :name) "wr-fails <carried text>")))
          (kill-buffer failing) (kill-buffer draft)
          (let* ((romacs-progress-state nil) (report (warm-restart-restore-image image)))
            (should (equal (plist-get report :companions)
                           '((:buffer "wr-fails" :carried-in "wr-fails <carried text>")))))
          (with-current-buffer "wr-fails <carried text>"
            (should (eq major-mode 'fundamental-mode))
            (should (equal (buffer-string) "typed and at risk ******* end"))
            (should (string-search "could not be saved" (format "%s" header-line-format))))
          (with-current-buffer "wr-draft" (should (equal (buffer-string) "a draft of its own"))))
      (dolist (name '("wr-fails" "wr-draft" "wr-fails <carried text>"))
        (when (get-buffer name) (with-current-buffer name (set-buffer-modified-p nil)) (kill-buffer name)))
      (delete-directory warm-restart-directory t))))

(ert-deftest warm-restart-a-record-that-fails-blocks-only-when-its-text-cannot-go ()
  (let* ((warm-restart-directory (make-temp-file "romacs-wr-companion-" t))
         (file (expand-file-name "saved.txt" warm-restart-directory))
         (saved nil) (typed (get-buffer-create "wr-typed")))
    (unwind-protect
        (progn
          (write-region "on the disk" nil file nil 'silent)
          (setq saved (find-file-noselect file))
          (with-current-buffer typed (insert "typed"))
          ;; Nothing at risk: listed lost, the image written.
          (warm-restart-tests--failing-record '("saved.txt")
            (let ((form (car (warm-restart--read (warm-restart-write)))))
              (should (seq-find (lambda (e) (equal (plist-get e :buffer) "saved.txt")) (plist-get form :lost)))
              (should-not (seq-find (lambda (r) (equal (plist-get r :companion) "saved.txt")) (plist-get form :buffers)))))
          ;; Text at risk and no companion either: the restart stops.
          (warm-restart-tests--failing-record '("wr-typed")
            (cl-letf (((symbol-function 'warm-restart--companion) (lambda (&rest _) (error "No room"))))
              (should-error (warm-restart-write) :type 'warm-restart-blocked)))
          (should-not (cl-some (lambda (b) (string-prefix-p " *warm-restart companion*" (buffer-name b))) (buffer-list))))
      (when (buffer-live-p saved) (kill-buffer saved))
      (with-current-buffer typed (set-buffer-modified-p nil)) (kill-buffer typed)
      (delete-directory warm-restart-directory t))))

(ert-deftest warm-restart-a-handler-s-unportable-data-sends-typed-text-in-a-companion ()
  (let* ((warm-restart-directory (make-temp-file "romacs-wr-companion-" t))
         (buffer (get-buffer-create "wr-handled-draft")))
    (unwind-protect
        (progn
          (define-derived-mode warm-restart-tests--drafting-mode text-mode "Drafting")
          (let ((desktop-buffer-mode-handlers (cons (cons 'warm-restart-tests--drafting-mode #'ignore) desktop-buffer-mode-handlers)))
            (with-current-buffer buffer
              (warm-restart-tests--drafting-mode)
              (setq-local desktop-save-buffer (lambda (_dir) (list (point-marker))))
              (insert "a draft its handler cannot save"))
            (let ((record (seq-find (lambda (r) (equal (plist-get r :companion) "wr-handled-draft"))
                                    (plist-get (car (warm-restart--read (warm-restart-write))) :buffers))))
              (should record)
              (should (string-search "not portable" (format "%s" (plist-get record :locals)))))))
      (with-current-buffer buffer (set-buffer-modified-p nil)) (kill-buffer buffer)
      (delete-directory warm-restart-directory t))))

(ert-deftest warm-restart-a-record-that-fails-to-restore-stops-no-other ()
  ;; A failure outside the record's dispatch (here, finding the buffer it
  ;; would reuse) is noted and the next record restores; a companion never
  ;; lands in a live buffer of its name.
  (let ((live (get-buffer-create "wr-x <carried text>")) problems)
    (unwind-protect
        (progn
          (with-current-buffer live (insert "the user's own") (set-buffer-modified-p t))
          (cl-letf* ((real (symbol-function 'warm-restart--made-buffer))
                     ((symbol-function 'warm-restart--made-buffer)
                      (lambda (record) (if (equal (plist-get record :name) "wr-broken") (error "Broken lookup")
                                         (funcall real record)))))
            (setq problems (warm-restart-tests--restore-older
                            (list (list :id "b" :name "wr-broken" :strategy 'text :mode 'text-mode :blob 0 :point 1 :directory "/")
                                  (list :id "c" :name "wr-x <carried text>" :strategy 'text :mode 'fundamental-mode :companion "wr-x"
                                        :blob 1 :point 1 :directory "/"))
                            (vector "broken" "carried"))))
          (should (seq-find (lambda (p) (equal (plist-get p :buffer) "wr-broken")) problems))
          (with-current-buffer live (should (equal (buffer-string) "the user's own")))
          (with-current-buffer "wr-x <carried text><2>" (should (equal (buffer-string) "carried"))))
      (dolist (name '("wr-broken" "wr-x <carried text>" "wr-x <carried text><2>"))
        (when (get-buffer name) (with-current-buffer name (set-buffer-modified-p nil)) (kill-buffer name))))))

(ert-deftest warm-restart-a-register-in-a-shell-s-file-jumps-to-its-transcript ()
  ;; A position in a shell's file buffer goes to its transcript, which
  ;; holds the text it was in, not to the file.
  (let* ((dir (make-temp-file "romacs-wr-register-" t))
         (warm-restart-directory dir)
         (file (expand-file-name "session.log" dir))
         (register-alist nil)
         (buffer (progn (write-region "on disk\n" nil file nil 'silent) (find-file-noselect file))))
    (unwind-protect
        (save-window-excursion
          (with-current-buffer buffer
            (warm-restart-tests--plain-shell-mode)
            (goto-char (point-max))
            (point-to-register ?w)
            (insert "$ typed"))
          (set-window-buffer nil (get-buffer-create "*scratch*"))
          (let ((image (warm-restart-write)))
            (with-current-buffer buffer (set-buffer-modified-p nil))
            (kill-buffer buffer)
            (setq register-alist nil)
            (warm-restart-tests--restore-image image nil))
          (jump-to-register ?w)
          (should (equal (buffer-name) "session.log (transcript)"))
          (should-not buffer-file-name)
          (should (looking-at-p "\\$ typed")))
      (dolist (name '("session.log (transcript)" "session.log"))
        (when (get-buffer name)
          (with-current-buffer name (set-buffer-modified-p nil))
          (kill-buffer name)))
      (delete-directory dir t))))

(ert-deftest warm-restart-a-transcript-s-copy-beside-an-edited-buffer-is-one-too ()
  ;; A buffer edited under a transcript's name before its turn keeps its
  ;; edits; the transcript's text beside it is still the transcript.
  (let* ((warm-restart-directory (make-temp-file "romacs-wr-beside-" t))
         (name "wr-beside (transcript)")
         (record (warm-restart--inert-record
                  (list :id "t" :name name :strategy 'text :mode 'eshell-mode :blob 0 :point 1
                        :directory default-directory)
                  'eshell-mode))
         (copy (concat name " (restored)")))
    (unwind-protect
        (progn
          (with-current-buffer (get-buffer-create name) (insert "mine"))
          (warm-restart--restore-one record (vector "$ ls") (make-hash-table :test #'equal))
          (with-current-buffer name (should (equal (buffer-string) "mine")))
          (with-current-buffer copy
            (should (equal (buffer-string) "$ ls"))
            (should (eq warm-restart--transcript-of 'eshell-mode)))
          (should (plist-get (warm-restart-tests--transcript-record copy) :transcript-of)))
      (dolist (b (list name copy))
        (when (get-buffer b) (with-current-buffer b (set-buffer-modified-p nil)) (kill-buffer b)))
      (delete-directory warm-restart-directory t))))

;;; warm-restart-tests.el ends here
