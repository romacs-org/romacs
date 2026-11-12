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
