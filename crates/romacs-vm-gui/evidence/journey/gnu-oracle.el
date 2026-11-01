;;; gnu-oracle.el --- GNU's answer for each motion/edit key of the ladder -*- lexical-binding: t -*-
;; emacs -Q --batch -l gnu-oracle.el FIXTURE KEYS-FILE OUT
;; Visits a copy of FIXTURE with find-file (so org-mode, as a user gets),
;; then for each line of KEYS-FILE (a `kbd' string) runs it as a keyboard
;; macro and records the command it ran and the state after it.
(require 'cl-lib)
(let* ((args command-line-args-left)
       (fixture (nth 0 args)) (keys-file (nth 1 args)) (out (nth 2 args))
       (copy (make-temp-file "journey-" nil ".org")))
  (setq command-line-args-left nil)
  (copy-file fixture copy t)
  (find-file copy)
  (setq-local line-move-visual nil)
  (let ((rows nil))
    (push (list :keys "" :command nil :point (point) :byte (1- (position-bytes (point)))
                :line (line-number-at-pos) :column (current-column)
                :modified (buffer-modified-p) :md5 (md5 (buffer-string)) :mode major-mode)
          rows)
    (dolist (k (with-temp-buffer (insert-file-contents keys-file) (split-string (buffer-string) "\n" t)))
     (if (equal k "RESET")
         (progn (set-buffer-modified-p nil) (kill-buffer (current-buffer))
                (copy-file fixture copy t) (find-file copy) (setq-local line-move-visual nil)
                (push (list :keys "RESET" :point (point) :byte (1- (position-bytes (point)))
                            :md5 (md5 (buffer-string)) :mode major-mode) rows))
      (let ((cmd (key-binding (kbd k))) (err nil))
        (condition-case e (execute-kbd-macro (kbd k)) (error (setq err (error-message-string e))))
        (push (list :keys k :command cmd :error err :point (point) :byte (1- (position-bytes (point)))
                    :line (line-number-at-pos) :column (current-column)
                    :modified (buffer-modified-p) :md5 (md5 (buffer-string))
                    :next-line-invisible (and (< (line-end-position) (point-max))
                                              (invisible-p (1+ (line-end-position))) t)
                    :line-text (buffer-substring-no-properties (line-beginning-position) (line-end-position)))
              rows))))
    (with-temp-file out
      (insert (json-encode (mapcar (lambda (r) (cl-loop for (k v) on r by #'cddr
                                                        collect (cons (substring (symbol-name k) 1)
                                                                      (if (symbolp v) (and v (symbol-name v)) v))))
                                   (nreverse rows)))))))
