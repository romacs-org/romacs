;;; admission.el --- Safe E1c admission observations -*- lexical-binding: t; -*-
(defun e1c-record (name value)
  (princ (format "(%s . " name)) (prin1 value) (princ ")\n"))
(let ((fun (make-byte-code 0 (unibyte-string 192 135 255) [42] 1)))
  (e1c-record 'admission-trailing-junk
              (cons (= (funcall fun) 42) (string-to-list (aref fun 1)))))
(let* ((code (unibyte-string 192 135))
       (fun (make-byte-code 0 code [42 99] 1)))
  (aset code 0 193)
  (e1c-record 'admission-source-mutation-before
              (cons (= (funcall fun) 99) (string-to-list (aref fun 1)))))
(e1c-record 'nested-condition-then-foreign-throw
  (catch 'outer
    (condition-case e
        (unwind-protect (signal 'error nil)
          (condition-case inner (signal 'error nil) (error inner))
          (throw 'outer 11))
      (error e))))
(e1c-record 'cleanup-local-catch-preserves-signal
  (condition-case e
      (catch 'outer
        (unwind-protect (signal 'error nil)
          (catch 'inner (throw 'inner 12))))
    (error e)))
(e1c-record 'cleanup-nil-replaces-throw
  (condition-case e
      (catch 'outer
        (unwind-protect (throw 'outer 99) (throw nil 1)))
    (no-catch e)))
