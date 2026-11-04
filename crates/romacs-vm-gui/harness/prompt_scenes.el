;;; prompt_scenes.el --- read-from-minibuffer menus, for popup_live.py --prompts  -*- lexical-binding: t -*-

;; The same three prompts for Romacs and for GNU: notmuch's jump menu (the
;; real notmuch-jump, from the system's site-lisp), read-multiple-choice
;; and a long y-or-n-p.  Each lays itself out for a minibuffer as wide as
;; the frame, as GNU's is.

(add-to-list 'load-path "/usr/share/emacs/site-lisp")
;; Keyboard prompts, not GUI dialogs, when run from a timer (GNU's
;; `use-dialog-box-p' holds then).
(setq use-dialog-box nil)

(defun prompt-scene-jump ()
  (interactive)
  (require 'notmuch-jump)
  (notmuch-jump '(("i" "Inbox" ignore) ("u" "Unread" ignore) ("f" "Flagged" ignore) ("s" "Sent" ignore)
                  ("d" "Drafts" ignore) ("a" "All mail" ignore) ("t" "Today" ignore) ("w" "This week" ignore)
                  ("m" "Mailing lists" ignore) ("r" "Recently replied" ignore) ("p" "Projects" ignore)
                  ("l" "Later" ignore))
                "Jump to: "))

(defun prompt-scene-rmc ()
  (interactive)
  (read-multiple-choice "The file has changed on disk since it was visited. Overwrite it"
                        '((?y "yes, overwrite") (?n "no, keep the disk version") (?d "diff the two")
                          (?s "save under another name") (?q "quit and leave it"))))

(defun prompt-scene-yn ()
  (interactive)
  (y-or-n-p "The buffer notes.org has unsaved changes and its file was changed by another program since you visited it; save anyway? "))

;; A prompt-only minibuffer that grows as the user types and shrinks as
;; they delete; a long outer prompt around a short nested one; a prompt
;; drawn at twice the height.
(defun prompt-scene-type ()
  (interactive)
  (read-string (concat (make-string 80 ?p) ": ")))

(defun prompt-scene-inner ()
  (interactive)
  (read-string "inner: "))

(defun prompt-scene-nested ()
  (interactive)
  (let ((minibuffer-local-map (let ((map (make-sparse-keymap)))
                                (set-keymap-parent map minibuffer-local-map)
                                (define-key map (kbd "<f11>") #'prompt-scene-inner)
                                map))
        (enable-recursive-minibuffers t))
    (read-string (concat (make-string 92 ?o) ": "))))

(defun prompt-scene-tall ()
  (interactive)
  (read-from-minibuffer (propertize (concat (make-string 47 ?s) ": ") 'face '(:height 2.0))))

(global-set-key (kbd "<f8>") #'prompt-scene-type)
(global-set-key (kbd "<f9>") #'prompt-scene-nested)
(global-set-key (kbd "<f10>") #'prompt-scene-tall)
(global-set-key (kbd "<f5>") #'prompt-scene-jump)
(global-set-key (kbd "<f6>") #'prompt-scene-rmc)
(global-set-key (kbd "<f7>") #'prompt-scene-yn)
