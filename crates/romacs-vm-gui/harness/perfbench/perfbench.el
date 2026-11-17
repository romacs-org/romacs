;;; perfbench.el --- perfbench.py's batch workloads, the same Lisp in GNU and Romacs  -*- lexical-binding: t -*-

;;; Commentary:
;; EMACS -Q --batch -l perfbench.el, with PERFBENCH_WORKLOAD naming one
;; of `perfbench-workloads', PERFBENCH_FIXTURES its fixture directory and
;; PERFBENCH_SKIP non-empty to run the setup alone (the run perfbench.py
;; subtracts).  Prints one line,
;;   PERFBENCH (:workload NAME :seconds S :check VALUE)
;; S being the body's wall time, VALUE what the body produced, compared
;; across engines so both did the same work.  `syntax' runs one operation
;; of the syntax-ppss lane's rig, PERFBENCH_OP, on PERFBENCH_FILE in
;; PERFBENCH_MODE: see `perfbench--syntax'.  Micro rows run the compiled
;; (GNU) or source (Romacs) PERFBENCH_MICRO file instead: see `micro'.
;; `elb-prepare' compiles GNU ELPA's elisp-benchmarks and reports its
;; entry names; `elb' runs one of them: see `perfbench--elb-load'.
;; Every result carries :loaded, the files the run loaded, which
;; perfbench.py checks against the compiled store's use log.

;;; Code:

(load (expand-file-name "perfbench-loaded.el" (file-name-directory load-file-name)) nil t t)

(defun perfbench--fixture (name)
  (expand-file-name name (getenv "PERFBENCH_FIXTURES")))

(defun perfbench--buffer (file)
  "A fundamental-mode buffer holding fixture FILE."
  (with-current-buffer (generate-new-buffer (file-name-nondirectory file))
    (insert-file-contents (perfbench--fixture file))
    (current-buffer)))

(defun perfbench--fontify (file mode)
  "Setup for font-lock-ensure on FILE in MODE: perf/font-lock's method."
  (lambda ()
    (let ((buffer (perfbench--buffer file)))
      (with-current-buffer buffer (funcall mode))
      (lambda () (with-current-buffer buffer (font-lock-ensure) (buffer-size))))))

(defun perfbench--in-buffer (file body)
  (lambda ()
    (let ((buffer (perfbench--buffer file)))
      (lambda () (with-current-buffer buffer (funcall body))))))

(defun perfbench--repo () (perfbench--fixture "emacs-45b0f769/"))

(defvar elb-bench-directory)
(defvar elb-speed)
(defvar elb-safety)
(defvar native-comp-speed)
(defvar compilation-safety)
(defvar native-comp-jit-compilation)
(defvar native-comp-eln-load-path)
(declare-function comp-el-to-eln-filename "comp")

