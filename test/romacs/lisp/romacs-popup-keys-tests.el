;;; romacs-popup-keys-tests.el --- Menus that take keys -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-popup)

(defvar romacs-popup-keys-tests--ran nil)

(defun romacs-popup-keys-tests--down ()
  (interactive)
  (forward-line 1)
  (push (list (current-buffer) (selected-window) (point)) romacs-popup-keys-tests--ran))

(defvar-keymap romacs-popup-keys-tests-map
  :suppress t
  "n" #'romacs-popup-keys-tests--down)

(defmacro romacs-popup-keys-tests--with (&rest body)
  "BODY with `menu', a buffer of lines shown in `window' (a menu's stand-in),
taking keys, and `home', the window the user was in."
  (declare (indent 0))
  `(let ((menu (generate-new-buffer " *menu*"))
         (home (selected-window))
         (romacs-popup-keys-tests--ran nil)
         (romacs-popup--keys-buffer nil)
         (romacs-popup--keys-exit nil)
         (overriding-terminal-local-map overriding-terminal-local-map)
         (hidden nil))
     (with-current-buffer menu
       (insert "one\ntwo\nthree\n")
       (goto-char (point-min))
       (use-local-map romacs-popup-keys-tests-map))
     (let ((window (split-window home)))
       (set-window-buffer window menu)
       (unwind-protect
           (cl-letf (((symbol-function 'romacs-popup-hide) (lambda (buffer) (push buffer hidden))))
             (romacs-popup-take-keys menu)
             ,@body)
         (when romacs-popup--keys-exit (funcall romacs-popup--keys-exit))
         (delete-window window)
         (kill-buffer menu)))))

(ert-deftest romacs-popup-keys-run-in-the-menu ()
  (romacs-popup-keys-tests--with
    (should (memq romacs-popup--keys-map (cdr overriding-terminal-local-map)))
    (romacs-popup--menu-key (kbd "n"))
    ;; The menu's own command, in its buffer and window: point moves there.
    (should (equal (car romacs-popup-keys-tests--ran) (list menu window 5)))
    (should (= (window-point window) 5))
    (should (eq (selected-window) home))
    (should (eq romacs-popup--keys-buffer menu))))

(ert-deftest romacs-popup-keys-another-key-hides-and-runs ()
  (romacs-popup-keys-tests--with
    (let ((unread-command-events nil))
      (romacs-popup--menu-key (kbd "C-x"))
      (should (equal unread-command-events (listify-key-sequence (kbd "C-x"))))
      (should (equal hidden (list menu)))
      (should-not romacs-popup--keys-buffer)
      (should-not (memq romacs-popup--keys-map (cdr overriding-terminal-local-map))))))

(ert-deftest romacs-popup-keys-a-remapped-key-is-the-menu-s ()
  ;; The menu's map remaps `self-insert-command': a letter stays in it.
  (romacs-popup-keys-tests--with
    (let ((unread-command-events nil))
      (should (eq (romacs-popup--menu-command (kbd "z") romacs-popup-keys-tests-map) 'undefined))
      (should (null unread-command-events)))))

(ert-deftest romacs-popup-keys-a-remap-whatever-the-user-s-buffer-binds ()
  ;; Typed from a buffer whose own map binds the letter (a chat's
  ;; transcript): the menu remaps the global command, so it stays the menu's.
  (with-temp-buffer
    (use-local-map (let ((map (make-sparse-keymap))) (define-key map "z" #'forward-char) map))
    (should (eq (romacs-popup--menu-command (kbd "z") romacs-popup-keys-tests-map) 'undefined))))

(ert-deftest romacs-popup-keys-a-function-key-reads-as-its-key ()
  ;; The menu reads raw events: M-<return> is bound as M-RET, <tab> as TAB.
  (let ((map (make-sparse-keymap)))
    (define-key map (kbd "M-RET") #'ignore)
    (define-key map (kbd "TAB") #'forward-char)
    (should (eq (romacs-popup--menu-command [M-return] map) #'ignore))
    (should (eq (romacs-popup--menu-command [tab] map) #'forward-char))
    (should-not (romacs-popup--menu-command [f5] map))))

(ert-deftest romacs-popup-keys-a-shown-buffer-goes-where-the-user-was ()
  ;; A command of the menu shows another buffer: in the user's window, the
  ;; menu hidden; its own buffer stays in the menu.
  (romacs-popup-keys-tests--with
    (let ((other (generate-new-buffer " *other*")) (where nil))
      (unwind-protect
          (progn
            (cl-letf (((symbol-function 'romacs-popup-keys-tests--down)
                       (lambda () (interactive) (setq where (display-buffer menu)))))
              (romacs-popup--menu-key (kbd "n")))
            (should (eq where window))
            (should-not hidden)
            (cl-letf (((symbol-function 'romacs-popup-keys-tests--down)
                       (lambda () (interactive) (pop-to-buffer other) (setq where (selected-window)))))
              (romacs-popup--menu-key (kbd "n")))
            (should (eq where home))
            (should (eq (window-buffer home) other))
            (should (equal hidden (list menu)))
            (should-not romacs-popup--keys-buffer))
        (kill-buffer other)))))

(ert-deftest romacs-popup-keys-quit-window-hides ()
  (romacs-popup-keys-tests--with
    (with-current-buffer menu
      (use-local-map (let ((map (make-sparse-keymap))) (define-key map "q" #'quit-window) map)))
    (romacs-popup--menu-key (kbd "q"))
    (should (equal hidden (list menu)))
    (should (eq (window-buffer window) menu))
    (should-not romacs-popup--keys-buffer)))

(ert-deftest romacs-popup-keys-its-post-command-hook-follows-each-key ()
  (romacs-popup-keys-tests--with
    (let ((seen nil))
      (with-current-buffer menu
        (add-hook 'post-command-hook (lambda () (push (point) seen)) nil t))
      (romacs-popup--menu-key (kbd "n"))
      (romacs-popup--menu-key (kbd "n"))
      (should (equal seen '(9 5))))))

(ert-deftest romacs-popup-keys-show-no-hints ()
  ;; The menu shows its keys itself: its map is no transient map's hints.
  (let ((romacs-popup--hint-maps (make-hash-table :test 'eq)))
    (advice-add 'set-transient-map :around #'romacs-popup--set-transient-map)
    (unwind-protect
        (romacs-popup-keys-tests--with
          (should-not (gethash romacs-popup--keys-map romacs-popup--hint-maps)))
      (advice-remove 'set-transient-map #'romacs-popup--set-transient-map))))

(ert-deftest romacs-popup-keys-one-map-however-often-asked ()
  (romacs-popup-keys-tests--with
    (romacs-popup-take-keys menu)
    (should (= (cl-count romacs-popup--keys-map (cdr overriding-terminal-local-map)) 1))))

(provide 'romacs-popup-keys-tests)
;;; romacs-popup-keys-tests.el ends here
