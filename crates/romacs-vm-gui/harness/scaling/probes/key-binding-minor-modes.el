;;; key-binding-minor-modes.el --- scaling probe  -*- lexical-binding: t -*-

;; A key's lookup with N minor modes on, half of them also in
;; `minor-mode-overriding-map-alist': the active keymaps are found by one
;; pass over the alists, each mode looked up in the overriding alist
;; without a scan of it, and a prefix every mode binds composes its N
;; bindings once.
(scaling-probe key-binding-minor-modes :n 100
  :setup (dotimes (i n)
           (let ((map (make-sparse-keymap)) (mode (intern (format "scaling-key-mode-%d" i))))
             (define-key map (vector 'f12 (+ #x1000 i)) #'forward-char)
             (set mode t)
             (push (cons mode map) minor-mode-map-alist)
             (when (= (% i 2) 0)
               (push (cons mode (make-sparse-keymap)) minor-mode-overriding-map-alist))))
  :body (dotimes (_ 50)
          (key-binding "a")
          (minor-mode-key-binding "a")
          (key-binding [f12])
          (minor-mode-key-binding [f12])
          (current-active-maps t)))
