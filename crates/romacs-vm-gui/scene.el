(progn
  (defface g1a-red '((t (:foreground "#ff0000"))) "G1a declared red face.")
  (set-buffer (get-buffer-create "*g1a*"))
  (insert "Ωμέγα Hello\n")
  (put-text-property 7 12 'face 'g1a-red)
  (set-window-buffer nil (current-buffer)))
