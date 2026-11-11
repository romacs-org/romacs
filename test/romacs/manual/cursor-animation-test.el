;;; cursor-animation-test.el --- Test cursor animation -*- lexical-binding: t -*-

;; Test smooth cursor animation, buffer crossfade, and scroll slide.
;; Designed to be driven by xdotool from run-cursor-animation-test.sh.

(defun cursor-animation-test--setup ()
  "Set up test buffers with content."
  ;; Create buffer A with numbered lines
  (let ((buf-a (get-buffer-create "*AnimTest-A*")))
    (with-current-buffer buf-a
      (erase-buffer)
      (dotimes (i 100)
        (insert (format "Buffer A - Line %03d: The quick brown fox jumps over the lazy dog\n" (1+ i))))
      (goto-char (point-min))))

  ;; Create buffer B with different content
  (let ((buf-b (get-buffer-create "*AnimTest-B*")))
    (with-current-buffer buf-b
      (erase-buffer)
      (dotimes (i 100)
        (insert (format "Buffer B - Line %03d: Pack my box with five dozen liquor jugs\n" (1+ i))))
      (goto-char (point-min))))

  ;; Switch to buffer A
  (switch-to-buffer "*AnimTest-A*")
  (goto-char (point-min))
  (message "Cursor animation test ready"))

(cursor-animation-test--setup)
