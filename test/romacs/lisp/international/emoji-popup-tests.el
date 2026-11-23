;;; emoji-popup-tests.el --- Emoji menu regressions -*- lexical-binding: t; -*-
(require 'ert)
(require 'emoji)

(ert-deftest emoji-popup-does-not-load-transient ()
  (should (featurep 'keymap-popup))
  (should-not (featurep 'transient))
  (dolist (command '(emoji-insert emoji-recent emoji-search emoji-list-select))
    (should (commandp command))))

(ert-deftest emoji-popup-recents-insert-the-selected-glyph ()
  (with-temp-buffer
    (let* ((map (emoji--popup-map "Recent" '("😀" "😖") t (current-buffer)))
           (command (keymap-lookup map "b")))
      (should (commandp command))
      (call-interactively command)
      (should (equal (buffer-string) "😖"))
      (should (equal (car (multisession-value emoji--recent)) "😖")))))

(ert-deftest emoji-popup-selection-closes-a-globally-persistent-popup ()
  (let ((keymap-popup-persistent t)
        (keymap-popup-backend #'keymap-popup-backend-side-window)
        (recent (multisession-value emoji--recent)))
    (unwind-protect
        (save-window-excursion
          (with-temp-buffer
            (switch-to-buffer (current-buffer))
            (execute-kbd-macro (kbd "C-e"))
            (setf (multisession-value emoji--recent) (list "😀" "😖"))
            (emoji-recent)
            (execute-kbd-macro (kbd "a b"))
            (should (equal (buffer-string) "😀b"))))
      (keymap-popup-dismiss)
      (setf (multisession-value emoji--recent) recent))))

(ert-deftest emoji-popup-glyph-faces-stay-in-the-popup ()
  (emoji--init)
  (let ((keymap-popup-backend #'keymap-popup-backend-side-window)
        (recent (multisession-value emoji--recent)))
    (unwind-protect
        (save-window-excursion
          (with-temp-buffer
            (switch-to-buffer (current-buffer))
            (execute-kbd-macro (kbd "C-e"))
            (emoji--show-popup "Emoji" '("😀" "👍") nil)
            (with-current-buffer (keymap-popup--popup-buffer)
              (dolist (choice '(("😀" . emoji) ("👍" . emoji-with-derivations)))
                (goto-char (point-min))
                (search-forward (car choice))
                (let ((face (get-text-property (1- (point)) 'face)))
                  (should (memq (cdr choice) (ensure-list face))))))
            (execute-kbd-macro (kbd "a"))
            (should (equal (buffer-string) "😀"))
            (should-not (get-text-property (point-min) 'face))))
      (keymap-popup-dismiss)
      (setf (multisession-value emoji--recent) recent))))

(ert-deftest emoji-popup-derived-glyphs-terminate-narrowing ()
  (emoji--init)
  (let* ((glyph "👍")
         (derived (gethash glyph emoji--derived))
         (entries (emoji--layout "Thumbs" (cons glyph derived) (list glyph))))
    (should derived)
    (should (= (length entries) (1+ (length derived))))
    (should-not (nth 3 (car entries)))
    (dolist (entry entries)
      (should (stringp (nth 5 entry)))
      (should-not (nth 3 entry)))))

(ert-deftest emoji-popup-search-preserves-the-chosen-glyph ()
  (with-temp-buffer
    (emoji-search "😀" nil)
    (should (equal (buffer-string) "😀"))))

(ert-deftest emoji-popup-refuses-a-replaced-destination ()
  (let* ((source (generate-new-buffer " *emoji-destination*"))
         (map (emoji--popup-map "Recent" '("😀") t source))
         (action (keymap-lookup map "a")))
    (unwind-protect
        (with-temp-buffer
          (should-error (call-interactively action) :type 'user-error)
          (should (equal (buffer-string) "")))
      (kill-buffer source))))

(provide 'emoji-popup-tests)
