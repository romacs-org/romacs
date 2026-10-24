;;; Checked normalization, not trusted decoding of malformed storage. -*- lexical-binding: t; -*-
(case malformed-boundaries chars-normalization-report
      (progn
        (defalias 'chars-normalization-report
          (lambda (inputs)
            (vconcat
             (mapcar
              (lambda (input)
                (let ((text (string-as-multibyte input)))
                  (vector (length text) (string-bytes text)
                          (vconcat (string-to-list text))
                          (string-as-unibyte text))))
              inputs))))
        (let ((inputs nil)
              (forms '((#xe0 #x80 #x80) (#xed #xa0 #x80)
                       (#xf0 #x80 #x80 #x80) (#xf8 #x87 #xbf #xbf #xbf)
                       (#xf8 #x88 #x80 #x80 #x80) (#xf8 #x8f #xbf #xbd #xbf)
                       (#xf8 #x8f #xbf #xbe #x80) (#xf9 #x88 #x80 #x80 #x80)
                       (#xc0 #x41) (#xc0 #xff) (#xc1 #x80) (#x80) (#xff)
                       (#xc2 #x80 #xc0 #x80 #xff))))
          (dotimes (head 256)
            (dolist (tail '(0 127 128 159 160 191 192 255))
              (push (unibyte-string head tail) inputs)))
          (dolist (bytes forms)
            (let ((form (apply #'unibyte-string bytes)))
              (dotimes (length (1+ (length form)))
                (push (substring form 0 length) inputs))
              (dotimes (index (1- (length form)))
                (dolist (illegal '(0 127 192 255))
                  (let ((copy (copy-sequence form)))
                    (aset copy (1+ index) illegal)
                    (push copy inputs))))))
          (vconcat (nreverse inputs)))))
