;;; romacs-agent-log-images-tests.el --- An agent chat's image links -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-agent-log-images)
(require 'romacs-attach)

(defconst romacs-agent-log-images-tests--png
  (base64-decode-string
   "iVBORw0KGgoAAAANSUhEUgAAAEAAAAAwEAIAAAB+uTcLAAAAy0lEQVR42u3awQ2DMBAFUXv9j3YPlEIp1JbcQ4eBA0XMgXkVIFk7XgR92/b9OJogqdXO60M/xnulVj//HgAmtboTAEqt5gSAnACYBwDzEoalpmsoyQTBTBDMN2HYMwFf+jHeyzsAZoJgXsIwEwQzQbAME4QyQTATBHMLgpkgmJ8kYanZf04AxwTBvIRhrqEwEwQzQTATBHMCYN4BMBMEM0EwEwR7EuR/QRgTBMswQSi3IFhqmiCSWxDMT5IwJwDmAcB8D4C5hsJMEOwGbYVxvuYzGUwAAAAASUVORK5CYII=")
  "A PNG, 64 by 48 pixels.")

(defmacro romacs-agent-log-images-tests--with (&rest body)
  "BODY with `dir' a fresh directory and `shot' a PNG in it; images shown,
their sizes read as a window system's host reads them (a batch session
reads none)."
  (declare (indent 0))
  `(let* ((dir (file-name-as-directory (make-temp-file "romacs-images-" t)))
          (shot (expand-file-name "shot.png" dir)))
     (unwind-protect
         (progn
           (let ((coding-system-for-write 'binary))
             (write-region romacs-agent-log-images-tests--png nil shot nil 'silent))
           (cl-letf (((symbol-function 'display-images-p) #'always)
                     ((symbol-function 'romacs-image-header-size)
                      (lambda (file) (and (file-exists-p file) '(64 . 48)))))
             ,@body))
       (delete-directory dir t))))

(ert-deftest romacs-agent-log-images-link-is-markdown ()
  ;; A link names the file where it is; a name with a space goes in
  ;; angle brackets, and every link reads back as the file it names.
  (should (equal (romacs-agent-log-images-link "/p/Screenshots/a.png") "[image](/p/Screenshots/a.png)"))
  (should (equal (romacs-agent-log-images-link "/p/a b.JPG") "[image](</p/a b.JPG>)"))
  (should (equal (romacs-agent-log-images-link "/p/notes.org") "[file](/p/notes.org)"))
  (let ((text (concat "see " (romacs-agent-log-images-link "/p/a b.png") " and [x](/q/c.webp), not (/r/d.png) "
                      "[t](/s/e.txt) [u](/s/f.png")))
    (should (equal (mapcar #'cl-third (romacs-agent-log-images-links text)) '("/p/a b.png" "/q/c.webp")))))

(ert-deftest romacs-agent-log-images-a-message-shows-its-images ()
  ;; A sent message keeps its text; a link shows as its file's name, and
  ;; its image's rows, slices of one image, go under it.  A link to no
  ;; image keeps its text and draws nothing.
  (romacs-agent-log-images-tests--with
    (let* ((text (format "look [image](%s) and [image](%smissing.png)" shot dir))
           (shown (romacs-agent-log-images-message text)))
      (should (equal (substring-no-properties (car shown)) text))
      (should (equal (get-text-property 5 'display (car shown)) "▣ shot.png"))
      (should-not (get-text-property (string-search "[image](/" text 10) 'display (car shown)))
      (let* ((rows (split-string (cdr shown) "\n"))
             (slices (mapcar (lambda (row) (get-text-property 2 'display row)) rows)))
        (should (= (length rows) (min romacs-agent-log-image-lines (max 1 (floor 48 (default-line-height))))))
        (should (cl-every (lambda (slice) (eq (car (car slice)) 'slice)) slices))
        (should (equal (plist-get (cdr (cadr (car slices))) :file) shot))))
    (should-not (cdr (romacs-agent-log-images-message "no image here")))))

(ert-deftest romacs-agent-log-images-go-side-by-side-while-they-fit ()
  ;; Images go side by side within the width, the next group under them.
  (let ((images (list (cons 100 '("a")) (cons 100 '("b")) (cons 100 '("c")))))
    (should (equal (mapcar #'length (romacs-agent-log-images--pack images 220)) '(2 1)))
    (should (equal (mapcar #'length (romacs-agent-log-images--pack images 1000)) '(3)))
    (should (equal (mapcar #'length (romacs-agent-log-images--pack images 50)) '(1 1 1)))))

(ert-deftest romacs-agent-log-images-a-margin-only-beside-one-height ()
  ;; Images of one height side by side are set off from the text by a
  ;; margin; beside a taller one, none, which would cut through it.
  (romacs-agent-log-images-tests--with
    (cl-flet ((margins (images)
                (let ((text (romacs-agent-log-images--stack images 1000)) (found nil))
                  (dotimes (i (length text))
                    (when-let* ((spec (cadr (get-text-property i 'display text))))
                      (when (eq (car-safe spec) 'image) (push (cdr (plist-get (cdr spec) :margin)) found))))
                  (delete-dups found))))
      (should (equal (margins (list (list 40 3 shot) (list 30 3 shot))) (list romacs-agent-log-images--margin)))
      (should (equal (margins (list (list 40 3 shot) (list 30 5 shot))) '(0))))))

(ert-deftest romacs-agent-log-images-the-composer-shows-its-links ()
  ;; The composer's link shows as its file's name and its image under the
  ;; draft, by overlays: its text stays what is sent.  Typing keeps them
  ;; and reads no file; a link broken by an edit shows its text again.
  (romacs-agent-log-images-tests--with
    (with-temp-buffer
      (let ((start (point-min-marker)) (reads 0)
            (header (symbol-function 'romacs-image-header-size)))
        (cl-letf (((symbol-function 'romacs-image-header-size)
                   (lambda (file) (cl-incf reads) (funcall header file))))
          (insert (romacs-agent-log-images-link shot))
          (romacs-agent-log-images-composer start)
          (pcase-let ((`(,links . ,block) romacs-agent-log-images--composer))
            (should (equal (overlay-get (car links) 'display) "▣ shot.png"))
            (should (string-prefix-p "\n  " (overlay-get block 'after-string)))
            (should (= reads 1))
            (insert " why")
            (romacs-agent-log-images-composer start)
            (should (eq (car romacs-agent-log-images--composer) links))
            (should (= (overlay-end block) (point-max)))
            (should (= reads 1))
            (should (equal (buffer-string) (format "[image](%s) why" shot)))
            ;; Its last character deleted, the images follow the new end.
            (delete-char -1)
            (romacs-agent-log-images-composer start)
            (should (eq (cdr romacs-agent-log-images--composer) block))
            (should (= (overlay-end block) (point-max)))
            (insert "y")
            (romacs-agent-log-images-composer start)
            (delete-region (- (point-max) 5) (point-max))
            (romacs-agent-log-images-composer start)
            (should-not romacs-agent-log-images--composer)
            (should-not (overlays-in (point-min) (point-max)))
            ;; A link to no file shows its text, and typing reads it no more.
            (insert ") [image](/no/such.png)")
            (romacs-agent-log-images-composer start)
            (setq reads 0)
            (insert "x")
            (romacs-agent-log-images-composer start)
            (should (= reads 0))))))))

