;;; extract-forms.el --- split a GNU test file into its top-level forms  -*- lexical-binding: t -*-

;; Run by GNU Emacs, never by romacs:
;;   emacs -Q --batch -l extract-forms.el FILE OUT.json
;;
;; GNU's own reader finds the boundaries, so every form romacs is later asked
;; to evaluate is exactly the text GNU's `load' would have read, decoded the
;; way GNU decodes the file (coding cookie included).  Nothing is evaluated.
;; A form whose text cannot be carried as JSON (raw bytes) is recorded as
;; unextractable rather than dropped, so the runner can report it.

(require 'json)

(let* ((file (expand-file-name (pop command-line-args-left)))
       (out (pop command-line-args-left))
       (forms nil) (ordinal 0) (deftests 0) (error-text nil) (shorthands nil))
  (with-temp-buffer
    (insert-file-contents file)
    ;; GNU's `load' honours file-local `read-symbol-shorthands' (fns-tests.el
    ;; reads ft- as fns-tests-), so deftest names must be read the same way.
    (let ((enable-local-variables :all) (enable-local-eval nil) (buffer-file-name file))
      (hack-local-variables))
    (setq shorthands read-symbol-shorthands)
    (with-syntax-table emacs-lisp-mode-syntax-table
      (goto-char (point-min))
      (condition-case err
          (while (progn (forward-comment (buffer-size)) (not (eobp)))
            (let* ((start (point))
                   (form (read (current-buffer)))
                   (end (point))
                   (line (line-number-at-pos start))
                   (name (and (eq (car-safe form) 'ert-deftest)
                              (symbolp (cadr form))
                              (symbol-name (cadr form))))
                   (text (buffer-substring-no-properties start end))
                   (entry (list :ordinal (setq ordinal (1+ ordinal))
                                :line line
                                :deftest (or name :null))))
              (when name (setq deftests (1+ deftests)))
              (condition-case nil
                  (progn (json-serialize text)
                         (setq entry (append entry (list :source text))))
                (error (setq entry (append entry (list :unextractable "form text is not valid Unicode")))))
              (push entry forms)))
        (error (setq error-text (format "%S" err))))))
  ;; `json-serialize' returns UTF-8 octets; write them as they are.
  (let ((coding-system-for-write 'binary))
   (with-temp-file out
    (set-buffer-multibyte nil)
    (insert (json-serialize
             (list :file file
                   :forms (vconcat (nreverse forms))
                   :toplevel_deftests deftests
                   :shorthands (vconcat (mapcar (lambda (c) (vector (car c) (cdr c))) shorthands))
                   :read_error (or error-text :null)))))))
