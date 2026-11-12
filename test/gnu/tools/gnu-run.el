;;; gnu-run.el --- run one GNU test file in GNU Emacs, one JSON line per test  -*- lexical-binding: t -*-

;; Run by GNU Emacs only:
;;   emacs -Q --batch -l gnu-run.el MODE FILE [PLAN.json [HARNESS.el]]
;;
;; MODE real  The ORACLE.  GNU's own ert.el; the file is `load'ed exactly as
;;            `make check' would, then every test DEFINED IN THAT FILE runs
;;            through `ert-run-test'.  These verdicts are the specification
;;            romacs is measured against.
;; MODE mini  The HARNESS FIDELITY check.  The romacs-mini-ert harness and the
;;            per-form evaluation romacs uses (PLAN.json from extract-forms.el),
;;            but in GNU.  Wherever this disagrees with MODE real, the harness,
;;            not romacs, is wrong; run.py refuses to count those tests as
;;            passing on romacs.
;;
;; Output lines (stdout), flushed one by one so a killed run keeps its prefix:
;;   {"event":"load","ok":BOOL,"error":...}
;;   {"event":"result","test":NAME,"outcome":O,"expected":E,"as_expected":B,
;;    "tags":[...],"detail":...}
;; O is passed | failed (an assertion) | errored (any other signal) | skipped
;; | quit.  E is the test's :expected-result type as written.

(require 'json)

(defun gnu-run--emit (plist)
  (princ (concat (decode-coding-string (json-serialize plist) 'utf-8) "\n")))

(defun gnu-run--string (value)
  (let ((print-length 20) (print-level 6))
    (condition-case nil
        (let ((s (format "%S" value)))
          (if (> (length s) 2000) (substring s 0 2000) s))
      (error "#<unprintable>"))))

(defun gnu-run--outcome-of-condition (condition)
  (if (eq (car-safe condition) 'ert-test-failed) "failed" "errored"))

(let* ((mode (pop command-line-args-left))
       (file (file-truename (expand-file-name (pop command-line-args-left))))
       (plan-file (pop command-line-args-left))
       (harness (pop command-line-args-left)))
  (setq command-line-args-left nil)
  (cond
   ((member mode '("real" "compiled"))
    ;; "real" loads the source (eager macroexpansion, interpreted closures);
    ;; "compiled" byte-compiles it first, as `make check' does.  GNU itself
    ;; can disagree across the two regimes (e.g. `nth' on a dotted list
    ;; signals different data from bytecode.c and fns.c); romacs matching
    ;; EITHER regime is parity.
    (require 'ert)
    (require 'bytecomp)
    (let ((load-error nil)
          (target file))
      (when (equal mode "compiled")
        (let* ((dir (make-temp-file "gnu-run-elc-" t))
               (byte-compile-dest-file-function
                (lambda (f) (expand-file-name (concat (file-name-nondirectory f) "c") dir))))
          (condition-case err
              (if (byte-compile-file file)
                  (setq target (funcall byte-compile-dest-file-function file))
                (setq load-error '(error "byte-compile-file failed")))
            (error (setq load-error err)))))
      (unless load-error
        (condition-case err
            (load target nil t t)
          (error (setq load-error err))))
      (gnu-run--emit (list :event "load" :ok (if load-error :false t)
                           :error (if load-error (gnu-run--string load-error) :null)))
      ;; `make check' runs the tests after the file's `load' has returned, so
      ;; `load-file-name' is nil then; this script is itself being `load'ed,
      ;; so unbind it explicitly or tests resolve resources next to THIS file.
      (let ((load-file-name nil) (load-true-file-name nil) (load-in-progress nil) (current-load-list nil))
      (dolist (test (ert-select-tests t t))
        (when (and (ert-test-file-name test)
                   (equal (file-truename (ert-test-file-name test)) file))
          (let* ((result (ert-run-test test))
                 (outcome (cond ((ert-test-passed-p result) "passed")
                                ((ert-test-skipped-p result) "skipped")
                                ((ert-test-quit-p result) "quit")
                                ((ert-test-failed-p result)
                                 (gnu-run--outcome-of-condition
                                  (ert-test-result-with-condition-condition result)))
                                (t "unknown")))
                 (detail (if (ert-test-result-with-condition-p result)
                             (gnu-run--string (ert-test-result-with-condition-condition result))
                           :null)))
            (gnu-run--emit
             (list :event "result"
                   :test (symbol-name (ert-test-name test))
                   :outcome outcome
                   :expected (gnu-run--string (ert-test-expected-result-type test))
                   :as_expected (if (ert-test-result-expected-p test result) t :false)
                   :tags (vconcat (mapcar #'gnu-run--string (ert-test-tags test)))
                   :detail detail))))))))
   ((equal mode "mini")
    (load harness nil t t)
    (let* ((plan (with-temp-buffer
                   (insert-file-contents plan-file)
                   (json-parse-buffer :object-type 'plist :null-object nil)))
           (load-file-name file)
           (load-true-file-name file)
           (load-in-progress t)
           ;; `macroexp-file-name' (hence `ert-resource-directory') reads
           ;; the innermost load from here, as it would under `load'.
           (current-load-list (list file)))
      (seq-doseq (form (plist-get plan :forms))
        (let ((source (plist-get form :source)))
          (if (not source)
              (gnu-run--emit (list :event "form" :ordinal (plist-get form :ordinal) :ok :false
                                   :error "not extractable"))
            (condition-case err
                (progn (eval (car (read-from-string source)) t)
                       (gnu-run--emit (list :event "form" :ordinal (plist-get form :ordinal) :ok t)))
              (error (gnu-run--emit (list :event "form" :ordinal (plist-get form :ordinal) :ok :false
                                          :error (gnu-run--string err)))))))))
    ;; Audit broad should-error catches without modifying mini's assertions.
    (defvar gnu-run--vacuous nil)
    (defvar gnu-run--caught nil)
    (advice-add 'romacs-gnu-tests--should-error :around
                (lambda (original form thunk type exclude)
                  (let* ((condition (funcall original form thunk type exclude))
                         (kind (car-safe condition))
                         (datum (cadr condition)))
                    (when (and (memq kind '(void-function void-variable invalid-function))
                               (not (memq kind (if (listp type) type (list type)))))
                      ;; Match the VM audit's representation, including multiplicity.
                      ;; A matching condition TYPE alone can hide a missing subject.
                      (push (vector (symbol-name kind)
                                    (if (and datum (symbolp datum)) (symbol-name datum) ""))
                            gnu-run--caught)
                      (when (eq kind 'void-function)
                        (push (symbol-name datum) gnu-run--vacuous)))
                    condition)))
    (let ((load-file-name nil) (load-true-file-name nil) (load-in-progress nil) (current-load-list nil))
    (dotimes (i (romacs-gnu-tests--count))
      (setq gnu-run--vacuous nil gnu-run--caught nil)
      (romacs-gnu-tests--run i)
      (gnu-run--emit
       (list :event "result"
             :test (romacs-gnu-tests--name i)
             :outcome (symbol-name (car romacs-gnu-tests--last))
             :expected (romacs-gnu-tests--expected i)
             :vacuous_void_function (vconcat (reverse gnu-run--vacuous))
             :caught (vconcat (reverse gnu-run--caught))
             :detail (gnu-run--string (cdr romacs-gnu-tests--last)))))))
   (t (error "gnu-run: MODE must be real or mini, not %S" mode)))
  (gnu-run--emit (list :event "done")))
