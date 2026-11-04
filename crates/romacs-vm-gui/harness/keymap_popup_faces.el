; keymap-popup's rows and faces for keymap_popup_fixture.el's menu in pinned GNU X11 (its side window), as
; popup_live.py --layer compares Romacs's panel with them.  Regenerate with
; crates/romacs-vm/tests/child_frames_oracle.py --probe THIS --output keymap_popup_faces.gnu.
(progn (require 'json) (add-to-list 'load-path (expand-file-name "../../../rlisp/emacs-lisp" (file-name-directory (getenv "PROBE")))) (load (expand-file-name "keymap_popup_fixture.el" (file-name-directory (getenv "PROBE"))) nil t) (setq keymap-popup-backend #'keymap-popup-backend-side-window) (featurep 'keymap_popup_fixture))
(progn (keymap-popup keymap-popup-fixture-map) (redisplay t) (bufferp (keymap-popup--popup-buffer)))
(let ((buf (keymap-popup--popup-buffer))) (json-encode (list (cons 'body (vconcat (mapcar #'vconcat (with-current-buffer buf (keymap-popup-fixture-runs (buffer-string)))))) (cons 'title (vconcat (mapcar #'vconcat (keymap-popup-fixture-runs (format-mode-line (buffer-local-value 'mode-line-format buf) nil (get-buffer-window buf) buf))))))))
