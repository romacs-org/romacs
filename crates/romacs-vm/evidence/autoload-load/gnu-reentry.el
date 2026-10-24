;;; -*- lexical-binding: t -*-
;; Count load-time require re-entry in GNU: a require of a feature whose
;; own require is still in progress. PROVIDED = it was already provided
;; (early provide; returns at once), NOT = would re-load the file.
(defvar cy-stack nil)
(defvar cy-hits nil)
(defvar cy-n 0)
(defvar cy-orig (symbol-function 'require))
(fset 'require
  (lambda (feature &rest r)
    (let ((f cy-orig))
    (setq cy-n (1+ cy-n))
    (when (memq feature cy-stack)
      (push (list feature (if (featurep feature) 'provided 'NOT) (car cy-stack)) cy-hits))
    (if (featurep feature)
        (apply f feature r)
      (let ((cy-stack (cons feature cy-stack)))
        (apply f feature r))))))
(dolist (feat (mapcar #'intern command-line-args-left))
  (condition-case e (require feat) (error (push (list feat 'ERROR e) cy-hits))))
(setq command-line-args-left nil)
(let ((tally (make-hash-table :test 'equal)))
  (dolist (h cy-hits) (puthash (cadr h) (1+ (gethash (cadr h) tally 0)) tally))
  (princ (format "requires=%d reentries=%d by-kind=%S\n" cy-n (length cy-hits)
                 (let (l) (maphash (lambda (k v) (push (cons k v) l)) tally) l)))
  (dolist (h (reverse cy-hits)) (princ (format "  %S\n" h))))
