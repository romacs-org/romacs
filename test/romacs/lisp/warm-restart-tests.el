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
          ;; Detached text: no reader, an older one included, visits the file,
          ;; so no file mode or hook of it can make it a shell again.
          (let* ((records (plist-get (car (warm-restart--read (warm-restart-write))) :buffers))
                 (named (lambda (name) (seq-find (lambda (r) (equal (plist-get r :name) name)) records))))
            (dolist (name '("session.log (transcript)" "saved.log (transcript)"))
              (let ((record (funcall named name)))
                (should (eq (plist-get record :strategy) 'text))
                (should-not (plist-get record :file))
                (should (eq (plist-get record :mode) 'fundamental-mode))
                (should (eq (plist-get record :transcript-of) 'warm-restart-tests--plain-shell-mode))
                (should (plist-get record :blob))))))
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

(ert-deftest warm-restart-a-register-follows-a-transcript-beside-an-edited-buffer ()
  ;; A buffer edited under the transcript's name before its turn keeps its
  ;; edits; the transcript goes beside it, and its register goes there.
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
            (with-current-buffer (get-buffer-create "session.log (transcript)") (insert "the user's own"))
            (warm-restart-tests--restore-image image nil))
          (jump-to-register ?w)
          (should (equal (buffer-name) "session.log (transcript) (restored)"))
          (should warm-restart--transcript-of)
          (should (looking-at-p "\\$ typed"))
          (with-current-buffer "session.log (transcript)" (should (equal (buffer-string) "the user's own"))))
      (dolist (name '("session.log (transcript)" "session.log (transcript) (restored)" "session.log"))
        (when (get-buffer name)
          (with-current-buffer name (set-buffer-modified-p nil))
          (kill-buffer name)))
      (delete-directory dir t))))

(ert-deftest warm-restart-a-register-in-a-left-out-shell-file-goes-by-its-file ()
  ;; A shell's file buffer the image leaves out: its register names the
  ;; file, not an id no record has.
  (let* ((dir (make-temp-file "romacs-wr-register-" t))
         (warm-restart-directory dir)
         (file (expand-file-name "session.log" dir))
         (register-alist nil)
         (buffer (progn (write-region "on disk\n" nil file nil 'silent) (find-file-noselect file))))
    (unwind-protect
        (with-current-buffer buffer
          (warm-restart-tests--plain-shell-mode)
          (setq-local warm-restart-left-out "left out")
          (point-to-register ?w)
          (should (equal (alist-get ?w (plist-get (car (warm-restart--read (warm-restart-write))) :registers))
                         (list 'file-query file 1))))
      (with-current-buffer buffer (set-buffer-modified-p nil))
      (kill-buffer buffer)
      (delete-directory dir t))))

(ert-deftest warm-restart-an-ordinary-record-over-a-transcript-ends-it ()
  ;; An older image's shell file comes back as its transcript; a later
  ;; image's ordinary record of that file, restored into the same buffer,
  ;; makes it that file's buffer again, its own locals applied.
  (let* ((dir (make-temp-file "romacs-wr-reused-" t))
         (warm-restart-directory dir)
         (file (expand-file-name "session.log" dir)))
    (unwind-protect
        (progn
          (write-region "$ ls\n" nil file nil 'silent)
          (warm-restart-tests--restore-older
           (list (list :id "s" :name "session.log" :strategy 'file :file file :mode 'warm-restart-tests--plain-shell-mode
                       :point 1 :directory dir))
           (vector))
          (with-current-buffer (find-buffer-visiting file) (should warm-restart--transcript-of))
          (warm-restart--restore-one
           (car (warm-restart--prepare
                 (list :transcripts t
                       :buffers (list (list :id "f" :name "session.log" :strategy 'file :file file :mode 'fundamental-mode
                                            :point 1 :directory dir :locals '((fill-column . 33)))))))
           (vector) (make-hash-table :test #'equal))
          (with-current-buffer (find-buffer-visiting file)
            (should-not warm-restart--transcript-of)
            (should-not header-line-format)
            (should (eql fill-column 33))
            (let ((record (warm-restart-tests--transcript-record (buffer-name))))
              (should (equal (plist-get record :file) file))
              (should-not (plist-get record :transcript-of)))))
      (when-let* ((b (find-buffer-visiting file))) (with-current-buffer b (set-buffer-modified-p nil)) (kill-buffer b))
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

(defvar warm-restart-tests--secret-local nil "A local a test fills with a secret.")
(defvar warm-restart-tests--secret-history nil "A history a test fills with a secret.")
(define-derived-mode warm-restart-tests--handled-mode text-mode "Handled")

(defun warm-restart-tests--secret (text)
  "TEXT marked a secret, as the password manager marks one."
  (propertize text 'romacs-secret t))

(ert-deftest warm-restart-no-image-holds-a-secret ()
  ;; A secret pasted or typed into a buffer, deleted but in its undo, in a
  ;; modified file, in a handler's data, a buffer-local or a history: none
  ;; of its text is in the image's bytes.  A buffer's secret comes back as
  ;; stars, one for one, and the restore says so.
  (let* ((warm-restart-directory (make-temp-file "romacs-wr-secrets-" t))
         (file (expand-file-name "notes.txt" warm-restart-directory))
         (desktop-locals-to-save (cons 'warm-restart-tests--secret-local desktop-locals-to-save))
         (desktop-buffer-mode-handlers (cons (cons 'warm-restart-tests--handled-mode #'ignore) desktop-buffer-mode-handlers))
         (warm-restart-history-variables (cons 'warm-restart-tests--secret-history warm-restart-history-variables))
         (warm-restart-tests--secret-history (list "plain" (warm-restart-tests--secret "hunter7")))
         (secrets '("hunter2" "hunter3" "hunter4" "hunter5" "hunter6" "hunter7"))
         draft visiting handled image)
    (unwind-protect
        (progn
          (write-region "on the disk\n" nil file nil 'silent)
          (setq draft (get-buffer-create "wr-secret-draft")
                visiting (find-file-noselect file)
                handled (get-buffer-create "wr-secret-handled"))
          (with-current-buffer draft
            (buffer-enable-undo)
            (insert "pasted " (warm-restart-tests--secret "hunter2") " typed")
            (undo-boundary)
            (insert (warm-restart-tests--secret "hunter3"))
            (undo-boundary)
            (delete-region (- (point) 7) (point))
            (setq-local warm-restart-tests--secret-local (list "a" (warm-restart-tests--secret "hunter6"))))
          (with-current-buffer visiting
            (goto-char (point-max))
            (insert "token " (warm-restart-tests--secret "hunter4")))
          (with-current-buffer handled
            (warm-restart-tests--handled-mode)
            (setq-local desktop-save-buffer
                        (lambda (_dir) (list :draft (concat "my " (warm-restart-tests--secret "hunter5"))))))
          (setq image (warm-restart-write))
          (with-temp-buffer
            (insert-file-contents-literally image)
            (dolist (secret secrets)
              (goto-char (point-min))
              (should-not (search-forward secret nil t))))
          (let* ((form (car (warm-restart--read image))) (records (plist-get form :buffers)))
            (should (plist-get (seq-find (lambda (r) (equal (plist-get r :name) "wr-secret-draft")) records) :starred))
            ;; The file's starred copy is set apart, where no older reader looks.
            (should-not (seq-find (lambda (r) (equal (plist-get r :file) file)) records))
            (let ((apart (seq-find (lambda (r) (equal (plist-get r :name) "notes.txt <starred>")) (plist-get form :starred))))
              (should (eq (plist-get apart :strategy) 'starred))
              (should-not (plist-get apart :file))
              (should (equal (plist-get apart :copy-of) file))))
          ;; No staging buffer outlives the write.
          (should-not (seq-find (lambda (b) (string-prefix-p " *warm-restart staged*" (buffer-name b))) (buffer-list)))
          (dolist (b (list draft visiting handled))
            (with-current-buffer b (set-buffer-modified-p nil))
            (kill-buffer b))
          (let* ((romacs-progress-state nil) (report (warm-restart-restore-image image)))
            (should (seq-find (lambda (p) (and (equal (plist-get p :buffer) "notes.txt <starred>")
                                               (string-search "as stars" (plist-get p :reason))))
                              (plist-get report :problems))))
          (with-current-buffer "wr-secret-draft" (should (equal (buffer-string) "pasted ******* typed")))
          (should-not (find-buffer-visiting file))
          (with-current-buffer "notes.txt <starred>"
            (should-not buffer-file-name)
            (should (equal (buffer-string) "on the disk\ntoken *******"))
            (should buffer-offer-save))
          (with-temp-buffer (insert-file-contents file) (should (equal (buffer-string) "on the disk\n"))))
      (dolist (name (list "wr-secret-draft" "wr-secret-handled" "notes.txt <starred>"))
        (when (get-buffer name) (with-current-buffer name (set-buffer-modified-p nil)) (kill-buffer name)))
      (when-let* ((b (find-buffer-visiting file))) (with-current-buffer b (set-buffer-modified-p nil)) (kill-buffer b))
      (delete-directory warm-restart-directory t))))

(defvar warm-restart-tests--secret-global nil "A global a test fills with a secret.")

(defun warm-restart-tests--image-holds (image text)
  "Whether IMAGE's bytes hold TEXT."
  (with-temp-buffer
    (insert-file-contents-literally image)
    (and (search-forward text nil t) t)))

(defun warm-restart-tests--secret-alone (secret setup)
  "Whether an image written after SETUP, a function that puts SECRET where
it is the only secret, is free of SECRET's text; SETUP returns the
buffers to kill after."
  (let ((warm-restart-directory (make-temp-file "romacs-wr-secrets-" t)) buffers)
    (unwind-protect
        (save-window-excursion
          (setq buffers (funcall setup))
          (not (warm-restart-tests--image-holds (warm-restart-write) secret)))
      (dolist (b buffers) (when (buffer-live-p b) (with-current-buffer b (set-buffer-modified-p nil)) (kill-buffer b)))
      (delete-directory warm-restart-directory t))))

(ert-deftest warm-restart-no-image-holds-a-secret-however-it-is-held ()
  ;; Each way a secret reaches session data, alone in its image, so no
  ;; other secret makes the writer look closer: its text is in no image.
  (let ((desktop-locals-to-save (cons 'warm-restart-tests--secret-local desktop-locals-to-save))
        (desktop-buffer-mode-handlers (cons (cons 'warm-restart-tests--handled-mode #'ignore) desktop-buffer-mode-handlers))
        (warm-restart-globals (cons 'warm-restart-tests--secret-global warm-restart-globals))
        (window-persistent-parameters (cons '(warm-restart-tests--param . writable) window-persistent-parameters))
        (handled (lambda (name saver)
                   (with-current-buffer (get-buffer-create name)
                     (warm-restart-tests--handled-mode)
                     (setq-local desktop-save-buffer saver)
                     (current-buffer)))))
    ;; A handler typing a secret into a buffer captured before it.
    (should (warm-restart-tests--secret-alone
             "hunter10"
             (lambda ()
               (list (with-current-buffer (get-buffer-create "wr-s-early") (insert "early text") (current-buffer))
                     (funcall handled "wr-s-handler"
                              (lambda (_dir)
                                (with-current-buffer "wr-s-early"
                                  (goto-char (point-max)) (insert (warm-restart-tests--secret "hunter10")))
                                (list :x 1)))))))
    ;; A handler's hash table: a key, then a value.
    (should (warm-restart-tests--secret-alone
             "hunter11"
             (lambda () (list (funcall handled "wr-s-key" (lambda (_dir) (let ((table (make-hash-table :test #'equal)))
                                                                         (puthash (warm-restart-tests--secret "hunter11") 1 table)
                                                                         (list :table table))))))))
    (should (warm-restart-tests--secret-alone
             "hunter19"
             (lambda () (list (funcall handled "wr-s-value" (lambda (_dir) (let ((table (make-hash-table :test #'equal)))
                                                                           (puthash "k" (warm-restart-tests--secret "hunter19") table)
                                                                           (list :table table))))))))
    ;; A global's vector.
    (should (let ((warm-restart-tests--secret-global (vector "plain" (warm-restart-tests--secret "hunter12"))))
              (warm-restart-tests--secret-alone "hunter12" (lambda () nil))))
    ;; A `hard' property's value.
    (should (warm-restart-tests--secret-alone
             "hunter13"
             (lambda () (list (with-current-buffer (get-buffer-create "wr-s-hard")
                                (insert "abcdef")
                                (put-text-property 1 3 'hard (warm-restart-tests--secret "hunter13"))
                                (current-buffer))))))
    ;; A property change in the undo list.
    (should (warm-restart-tests--secret-alone
             "hunter14"
             (lambda () (list (with-current-buffer (get-buffer-create "wr-s-undo")
                                (buffer-enable-undo)
                                (insert "abcdef")
                                (undo-boundary)
                                (put-text-property 3 5 'help-echo (warm-restart-tests--secret "hunter14"))
                                (undo-boundary)
                                (remove-text-properties 3 5 '(help-echo nil))
                                (current-buffer))))))
    ;; A saved local's string, the secret in its property.
    (should (warm-restart-tests--secret-alone
             "hunter15"
             (lambda () (list (with-current-buffer (get-buffer-create "wr-s-local")
                                (insert "x")
                                (setq-local warm-restart-tests--secret-local
                                            (propertize "plain" 'help-echo (warm-restart-tests--secret "hunter15")))
                                (current-buffer))))))
    ;; A left-out buffer's reason.
    (should (warm-restart-tests--secret-alone
             "hunter16"
             (lambda () (list (with-current-buffer (get-buffer-create "wr-s-out")
                                (setq-local warm-restart-left-out (warm-restart-tests--secret "hunter16"))
                                (current-buffer))))))
    ;; A handler's circular data.
    (should (warm-restart-tests--secret-alone
             "hunter17"
             (lambda () (list (funcall handled "wr-s-cycle"
                                       (lambda (_dir) (let ((cell (list (warm-restart-tests--secret "hunter17"))))
                                                        (setcdr cell cell) cell)))))))
    ;; A window parameter's vector.
    (should (warm-restart-tests--secret-alone
             "hunter18"
             (lambda () (set-window-parameter nil 'warm-restart-tests--param (vector (warm-restart-tests--secret "hunter18")))
               nil)))))

(ert-deftest warm-restart-a-buffer-is-written-as-it-was-captured ()
  ;; A handler later types a secret into a buffer captured before it, and
  ;; replaces another's text: the image holds each as captured, its undo
  ;; matching its text, and no secret.
  (let ((warm-restart-directory (make-temp-file "romacs-wr-secrets-" t))
        (desktop-buffer-mode-handlers (cons (cons 'warm-restart-tests--handled-mode #'ignore) desktop-buffer-mode-handlers))
        image)
    (unwind-protect
        (progn
          (with-current-buffer (get-buffer-create "wr-s-early") (insert "early text"))
          (with-current-buffer (get-buffer-create "wr-s-draft")
            (buffer-enable-undo)
            (insert "nine char")
            (undo-boundary))
          (with-current-buffer (get-buffer-create "wr-s-handler")
            (warm-restart-tests--handled-mode)
            (setq-local desktop-save-buffer
                        (lambda (_dir)
                          (with-current-buffer "wr-s-early"
                            (goto-char (point-max)) (insert (warm-restart-tests--secret "hunter10")))
                          (with-current-buffer "wr-s-draft"
                            (erase-buffer) (insert (warm-restart-tests--secret "abc")))
                          (list :x 1))))
          (setq image (warm-restart-write))
          (should-not (warm-restart-tests--image-holds image "hunter10"))
          (let ((records (plist-get (car (warm-restart--read image)) :buffers)))
            (should-not (plist-get (seq-find (lambda (r) (equal (plist-get r :name) "wr-s-early")) records) :starred))
            (should (eql (plist-get (seq-find (lambda (r) (equal (plist-get r :name) "wr-s-draft")) records) :size) 9)))
          (dolist (name '("wr-s-early" "wr-s-draft" "wr-s-handler")) (with-current-buffer name (set-buffer-modified-p nil)) (kill-buffer name))
          (let* ((romacs-progress-state nil) (report (warm-restart-restore-image image)))
            (should-not (seq-find (lambda (p) (and (member (plist-get p :buffer) '("wr-s-early" "wr-s-draft")) (plist-get p :text)))
                                  (plist-get report :problems))))
          (with-current-buffer "wr-s-early" (should (equal (buffer-string) "early text")))
          (with-current-buffer "wr-s-draft"
            (should (equal (buffer-string) "nine char"))
            ;; Its undo is the captured text's.
            (goto-char (point-max))
            (undo)
            (should (equal (buffer-string) ""))))
      (dolist (name '("wr-s-early" "wr-s-draft" "wr-s-handler"))
        (when (get-buffer name) (with-current-buffer name (set-buffer-modified-p nil)) (kill-buffer name)))
      (delete-directory warm-restart-directory t))))

(cl-defstruct warm-restart-tests--holder value)

(ert-deftest warm-restart-a-secret-in-a-record-is-starred-and-strings-keep-their-identity ()
  ;; A record in a window parameter holding a secret, alone in its image;
  ;; and handler data whose eq table is keyed by a starred string: the
  ;; table finds it after the walk and after the image's round trip.
  (let ((window-persistent-parameters (cons '(warm-restart-tests--param . writable) window-persistent-parameters))
        (desktop-buffer-mode-handlers (cons (cons 'warm-restart-tests--handled-mode #'ignore) desktop-buffer-mode-handlers)))
    (should (warm-restart-tests--secret-alone
             "hunter21"
             (lambda () (set-window-parameter nil 'warm-restart-tests--param
                                              (make-warm-restart-tests--holder :value (warm-restart-tests--secret "hunter21")))
               nil)))
    (let* ((key (warm-restart-tests--secret "hunter22"))
           (table (let ((h (make-hash-table :test #'eq))) (puthash key 17 h) h))
           (data (list key table))
           (walked (warm-restart--without-secrets data))
           (read (car (read-from-string (warm-restart--print walked)))))
      (should (eql (gethash (car walked) (cadr walked)) 17))
      (should (eql (gethash (car read) (cadr read)) 17)))))

(ert-deftest warm-restart-a-starred-file-is-never-reattached ()
  ;; A starred file buffer whose saved locals name its file, restored with
  ;; a buffer visiting that file renamed to the copy's name first: the copy
  ;; is a new buffer that visits no file, and the visiting one keeps the
  ;; file's text.
  (let* ((warm-restart-directory (make-temp-file "romacs-wr-secrets-" t))
         (file (expand-file-name "notes.txt" warm-restart-directory))
         (desktop-locals-to-save (cons 'buffer-file-name desktop-locals-to-save))
         image visiting)
    (unwind-protect
        (progn
          (write-region "on the disk\n" nil file nil 'silent)
          (with-current-buffer (find-file-noselect file)
            (goto-char (point-max)) (insert "token " (warm-restart-tests--secret "hunter23")))
          (setq image (warm-restart-write))
          (let ((apart (car (plist-get (car (warm-restart--read image)) :starred))))
            (should-not (assq 'buffer-file-name (plist-get apart :locals))))
          (with-current-buffer (find-buffer-visiting file) (set-buffer-modified-p nil) (kill-buffer))
          (setq visiting (find-file-noselect file))
          (with-current-buffer visiting (rename-buffer "notes.txt <starred>"))
          (let ((romacs-progress-state nil)) (warm-restart-restore-image image))
          (with-current-buffer visiting
            (should-not (buffer-modified-p))
            (should (equal (buffer-string) "on the disk\n")))
          (with-current-buffer "notes.txt <starred><2>"
            (should-not buffer-file-name)
            (should (equal (buffer-string) "on the disk\ntoken ********"))))
      (dolist (b (buffer-list))
        (when (string-prefix-p "notes.txt" (buffer-name b)) (with-current-buffer b (set-buffer-modified-p nil)) (kill-buffer b)))
      (delete-directory warm-restart-directory t))))

(ert-deftest warm-restart-a-char-table-holding-a-secret-is-not-carried ()
  (let ((window-persistent-parameters (cons '(warm-restart-tests--param . writable) window-persistent-parameters)))
    (should (warm-restart-tests--secret-alone
             "hunter24"
             (lambda () (let ((table (make-char-table 'warm-restart-tests nil)))
                          (aset table ?a (warm-restart-tests--secret "hunter24"))
                          (set-window-parameter nil 'warm-restart-tests--param table))
               nil)))))

(ert-deftest warm-restart-a-text-changed-while-captured-keeps-no-stale-undo ()
  ;; Code run while the record is made (as a hook after a collection may)
  ;; changes the text: the snapshot is the new text, and no undo or
  ;; property run of the old one comes with it.
  (let ((warm-restart-directory (make-temp-file "romacs-wr-secrets-" t)) image)
    (unwind-protect
        (progn
          (with-current-buffer (get-buffer-create "wr-s-moving")
            (buffer-enable-undo) (insert "nine char") (undo-boundary))
          (cl-letf* ((real (symbol-function 'warm-restart--snapshot))
                     ((symbol-function 'warm-restart--snapshot)
                      (lambda (buffer)
                        (when (equal (buffer-name buffer) "wr-s-moving")
                          (with-current-buffer buffer (erase-buffer) (insert (warm-restart-tests--secret "abc"))))
                        (funcall real buffer))))
            (setq image (warm-restart-write)))
          (let ((record (seq-find (lambda (r) (equal (plist-get r :name) "wr-s-moving"))
                                  (plist-get (car (warm-restart--read image)) :buffers))))
            (should-not (plist-get record :undo))
            (should (eql (plist-get record :size) 3)))
          (with-current-buffer "wr-s-moving" (set-buffer-modified-p nil) (kill-buffer))
          (let ((romacs-progress-state nil)) (warm-restart-restore-image image))
          (with-current-buffer "wr-s-moving" (should (equal (buffer-string) "***"))))
      (when (get-buffer "wr-s-moving") (with-current-buffer "wr-s-moving" (set-buffer-modified-p nil)) (kill-buffer "wr-s-moving"))
      (delete-directory warm-restart-directory t))))

(ert-deftest warm-restart-a-starred-file-s-clone-goes-apart-with-it ()
  ;; A starred file buffer with an indirect clone: `:buffers' names no
  ;; base it lacks, and the restored clone is on the starred copy.
  (let* ((warm-restart-directory (make-temp-file "romacs-wr-secrets-" t))
         (file (expand-file-name "notes.txt" warm-restart-directory))
         image)
    (unwind-protect
        (progn
          (write-region "on the disk\n" nil file nil 'silent)
          (with-current-buffer (find-file-noselect file)
            (goto-char (point-max)) (insert "token " (warm-restart-tests--secret "hunter25"))
            (make-indirect-buffer (current-buffer) "notes.txt view" t))
          (setq image (warm-restart-write))
          (let* ((form (car (warm-restart--read image)))
                 (ids (mapcar (lambda (r) (plist-get r :id)) (plist-get form :buffers))))
            (should-not (seq-find (lambda (r) (and (eq (plist-get r :strategy) 'indirect)
                                                   (not (member (plist-get r :base) ids))))
                                  (plist-get form :buffers)))
            (should (seq-find (lambda (r) (equal (plist-get r :name) "notes.txt view")) (plist-get form :starred))))
          (dolist (name '("notes.txt view" "notes.txt")) (with-current-buffer name (set-buffer-modified-p nil)) (kill-buffer name))
          (let ((romacs-progress-state nil)) (warm-restart-restore-image image))
          (with-current-buffer "notes.txt view"
            (should (equal (buffer-name (buffer-base-buffer)) "notes.txt <starred>"))
            (should-not (string-search "hunter25" (buffer-string)))))
      (dolist (b (buffer-list))
        (when (string-prefix-p "notes.txt" (buffer-name b)) (with-current-buffer b (set-buffer-modified-p nil)) (kill-buffer b)))
      (delete-directory warm-restart-directory t))))

(ert-deftest warm-restart-a-char-table-s-default-secret-is-not-carried ()
  (let ((window-persistent-parameters (cons '(warm-restart-tests--param . writable) window-persistent-parameters)))
    (should (warm-restart-tests--secret-alone
             "hunter26"
             (lambda () (let ((table (make-char-table 'warm-restart-tests nil)))
                          (set-char-table-range table nil (warm-restart-tests--secret "hunter26"))
                          (set-char-table-range table t "everywhere")
                          (set-window-parameter nil 'warm-restart-tests--param table))
               nil)))))

(ert-deftest warm-restart-a-starred-copy-stays-one-restart-after-restart ()
  ;; Restored, a starred copy keeps its header, its offer to be saved and
  ;; its place apart at the next image; that restore again never lands it
  ;; in a buffer visiting the file, whatever that buffer is named.  An
  ;; older reader is told the copy was not carried; this one is not.
  (let* ((warm-restart-directory (make-temp-file "romacs-wr-secrets-" t))
         (file (expand-file-name "notes.txt" warm-restart-directory))
         image report)
    (unwind-protect
        (progn
          (write-region "on the disk\n" nil file nil 'silent)
          (with-current-buffer (find-file-noselect file)
            (goto-char (point-max)) (insert "token " (warm-restart-tests--secret "hunter27")))
          (setq image (warm-restart-write))
          (should (seq-find (lambda (e) (and (plist-get e :starred) (plist-get e :unsaved)))
                            (plist-get (car (warm-restart--read image)) :lost)))
          (with-current-buffer (find-buffer-visiting file) (set-buffer-modified-p nil) (kill-buffer))
          (let ((romacs-progress-state nil)) (setq report (warm-restart-restore-image image)))
          (should-not (seq-find (lambda (e) (plist-get e :starred)) (plist-get report :lost)))
          ;; The next restart: the copy is set apart again.
          (setq image (warm-restart-write))
          (let ((apart (seq-find (lambda (r) (equal (plist-get r :name) "notes.txt <starred>"))
                                 (plist-get (car (warm-restart--read image)) :starred))))
            (should (equal (plist-get apart :copy-of) file)))
          (with-current-buffer "notes.txt <starred>" (set-buffer-modified-p nil) (kill-buffer))
          (with-current-buffer (find-file-noselect file) (rename-buffer "notes.txt <starred>"))
          (let ((romacs-progress-state nil)) (warm-restart-restore-image image))
          (with-current-buffer "notes.txt <starred>"
            (should (equal buffer-file-name file))
            (should-not (buffer-modified-p))
            (should (equal (buffer-string) "on the disk\n")))
          (with-current-buffer "notes.txt <starred><2>"
            (should-not buffer-file-name)
            (should buffer-offer-save)
            (should (string-search "its secrets starred" (format "%s" header-line-format)))
            (should (equal (buffer-string) "on the disk\ntoken ********"))))
      (dolist (b (buffer-list))
        (when (string-prefix-p "notes.txt" (buffer-name b)) (with-current-buffer b (set-buffer-modified-p nil)) (kill-buffer b)))
      (delete-directory warm-restart-directory t))))

(ert-deftest warm-restart-a-starred-copy-stays-one-through-a-change-of-mode ()
  ;; A restored copy put in text-mode is still the copy: set apart at the
  ;; next image.  Given a file, it is that file's buffer: not set apart.
  (let* ((warm-restart-directory (make-temp-file "romacs-wr-secrets-" t))
         (file (expand-file-name "notes.txt" warm-restart-directory))
         (other (expand-file-name "kept.txt" warm-restart-directory))
         image)
    (unwind-protect
        (progn
          (write-region "on the disk\n" nil file nil 'silent)
          (with-current-buffer (find-file-noselect file)
            (goto-char (point-max)) (insert "token " (warm-restart-tests--secret "hunter28")))
          (setq image (warm-restart-write))
          (with-current-buffer (find-buffer-visiting file) (set-buffer-modified-p nil) (kill-buffer))
          (let ((romacs-progress-state nil)) (warm-restart-restore-image image))
          (with-current-buffer "notes.txt <starred>" (text-mode))
          (should (seq-find (lambda (r) (equal (plist-get r :copy-of) file))
                            (plist-get (car (warm-restart--read (warm-restart-write))) :starred)))
          (with-current-buffer "notes.txt <starred>"
            (let ((change-major-mode-with-file-name nil)) (set-visited-file-name other)))
          (let ((form (car (warm-restart--read (warm-restart-write)))))
            (should-not (plist-get form :starred))
            (should (seq-find (lambda (r) (equal (plist-get r :file) other)) (plist-get form :buffers)))))
      (dolist (b (buffer-list))
        (when (or (string-prefix-p "notes.txt" (buffer-name b)) (string-prefix-p "kept.txt" (buffer-name b)))
          (with-current-buffer b (set-buffer-modified-p nil)) (kill-buffer b)))
      (delete-directory warm-restart-directory t))))

(ert-deftest warm-restart-a-long-undo-with-a-secret-is-written ()
  ;; A buffer's undo 5000 entries long, one of them a secret's deletion:
  ;; the image is written, without it.
  (let ((warm-restart-directory (make-temp-file "romacs-wr-secrets-" t)) image)
    (unwind-protect
        (progn
          (with-current-buffer (get-buffer-create "wr-s-long")
            (buffer-enable-undo)
            (let ((undo-limit most-positive-fixnum) (undo-strong-limit most-positive-fixnum) (undo-outer-limit nil))
              (dotimes (i 2500)
                (insert (if (= i 1200) (warm-restart-tests--secret "hunter20") "x"))
                (undo-boundary))
              (delete-region (point-min) (point-max))))
          (setq image (warm-restart-write))
          (should-not (warm-restart-tests--image-holds image "hunter20")))
      (when (get-buffer "wr-s-long") (with-current-buffer "wr-s-long" (set-buffer-modified-p nil)) (kill-buffer "wr-s-long"))
      (delete-directory warm-restart-directory t))))

(ert-deftest warm-restart-a-register-in-a-starred-file-goes-to-its-copy ()
  ;; A register in a file buffer whose secret was starred lands in the
  ;; starred copy at its place, though the file is visited again.
  (let* ((warm-restart-directory (make-temp-file "romacs-wr-secrets-" t))
         (file (expand-file-name "notes.txt" warm-restart-directory))
         (register-alist nil)
         image)
    (unwind-protect
        (save-window-excursion
          (write-region "on the disk\n" nil file nil 'silent)
          (with-current-buffer (find-file-noselect file)
            (goto-char (point-max))
            (insert "token " (warm-restart-tests--secret "hunter29") " ")
            (point-to-register ?s)
            (insert "after"))
          (set-window-buffer nil (get-buffer-create "*scratch*"))
          (setq image (warm-restart-write))
          (should (eq (car (alist-get ?s (plist-get (car (warm-restart--read image)) :registers))) 'warm-restart-position))
          (with-current-buffer (find-buffer-visiting file) (set-buffer-modified-p nil) (kill-buffer))
          (setq register-alist nil)
          (find-file-noselect file)
          (let ((romacs-progress-state nil)) (warm-restart-restore-image image))
          (jump-to-register ?s)
          (should (equal (buffer-name) "notes.txt <starred>"))
          (should-not buffer-file-name)
          (should (looking-at-p "after")))
      (dolist (b (buffer-list))
        (when (string-prefix-p "notes.txt" (buffer-name b)) (with-current-buffer b (set-buffer-modified-p nil)) (kill-buffer b)))
      (delete-directory warm-restart-directory t))))

(ert-deftest warm-restart-a-transcript-s-name-is-free-of-the-records-set-apart ()
  ;; A starred file's clone already has the name a live shell's
  ;; transcript would take: the transcript takes the next free one.
  (let ((named (warm-restart--name-transcripts
                (list (list :id "s" :name "*sh*" :transcript-of 'eshell-mode))
                '("*sh*")
                '("*sh* (transcript)"))))
    (should (equal (plist-get (caar named) :name) "*sh* (transcript)<2>"))
    (should (equal (cdr named) '(("*sh*" . "*sh* (transcript)<2>"))))))

;;; warm-restart-tests.el ends here
