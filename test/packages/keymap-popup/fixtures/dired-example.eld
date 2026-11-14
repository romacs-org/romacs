(keymap-popup-annotate dired-mode-map
  :popup-key "C-c C-p"
  :persistent t
  :description (lambda ()
                 (format "Dired: %s" (abbreviate-file-name default-directory)))
  :group "File"
  dired-find-file "Open"
  dired-do-copy "Copy"
  dired-do-rename "Rename"
  dired-do-delete "Delete"
  :group "Navigate"
  dired-up-directory "Up"
  dired-previous-line "Prev"
  dired-next-line "Next"
  :group "More"
  "M" ("Mark operations" :keymap my-dired-mark-map))
