;;; font-objects-opened.el --- scaling probe  -*- lexical-binding: t -*-

;; The frame's font entity at N pixel sizes, each opened once by the
;; setup; the counted work opens all N again, twenty times.  The display
;; keeps one font-object per font opened (font.c font_open_entity's object
;; list), indexed by font and size, so each repeat is a lookup that
;; returns the object opened before, and the work is linear in N.  The
;; entity is resolved once, so a repeat does no name matching: what it
;; counts is the store's lookup, a few thousand instructions, and a lookup
;; that grew steeply with the store shows in the exponent.  At these sizes
;; a short scan of the store would not: opening a font costs over a
;; hundred million instructions, so the setup's N opens bound the
;; session's time on a loaded host and N stays small.
(scaling-probe font-objects-opened :gui t :n 25 :steps 1
  :setup (let ((entity (find-font (font-spec :family (face-attribute 'default :family))))
               (sizes (number-sequence 6 (+ 5 n))))
           (dolist (size sizes) (open-font entity size))
           (list (switch-to-buffer (get-buffer-create "scaling-font-objects")) entity sizes))
  ;; The step paints (a character typed in the shown buffer), as a GUI
  ;; probe's must.
  :body (pcase-let ((`(,buffer ,entity ,sizes) it))
          (dotimes (_ 20) (dolist (size sizes) (open-font entity size)))
          (with-current-buffer buffer (insert "x"))))
