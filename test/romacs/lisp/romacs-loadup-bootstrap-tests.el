;;; romacs-loadup-bootstrap-tests.el --- The chain's own startup steps -*- lexical-binding: t; -*-
(require 'ert)

(defmacro romacs-loadup-bootstrap-tests--in-home (&rest body)
  "Run BODY with `old' the home directory's project list and `new' the
per-user one, in a scratch directory removed afterwards."
  (declare (indent 0))
  `(let* ((home (make-temp-file "romacs-project-list-" t))
          (old (expand-file-name "projects.eld" home))
          (new (expand-file-name "config/emacs/projects.eld" home)))
     (unwind-protect (progn ,@body)
       (delete-directory home t))))

(defun romacs-loadup-bootstrap-tests--write (file text)
  (make-directory (file-name-directory file) t)
  (with-temp-file file (insert text)))

(defun romacs-loadup-bootstrap-tests--read (file)
  (with-temp-buffer (insert-file-contents-literally file) (buffer-string)))

(ert-deftest romacs-migrate-project-list-copies-the-old-list-and-keeps-it ()
  (romacs-loadup-bootstrap-tests--in-home
    (romacs-loadup-bootstrap-tests--write old ";;; -*- lisp-data -*-\n((\"~/src/a/\"))\n")
    (romacs--migrate-project-list old new)
    (should (file-regular-p new))
    (should (file-regular-p old))
    (should (equal (romacs-loadup-bootstrap-tests--read new)
                   (romacs-loadup-bootstrap-tests--read old)))))

(ert-deftest romacs-migrate-project-list-leaves-an-existing-list-alone ()
  (romacs-loadup-bootstrap-tests--in-home
    (romacs-loadup-bootstrap-tests--write old "((\"~/old/\"))\n")
    (romacs-loadup-bootstrap-tests--write new "((\"~/new/\"))\n")
    (romacs--migrate-project-list old new)
    (should (equal (romacs-loadup-bootstrap-tests--read new) "((\"~/new/\"))\n"))
    (should (equal (romacs-loadup-bootstrap-tests--read old) "((\"~/old/\"))\n"))))

(ert-deftest romacs-migrate-project-list-makes-nothing-without-an-old-list ()
  (romacs-loadup-bootstrap-tests--in-home
    (romacs--migrate-project-list old new)
    (should-not (file-exists-p new))
    (should-not (file-exists-p (file-name-directory new)))
    (should-not (file-exists-p old))))

(ert-deftest romacs-migrate-project-list-skips-a-directory-or-unreadable-old-list ()
  (romacs-loadup-bootstrap-tests--in-home
    (make-directory old)
    (romacs--migrate-project-list old new)
    (should-not (file-exists-p new))
    (delete-directory old)
    (romacs-loadup-bootstrap-tests--write old "((\"~/src/a/\"))\n")
    (set-file-modes old #o000)
    (unwind-protect
        (progn
          (romacs--migrate-project-list old new)
          (should-not (file-exists-p new)))
      (set-file-modes old #o600))))

(provide 'romacs-loadup-bootstrap-tests)
;;; romacs-loadup-bootstrap-tests.el ends here
