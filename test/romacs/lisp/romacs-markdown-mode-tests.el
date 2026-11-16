;;; romacs-markdown-mode-tests.el --- Editing Markdown -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-markdown-mode)

(defmacro romacs-markdown-mode-tests--with (text &rest body)
  "BODY in a `romacs-markdown-mode' buffer holding TEXT, point at its start."
  (declare (indent 1))
  `(let ((buffer (generate-new-buffer "romacs-markdown-mode-test.md")))
     (unwind-protect
         (with-current-buffer buffer
           (insert ,text)
           (romacs-markdown-mode)
           (goto-char (point-min))
           ,@body)
       (kill-buffer buffer))))

(defun romacs-markdown-mode-tests--faces-at (needle)
  "The faces at the first NEEDLE, as a list."
  (save-excursion
    (goto-char (point-min))
    (search-forward needle)
    (ensure-list (get-text-property (match-beginning 0) 'face))))

(ert-deftest romacs-markdown-mode-faces-the-buffer ()
  (romacs-markdown-mode-tests--with "# Title\n\nSome **bold** and `code`.\n\n- item\n"
    (font-lock-ensure)
    (should (memq 'romacs-markdown-heading-1 (romacs-markdown-mode-tests--faces-at "Title")))
    (should (memq 'romacs-markdown-strong (romacs-markdown-mode-tests--faces-at "bold")))
    (should (memq 'romacs-markdown-code (romacs-markdown-mode-tests--faces-at "code`")))
    (should (memq 'romacs-markdown-list-marker (romacs-markdown-mode-tests--faces-at "- item")))))

(ert-deftest romacs-markdown-mode-faces-only-the-blocks-asked-for ()
  (romacs-markdown-mode-tests--with (concat (mapconcat (lambda (i) (format "Para *%d*." i)) (number-sequence 1 200) "\n\n") "\n")
    (let* ((beg (progn (goto-char (point-min)) (search-forward "Para *100*") (match-beginning 0)))
           (bounds (romacs-markdown--font-lock-region beg (1+ beg))))
      (should (eq (car bounds) 'jit-lock-bounds))
      ;; The one block holding the region, not the buffer.
      (should (= (cadr bounds) beg))
      (should (< (- (cddr bounds) (cadr bounds)) 20))
      (should (memq 'romacs-markdown-emphasis (romacs-markdown-mode-tests--faces-at "100*")))
      (should-not (romacs-markdown-mode-tests--faces-at "1*.")))))

(ert-deftest romacs-markdown-mode-refaces-what-an-edit-reshapes ()
  "An edit that reshapes blocks outside the region asked for leaves them
unfontified, so redisplay asks for them again."
  (romacs-markdown-mode-tests--with "Para one.\n\nPara two.\n\nPara three.\n\nLast *x*.\n"
    (font-lock-ensure)
    (put-text-property (point-min) (point-max) 'fontified t)
    ;; A setext underline: the paragraph above becomes a heading.
    (search-forward "Para two.")
    (insert "\n===")
    (let ((last (save-excursion (search-forward "Last") (match-beginning 0))))
      (romacs-markdown--font-lock-region last (1+ last)))
    (should-not (get-text-property (save-excursion (goto-char (point-min)) (search-forward "two") (point)) 'fontified))
    (font-lock-ensure)
    (should (memq 'romacs-markdown-heading-1 (romacs-markdown-mode-tests--faces-at "Para two")))))

(ert-deftest romacs-markdown-mode-opened-fence-makes-code ()
  (romacs-markdown-mode-tests--with "Intro.\n\n# Below\n\nMore *text*.\n"
    (font-lock-ensure)
    (should (memq 'romacs-markdown-heading-1 (romacs-markdown-mode-tests--faces-at "Below")))
    (insert "```\n")
    (font-lock-ensure)
    (should (equal (romacs-markdown-mode-tests--faces-at "Below") '(romacs-markdown-code-block)))
    (should-not (memq 'romacs-markdown-emphasis (romacs-markdown-mode-tests--faces-at "text*")))))

(ert-deftest romacs-markdown-mode-outline-skips-code ()
  (romacs-markdown-mode-tests--with "# One\n\n```sh\n# not a heading\n```\n\n## Two\n"
    (outline-next-heading)
    (should (looking-at "## Two"))
    (should (= (funcall outline-level) 2))
    (should (equal (mapcar #'car (romacs-markdown--imenu)) '("One" "Two")))))

(ert-deftest romacs-markdown-mode-continues-list-items ()
  (dolist (case '(("- a" . "- a\n- ") ("9. x" . "9. x\n10. ") ("  * [x] done" . "  * [x] done\n  * [ ] ")
                  ("plain" . "plain\n")))
    (romacs-markdown-mode-tests--with (car case)
      (goto-char (point-max))
      (romacs-markdown-insert-item)
      (should (equal (buffer-string) (cdr case))))))

(ert-deftest romacs-markdown-mode-aligns-tables ()
  (romacs-markdown-mode-tests--with "Text\n\n| a | longer |\n|:-|-:|\n| wide cell | b |\n\nAfter\n"
    (search-forward "wide")
    (romacs-markdown-align-table)
    (should (string-match-p (regexp-quote "| a         | longer |\n| :-------- | -----: |\n| wide cell |      b |\n")
                            (buffer-string)))
    (should (string-prefix-p "Text\n\n" (buffer-string)))
    (should (string-suffix-p "\n\nAfter\n" (buffer-string)))
    ;; Point stays in its cell.
    (should (looking-at "wide cell"))))

(ert-deftest romacs-markdown-mode-follows-links ()
  (romacs-markdown-mode-tests--with "See [docs](notes/a.md#top) and [site][s].\n\n[s]: https://example.org\n"
    (let (opened browsed)
      (cl-letf (((symbol-function 'find-file) (lambda (file) (setq opened file)))
                ((symbol-function 'browse-url) (lambda (url) (setq browsed url))))
        (search-forward "docs")
        (romacs-markdown-follow-link)
        (should (equal opened (expand-file-name "notes/a.md" default-directory)))
        (search-forward "site")
        (romacs-markdown-follow-link)
        (should (equal browsed "https://example.org"))))))

(ert-deftest romacs-markdown-mode-follows-references-defined-elsewhere ()
  (romacs-markdown-mode-tests--with "[go][r]\n\nOther.\n\n[r]: https://example.org/r\n"
    (let (browsed)
      (cl-letf (((symbol-function 'browse-url) (lambda (url) (setq browsed url))))
        (search-forward "go")
        (romacs-markdown-follow-link)
        (should (equal browsed "https://example.org/r"))))))

(ert-deftest romacs-markdown-mode-colours-each-block-afresh ()
  "A block coloured by its mode's Font Lock in the reused hidden buffer is
coloured as in a fresh one: no syntax state of the block before."
  (skip-unless (fboundp 'scheme-mode))
  (romacs-markdown-mode-tests--with
      "```scheme\n(define (long-function-name argument) \"a string\" (list 1 2 3))\n```\n\n```scheme\n#;\n(define ignored 1)\n```\n"
    (font-lock-ensure)
    (should (memq 'font-lock-comment-face (romacs-markdown-mode-tests--faces-at "ignored")))))

(ert-deftest romacs-markdown-mode-runs-markdown-mode-hook ()
  (require 'markdown-mode)
  (let* ((ran nil) (hook (lambda () (setq ran t))))
    (add-hook 'markdown-mode-hook hook)
    (unwind-protect
        (romacs-markdown-mode-tests--with "# x\n" (should ran))
      (remove-hook 'markdown-mode-hook hook))
    (should (eq (indirect-function 'markdown-mode) (indirect-function 'romacs-markdown-mode)))))

(ert-deftest romacs-markdown-mode-colours-code-by-language ()
  (romacs-markdown-mode-tests--with "```elisp\n(defun f () nil)\n```\n"
    (font-lock-ensure)
    (let ((faces (romacs-markdown-mode-tests--faces-at "defun")))
      ;; The language's face in front, the block's behind.
      (should (memq 'romacs-markdown-code-block faces))
      (should (memq 'font-lock-keyword-face faces))
      (should (eq (car faces) 'font-lock-keyword-face)))))

(ert-deftest romacs-markdown-mode-colours-code-by-tree-sitter ()
  (skip-unless (romacs-treesit-code-language-p 'rust))
  (romacs-markdown-mode-tests--with "```rust\nfn main() { let x = 1; }\n```\n"
    (font-lock-ensure)
    (let ((faces (romacs-markdown-mode-tests--faces-at "fn main")))
      (should (memq 'romacs-markdown-code-block faces))
      (should (> (length faces) 1))))
  ;; Text in no buffer of its own (a reply rendered as a string) too.
  (let* ((out (romacs-markdown-string "```rs\nfn main() {}\n```\n"))
         (faces (ensure-list (get-text-property (string-match "fn" out) 'face out))))
    (should (memq 'romacs-markdown-code-block faces))
    (should (> (length faces) 1))))

(ert-deftest romacs-markdown-view-mode-is-read-only ()
  (romacs-markdown-mode-tests--with "# Shown\n"
    (romacs-markdown-view-mode)
    (should buffer-read-only)
    (font-lock-ensure)
    (should (memq 'romacs-markdown-heading-1 (romacs-markdown-mode-tests--faces-at "Shown")))))

(provide 'romacs-markdown-mode-tests)
;;; romacs-markdown-mode-tests.el ends here
