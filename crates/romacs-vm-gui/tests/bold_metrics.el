; Bold rows beside regular ones in Iosevka, whose Bold has Regular's
; metrics: GNU's line height is the largest of its glyphs' fonts and a
; monospace family's columns stay on the frame's character width, so
; every answer is in whole rows and columns.  Regenerate with
; crates/romacs-vm/tests/child_frames_oracle.py --probe THIS --output bold_metrics.gnu.
(setq bm (get-buffer-create "bold-metrics"))
(progn (set-face-attribute 'default nil :family "Iosevka" :height 140) (delete-other-windows) (switch-to-buffer bm) (erase-buffer) (insert (propertize "Navigate" 'face 'bold) "   Chats\n  n  Next\n" (propertize "Tools" 'face 'bold) "\n") (goto-char (point-min)) (redisplay t) (list (buffer-size) (count-lines 1 (point-max))))
(let ((f (face-font 'bold))) (list (and (string-match-p "iosevka" (downcase f)) t) (and (string-match-p "bold" (downcase f)) t)))
(let ((h (line-pixel-height))) (forward-line 1) (prog1 (list (= h (line-pixel-height)) (= h (frame-char-height))) (goto-char (point-min))))
(list (= (car (window-text-pixel-size nil 1 9)) (* 8 (frame-char-width))) (= (car (window-text-pixel-size nil 9 17)) (* 8 (frame-char-width))))
(let ((s (window-text-pixel-size nil 1 17))) (list (/ (car s) (frame-char-width)) (% (car s) (frame-char-width)) (/ (cdr s) (frame-char-height)) (% (cdr s) (frame-char-height))))
(let ((s (window-text-pixel-size nil (point-min) (point-max)))) (list (/ (car s) (frame-char-width)) (% (car s) (frame-char-width)) (/ (cdr s) (frame-char-height)) (% (cdr s) (frame-char-height))))
(let ((s (buffer-text-pixel-size bm))) (list (/ (car s) (frame-char-width)) (% (car s) (frame-char-width)) (/ (cdr s) (frame-char-height)) (% (cdr s) (frame-char-height))))
(progn (modify-frame-parameters nil '((bottom-divider-width . 0) (right-divider-width . 0))) (setq bm-low (split-window-below)) (set-window-buffer bm-low bm) (fit-window-to-buffer bm-low nil 1) (redisplay t) (prog1 (list (/ (window-body-height bm-low t) (frame-char-height)) (% (window-body-height bm-low t) (frame-char-height))) (delete-window bm-low)))
(setq bm-faces (progn (dolist (spec '((bm-italic "DejaVu Sans Mono" italic normal) (bm-upright "DejaVu Sans Mono" normal normal) (bm-condensed "DejaVu Sans" normal condensed) (bm-wide "DejaVu Sans" normal normal))) (make-face (car spec)) (set-face-attribute (car spec) nil :family (nth 1 spec) :slant (nth 2 spec) :width (nth 3 spec))) '(bm-italic bm-upright bm-italic bm-condensed bm-wide bm-condensed)))
(mapcar (lambda (f) (let ((n (downcase (face-font f)))) (list (and (string-match-p "oblique\\|italic" n) t) (and (string-match-p "condensed" n) t)))) bm-faces)
(progn (set-face-attribute 'default nil :weight 'bold) (prog1 (font-get (car (internal-char-font nil ?a)) :weight) (set-face-attribute 'default nil :weight 'normal)))
(font-get (car (internal-char-font nil ?a)) :weight)
(progn (unless (featurep 'keymap-popup) (load (expand-file-name "../../../lisp/romacs/emacs-lisp/keymap-popup.el" (file-name-directory (getenv "PROBE"))) nil t)) (featurep 'keymap-popup))
(let* ((b (get-buffer-create "bm-popup")) (after-make-frame-functions nil) (f (make-frame `((parent-frame . ,(selected-frame)) (minibuffer . ,(minibuffer-window)) (visibility . nil) ,@keymap-popup-child-frame-parameters)))) (with-current-buffer b (erase-buffer) (insert (make-string 30 ?x) "\n") (setq-local left-margin-width 1 right-margin-width 1)) (set-window-buffer (frame-root-window f) b) (prog1 (list (progn (keymap-popup--fit-frame f) (window-body-width (frame-root-window f))) (let ((fit-frame-to-buffer-sizes '(nil nil 20 nil))) (keymap-popup--fit-frame f) (window-body-width (frame-root-window f)))) (delete-frame f)))
