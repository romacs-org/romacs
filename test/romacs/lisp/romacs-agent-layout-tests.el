;;; romacs-agent-layout-tests.el --- A session record's layout changed under live sessions -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))

(defmacro romacs-agent-layout-tests--warnings (&rest body)
  "The messages of the `romacs-agent' warnings BODY gave, in order."
  `(let ((warned nil))
     (cl-letf (((symbol-function 'display-warning)
                (lambda (type message &rest _) (when (eq type 'romacs-agent) (push message warned)))))
       (let ((romacs-agent--layout-warned nil)) ,@body))
     (nreverse warned)))

(ert-deftest romacs-agent-layout-a-change-under-live-records-warns-and-loads ()
  ;; An older layout, with a record alive in the sessions table.
  (eval '(romacs-agent--defstruct (romacs-agent-layout-probe (:constructor romacs-agent-layout-probe--make))
           "A probe." id state)
        t)
  (let* ((romacs-agent--sessions (make-hash-table :test #'equal))
         (record (funcall 'romacs-agent-layout-probe--make :id "a" :state 'idle)))
    (puthash "a" record romacs-agent--sessions)
    ;; The new layout loaded live: it loads, with one warning naming what
    ;; changed and the restart that carries sessions over.
    (let ((warned (romacs-agent-layout-tests--warnings
                   (eval '(romacs-agent--defstruct (romacs-agent-layout-probe (:constructor romacs-agent-layout-probe--make))
                            "A probe." id state stopped)
                         t))))
      (should (= (length warned) 1))
      (should (string-match-p "fields of .romacs-agent-layout-probe. (added: stopped; removed: ) while 1 live records" (car warned)))
      (should (string-match-p "restart Romacs" (car warned))))
    (should (equal (mapcar #'car (cdr (cl-struct-slot-info 'romacs-agent-layout-probe))) '(id state stopped)))
    ;; Fields in another order: warned of as well.
    (should (romacs-agent-layout-tests--warnings
             (eval '(romacs-agent--defstruct (romacs-agent-layout-probe (:constructor romacs-agent-layout-probe--make))
                      "A probe." state id stopped)
                   t)))
    ;; With no live record of the old layout, nothing to warn of.
    (clrhash romacs-agent--sessions)
    (should-not (romacs-agent-layout-tests--warnings
                 (eval '(romacs-agent--defstruct (romacs-agent-layout-probe (:constructor romacs-agent-layout-probe--make))
                          "A probe." id state stopped)
                       t)))
    (should (eq (funcall 'romacs-agent-layout-probe-stopped (funcall 'romacs-agent-layout-probe--make :stopped t)) t))))

(ert-deftest romacs-agent-layout-a-record-a-view-keeps-counts ()
  ;; A history view keeps its own session record, in no table.
  (eval '(romacs-agent--defstruct (romacs-agent-layout-view (:constructor romacs-agent-layout-view--make))
           "A probe." id)
        t)
  (let ((romacs-agent--sessions (make-hash-table :test #'equal)))
    (with-temp-buffer
      (setq-local romacs-agent-layout-tests--session (funcall 'romacs-agent-layout-view--make :id "v"))
      (should (romacs-agent-layout-tests--warnings
               (eval '(romacs-agent--defstruct (romacs-agent-layout-view (:constructor romacs-agent-layout-view--make))
                        "A probe." id stopped)
                     t))))))

(defvar romacs-agent-layout-tests--session nil)

(ert-deftest romacs-agent-layout-reloading-the-same-file-is-allowed ()
  ;; romacs-agent.el loaded again over live sessions of its own layout.
  (let ((romacs-agent--sessions (make-hash-table :test #'equal))
        (romacs-mcp-call-function romacs-mcp-call-function)
        (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
    (puthash "s" (romacs-agent--make :id "s" :harness 'claude) romacs-agent--sessions)
    (load "romacs-agent" nil t)
    (should (equal (romacs-agent-id (gethash "s" romacs-agent--sessions)) "s"))))

(defvar romacs-agent-layout-tests--evaluated nil)

(defmacro romacs-agent-layout-tests--with-file (text &rest body)
  "BODY with `file', a romacs-agent.el of TEXT, and a live session."
  (declare (indent 1))
  `(let* ((dir (make-temp-file "romacs-layout-" t))
          (file (expand-file-name "romacs-agent.el" dir))
          (romacs-agent--sessions (make-hash-table :test #'equal)))
     (unwind-protect
         (progn (with-temp-file file (insert ,text))
                (puthash "s" (romacs-agent--make :id "s" :harness 'claude) romacs-agent--sessions)
                ,@body)
       (delete-directory dir t))))

(defun romacs-agent-layout-tests--definition (head slots)
  "A file's text: a marker, then the session record defined by HEAD with SLOTS."
  (format "(setq romacs-agent-layout-tests--evaluated t)\n(%s (romacs-agent (:constructor romacs-agent--make))\n  \"One.\" %s)\n"
          head (mapconcat #'symbol-name slots " ")))

(ert-deftest romacs-agent-layout-a-file-without-the-guard-warns-and-loads ()
  ;; Its own definition would not check: the live one reads it first and
  ;; warns, and the file loads.
  (romacs-agent-layout-tests--with-file
      (romacs-agent-layout-tests--definition "cl-defstruct" (romacs-agent--slot-names 'romacs-agent))
    (setq romacs-agent-layout-tests--evaluated nil)
    (should (string-match-p "predates the session layout guard"
                            (car (romacs-agent-layout-tests--warnings (load file nil t)))))
    (should romacs-agent-layout-tests--evaluated)
    ;; Without live sessions there is nothing it can break.
    (clrhash romacs-agent--sessions)
    (should-not (romacs-agent--load-check file))))

(ert-deftest romacs-agent-layout-a-file-with-other-fields-warns ()
  ;; The same fields in another order: as breaking as a new one.
  (romacs-agent-layout-tests--with-file
      (romacs-agent-layout-tests--definition "romacs-agent--defstruct"
                                             (reverse (romacs-agent--slot-names 'romacs-agent)))
    (should (string-match-p "another session record than the live one"
                            (car (romacs-agent-layout-tests--warnings (romacs-agent--load-check file)))))
    (let ((romacs-agent-load-unchecked t))
      (should-not (romacs-agent-layout-tests--warnings (romacs-agent--load-check file))))))

(ert-deftest romacs-agent-layout-a-file-without-the-live-commit-warns ()
  ;; The live one came from commit "live"; a checkout without it would undo
  ;; its changes.
  (romacs-agent-layout-tests--with-file
      (romacs-agent-layout-tests--definition "romacs-agent--defstruct" (romacs-agent--slot-names 'romacs-agent))
    (let ((romacs-agent--loaded-from "live") (holds nil))
      (cl-letf (((symbol-function 'romacs-agent--checkout-commit) (lambda (_file) "other"))
                ((symbol-function 'romacs-agent--descends-p) (lambda (_file commit) (and (equal commit "live") holds))))
        (should (string-match-p "without commit live"
                                (car (romacs-agent-layout-tests--warnings (romacs-agent--load-check file)))))
        ;; Git cannot tell (the commit not in that repository): warned of too.
        (setq holds 'unknown)
        (should (string-match-p "cannot tell whether it holds commit live"
                                (car (romacs-agent-layout-tests--warnings (romacs-agent--load-check file)))))
        (setq holds t)
        (should-not (romacs-agent-layout-tests--warnings (romacs-agent--load-check file)))))))

(ert-deftest romacs-agent-layout-the-file-load-reads-is-checked ()
  ;; A relative name: `load' searches load-path, never the current
  ;; directory, so a guarded copy there neither warns nor excuses it.
  (romacs-agent-layout-tests--with-file
      (romacs-agent-layout-tests--definition "cl-defstruct" (romacs-agent--slot-names 'romacs-agent))
    (let* ((elsewhere (make-temp-file "romacs-layout-cwd-" t))
           (default-directory (file-name-as-directory elsewhere))
           (load-path (cons (file-name-directory file) load-path)))
      (unwind-protect
          (progn
            (with-temp-file (expand-file-name "romacs-agent.el" elsewhere)
              (insert (romacs-agent-layout-tests--definition "romacs-agent--defstruct"
                                                             (romacs-agent--slot-names 'romacs-agent))))
            (dolist (name '("romacs-agent" "romacs-agent.el"))
              (should (romacs-agent-layout-tests--warnings (romacs-agent--load-check name))))
            ;; As load itself is called, its suffix arguments too: each
            ;; warns, and the file runs.
            (dolist (call `(("romacs-agent.el" nil t nil t) ("romacs-agent" nil t nil t)
                            (,file nil t nil t) (,file nil t t) ("romacs-agent.el" nil t t)))
              (setq romacs-agent-layout-tests--evaluated nil)
              (should (romacs-agent-layout-tests--warnings (apply #'load call)))
              (should romacs-agent-layout-tests--evaluated)))
        (delete-directory elsewhere t)))))

(ert-deftest romacs-agent-layout-shorter-records-warn ()
  ;; Records an older definition made, shorter than this one's, though the
  ;; layout recorded is the same.
  (eval '(romacs-agent--defstruct (romacs-agent-layout-short (:constructor romacs-agent-layout-short--make))
           "A probe." id state)
        t)
  (let ((romacs-agent--sessions (make-hash-table :test #'equal)))
    (puthash "a" (record 'romacs-agent-layout-short "a") romacs-agent--sessions)
    (should (romacs-agent-layout-tests--warnings
             (eval '(romacs-agent--defstruct (romacs-agent-layout-short (:constructor romacs-agent-layout-short--make))
                      "A probe." id state)
                   t)))))

(provide 'romacs-agent-layout-tests)
;;; romacs-agent-layout-tests.el ends here
