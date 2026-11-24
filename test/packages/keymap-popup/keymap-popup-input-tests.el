;;; keymap-popup-input-tests.el --- Native popup input tests -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.

(require 'ert)
(require 'keymap-popup)

(defmacro keymap-popup-input--with-source (&rest body)
  "Run BODY with isolated command-loop state and a visible source buffer."
  (declare (indent 0) (debug t))
  `(let ((overriding-terminal-local-map nil)
         (pre-command-hook nil) (post-command-hook nil)
         (minibuffer-setup-hook nil) (minibuffer-exit-hook nil)
         (prefix-arg nil) (current-prefix-arg nil)
         (keymap-popup-persistent nil)
         (keymap-popup--buffer-name " *keymap-popup-input*")
         (source (generate-new-buffer " *keymap-popup-input-source*")))
     (unwind-protect
         (save-window-excursion
           (switch-to-buffer source)
           ;; Let a disposable terminal finish its initial command setup.
           (execute-kbd-macro (kbd "C-e"))
           ,@body)
       (keymap-popup-dismiss)
       (kill-buffer source))))

(defun keymap-popup-input--clean-p ()
  "Assert that native popup maps, buffers and hooks have been removed."
  (should-not (get-buffer keymap-popup--buffer-name))
  (should-not overriding-terminal-local-map)
  (should-not pre-command-hook)
  (should-not post-command-hook)
  (should-not minibuffer-setup-hook)
  (should-not minibuffer-exit-hook))

(defun keymap-popup-input--settled-p ()
  "Assert that the visible popup no longer advertises a pending prefix."
  (let ((buf (get-buffer keymap-popup--buffer-name)))
    (should (buffer-live-p buf))
    (should-not (keymap-popup--session-get buf :prefix-mode))
    (with-current-buffer buf
      (goto-char (point-min))
      (search-forward "(Prefix)")
      (should (eq (get-text-property (1- (point)) 'face) 'shadow)))))

(defun keymap-popup-input--suffix-map (command persistent &rest properties)
  "Return an action map for COMMAND, PERSISTENT and entry PROPERTIES."
  (let ((map (make-sparse-keymap)))
    (keymap-set map "a" command)
    (keymap-popup-attach
     map (list "a" (append (list "Action" command :c-u "Prefix") properties))
     :persistent persistent)
    map))

(ert-deftest keymap-popup-input-native-prefix-control ()
  (keymap-popup-input--with-source
    (let ((observed nil) (map (make-sparse-keymap)))
      (keymap-set map "a" (lambda (arg) (interactive "P") (push arg observed)))
      (use-local-map map)
      (execute-kbd-macro (kbd "C-u 3 a C-u a C-u - 3 a C-u C-u a a"))
      (should (equal (reverse observed) '(3 (4) -3 (16) nil))))))

(defun keymap-popup-input--repeat-prefix (persistent)
  "Check repeated accepted prefixes with PERSISTENT or stay-open dispatch."
  (keymap-popup-input--with-source
    (let* ((observed nil)
           (command (lambda (arg) (interactive "P") (push arg observed))))
      (keymap-popup (apply #'keymap-popup-input--suffix-map command persistent
                          (unless persistent '(:stay-open t))))
      (execute-kbd-macro (kbd "C-u 3 a"))
      (keymap-popup-input--settled-p)
      (execute-kbd-macro (kbd "C-u a C-u - 3 a C-u C-u a a"))
      (should (equal (reverse observed) '(3 (4) -3 (16) nil)))
      (keymap-popup-input--settled-p)
      (execute-kbd-macro (kbd "q"))
      (keymap-popup-input--clean-p))))

(ert-deftest keymap-popup-input-stay-open-consumes-prefix ()
  (keymap-popup-input--repeat-prefix nil))

(ert-deftest keymap-popup-input-persistent-consumes-prefix ()
  (keymap-popup-input--repeat-prefix t))

(ert-deftest keymap-popup-input-inapt-retains-negative-prefix ()
  (dolist (persistent '(nil t))
    (keymap-popup-input--with-source
      (let* ((observed nil)
             (command (lambda (arg) (interactive "P") (push arg observed)))
             (map (keymap-popup-input--suffix-map command persistent :stay-open t)))
        (keymap-set map "b" command)
        (keymap-popup-attach
         map (list "a" (list "Action" command :stay-open t
                             :inapt-if (lambda () t))
                   "b" (list "Accept" command :stay-open t :c-u "Prefix"))
         :persistent persistent)
        (keymap-popup map)
        ;; Keep the refused and accepted keys in one native macro: starting
        ;; another execute-kbd-macro resets the pending native prefix.
        (execute-kbd-macro (kbd "C-u - 3 a b"))
        (should (equal observed '(-3)))
        (keymap-popup-input--settled-p)
        (execute-kbd-macro (kbd "q"))
        (keymap-popup-input--clean-p)))))

(defun keymap-popup-input--failing-prefix (persistent condition)
  "Check prefix settlement when PERSISTENT dispatch signals CONDITION."
  (keymap-popup-input--with-source
    (let* ((observed nil) (fail t)
           (command (lambda (arg)
                      (interactive "P")
                      (push arg observed)
                      (when fail (signal condition '("Input test"))))))
      (keymap-popup (apply #'keymap-popup-input--suffix-map command persistent
                          (unless persistent '(:stay-open t))))
      (condition-case nil
          (execute-kbd-macro (kbd "C-u 3 a"))
        ((error quit) nil))
      ;; A command error need not run post-command-hook.
      (should (equal observed '(3)))
      (keymap-popup-input--settled-p)
      (setq fail nil)
      (execute-kbd-macro (kbd "C-u a"))
      (should (equal observed '((4) 3)))
      (execute-kbd-macro (kbd "q"))
      (keymap-popup-input--clean-p))))

(ert-deftest keymap-popup-input-stay-open-error-consumes-prefix ()
  (keymap-popup-input--failing-prefix nil 'error))

(ert-deftest keymap-popup-input-stay-open-quit-consumes-prefix ()
  (keymap-popup-input--failing-prefix nil 'quit))

(ert-deftest keymap-popup-input-persistent-error-consumes-prefix ()
  (keymap-popup-input--failing-prefix t 'error))

(ert-deftest keymap-popup-input-persistent-quit-consumes-prefix ()
  (keymap-popup-input--failing-prefix t 'quit))

(defun keymap-popup-input--submenu-fallthrough (second-key)
  "Exercise a parent-only submenu SECOND-KEY through native map teardown."
  (keymap-popup-input--with-source
    (let* ((child (keymap-popup-input--suffix-map #'ignore nil))
           (other (keymap-popup-input--suffix-map #'ignore nil))
           (root (make-sparse-keymap)))
      (keymap-set root "s" (lambda () (interactive) (keymap-popup child)))
      (keymap-set root "d" (lambda () (interactive) (keymap-popup other)))
      (keymap-popup-attach root
                          (list "s" (list "Child" :keymap child)
                                "d" (list "Other" :keymap other)))
      (keymap-popup root)
      (execute-kbd-macro (kbd "s"))
      (let ((old (get-buffer keymap-popup--buffer-name)))
        (execute-kbd-macro (kbd second-key))
        (let ((buf (get-buffer keymap-popup--buffer-name)))
          (should (buffer-live-p buf))
          (should-not (eq old buf))
          (should (eq (keymap-popup--active-get buf :keymap)
                      (if (equal second-key "s") child other)))
          (should-not (keymap-popup--session-get buf :stack))))
      (execute-kbd-macro (kbd "q"))
      (keymap-popup-input--clean-p))))

(ert-deftest keymap-popup-input-repeated-parent-submenu ()
  (keymap-popup-input--submenu-fallthrough "s"))

(ert-deftest keymap-popup-input-different-parent-submenu ()
  (keymap-popup-input--submenu-fallthrough "d"))

(ert-deftest keymap-popup-input-child-binding-wins ()
  (keymap-popup-input--with-source
    (let ((child (make-sparse-keymap)) (root (make-sparse-keymap))
          (called nil))
      (keymap-set child "s" (lambda () (interactive) (setq called t)))
      (keymap-set root "s" (lambda () (interactive) (keymap-popup child)))
      (keymap-popup-attach
       child (list "s" (list "Child action" (keymap-lookup child "s"))))
      (keymap-popup-attach root (list "s" (list "Child" :keymap child)))
      (keymap-popup root)
      (execute-kbd-macro (kbd "s s"))
      (should called)
      (keymap-popup-input--clean-p))))

(ert-deftest keymap-popup-input-preserves-native-command-state ()
  (dolist (persistent '(nil t))
    (keymap-popup-input--with-source
      (let* ((observed nil) (identity nil)
             (command (lambda (arg)
                        (interactive "P")
                        (setq identity this-command)
                        (push arg observed)
                        (setq prefix-arg 7)))
             (next (lambda (arg) (interactive "P") (push arg observed)))
             (map (keymap-popup-input--suffix-map command persistent :stay-open t)))
        (keymap-set map "b" next)
        (keymap-popup-attach
         map (list "a" (list "Action" command :stay-open t :c-u "Prefix")
                   "b" (list "Next" next :stay-open t))
         :persistent persistent)
        (keymap-popup map)
        (execute-kbd-macro (kbd "C-u a b"))
        (should (equal observed '(7 (4))))
        (should (eq identity command))
        (should (eq last-command next))
        (execute-kbd-macro (kbd "q"))
        (keymap-popup-input--clean-p)))))

(ert-deftest keymap-popup-input-expired-submenu-uses-live-binding ()
  (keymap-popup-input--with-source
    (let ((root (make-sparse-keymap))
          (child (keymap-popup-input--suffix-map #'ignore nil))
          (called nil))
      (keymap-set root "s" (lambda () (interactive) (keymap-popup child)))
      (keymap-popup-attach root (list "s" (list "Child" :keymap child)))
      (keymap-popup root)
      (execute-kbd-macro (kbd "s"))
      (keymap-set root "s" (lambda () (interactive) (setq called t)))
      (execute-kbd-macro (kbd "s"))
      (should called)
      (keymap-popup-input--clean-p))))

(defun keymap-popup-input--drawn-entry (persistent)
  "Check that a press with PERSISTENT (else `:stay-open') dispatch acts on
its row as drawn: settling the prefix runs no label again before the
command, which sees the label its row showed and the label's properties.
A row hidden when the popup drew stays out of the repaint, its label
not run, though it would show now."
  (keymap-popup-input--with-source
    (let* ((draws 0) (at-press nil) (seen nil) (later nil) (later-draws 0)
           (label (lambda () (propertize (format "Row %d" (cl-incf draws)) 'input-test-datum draws)))
           (command (lambda () (interactive) (push (list draws (keymap-popup-entry) later-draws) seen)))
           ;; The hidden row would show from the press of "a" on.
           (note (lambda () (setq at-press draws)
                   (when (equal (key-description (this-command-keys-vector)) "a") (setq later t))))
           (map (make-sparse-keymap)))
      (keymap-set map "a" command)
      (keymap-set map "b" #'ignore)
      (keymap-popup-attach
       map (list "a" (append (list label command :c-u "Prefix") (unless persistent '(:stay-open t)))
                 "b" (list (lambda () (cl-incf later-draws) "Later") #'ignore :if (lambda () later)))
       :persistent persistent)
      (keymap-popup map)
      (add-hook 'pre-command-hook note -100)
      (execute-kbd-macro (kbd "C-u a"))
      (remove-hook 'pre-command-hook note)
      (pcase-let ((`((,draws-then ,entry ,later-then)) seen))
        ;; No label ran between the key and its command.
        (should (= draws-then at-press))
        (should (= later-then 0))
        (should (equal (plist-get entry :key) "a"))
        (should (equal (substring-no-properties (plist-get entry :description)) (format "Row %d" at-press)))
        (should (eql (get-text-property 0 'input-test-datum (plist-get entry :description)) at-press)))
      (keymap-popup-input--settled-p)
      ;; Outside a popup's dispatch there is no entry.
      (should-not (keymap-popup-entry))
      (execute-kbd-macro (kbd "q"))
      (keymap-popup-input--clean-p))))

(ert-deftest keymap-popup-input-persistent-press-acts-on-drawn-entry ()
  (keymap-popup-input--drawn-entry t))

(ert-deftest keymap-popup-input-stay-open-press-acts-on-drawn-entry ()
  (keymap-popup-input--drawn-entry nil))

(ert-deftest keymap-popup-input-closing-press-sees-drawn-entry ()
  "A suffix that closes its popup before it runs still sees the row."
  (keymap-popup-input--with-source
    (let* ((seen nil)
           (command (lambda () (interactive) (push (keymap-popup-entry) seen)))
           (map (make-sparse-keymap)))
      (keymap-set map "a" command)
      (keymap-popup-attach map (list "a" (list (lambda () "Shown") command)))
      (keymap-popup map)
      (execute-kbd-macro (kbd "a"))
      (should (equal (substring-no-properties (plist-get (car seen) :description)) "Shown"))
      (keymap-popup-input--clean-p)
      (funcall command)
      (should-not (car seen)))))

(ert-deftest keymap-popup-input-reading-press-keeps-drawn-entry ()
  "A row whose command reads the minibuffer still sees its row after the
read; the minibuffer's own commands see none."
  (keymap-popup-input--with-source
    (let* ((seen nil) (inner nil)
           (note (lambda () (when (minibufferp) (push (keymap-popup-entry) inner))))
           (command (lambda () (interactive)
                      (add-hook 'pre-command-hook note)
                      (unwind-protect (read-string "Text: ")
                        (remove-hook 'pre-command-hook note))
                      (push (keymap-popup-entry) seen)))
           (map (make-sparse-keymap)))
      (keymap-set map "a" command)
      (keymap-popup-attach map (list "a" (list "Read" command :stay-open t)))
      (keymap-popup map)
      (execute-kbd-macro (kbd "a x y RET"))
      (should (equal (plist-get (car seen) :key) "a"))
      (should inner)
      (should-not (seq-some #'identity inner))
      (execute-kbd-macro (kbd "q"))
      (keymap-popup-input--clean-p))))

(defun keymap-popup-input--check-exit-hook (command drive &optional stay-open)
  "Open a popup for COMMAND and check its exit hook after DRIVE.
With STAY-OPEN, exercise the non-exiting action before DRIVE."
  (keymap-popup-input--with-source
    (let* ((origin-buffer (current-buffer))
           (origin-window (selected-window))
           (keymap-popup-backend #'keymap-popup-backend-side-window)
           (window nil)
           (observed nil)
           (record (lambda ()
                     (push (list (current-buffer) (selected-window)
                                 (window-live-p window))
                           observed))))
      (unwind-protect
          (progn
            (when (boundp 'keymap-popup-exit-hook)
              (add-hook 'keymap-popup-exit-hook record))
            (keymap-popup (keymap-popup-input--suffix-map command nil
                                                         :stay-open stay-open))
            (setq window (get-buffer-window (get-buffer keymap-popup--buffer-name)))
            (should (window-live-p window))
            (should-not (eq window origin-window))
            (should-not observed)
            (when stay-open
              (execute-kbd-macro (kbd "a"))
              (should (window-live-p window))
              (should-not observed))
            (funcall drive)
            (should-not (window-live-p window))
            (should (boundp 'keymap-popup-exit-hook))
            (should (equal observed (list (list origin-buffer origin-window nil)))))
        (when (boundp 'keymap-popup-exit-hook)
          (remove-hook 'keymap-popup-exit-hook record))))))

(ert-deftest keymap-popup-exit-hook-selection ()
  (let ((invocations 0))
    (keymap-popup-input--check-exit-hook
     (lambda () (interactive) (cl-incf invocations))
     (lambda ()
       (execute-kbd-macro (kbd "a"))
       (should (= invocations 1))))))

(ert-deftest keymap-popup-exit-hook-quit-key ()
  (keymap-popup-input--check-exit-hook
   #'ignore
   (lambda ()
     (with-temp-buffer
       (let ((other (split-window-right)))
         (set-window-buffer other (current-buffer))
         (select-window other)
         (execute-kbd-macro (kbd "q")))))))

(ert-deftest keymap-popup-exit-hook-keyboard-quit ()
  (keymap-popup-input--check-exit-hook
   #'ignore
   (lambda ()
     (condition-case nil (execute-kbd-macro (kbd "C-g")) (quit nil)))))

(ert-deftest keymap-popup-exit-hook-command-error ()
  (keymap-popup-input--check-exit-hook
   (lambda () (interactive) (user-error "Exit hook test command failed"))
   (lambda ()
     (should (equal (should-error (execute-kbd-macro (kbd "a")) :type 'user-error)
                    '(user-error "Exit hook test command failed"))))))

(ert-deftest keymap-popup-exit-hook-stay-open ()
  (let ((invocations 0))
    (keymap-popup-input--check-exit-hook
     (lambda () (interactive) (cl-incf invocations))
     (lambda ()
       (should (= invocations 1))
       (execute-kbd-macro (kbd "q")))
     t)))

(defun keymap-popup-input--check-deleted-display (delete)
  "Run DELETE on the popup display and reject a key resolved from that popup."
  (keymap-popup-input--with-source
    (let* ((configuration (current-window-configuration))
           (keymap-popup-backend #'keymap-popup-backend-side-window)
           (hits 0)
           (exits 0)
           (keymap-popup-exit-hook (list (lambda () (cl-incf exits)))))
      (keymap-popup
       (keymap-popup-input--suffix-map
        (lambda () (interactive) (cl-incf hits)) nil :stay-open t))
      (let ((window (get-buffer-window (get-buffer keymap-popup--buffer-name))))
        (should (window-live-p window))
        (funcall delete window configuration)
        (should-not (window-live-p window))
        (execute-kbd-macro (kbd "a"))
        (should (= hits 0))
        (should (= exits 1))
        (keymap-popup-input--clean-p)))))

(ert-deftest keymap-popup-input-display-deleted-by-delete-other-windows ()
  (keymap-popup-input--check-deleted-display
   (lambda (_window _configuration) (delete-other-windows))))

(ert-deftest keymap-popup-input-display-deleted-by-configuration-restore ()
  (keymap-popup-input--check-deleted-display
   (lambda (_window configuration) (set-window-configuration configuration))))

(ert-deftest keymap-popup-input-display-deleted-with-parameters-ignored ()
  (keymap-popup-input--check-deleted-display
   (lambda (window _configuration)
     (let ((ignore-window-parameters t)) (delete-window window)))))

(provide 'keymap-popup-input-tests)
;;; keymap-popup-input-tests.el ends here
