;;; romacs-markdown-tests.el --- Markdown parsed in Rust -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-markdown)

(defun romacs-markdown-tests--face-at (needle &optional offset)
  "The `face' at the first NEEDLE in the current buffer, OFFSET chars in."
  (save-excursion
    (goto-char (point-min))
    (search-forward needle)
    (get-text-property (+ (match-beginning 0) (or offset 0)) 'face)))

(defmacro romacs-markdown-tests--with (text &rest body)
  "BODY in a buffer holding TEXT, fontified."
  (declare (indent 1))
  `(with-temp-buffer
     (insert ,text)
     (romacs-markdown-fontify-region (point-min) (point-max))
     ,@body))

(ert-deftest romacs-markdown-faces-elements-innermost-first ()
  (romacs-markdown-tests--with "# Head **bold**\n\nText `code` and [a link](http://x).\n"
    (should (equal (romacs-markdown-tests--face-at "# ") '(romacs-markdown-markup romacs-markdown-heading-1)))
    (should (equal (romacs-markdown-tests--face-at "bold") '(romacs-markdown-strong romacs-markdown-heading-1)))
    (should (equal (romacs-markdown-tests--face-at "**bold")
                   '(romacs-markdown-markup romacs-markdown-strong romacs-markdown-heading-1)))
    (should (eq (romacs-markdown-tests--face-at "code" ) 'romacs-markdown-code))
    (should (eq (romacs-markdown-tests--face-at "a link") 'romacs-markdown-link))
    (should (equal (romacs-markdown-tests--face-at "(http") '(romacs-markdown-url romacs-markdown-link)))
    (should-not (romacs-markdown-tests--face-at "Text"))))

(ert-deftest romacs-markdown-faces-are-silent ()
  (with-temp-buffer
    (insert "*a*\n")
    (set-buffer-modified-p nil)
    (setq buffer-undo-list nil)
    (let ((changes 0))
      (add-hook 'after-change-functions (lambda (&rest _) (setq changes (1+ changes))) nil t)
      (romacs-markdown-fontify-region (point-min) (point-max))
      (should (equal (get-text-property 2 'face) 'romacs-markdown-emphasis))
      (should-not (buffer-modified-p))
      (should-not buffer-undo-list)
      (should (= changes 0)))))

(ert-deftest romacs-markdown-primitive-refuses-noisy-writes ()
  (with-temp-buffer
    (insert "*a*\n")
    (should-error (romacs-markdown--fontify (point-min) (point-max) romacs-markdown-faces))))

(ert-deftest romacs-markdown-faces-only-the-region ()
  (with-temp-buffer
    (insert (propertize "before *x*\n" 'face 'bold) "*in*\n" (propertize "after\n" 'face 'italic))
    (romacs-markdown-fontify-region 12 (- (point-max) 6))
    (should (eq (get-text-property 1 'face) 'bold))
    (should (eq (get-text-property 9 'face) 'bold))
    (should (eq (get-text-property 13 'face) 'romacs-markdown-emphasis))
    (should (eq (get-text-property (- (point-max) 2) 'face) 'italic))))

(ert-deftest romacs-markdown-kinds-follow-the-option ()
  (let ((romacs-markdown-faces '((emphasis . underline) (markup . nil))))
    (romacs-markdown-tests--with "*a* **b**\n"
      (should (eq (romacs-markdown-tests--face-at "a") 'underline))
      ;; Unfaced markup takes its element's face.
      (should (eq (romacs-markdown-tests--face-at "*a") 'underline))
      (should-not (romacs-markdown-tests--face-at "b")))))

(ert-deftest romacs-markdown-refaces-a-stream-from-its-last-blocks ()
  "Text arriving chunk by chunk, faced after each from where it changed,
ends faced as the whole would be, and earlier blocks are not rewritten."
  (let* ((text "# Plan\n\nIntro *x*.\n\n1. one\n2. two `c`\n\n```rust\nfn f() {}\n```\n\n| a | b |\n|---|---|\n| c | d |\n")
         (whole (romacs-markdown-string text)))
    (with-temp-buffer
      (let ((at 0) (starts nil))
        (while (< at (length text))
          (let ((chunk (substring text at (min (length text) (+ at 7)))))
            (goto-char (point-max))
            (let ((from (point)))
              (insert chunk)
              (push (romacs-markdown-fontify-region (point-min) (point-max) from) starts))
            (setq at (+ at (length chunk)))))
        (should (equal-including-properties (buffer-string) whole))
        ;; The heading's block was left alone once later blocks arrived.
        (should (> (car starts) 1))))))

(ert-deftest romacs-markdown-blocks-describe-structure ()
  (with-temp-buffer
    (insert "# T\n\n```py\nx = 1\n```\n\n- a\n  1. b\n\n| h |\n|---|\n")
    (let ((blocks (romacs-markdown-blocks (point-min) (point-max))))
      (should (equal (mapcar #'car blocks) '(heading code item item table)))
      (let ((heading (nth 0 blocks)) (code (nth 1 blocks)) (inner (nth 3 blocks)))
        (should (equal (plist-get (nthcdr 3 heading) :level) 1))
        (should (equal (plist-get (nthcdr 3 code) :language) "py"))
        (let ((body (plist-get (nthcdr 3 code) :body)))
          (should (equal (buffer-substring (car body) (cdr body)) "x = 1\n")))
        (should (equal (plist-get (nthcdr 3 inner) :depth) 1))
        (should (eq (plist-get (nthcdr 3 inner) :ordered) t))))))

(ert-deftest romacs-markdown-positions-count-characters ()
  "Positions are characters, whatever the text's bytes."
  (romacs-markdown-tests--with "é ü *ö* `ß`\n"
    (should (eq (get-text-property 6 'face) 'romacs-markdown-emphasis))
    (should (eq (get-text-property 10 'face) 'romacs-markdown-code))))

(ert-deftest romacs-markdown-string-copies ()
  (let* ((text "**s**")
         (out (romacs-markdown-string text)))
    (should (equal out text))
    (should-not (text-properties-at 0 text))
    (should (equal (get-text-property 2 'face out) 'romacs-markdown-strong))))

(provide 'romacs-markdown-tests)
;;; romacs-markdown-tests.el ends here