(defmacro romacs-agent-log-images-tests--clipboard (&rest body)
  "BODY with a `wl-paste' on PATH that writes `shot''s bytes, the save
directory `saved' under `dir', and `done' the file it gave, read by `take'."
  (declare (indent 0))
  `(romacs-agent-log-images-tests--with
     (let* ((bin (expand-file-name "bin/" dir))
            (saved (expand-file-name "Screenshots/" dir))
            (romacs-attach-save-directory saved)
            (process-environment (cons (concat "PATH=" bin ":" (getenv "PATH")) process-environment))
            (exec-path (cons bin exec-path)))
       (make-directory bin) (make-directory saved)
       (with-temp-file (expand-file-name "wl-paste" bin)
         (insert "#!/bin/sh\nexec cat " (shell-quote-argument shot) "\n"))
       (set-file-modes (expand-file-name "wl-paste" bin) #o755)
       (cl-flet ((take ()
                   (let ((given 'waiting) (deadline (+ (float-time) 20)))
                     (romacs-attach-clipboard-file "image/png" (lambda (file) (setq given file)))
                     (while (and (eq given 'waiting) (< (float-time) deadline))
                       (accept-process-output nil 0.05))
                     given)))
         ,@body))))

(ert-deftest romacs-agent-log-images-the-clipboard-s-screenshot-is-its-file ()
  ;; An image copied as it was saved is the saved file: nothing is copied.
  (romacs-agent-log-images-tests--clipboard
    (let ((older (expand-file-name "screenshot-1.png" saved))
          (same (expand-file-name "screenshot-2.png" saved)))
      (with-temp-file older (insert "an older one"))
      (copy-file shot same)
      (should (equal (take) same))
      (should (equal (directory-files saved nil "\\`[^.]") '("screenshot-1.png" "screenshot-2.png")))
      (should-not (directory-files (romacs-attach-staging-directory) nil "\\`clipboard-")))))

(ert-deftest romacs-agent-log-images-a-clipboard-image-of-no-file-is-saved-once ()
  ;; An image no recent file has is saved once in the save directory,
  ;; named by the time; the next take of it is that file.
  (romacs-agent-log-images-tests--clipboard
    (let ((file (take)))
      (should (string-prefix-p (expand-file-name "clipboard-" saved) file))
      (should (string-suffix-p ".png" file))
      (should (equal (with-temp-buffer (set-buffer-multibyte nil) (insert-file-contents-literally file) (buffer-string))
                     romacs-agent-log-images-tests--png))
      (should (equal (take) file))
      (should (= (length (directory-files saved nil "\\`[^.]")) 1)))))

(provide 'romacs-agent-log-images-tests)
;;; romacs-agent-log-images-tests.el ends here
