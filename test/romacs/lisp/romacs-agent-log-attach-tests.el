;;; romacs-agent-log-attach-tests.el --- A chat's attachments are its composer's links -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-log))

(defconst romacs-agent-log-attach-tests--png
  (base64-decode-string "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==")
  "A PNG of one pixel.")

(defmacro romacs-agent-log-attach-tests--with (&rest body)
  "BODY in a chat whose composer holds nothing, with `a', `b' and `c', three
PNG files in a fresh directory, and `composer' its draft's text."
  (declare (indent 0))
  `(let* ((dir (file-name-as-directory (make-temp-file "romacs-attach-" t)))
          (a (expand-file-name "a.png" dir)) (b (expand-file-name "b.png" dir)) (c (expand-file-name "c.png" dir)))
     (unwind-protect
         (progn
           (dolist (file (list a b c))
             (let ((coding-system-for-write 'binary))
               (write-region romacs-agent-log-attach-tests--png nil file nil 'silent)))
           (with-temp-buffer
             (romacs-agent-log-mode)
             (setq romacs-agent-log--end (point-min-marker)
                   romacs-agent-log--input (point-min-marker))
             (cl-flet ((composer () (buffer-substring-no-properties romacs-agent-log--input (point-max)))
                       (link (file) (romacs-agent-log-images-link file))
                       (files () (mapcar (lambda (item) (plist-get item :path)) romacs-attach-items)))
               (ignore #'composer #'link #'files)
               ,@body)))
       (delete-directory dir t))))

(ert-deftest romacs-agent-log-attach-links-are-the-chips ()
  (romacs-agent-log-attach-tests--with
    (insert "see " (link a) " and [the second](" b ")")
    (romacs-agent-log-attach-sync)
    (should (equal (files) (list a b)))
    (should (equal (mapcar (lambda (item) (plist-get item :caption)) romacs-attach-items) '(nil "the second")))
    ;; The chips show above the draft, the images not again under it.
    (should (overlay-get romacs-agent-log-attach--overlay 'before-string))
    (should-not romacs-agent-log-images-under-draft)
    ;; No links, no chips.
    (delete-region romacs-agent-log--input (point-max))
    (romacs-agent-log-attach-sync)
    (should-not romacs-attach-items)))

(ert-deftest romacs-agent-log-attach-editing-a-chip-edits-its-link ()
  (romacs-agent-log-attach-tests--with
    (insert "first " (link a) " then " (link b) " last " (link c))
    (romacs-agent-log-attach-sync)
    (let ((ids (mapcar (lambda (item) (plist-get item :id)) romacs-attach-items)))
      ;; Removed: its link goes, with one space beside it.
      (romacs-attach-set-items (romacs-attach-remove romacs-attach-items (nth 1 ids)))
      (should (equal (composer) (concat "first " (link a) " then last " (link c))))
      ;; Moved: the links trade places, the words stay.
      (romacs-attach-set-items (romacs-attach-move romacs-attach-items (nth 0 ids) 1))
      (should (equal (composer) (concat "first " (link c) " then last " (link a))))
      ;; Captioned: the caption is its link's label.
      (romacs-attach-set-items (romacs-attach-caption romacs-attach-items (nth 2 ids) "the plan"))
      (should (string-prefix-p "first [the plan](" (composer)))
      ;; One back (an undone removal): linked at the composer's end.
      (romacs-attach-set-items (append romacs-attach-items (list (romacs-attach-item b))))
      (should (string-suffix-p (concat " " (link b)) (composer)))
      (should (equal (files) (list c a b))))))

(ert-deftest romacs-agent-log-attach-yank-pastes-a-clipboard-image ()
  (romacs-agent-log-attach-tests--with
    (cl-letf (((symbol-function 'romacs-attach-clipboard-image-type) (lambda () "image/png"))
              ((symbol-function 'romacs-attach-clipboard-file) (lambda (_type then) (funcall then a))))
      (insert "look: ")
      (romacs-agent-log-attach-yank)
      (should (equal (composer) (concat "look: " (link a))))
      (should (equal (files) (list a))))
    ;; No image on the clipboard: a yank, as ever.
    (cl-letf (((symbol-function 'romacs-attach-clipboard-image-type) (lambda () nil)))
      (kill-new " words")
      (romacs-agent-log-attach-yank)
      (should (string-suffix-p " words" (composer))))))

(ert-deftest romacs-agent-log-attach-del-at-the-start-edits-the-chips ()
  (romacs-agent-log-attach-tests--with
    (insert (link a))
    (romacs-agent-log-attach-sync)
    (let ((edited nil))
      (cl-letf (((symbol-function 'romacs-attach-edit) (lambda (&rest _) (setq edited t))))
        (goto-char romacs-agent-log--input)
        (romacs-agent-log-attach-delete-backward)
        (should edited)
        (setq edited nil)
        (goto-char (point-max))
        (romacs-agent-log-attach-delete-backward)
        (should-not edited)
        (should (= (length (composer)) (1- (length (link a)))))))))

(provide 'romacs-agent-log-attach-tests)
;;; romacs-agent-log-attach-tests.el ends here
