;;; warm-restart-tests.el --- A session image's windows laid out again -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'warm-restart)

;; What `romacs-startup-progress' keeps: bound here around each restore.
(defvar romacs-progress-state)

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

;;; warm-restart-tests.el ends here
