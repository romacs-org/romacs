;; Primitive value transport, including finite IEEE boundary cases.
(case tree identity '[1 "λ" (a . b) 2.5])
(case negative-zero identity -0.0)
(case subnormal identity 4.9406564584124654e-324)
(case infinity identity 1.0e+INF)
;; Trailing comments are not an extra or missing case.
