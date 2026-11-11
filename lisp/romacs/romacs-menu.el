;;; romacs-menu.el --- menu.c's popup menus and dialogs over tmm  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; GNU's menu.c pops `x-popup-menu' and `x-popup-dialog' up through the
;; frame's toolkit (GTK on pinned GNU pgtk) or, on a text terminal, its
;; own text menus.  Romacs's window has no popup menu yet, so both are
;; GNU's own text-menu machinery, tmm's `tmm-prompt', with GNU's calling
;; contract (VM2-MENU1): POSITION an event, t or ((XOFFSET YOFFSET)
;; WINDOW), nil to show nothing; MENU a keymap, a list of keymaps (each a
;; pane) or (TITLE (PANE-TITLE ITEM...)...); the value the list of events
;; leading to the chosen item of a keymap menu, or the chosen item's VALUE
;; of a pane menu.  Getting rid of the menu quits, except that a menu a
;; mouse event popped returns nil.  `menu-bar-open' (F10) and `popup-menu'
;; reach these on Romacs's `pgtk' frame.

;;; Code:

(defun romacs-menu--leaf (path)
  "A command standing for the menu item at event PATH, which it returns."
  (lambda () (interactive) path))

(defun romacs-menu--keymap (map prefix)
  "MAP, a menu keymap, with each item's command a `romacs-menu--leaf'
for its events after PREFIX, and each submenu likewise."
  (let ((items nil))
    (map-keymap
     (lambda (event binding)
       (let ((path (append prefix (list event))))
         (push (cons event (romacs-menu--item binding path)) items)))
     map)
    (let ((prompt (keymap-prompt map)))
      `(keymap ,@(and prompt (list prompt)) ,@(nreverse items)))))

(defun romacs-menu--target (definition path)
  "DEFINITION as tmm should see it at event PATH."
  (let ((keymap (and definition (keymapp definition) (indirect-function definition))))
    (cond (keymap (romacs-menu--keymap keymap path))
          ((and definition (not (eq definition 'undefined))) (romacs-menu--leaf path))
          (t definition))))

(defun romacs-menu--item (binding path)
  "BINDING, a menu keymap entry at event PATH, for `romacs-menu--keymap'."
  (cond
   ((eq (car-safe binding) 'menu-item)
    ;; (menu-item NAME DEFINITION . PROPS): the filter applies here.
    (let* ((props (nthcdr 3 binding))
           (filter (plist-get props :filter))
           (definition (if filter (funcall filter (nth 2 binding)) (nth 2 binding))))
      `(menu-item ,(nth 1 binding) ,(romacs-menu--target definition path)
                  ,@(romacs-menu--explicit (if filter (romacs-menu--without props :filter) props)))))
   ((and (stringp (car-safe binding)) (stringp (car-safe (cdr binding))))
    ;; (NAME HELP . DEFINITION)
    `(,(car binding) ,(cadr binding) . ,(romacs-menu--target (cddr binding) path)))
   ((stringp (car-safe binding))
    `(,(car binding) . ,(romacs-menu--target (cdr binding) path)))
   (t binding)))

(defun romacs-menu--explicit (props)
  "PROPS with a literal nil `:visible' or `:enable' as a form, so tmm,
which takes a nil property for an absent one, hides or disables the
item as menu.c does."
  (let (out)
    (while props
      (push (car props) out)
      (push (if (and (memq (car props) '(:visible :enable)) (null (cadr props))) ''nil (cadr props)) out)
      (setq props (cddr props)))
    (nreverse out)))

(defun romacs-menu--without (plist key)
  "PLIST without KEY's entry."
  (let (out)
    (while plist
      (unless (eq (car plist) key) (setq out (cons (cadr plist) (cons (car plist) out))))
      (setq plist (cddr plist)))
    (nreverse out)))

(defun romacs-menu--mouse-p (position)
  "Whether POSITION is a mouse button or touch screen event: an event
with a position, not ((XOFFSET YOFFSET) WINDOW)."
  (and (consp position) (symbolp (car position)) (posnp (car-safe (cdr position)))))

(defun x-popup-menu (position menu)
  "Pop up a deck-of-cards menu and return user's selection.
POSITION is a mouse button event, a list ((XOFFSET YOFFSET) WINDOW), or
t for the mouse position; nil shows nothing.  MENU is a keymap, a list
of keymaps, each a pane, or (TITLE PANE...), each PANE (TITLE ITEM...)
with each ITEM (STRING . VALUE) or a nonselectable string.  For a keymap
menu the value is the list of events leading to the chosen item, which
is not executed; for a pane menu the chosen ITEM's VALUE.  Getting rid of
the menu quits, unless a mouse event popped it, when the value is nil.
Romacs shows the menu as GNU's text menus, `tmm-prompt'."
  (when position
    (unless (or (eq position t) (consp position))
      (signal 'wrong-type-argument (list 'consp position)))
    (let ((menu (cond ((keymapp menu) (romacs-menu--keymap menu nil))
                      ((and (consp menu) (keymapp (car menu)))
                       (if (cdr menu)
                           ;; Each keymap a pane; its events start the value.
                           `(keymap ,@(let ((pane 0))
                                        (mapcar (lambda (map)
                                                  (setq pane (1+ pane))
                                                  (cons (intern (format "pane-%d" pane))
                                                        (cons (or (keymap-prompt map) (format "Pane %d" pane))
                                                              (romacs-menu--keymap map nil))))
                                                menu)))
                         (romacs-menu--keymap (car menu) nil)))
                      (t (unless (stringp (car-safe menu))
                           (signal 'wrong-type-argument (list 'stringp (car-safe menu))))
                         menu))))
      (condition-case nil
          (let ((choice (tmm-prompt menu nil nil t)))
            ;; A keymap menu's choice is its leaf, or the leaf's value when
            ;; tmm ran it after `^' went back up, which drops NO-EXECUTE.
            (if (keymapp menu)
                (cond ((functionp choice) (funcall choice)) ((consp choice) choice))
              choice))
        (quit (if (romacs-menu--mouse-p position) nil (signal 'quit nil)))))))

(defun x-popup-dialog (position contents &optional _header)
  "Pop up a dialog box and return user's selection.
POSITION is a mouse button event, a window or frame, or t.  CONTENTS is
\(DIALOG ITEM...), each ITEM (STRING . VALUE), a nonselectable string, or
nil.  The value is the chosen ITEM's VALUE; getting rid of the dialog
quits.  Romacs's window has no dialog boxes, so the dialog is a menu, as
GNU's emulate_dialog_with_menu shows it on a frame without them."
  (let ((window (cond ((or (eq position t) (memq (car-safe position) '(menu-bar tab-bar tool-bar)))
                       (selected-window))
                      ((consp position)
                       (if (consp (car position)) (cadr position) (car (cadr position))))
                      ((or (windowp position) (framep position)) position))))
    (unless (or (framep window) (window-live-p window))
      (signal 'wrong-type-argument (list 'window-live-p window)))
    (let ((frame (if (framep window) window (window-frame window))))
      (x-popup-menu (list (list (/ (frame-native-width frame) 2) (/ (frame-native-height frame) 2)) frame)
                    (list (car contents) (cons (car contents) (delq nil (copy-sequence (cdr contents)))))))))

(provide 'romacs-menu)
;;; romacs-menu.el ends here
