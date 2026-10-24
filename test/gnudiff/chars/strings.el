;;; Unibyte <-> multibyte conversions on GNU internal bytes.  -*- lexical-binding: t; -*-
(case to-multibyte-all-bytes string-to-multibyte
      (apply #'unibyte-string (number-sequence 0 255)))
(case to-multibyte-utf8-bytes string-to-multibyte (encode-coding-string "aλ雪😀" 'utf-8))
(case to-multibyte-empty string-to-multibyte "")
(case to-multibyte-multibyte string-to-multibyte "aλ")
(case as-unibyte-all-bytes string-as-unibyte
      (string-to-multibyte (apply #'unibyte-string (number-sequence 0 255))))
(case as-unibyte-mixed string-as-unibyte (string ?a ?λ #x3fff80 ?雪 #x3fffff #x10ffff #x110000 #x3fff7f))
(case as-unibyte-unibyte string-as-unibyte (unibyte-string 200 65))
(case as-unibyte-stride string-as-unibyte
      (apply #'string (number-sequence 0 #x3fffff 97)))
(case as-multibyte-mixed string-as-multibyte
      (concat (encode-coding-string "aλ雪😀" 'utf-8) (unibyte-string #xc0 #x80 #xc2 #xf8 #x88 #x80 #x80 #x80 #xff)))
(case as-multibyte-all-bytes string-as-multibyte
      (apply #'unibyte-string (number-sequence 0 255)))
(case to-unibyte-raw string-to-unibyte (string ?a #x3fff80 #x3fffff))
(case to-unibyte-error string-to-unibyte (string ?a ?λ))
(case bytes-unibyte string-bytes (apply #'unibyte-string (number-sequence 0 255)))
(case bytes-raw string-bytes (string #x3fff80 #x3fffff))
(case bytes-classes string-bytes (string ?a #x7ff #xffff #x1fffff #x3fff7f))
(case bytes-stride string-bytes (apply #'string (number-sequence 0 #x3fffff 97)))
(case bytes-wrong-type string-bytes 7)
