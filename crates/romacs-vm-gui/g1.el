(progn
  (defface g1-face '((t (:foreground "#ff0000"))) "Declared G1 red")
  (defface mode-line '((t (:foreground "#ffffff" :background "#303030"))) "Declared G1 band")
  (set-buffer (get-buffer-create "g1"))
  (insert "Ωμέγα Hello\n")
  (put-text-property 7 12 'face 'g1-face)
  (set-window-buffer nil (current-buffer))
  (setq mode-line-format '(" %b " (:eval (progn (set-window-vscroll nil 7 t) (format "v=%S" (window-vscroll nil t)))))))
