;;; props-bench-test.el --- P0 benchmark contract checks -*- lexical-binding: t; -*-
(require 'ert)
(load (expand-file-name "core-bench.el" (file-name-directory load-file-name)) nil t)

(ert-deftest props-bench-all-thirteen-rows-registered ()
  (should (= 13 (length core-bench--props-rows)))
  (dolist (row core-bench--props-rows)
    (should (functionp (cdr (assq (intern (car row)) core-bench--all)))))
  (should (= 5 (length core-bench--props-memory-rows))))

(ert-deftest props-bench-scalar-wrong-work-is-red ()
  (dolist (row '("buf-prop-get-prop-rand" "buf-prop-next-single-scan"
                 "buf-prop-props-at-scan" "buf-markers-1e4-edits"
                 "buf-markers-1e3-typing" "buf-markers-make-1e5"))
    (let ((core-bench--props-result 0))
      (should-error (core-bench--check row 0)))))

(ert-deftest props-bench-property-wrong-work-is-red ()
  (let ((core-bench--props-buffer (generate-new-buffer " *props-wrong*")))
    (unwind-protect
        (progn
          (with-current-buffer core-bench--props-buffer
            (insert (propertize "wrong" 'face 'italic)))
          (dolist (row '("buf-prop-tablist-print" "buf-prop-tablist-sort"
                         "buf-prop-insert-propertized" "buf-prop-insert-then-add"
                         "buf-prop-fontlock-put" "buf-prop-fontlock-refont"
                         "buf-prop-fontlock-elisp"))
            (should-error (core-bench--check row 0))))
      (kill-buffer core-bench--props-buffer))))

(ert-deftest props-bench-prepare-releases-previous-buffer-and-markers ()
  (let ((core-bench--props-buffer nil) (core-bench--props-keep nil))
    (unwind-protect
        (progn
          (core-bench--props-prepare "buf-markers-make-1e5")
          (let ((previous core-bench--props-buffer))
            (with-current-buffer previous
              (insert "x")
              (setq core-bench--props-keep (list (copy-marker 1))))
            (core-bench--props-prepare "buf-markers-make-1e5")
            (should-not (buffer-live-p previous))
            (should-not core-bench--props-keep)))
      (when (buffer-live-p core-bench--props-buffer)
        (kill-buffer core-bench--props-buffer)))))
