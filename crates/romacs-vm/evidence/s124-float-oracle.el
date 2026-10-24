;; -*- lexical-binding: t -*-
(prin1 (list (equal 0.0 -0.0) (equal (list 0.0) (list -0.0)) (= 0.0 -0.0))) (terpri)
(prin1 (list (equal 0.0e+NaN 0.0e+NaN) (equal 0.0e+NaN -0.0e+NaN) (equal [0.0e+NaN] [0.0e+NaN]) (equal [0.0e+NaN] [-0.0e+NaN]))) (terpri)
