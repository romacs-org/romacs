;;; word-motion-test.el --- Native word motion contract -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

(require 'romacs-test)

(romacs-deftest word-motion-default-and-zero ()
  (with-temp-buffer
    (insert "one two")
    (goto-char 1)
    (romacs-should (equal (list (forward-word 0) (point)) '(t 1)))
    (romacs-should (equal (list (forward-word) (point)) '(t 4)))
    (romacs-should (equal (list (forward-word nil) (point)) '(t 8)))
    (romacs-should (equal (list (forward-word) (point)) '(nil 8)))))

(romacs-deftest word-motion-punctuation-and-symbols ()
  (with-temp-buffer
    (insert "one_two three-four")
    (goto-char 1)
    (let (positions)
      (dotimes (_ 4)
        (romacs-should (forward-word))
        (setq positions (cons (point) positions)))
      (romacs-should (equal (nreverse positions) '(4 8 14 19))))
    (romacs-should (equal (list (forward-word -1) (point)) '(t 15)))
    (romacs-should (equal (list (forward-word -3) (point)) '(t 1)))
    (romacs-should (equal (list (forward-word -1) (point)) '(nil 1)))))

(romacs-deftest word-motion-unicode-scalars ()
  (with-temp-buffer
    (insert "άλφα русский café é")
    (goto-char 1)
    (romacs-should (equal (list (forward-word 4) (point)) '(t 21)))
    (romacs-should (equal (list (forward-word -4) (point)) '(t 1)))))

(romacs-deftest word-motion-unicode-punctuation ()
  (with-temp-buffer
    (insert "α—β")
    (goto-char 1)
    (romacs-should (equal (list (forward-word) (point)) '(t 2)))
    (romacs-should (equal (list (forward-word) (point)) '(t 4)))
    (romacs-should (equal (list (forward-word -1) (point)) '(t 3)))))

(romacs-deftest word-motion-syntax-overrides ()
  (with-temp-buffer
    (set-syntax-table (copy-syntax-table))
    (modify-syntax-entry ?_ "w")
    (modify-syntax-entry ?- "w")
    (modify-syntax-entry ?β ".")
    (insert "one_two three-four αβγ")
    (goto-char 1)
    (romacs-should (equal (list (forward-word 2) (point)) '(t 19)))
    (romacs-should (equal (list (forward-word) (point)) '(t 21)))
    (romacs-should (equal (list (forward-word) (point)) '(t 23)))))

(romacs-deftest word-motion-narrowing ()
  (with-temp-buffer
    (insert "éxx one two yyλ")
    (narrow-to-region 5 12)
    (goto-char (point-min))
    (romacs-should (equal (list (forward-word 2) (point)) '(t 12)))
    (romacs-should (equal (list (forward-word) (point)) '(nil 12)))
    (romacs-should (equal (list (forward-word -3) (point)) '(nil 5)))
    (romacs-should (equal (buffer-substring-no-properties (point-min) (point-max)) "one two"))))

(romacs-deftest word-motion-empty-and-separators ()
  (with-temp-buffer
    (romacs-should (equal (list (forward-word) (point)) '(nil 1)))
    (romacs-should (equal (list (forward-word -1) (point)) '(nil 1)))
    (insert " -- ")
    (goto-char 1)
    (romacs-should (equal (list (forward-word 2) (point)) '(nil 5)))
    (romacs-should (equal (list (forward-word -2) (point)) '(nil 1)))))

(romacs-deftest word-motion-read-only-no-edit ()
  (with-temp-buffer
    (insert "α one")
    (goto-char 1)
    (let ((buffer-read-only t)
          (tick (buffer-chars-modified-tick)))
      (romacs-should (forward-word 2))
      (romacs-should (forward-word -2))
      (romacs-should (equal (buffer-chars-modified-tick) tick))
      (romacs-should (equal (buffer-string) "α one")))))

(romacs-deftest word-motion-count-type ()
  (romacs-should-error (forward-word "1") :type 'wrong-type-argument)
  (romacs-should-error (forward-word 1.0) :type 'wrong-type-argument)
  (romacs-should
   (equal (condition-case error (forward-word 1.0) (error error))
          '(wrong-type-argument fixnump 1.0))))

(provide 'word-motion-test)
;;; word-motion-test.el ends here
