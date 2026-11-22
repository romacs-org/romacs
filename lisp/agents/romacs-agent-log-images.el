;;; romacs-agent-log-images.el --- An agent chat's image links  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A file goes to an agent as a Markdown link in the message's text,
;; [image](PATH): the agent reads it where it is, and nothing is copied.
;; The chat (romacs-agent-log.el) shows each image link as its file's
;; name, and its image under the text: a sent message's in the
;; transcript, the composer's under the draft, by overlays there (the
;; composer's text stays the user's alone, and is sent as typed).
;;
;; An image is drawn as rows of slices, a line tall each, so the chat
;; scrolls a line at a time past it; images go side by side while they
;; fit, a thumbnail under the draft and larger in the transcript.  Its size comes from its header
;; (`romacs-image-header-size'); it is decoded when a row of it is
;; drawn, off the evaluator.

;;; Code:

(require 'cl-lib)
(require 'subr-x)
(require 'romacs-attach)
(require 'romacs-agent-glyphs)

(defcustom romacs-agent-log-image-lines 8
  "How many lines tall a sent message's image is shown at most in an
agent chat; never enlarged, and narrower in a narrower window."
  :type 'natnum
  :group 'romacs-agent)

(defcustom romacs-agent-log-image-composer-lines 5
  "How many lines tall an image is shown at most under the composer's draft."
  :type 'natnum
  :group 'romacs-agent)

(defface romacs-agent-log-image-link '((t :inherit romacs-attach-chip))
  "An image link in an agent chat, shown as its file's name: an
attachment's chip.")

(defconst romacs-agent-log-images-regexp
  (let ((type "\\.\\(?:[pP][nN][gG]\\|[jJ][pP][eE]?[gG]\\|[wW][eE][bB][pP]\\|[gG][iI][fF]\\)"))
    (concat "\\[[^]\n]*\\](\\(?:<\\([^<>\n]+" type "\\)>\\|\\([^ \t\n()<>]+" type "\\)\\))"))
  "A Markdown link to an image file: its target in group 1, written in
angle brackets, or else in group 2.")

(defconst romacs-agent-log-images--indent "  "
  "What an image's rows start with: under the text after the prompt's mark.")

(defconst romacs-agent-log-images--gap 12
  "Pixels between two images side by side.")

(defconst romacs-agent-log-images--margin 6
  "Pixels above and below a row of images, apart from the text.")

(defun romacs-agent-log-images-link (file)
  "FILE as a message links it: [image](FILE) for an image, else
[file](FILE); in angle brackets when its name has a space or parenthesis."
  (let ((file (expand-file-name file)))
    (format "[%s](%s)"
            (if (string-match-p "\\.\\(?:png\\|jpe?g\\|webp\\|gif\\)\\'" (downcase file)) "image" "file")
            (if (string-match-p "[ \t()<>]" file) (concat "<" file ">") file))))

(defun romacs-agent-log-images-links (string &optional start end)
  "The image links in STRING, or in the current buffer from START to END
when STRING is nil: (BEG END FILE) each, in order, FILE expanded."
  (let ((case-fold-search nil) (links nil))
    (if string
        (let ((at 0))
          (while (string-match romacs-agent-log-images-regexp string at)
            (push (list (match-beginning 0) (match-end 0)
                        (expand-file-name (or (match-string 1 string) (match-string 2 string))))
                  links)
            (setq at (match-end 0))))
      (save-excursion
        (goto-char start)
        (while (re-search-forward romacs-agent-log-images-regexp end t)
          (push (list (match-beginning 0) (match-end 0) (expand-file-name (or (match-string 1) (match-string 2))))
                links))))
    (nreverse links)))

(defun romacs-agent-log-images--width ()
  "The width images are laid out in, in the current chat, in pixels; nil
while no window shows it."
  (when-let* ((window (get-buffer-window nil t)))
    (max 1 (- (window-body-width window t) (* (1+ (length romacs-agent-log-images--indent))
                                              (frame-char-width (window-frame window)))))))

(defun romacs-agent-log-images--image (file width lines)
  "FILE's image within WIDTH pixels by LINES lines, never enlarged, in
whole lines: (W ROWS FILE), W its width in pixels and ROWS its height in
lines; nil when images cannot be shown or FILE's header gives no size."
  (when-let* (((display-images-p))
              (pixels (romacs-image-header-size file)))
    (let* ((line (default-line-height))
           ;; Whole lines, rounded down: a last row shorter than a line
           ;; would stand apart from the rest.
           (rows (max 1 (min lines (/ (cdr pixels) line) (/ (* (cdr pixels) width) (* (car pixels) line))))))
      (list (max 1 (min width (/ (* (car pixels) rows line) (cdr pixels)))) rows file))))

(defun romacs-agent-log-images--slices (image margin)
  "IMAGE's rows, (W ROWS FILE), as slices a line tall each, MARGIN pixels
above its first and below its last."
  (pcase-let* ((`(,w ,rows ,file) image)
               (line (default-line-height))
               (spec (create-image file nil nil :width w :height (* rows line) :ascent 'center
                                   :margin (cons 0 margin))))
    (mapcar (lambda (row) (propertize " " 'display `((slice 0 ,(* row line) ,w ,line) ,spec)))
            (number-sequence 0 (1- rows)))))

