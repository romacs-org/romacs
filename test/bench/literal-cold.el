;;; literal-cold.el --- First folded literal search, no warmup -*- lexical-binding: t; -*-
;; Run each sample in a fresh -Q --batch process with -l this file.
;; Time the first search itself, including lazy standard-fold construction.
(let ((case-fold-search t)
      (gc-cons-threshold most-positive-fixnum))
  (with-temp-buffer
    (insert "prefix ÀΣЖ suffix")
    (goto-char (point-min))
    (let* ((start (float-time))
           (found (search-forward "àσж" nil t))
           (elapsed (* 1000.0 (- (float-time) start))))
      (unless (and (equal found 11)
                   (equal (match-beginning 0) 8)
                   (equal (match-end 0) 11))
        (error "Cold literal result mismatch: %S %S" found (match-data)))
      (princ (format "literal-cold-ms %.9f\n" elapsed)))))
