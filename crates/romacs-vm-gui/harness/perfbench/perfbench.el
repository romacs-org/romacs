;;; perfbench.el --- perfbench.py's batch workloads, the same Lisp in GNU and Romacs  -*- lexical-binding: t -*-

;;; Commentary:
;; EMACS -Q --batch -l perfbench.el, with PERFBENCH_WORKLOAD naming one
;; of `perfbench-workloads', PERFBENCH_FIXTURES its fixture directory and
;; PERFBENCH_SKIP non-empty to run the setup alone (the run perfbench.py
;; subtracts).  Prints one line,
;;   PERFBENCH (:workload NAME :seconds S :check VALUE)
;; S being the body's wall time, VALUE what the body produced, compared
;; across engines so both did the same work.  Micro rows run the compiled
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

(defun perfbench--wait-processes (buffer)
  "Wait until BUFFER has no live process and its sentinels have run."
  (while (get-buffer-process buffer)
    (accept-process-output (get-buffer-process buffer) 0.02))
  (accept-process-output nil 0))

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
the file.  With COMPILE, compile them first.  JIT native compilation is
off, so `elc' stays bytecode."
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
          ("elc" (unless (byte-compile-file file) (error "byte-compile-file %s failed" file)))
          ("eln" (let ((native-comp-speed elb-speed) (compilation-safety elb-safety))
                   (native-compile file))))))
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

(defconst perfbench-workloads
  `((startup . ,(lambda () (lambda () nil)))
    (font-lock-fileio-c . ,(perfbench--fontify "emacs-45b0f769/src/fileio.c" #'c-mode))
    (font-lock-subr-el . ,(perfbench--fontify "emacs-45b0f769/lisp/subr.el" #'emacs-lisp-mode))
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
    (vc-log-2000
     . ,(lambda () (require 'vc) (require 'vc-git)
          (lambda () (let ((default-directory (perfbench--repo)))
                       (vc-print-log-internal 'Git (list (perfbench--repo)) "45b0f7699df36b3849edc3bdef37d4cb9e75b9ea"
                                              t 2000)
                       (perfbench--wait-processes "*vc-change-log*")
                       (with-current-buffer "*vc-change-log*" (count-lines (point-min) (point-max)))))))
    (occur-xdisp . ,(perfbench--in-buffer "emacs-45b0f769/src/xdisp.c"
                                          (lambda () (occur "glyph_row")
                                            (with-current-buffer "*Occur*" (count-lines (point-min) (point-max))))))
    (how-many-xdisp . ,(perfbench--in-buffer "emacs-45b0f769/src/xdisp.c"
                                             (lambda () (how-many "\\_<int\\_>" (point-min) (point-max)))))
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