(defun romacs-agent-log-images--label (file)
  "What an image link to FILE shows in place of its text."
  (propertize (concat (romacs-agent-glyph 'image) " " (file-name-nondirectory file)) 'face 'romacs-agent-log-image-link))

(defun romacs-agent-log-images--shown (links width lines)
  "Of LINKS, (BEG END FILE) each, those whose file can be shown within
WIDTH pixels by LINES lines, each with its image: (BEG END FILE IMAGE)."
  (delq nil (mapcar (lambda (link)
                      (when-let* ((image (romacs-agent-log-images--image (nth 2 link) width lines)))
                        (append link (list image))))
                    links)))

(defun romacs-agent-log-images--pack (images width)
  "IMAGES, (W ROWS FILE) each, in groups side by side within WIDTH, nil
for each its own group."
  (let ((groups nil) (group nil) (used 0))
    (dolist (image images)
      (let ((more (+ (car image) (if group romacs-agent-log-images--gap 0))))
        (when (and group (> (+ used more) (or width 0)))
          (push (nreverse group) groups)
          (setq group nil used 0 more (car image)))
        (push image group)
        (setq used (+ used more))))
    (when group (push (nreverse group) groups))
    (nreverse groups)))

(defun romacs-agent-log-images--stack (images width)
  "IMAGES, (W ROWS FILE) each, as lines: side by side while they fit in
WIDTH (nil: one under another), a blank line between groups; the last
line's newline left to what follows.  A group's images are set off from
the text by a margin only when they are of one height: a margin makes
its row taller, which would cut through a taller image beside it."
  (let ((gap (propertize " " 'display `(space :width (,romacs-agent-log-images--gap)))))
    (mapconcat
     (lambda (group)
       (let* ((rows (mapcar #'cadr group))
              (margin (if (cl-every (lambda (n) (= n (car rows))) rows) romacs-agent-log-images--margin 0))
              (slices (mapcar (lambda (image) (cons (car image) (romacs-agent-log-images--slices image margin))) group)))
         (mapconcat
          (lambda (row)
            (concat romacs-agent-log-images--indent
                    (mapconcat (lambda (image)
                                 (or (nth row (cdr image))
                                     (propertize " " 'display `(space :width (,(car image))))))
                               slices gap)))
          (number-sequence 0 (1- (apply #'max rows)))
          "\n")))
     (romacs-agent-log-images--pack images width)
     "\n\n")))

;;; A sent message

(defun romacs-agent-log-images-message (text)
  "TEXT, a sent message, as the transcript shows it: (TEXT . ROWS), its
image links shown as their files' names, and ROWS their images' rows to
draw under it, nil when it links none that can be shown.  The text is
the message's still, for a copy or a search."
  (let* ((width (romacs-agent-log-images--width))
         (shown (romacs-agent-log-images--shown (romacs-agent-log-images-links text)
                                                (or width (* 80 (frame-char-width))) romacs-agent-log-image-lines)))
    (if (null shown) (cons text nil)
      (let ((text (copy-sequence text)))
        (pcase-dolist (`(,beg ,end ,file ,_) shown)
          (add-text-properties beg end (list 'display (romacs-agent-log-images--label file) 'help-echo file) text))
        (cons text (romacs-agent-log-images--stack (mapcar (lambda (link) (nth 3 link)) shown) width))))))

;;; The composer

(defvar-local romacs-agent-log-images--composer nil
  "The composer's image links as drawn: (LINKS . BLOCK), LINKS an overlay
over each link (shown as its file's name when its file can be shown),
and BLOCK the overlay on the draft's last character whose after-string
is their images; nil when it has none.")

(defun romacs-agent-log-images--drawn-p (links)
  "Whether the composer's overlays still stand for LINKS, (BEG END FILE) each."
  (let ((drawn (car romacs-agent-log-images--composer)))
    (and (= (length drawn) (length links))
         (cl-every (lambda (overlay link)
                     (and (overlay-buffer overlay)
                          (equal (list (overlay-start overlay) (overlay-end overlay)
                                       (overlay-get overlay 'romacs-agent-log-image))
                                 link)))
                   drawn links))))

(defun romacs-agent-log-images--clear ()
  "Take the composer's image overlays away."
  (when romacs-agent-log-images--composer
    (mapc #'delete-overlay (car romacs-agent-log-images--composer))
    (when-let* ((block (cdr romacs-agent-log-images--composer))) (delete-overlay block))
    (setq romacs-agent-log-images--composer nil)))

(defun romacs-agent-log-images--link-overlay (link image)
  "An overlay over LINK, (BEG END FILE), shown as its file's name when
IMAGE, its file's, is non-nil."
  (let ((overlay (make-overlay (nth 0 link) (nth 1 link) nil t nil)))
    (overlay-put overlay 'evaporate t)
    (overlay-put overlay 'romacs-agent-log-image (nth 2 link))
    (when image
      (overlay-put overlay 'display (romacs-agent-log-images--label (nth 2 link)))
      (overlay-put overlay 'help-echo (nth 2 link)))
    overlay))

(defun romacs-agent-log-images-composer (start)
  "Show the image links of the composer, from START to the buffer's end:
each as its file's name, their images under the draft.  Drawn again only
when its links change, which alone reads their files (their headers);
between, their images follow the draft's end."
  (let ((links (and (< start (point-max)) (romacs-agent-log-images-links nil start (point-max)))))
    (if (romacs-agent-log-images--drawn-p links)
        (when-let* ((block (cdr romacs-agent-log-images--composer)))
          (unless (= (overlay-end block) (point-max))
            (move-overlay block (max start (1- (point-max))) (point-max))))
      (romacs-agent-log-images--clear)
      (when links
        (let* ((width (romacs-agent-log-images--width))
               (images (mapcar (lambda (link)
                                 (romacs-agent-log-images--image (nth 2 link) (or width (* 80 (frame-char-width)))
                                                                 romacs-agent-log-image-composer-lines))
                               links))
               (shown (delq nil (copy-sequence images)))
               (block (when shown
                        ;; Not evaporating: a deleted last character leaves
                        ;; it to follow the draft's new end.
                        (let ((overlay (make-overlay (1- (point-max)) (point-max) nil t nil)))
                          ;; The cursor at the draft's end stays on its text.
                          (overlay-put overlay 'after-string
                                       (concat (propertize "\n" 'cursor t) (romacs-agent-log-images--stack shown width)))
                          overlay))))
          (setq romacs-agent-log-images--composer
                (cons (cl-mapcar #'romacs-agent-log-images--link-overlay links images) block)))))))

(provide 'romacs-agent-log-images)
;;; romacs-agent-log-images.el ends here
