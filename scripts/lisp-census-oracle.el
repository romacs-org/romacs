;;; lisp-census-oracle.el --- GNU reader census oracle -*- lexical-binding: t; -*-

;;; Commentary:

;;; Run: emacs -Q --batch --script scripts/lisp-census-oracle.el --manifest FILE
;;; FILE is a JSON array of source path strings.  One process reads all files,
;;; without evaluation.  Output is a JSON object with a `files' array in manifest
;;; order, including duplicates.  Each file has status, form_count, forms and
;;; error.  Forms have zero-based, end-exclusive CHARACTER start/end offsets,
;;; a one-based start line, and head (the car's symbol name, or null).  Slice
;;; Python UTF-8 text with newline="" using these offsets; CRLF is preserved.
;;; form_count counts only complete forms; it is a whole-file total only when
;;; status is "ok".  A parse error retains the complete prefix and stops the
;;; file, never resynchronizing or treating premature EOF as success.
;;;
;;; Optional execution verdict (one file in a fresh process):
;;; emacs -Q --batch --script scripts/lisp-census-oracle.el --load-file FILE
;;; This DOES evaluate FILE and is NOT a sandbox.  Never use it for splitting.
;;; It reports only the first load error; no later forms are independently tried.
;;; Source stdout may precede the load verdict; read its LAST nonempty line as
;;; JSON.  Split mode emits only JSON.  A load can terminate the process outright.
;;;
;;; A JSON verdict exits zero even for file errors.  Invalid CLI/manifest input
;;; exits 2.  Process termination/timeouts during optional loading are the
;;; caller's responsibility, not successful verdicts.
;;;
;;; GNU reference: src/lread.c:2270-2287 distinguishes clean EOF before read;
;;; 2318 and 2334-2338 read then evaluate, with no per-form error catcher;
;;; 2116-2129 signals end-of-file inside a form.  The splitter intentionally
;;; omits the evaluation step.

;;; Code:

(require 'json)
(require 'seq)

(defun lisp-census-oracle--error (condition start line)
  "Describe CONDITION at zero-based START and one-based LINE."
  `((condition . ,(symbol-name (car condition)))
    (message . ,(error-message-string condition))
    (start . ,start)
    (line . ,line)
    (premature_eof . ,(if (eq (car condition) 'end-of-file) t :json-false))))

(defun lisp-census-oracle--skip-trivia ()
  "Skip reader whitespace and semicolon comments, without reading a form."
  (let ((again t))
    (while again
      (skip-chars-forward " \t\n\f\r\u00a0")
      (if (eq (char-after) ?\;)
          (forward-line 1)
        (setq again nil)))))

(defun lisp-census-oracle--split (path)
  "Read top-level forms in UTF-8 source PATH, never evaluating them."
  (with-temp-buffer
    (condition-case condition
        (progn
          (insert-file-contents-literally path)
          (decode-coding-region (point-min) (point-max) 'utf-8-unix)
          (goto-char (point-min))
          (let ((forms nil)
                (failure nil)
                (read-symbol-shorthands nil)
                (read-circle t))
            (lisp-census-oracle--skip-trivia)
            (while (and (not failure) (not (eobp)))
              (let ((start (1- (point)))
                    (line (line-number-at-pos)))
                (condition-case condition
                    (let* ((form (read (current-buffer)))
                           (head (and (consp form) (symbolp (car form))
                                      (symbol-name (car form)))))
                      (push `((start . ,start) (end . ,(1- (point)))
                              (line . ,line) (head . ,head))
                            forms))
                  (error
                   (setq failure
                         (lisp-census-oracle--error condition start line)))))
              (unless failure (lisp-census-oracle--skip-trivia)))
            `((path . ,path)
              (status . ,(if failure "parse-error" "ok"))
              (character_count . ,(buffer-size))
              (form_count . ,(length forms))
              (forms . ,(vconcat (nreverse forms)))
              (error . ,failure))))
      (error
       `((path . ,path) (status . "io-error") (form_count . 0)
         (forms . [])
         (error . ,(lisp-census-oracle--error condition 0 1)))))))

(defun lisp-census-oracle--load (path)
  "Return the first standalone GNU load verdict for PATH.
Load error locations are unknown, so start and line are null."
  (condition-case condition
      (progn
        (load (expand-file-name path) nil t t)
        `((path . ,path) (status . "ok") (error . nil)))
    ((error quit)
     `((path . ,path) (status . "load-error")
       (error . ,(lisp-census-oracle--error condition nil nil))))))

(defun lisp-census-oracle--manifest (path)
  "Return the JSON array of source paths from PATH, rejecting other shapes."
  (let ((paths (with-temp-buffer
                 (insert-file-contents path)
                 (json-parse-buffer :array-type 'array :null-object nil
                                    :false-object :json-false))))
    (unless (and (vectorp paths) (seq-every-p #'stringp paths))
      (error "Manifest must be a JSON array of path strings"))
    paths))

(defun lisp-census-oracle-main ()
  "Emit a manifest reader census or one explicitly requested load verdict."
  (condition-case condition
      (let* ((args (if (equal (car command-line-args-left) "--")
                       (cdr command-line-args-left)
                     command-line-args-left))
             (option (car args))
             (path (cadr args)))
        (setq command-line-args-left nil)
        (unless (and (= (length args) 2)
                     (member option '("--manifest" "--load-file")))
          (error "Use --manifest JSON-PATH or --load-file SOURCE-PATH"))
        (let ((report
               `((schema_version . 1)
                 (emacs_version . ,emacs-version)
                 (mode . ,(if (equal option "--manifest") "split" "load"))
                 (files . ,(if (equal option "--manifest")
                               (vconcat (mapcar #'lisp-census-oracle--split
                                                (lisp-census-oracle--manifest path)))
                             (vector (lisp-census-oracle--load path)))))))
          (when (equal option "--load-file") (terpri))
          (princ (json-encode report))
          (terpri)))
    (error
     (princ (json-encode `((error . ,(error-message-string condition)))))
     (terpri)
     (kill-emacs 2))))

(when noninteractive
  (lisp-census-oracle-main))

;;; lisp-census-oracle.el ends here
