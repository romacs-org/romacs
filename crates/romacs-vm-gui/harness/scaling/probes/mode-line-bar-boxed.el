;;; mode-line-bar-boxed.el --- scaling probe  -*- lexical-binding: t -*-

;; The same bar with a box, and a context window's line folded in: the
;; stretches of its box run, and the context's background swept with the
;; characters, never a walk over the segments for each one.  N stays small
;; enough to fit: a bar too full drops the context and clips the pills.
;; The body refuses a run whose bar, at its widest after the last key, has
;; lost the context, which follows every pill, so the pills are shown too.
(scaling-probe mode-line-bar-boxed :gui t :n 6 :steps 20
  :setup (progn
           (require 'romacs-mode-line)
           (romacs-mode-line-mode 1)
           (set-face-attribute 'mode-line nil :box '(:line-width 1 :color "#808080"))
           (dotimes (i n)
             (let ((face (intern (format "bar-boxed-face-%d" i))))
               (make-face face)
               (set-face-attribute face nil :background (format "#%02x3040" (% (* 5 i) 256))
                                   :foreground (format "#e0%02xa0" (% (* 3 i) 256)))
               (eval `(romacs-mode-line-define-segment ,(intern (format "bar-boxed-%d" i)) "A probe."
                        :pill ,face :pill-text ,face
                        :format (" " (:propertize "%*" romacs-icon ((?* . modified) (?- . saved))) " "))
                     t))
             (push (intern (format "bar-boxed-%d" i)) romacs-mode-line-right))
           (romacs-mode-line-rebuild)
           (let ((buffer (scaling--lines-buffer "scaling-bar-boxed" 50)))
             (switch-to-buffer buffer)
             (with-selected-window (split-window-below)
               (switch-to-buffer (get-buffer-create "scaling-bar-context"))
               (setq-local mode-line-format '("q  " "context")))
             buffer))
  :body (progn
          (with-current-buffer it (insert "x"))
          (when (= i 19)
            (let ((shown (romacs-status-bar-string)))
              (unless (string-search "context" shown)
                (scaling--mark (list :scaling "unfit" :shown shown))
                (kill-emacs 3))))))
