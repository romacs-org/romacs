;;; romacs-help-index-tests.el --- The index of definitions help does not see yet -*- lexical-binding: t; -*-
(require 'ert)
(require 'cl-lib)
(require 'help-fns)
(require 'romacs-help-index)

(defconst romacs-help-index-tests--function-p
  (lambda (f) (or (fboundp f) (get f 'function-documentation)))
  "describe-function's predicate.")

(defconst romacs-help-index-tests--command-p #'commandp
  "describe-command's predicate.")

(defconst romacs-help-index-tests--variable-p
  (lambda (v) (or (get v 'variable-documentation) (and (not (keywordp v)) (boundp v))))
  "describe-variable's predicate, without its buffer.")

(defconst romacs-help-index-tests--symbol-p
  (lambda (s) (or (fboundp s) (boundp s) (facep s)))
  "What describe-symbol's backends accept.")

(defmacro romacs-help-index-tests--with-index (libraries &rest body)
  "Run BODY with the index of LIBRARIES (strings), none loaded yet, made
as a worker makes it, kept in a fresh cache file bound to `cache'."
  (declare (indent 1))
  `(let* ((cache (make-temp-file "romacs-help-index-cache-"))
          (romacs-help-index nil))
     (unwind-protect
         (progn
           (dolist (library ,libraries) (should-not (featurep (intern library))))
           (setq romacs-help-index (romacs-help-index-build ,libraries load-path cache))
           ,@body)
       (delete-file cache))))

(defun romacs-help-index-tests--read (name pred)
  "A help prompt over PRED returning NAME, as its completing read does."
  (let ((completing-read-function (lambda (&rest _) name)))
    (completing-read "Describe: " #'help--symbol-completion-table pred t)))

(ert-deftest romacs-help-index-reads-definitions-by-kind ()
  "The index has each definition by kind, commands apart, and making it
loads none of the libraries."
  (romacs-help-index-tests--with-index '("5x5" "mpuz" "icalendar-mode" "imenu" "python")
    (should (equal (gethash "5x5-grid-size" romacs-help-index) '((variable . "5x5"))))
    (should (assq 'command (gethash "5x5" romacs-help-index)))
    (should (assq 'function (gethash "5x5" romacs-help-index)))
    (should-not (assq 'command (gethash "5x5-draw-grid" romacs-help-index)))
    ;; A lambda in its body is interactive, the function is not.
    (should (gethash "imenu--create-keymap" romacs-help-index))
    (should-not (assq 'command (gethash "imenu--create-keymap" romacs-help-index)))
    (should (equal (gethash "imenu-sort-function" romacs-help-index) '((variable . "imenu"))))
    (should (assq 'variable (gethash "python-base-mode-map" romacs-help-index)))
    (should (equal (gethash "mpuz-unsolved" romacs-help-index) '((face . "mpuz"))))
    ;; Read as `read-symbol-shorthands' make it.
    (should (gethash "icalendar--format-decode-buffer" romacs-help-index))
    (should-not (gethash "ical:-format-decode-buffer" romacs-help-index))
    (should-not (featurep '5x5))))

(ert-deftest romacs-help-index-keeps-and-renews-its-cache ()
  "The index is read from its cache while no source changed, and read
from the sources again once one did."
  (let ((dir (make-temp-file "romacs-help-index-" t))
        (cache (make-temp-file "romacs-help-index-cache-")))
    (unwind-protect
        (let ((file (expand-file-name "rhi-probe.el" dir)) (path (list dir)))
          (with-temp-file file (insert "(defun rhi-probe-one () nil)\n"))
          (should (gethash "rhi-probe-one" (romacs-help-index-build '("rhi-probe") path cache)))
          (let ((key (romacs-help-index--key (romacs-help-index--sources '("rhi-probe") path))))
            (should (romacs-help-index--read-cache cache key)))
          ;; Another version of the code that reads the sources makes
          ;; another key: the cache is stale.
          (let ((romacs-help-index--file file))
            (should-not (romacs-help-index--read-cache
                         cache (romacs-help-index--key (romacs-help-index--sources '("rhi-probe") path)))))
          (with-temp-file file (insert "(defun rhi-probe-two () nil)\n"))
          (set-file-times file (time-add nil 5))
          (let ((index (romacs-help-index-build '("rhi-probe") path cache)))
            (should (gethash "rhi-probe-two" index))
            (should-not (gethash "rhi-probe-one" index))))
      (delete-directory dir t)
      (delete-file cache))))

(ert-deftest romacs-help-index-offers-unloaded-definitions-by-kind ()
  "Help lists an unloaded library's definitions of the kinds its prompt
accepts, C-h x its commands alone, and loads no file."
  (romacs-help-index-tests--with-index '("5x5" "mpuz")
    (let ((history (length load-history))
          (complete (lambda (input pred) (all-completions input #'help--symbol-completion-table pred))))
      (should (member "5x5-draw-grid" (funcall complete "5x5-" romacs-help-index-tests--function-p)))
      (should-not (member "5x5-grid-size" (funcall complete "5x5-" romacs-help-index-tests--function-p)))
      (should (member "5x5-grid-size" (funcall complete "5x5-" romacs-help-index-tests--variable-p)))
      (should (member "5x5" (funcall complete "5x5" romacs-help-index-tests--command-p)))
      (should-not (member "5x5-draw-grid" (funcall complete "5x5-" romacs-help-index-tests--command-p)))
      (should (member "mpuz-unsolved" (funcall complete "mpuz-" romacs-help-index-tests--symbol-p)))
      (should-not (member "mpuz-unsolved" (funcall complete "mpuz-" romacs-help-index-tests--function-p)))
      (should (test-completion "5x5-draw-grid" #'help--symbol-completion-table
                               romacs-help-index-tests--function-p))
      (should (= (length load-history) history))
      (should-not (featurep '5x5)))))

(ert-deftest romacs-help-index-loads-what-a-prompt-returns ()
  "A help prompt returning a name only the index knows loads its library,
though nothing has interned the name yet; one that quits loads none; C-h v
on an indexed variable whose name is a function too loads its library."
  (romacs-help-index-tests--with-index '("zone" "life")
    ;; Made here, so the reader interns no symbol of these names.
    (let ((name (concat "zone-when" "-idle")))
      (should-not (intern-soft name))
      (let ((completing-read-function (lambda (&rest _) (signal 'quit nil))))
        (should (eq (condition-case nil
                        (completing-read "Describe: " #'help--symbol-completion-table
                                         romacs-help-index-tests--function-p t)
                      (quit 'quit))
                    'quit)))
      (should-not (featurep 'zone))
      (should (equal (romacs-help-index-tests--read name romacs-help-index-tests--function-p) name))
      (should (featurep 'zone))
      (should (fboundp (intern name))))
    (let* ((name (concat "life-step" "-time")) (symbol (intern name)))
      (defalias symbol #'ignore)
      (unwind-protect
          (progn
            (romacs-help-index-tests--read name romacs-help-index-tests--variable-p)
            (should (featurep 'life))
            (should (boundp symbol)))
        (fmakunbound symbol)))))

(ert-deftest romacs-help-index-is-made-by-a-worker ()
  "The first help listing asks a worker for the index and returns at once;
the index arrives when the job does."
  ;; A worker repeats its spawner's boot: a bare batch VM's has no GNU Lisp.
  (skip-unless (and (fboundp 'isolate-spawn) (isolate-main-p) (eq isolate-boot 'world)))
  (let ((noninteractive nil) (romacs-help-index nil) (romacs-help-index--asked nil)
        (romacs-help-index--future nil) (romacs-help-index-last-error nil)
        (definition-prefixes (make-hash-table :test 'equal))
        (cache (make-temp-file "romacs-help-index-cache-")))
    (cl-letf (((symbol-function 'romacs-help-index--cache-file) (lambda () cache)))
      (unwind-protect
          (progn
            (puthash "5x5-" '("5x5") definition-prefixes)
            (all-completions "5x5-" #'help--symbol-completion-table romacs-help-index-tests--function-p)
            (should romacs-help-index--future)
            (should-not romacs-help-index)
            (isolate-await romacs-help-index--future 120)
            (should-not romacs-help-index-last-error)
            (should (gethash "5x5-draw-grid" romacs-help-index))
            (should-not (featurep '5x5)))
        (delete-file cache)))))

;;; romacs-help-index-tests.el ends here
