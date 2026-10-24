;;; gnu_driver.el --- one-case GNU oracle for varh  -*- lexical-binding: t -*-
;; Variables are global state, so varh.py starts a FRESH `emacs -Q --batch'
;; (private temporary HOME) for every case and every regime.  This driver
;; evaluates exactly ONE case in ONE regime and prints one JSON line:
;;
;;   VARH_CASE    file holding {"setup": TEXT-or-null, "source": TEXT}
;;   VARH_REGIME  "compiled"    -> (funcall (byte-compile `(lambda () ,FORM)))
;;                "interpreted" -> (eval FORM t)
;;
;; SETUP (when present) is evaluated first, interpreted, as its own top-level
;; form -- the VM2 replay evaluates it as its own top-level source too -- so a
;; `defvar' in it is known to the byte compiler when SOURCE is compiled.
;; SOURCE is primh's case wrapper, so it always returns a string; anything
;; else (or an escaped error) is reported as HARNESS-ERROR and varh.py
;; refuses to write evidence.
(setq byte-compile-warnings nil)
(set-language-environment "UTF-8")
(set-terminal-coding-system 'utf-8-unix)
(set-keyboard-coding-system 'utf-8-unix)
(defun varh-out (x) (message "%s" (decode-coding-string (json-serialize x) 'utf-8)))
(let* ((case (with-temp-buffer
               (insert-file-contents (getenv "VARH_CASE"))
               (json-parse-buffer :object-type 'alist :array-type 'list
                                  :null-object nil)))
       (regime (getenv "VARH_REGIME"))
       (setup (alist-get 'setup case))
       (source (alist-get 'source case))
       (out (condition-case e
                (progn
                  (when setup (eval (car (read-from-string setup)) t))
                  (let* ((form (car (read-from-string source)))
                         (v (cond ((equal regime "compiled")
                                   (funcall (byte-compile `(lambda () ,form))))
                                  ((equal regime "interpreted") (eval form t))
                                  (t (error "unknown regime %S" regime)))))
                    (if (stringp v) v (format "HARNESS-ERROR non-string %S" v))))
              (error (format "HARNESS-ERROR %S" e)))))
  (varh-out (list (cons 'version emacs-version)
                  (cons 'revision (or emacs-repository-version ""))
                  (cons 'regime regime)
                  (cons 'outcome out))))