(defun perfbench--elb-load (compile)
  "Load GNU ELPA's elisp-benchmarks from PERFBENCH_ELB_DIR; its entry names.
PERFBENCH_ELB_MODE says how: `source' loads each member file's source,
`elc' its byte-compiled file, `eln' its native-compiled file (in
PERFBENCH_ELN), compiled at the suite's own `elb-speed' and
`elb-safety' as its driver does, and `store' each by its name alone,
which Romacs satisfies from its compiled store given the `.elc' beside
the file.  With COMPILE, compile those not compiled yet: the package
directory is kept between runs.  JIT native compilation is off, so
`elc' stays bytecode."
  (add-to-list 'load-path (getenv "PERFBENCH_ELB_DIR"))
  (require 'elisp-benchmarks)
  (let ((mode (getenv "PERFBENCH_ELB_MODE"))
        (files (directory-files elb-bench-directory t "\\.el\\'"))
        (entries nil))
    (setq native-comp-jit-compilation nil)
    (when (equal mode "eln")
      (push (getenv "PERFBENCH_ELN") native-comp-eln-load-path))
    (when compile
      (when (equal mode "elc")
        (byte-compile-file (expand-file-name "elisp-benchmarks.el" (getenv "PERFBENCH_ELB_DIR"))))
      (dolist (file files)
        (pcase mode
          ("elc" (unless (or (file-exists-p (concat file "c")) (byte-compile-file file))
                   (error "byte-compile-file %s failed" file)))
          ("eln" (unless (file-exists-p (comp-el-to-eln-filename file))
                   (let ((native-comp-speed elb-speed) (compilation-safety elb-safety))
                     (native-compile file)))))))
    (dolist (file files)
      (load (pcase mode
              ("source" file)
              ("store" (file-name-sans-extension file))
              ("elc" (concat file "c"))
              ("eln" (comp-el-to-eln-filename file))
              (_ (error "PERFBENCH_ELB_MODE %S is not source, store, elc or eln" mode)))
            nil t (not (equal mode "store"))))
    (mapatoms (lambda (symbol)
                (when (and (fboundp symbol)
                           (string-match "\\`elb-\\(.*\\)-entry\\'" (symbol-name symbol)))
                  (push (match-string 1 (symbol-name symbol)) entries))))
    (sort entries #'string-lessp)))

(defun perfbench--elb-member ()
  "Setup for one member, PERFBENCH_ELB_ENTRY: the suite's own step, a
`garbage-collect' before the entry point runs."
  (perfbench--elb-load nil)
  (let ((entry (intern (format "elb-%s-entry" (getenv "PERFBENCH_ELB_ENTRY")))))
    (unless (or (getenv "PERFBENCH_SKIP") (fboundp entry))
      (error "No %s in this engine" entry))
    (garbage-collect)
    (lambda () (funcall entry) nil)))

(defun perfbench--key ()
  "One self-inserted character, then what the command loop's redisplay
and idle time would do after it: the syntax-ppss rig's key."
  (let ((last-command-event ?x) (this-command 'self-insert-command))
    (self-insert-command 1))
  (font-lock-ensure (line-beginning-position) (line-end-position))
  (ignore-errors (funcall show-paren-data-function)))

(defun perfbench--syntax ()
  "Setup for one syntax operation, PERFBENCH_OP, on PERFBENCH_FILE in
PERFBENCH_MODE.  Every operation but `font-lock-ensure' starts warm: the
buffer's syntax state computed and one key already typed, as in a buffer
being edited.  `font-lock-ensure' starts with the mode just turned on,
so a tree-sitter mode's first parse is part of it.  The setup does not depend on the
operation beyond that, so operations share their setup-only runs."
  (let ((op (getenv "PERFBENCH_OP"))
        (buffer (perfbench--buffer (getenv "PERFBENCH_FILE"))))
    (set-buffer buffer)
    (funcall (intern (getenv "PERFBENCH_MODE")))
    (electric-pair-mode 1)
    (show-paren-mode 1)
    (perfbench-middle)
    (let ((middle (point)) depth)
      (unless (equal op "font-lock-ensure")
        (syntax-ppss (point-max))
        (setq depth (car (syntax-ppss middle)))
        (perfbench--key)
        (setq middle (point)))
      (pcase op
        ;; Ten edits near the start, each followed by the state at the end:
        ;; the cache the edit invalidates, rebuilt over the whole buffer.
        ("syntax-ppss" (lambda () (dotimes (_ 10)
                                    (save-excursion (goto-char 200) (insert "x"))
                                    (syntax-ppss (point-max)))
                         (car (syntax-ppss (point-max)))))
        ;; Out to the top-level form around the middle, back and forth.
        ("scan-lists" (lambda () (let (ends)
                                   (dotimes (_ 500)
                                     (setq ends (list (scan-lists middle -1 depth) (scan-lists middle 1 depth))))
                                   ends)))
        ;; Over the file's leading comments and back.
        ("forward-comment" (lambda () (dotimes (_ 1000)
                                        (goto-char (point-min))
                                        (forward-comment (buffer-size))
                                        (forward-comment (- (buffer-size))))
                             (save-excursion (goto-char (point-min)) (forward-comment (buffer-size)) (point))))
        ("key" (lambda () (dotimes (_ 20) (perfbench--key)) (buffer-size)))
        ;; The parsers the mode made: a tree-sitter mode without its grammar
        ;; would fontify next to nothing, and this tells it apart.
        ("font-lock-ensure" (lambda () (font-lock-ensure)
                              (if (fboundp 'treesit-parser-list) (length (treesit-parser-list)) 0)))
        (_ (error "PERFBENCH_OP %S is unknown" op))))))

(defconst perfbench-workloads
  `((startup . ,(lambda () (lambda () nil)))
    (syntax . ,#'perfbench--syntax)
    (font-lock-org-news . ,(lambda () (require 'org)
                             (funcall (perfbench--fontify "emacs-45b0f769/etc/ORG-NEWS" #'org-mode))))
    (dired-5000 . ,(lambda () (require 'dired)
                     (lambda () (with-current-buffer (dired-noselect (perfbench--fixture "d5000/"))
                                  (count-lines (point-min) (point-max))))))
    (project-find-regexp
     . ,(lambda () (require 'project) (require 'xref)
          ;; project-current records the project in this file.
          (setq project-list-file (expand-file-name "projects.eld" temporary-file-directory))
          (lambda () (let ((default-directory (perfbench--repo)))
                       (project-find-regexp "\\<make_fixnum\\>")
                       (with-current-buffer "*xref*" (count-lines (point-min) (point-max)))))))
    (occur-xdisp . ,(perfbench--in-buffer "emacs-45b0f769/src/xdisp.c"
                                          (lambda () (occur "glyph_row")
                                            (with-current-buffer "*Occur*" (count-lines (point-min) (point-max))))))
    (describe-function
     . ,(lambda () (require 'help-fns)
          (lambda () (describe-function 'find-file)
            (with-current-buffer "*Help*"
              (goto-char (point-min))
              (if (search-forward "FILENAME" nil t) 1 0)))))
    (shell-command
     . ,(lambda () (lambda () (let ((length 0))
                                (dotimes (_ 20) (setq length (+ length (length (shell-command-to-string "echo perfbench")))))
                                length))))
    (micro . ,(lambda () (load (getenv "PERFBENCH_MICRO") nil t)
                (lambda () (m-run (string-to-number (getenv "PERFBENCH_N"))))))
    (elb-prepare . ,(lambda () (let ((entries (perfbench--elb-load t))) (lambda () entries))))
    (elb . ,#'perfbench--elb-member))
  "Workload name -> setup function returning the body to time.")

(let* ((name (intern (getenv "PERFBENCH_WORKLOAD")))
       (body (funcall (alist-get name perfbench-workloads))))
  (if (getenv "PERFBENCH_SKIP")
      ;; `:loaded' here too: what perfbench.py subtracts includes reporting it.
      (princ (format "PERFBENCH %S\n" (list :workload name :skipped t :loaded (perfbench-loaded))))
    (let* ((start (float-time))
           (check (funcall body))
           (seconds (- (float-time) start)))
      (princ (format "PERFBENCH %S\n" (list :workload name :seconds seconds :check check
                                             :loaded (perfbench-loaded)))))))

;;; perfbench.el ends here
