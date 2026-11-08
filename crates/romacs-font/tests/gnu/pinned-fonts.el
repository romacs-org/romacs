;;; pinned-fonts.el --- GNU's font choice for romacs-font's tests  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;; Run on a window system: emacs -Q --no-splash -l pinned-fonts.el, with
;; ORACLE_OUTPUT naming the transcript and ORACLE_FACES a list of face
;; attribute lists, read as Lisp.  For each, the font `font-at' reports for
;; every character of "Καλημέρα Жж A" after redisplay, and the pixel width
;; of "Καλημέρα", over a default face of Liberation Mono at :height 120
;; (16 pixels at 96 dpi).  pinned-fonts.txt is the transcript.

(require 'cl-lib)
(with-temp-file (getenv "ORACLE_OUTPUT")
  (insert (format "emacs-version: %s\nemacs-repository-version: %s\nwindow-system: %S\npgtk: %S\nFONTCONFIG_FILE: %s\ndisplay-mm: %S\ndisplay-pixels: %S\n"
                  emacs-version emacs-repository-version window-system (featurep 'pgtk) (getenv "FONTCONFIG_FILE")
                  (cons (display-mm-width) (display-mm-height)) (cons (display-pixel-width) (display-pixel-height)))))
(dolist (attrs (car (read-from-string (getenv "ORACLE_FACES"))))
  (with-current-buffer (get-buffer-create "*font-oracle*")
    (erase-buffer)
    (set-face-attribute 'default nil :family "Liberation Mono" :height 120 :weight 'normal :slant 'normal :width 'normal)
    (insert (propertize "Καλημέρα Жж A" 'face attrs))
    (switch-to-buffer (current-buffer))
    (redisplay t)
    (let ((rows (cl-loop for p from (point-min) below (point-max)
                         for f = (font-at p)
                         collect (list (char-after p) (font-get f :family) (font-get f :size) (font-get f :weight)
                                       (font-get f :slant) (font-get f :width)
                                       (file-name-nondirectory (or (aref (font-info f) 12) "")))))
          (width (string-pixel-width (propertize "Καλημέρα" 'face attrs))))
      (with-temp-buffer
        (insert (format "\nface: %S (default Liberation Mono :height 120)\n" attrs))
        (dolist (r rows)
          (insert (apply #'format "U+%04X %c family=%S size=%S weight=%S slant=%S width=%S file=%s\n" (car r) r)))
        (insert (format "string-pixel-width Καλημέρα: %S\n" width))
        (append-to-file (point-min) (point-max) (getenv "ORACLE_OUTPUT"))))))
(kill-emacs 0)
