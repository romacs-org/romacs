;;; Oracle-only contract probe.  -*- lexical-binding: t; -*-
;;; Run with the pinned GNU executable, -Q --batch -l.
(defun current-time-contract-check (value name)
  (unless value (error "CURRENT_TIME_ORACLE %s" name)))
(current-time-contract-check
 (and (fboundp 'current-time) (functionp 'current-time)
      (subrp (symbol-function 'current-time))) "callable")
(let* ((a (current-time)) (b (funcall 'current-time)))
  (current-time-contract-check
   (and (integerp (car a)) (= (cdr a) 1000000000)
        (<= (car a) (car b))) "pair-and-order"))
(let* ((current-time-list 'nonboolean) (a (current-time)))
  (current-time-contract-check
   (and (= (length a) 4) (integerp (nth 0 a))
        (<= 0 (nth 1 a)) (< (nth 1 a) 65536)
        (<= 0 (nth 2 a)) (< (nth 2 a) 1000000)
        (<= 0 (nth 3 a)) (< (nth 3 a) 1000000)
        (= (% (nth 3 a) 1000) 0)) "list-ranges"))
(current-time-contract-check
 (eq (condition-case e (apply 'current-time '(nil)) (error (car e)))
     'wrong-number-of-arguments) "arity")
(dolist (ticks '(-2305843009213693953 -2305843009213693952 -1 0 1
                2305843009213693951 2305843009213693952
                -9223372036854775809 9223372036854775808))
  (let ((time (cons ticks 1000000000)))
    (current-time-contract-check (equal (time-convert time t) time) "exact-ticks")
    (prin1 (list 'ticks ticks (time-convert time t))) (terpri)))
(dolist (case '((-1 (-1 65535 999999 999000))
                (0 (0 0 0 0))
                (65538000003004 (1 2 3 4000))
                (2621440000000000000 (40000 0 0 0))
                (-9223372036854775808000000000 (-140737488355328 0 0 0))
                (9223372036854775807999999999 (140737488355327 65535 999999 999000))))
  (let ((actual (time-convert (cons (car case) 1000000000) 'list)))
    (current-time-contract-check (equal actual (nth 1 case)) "exact-list-components")
    (prin1 (list 'list (car case) actual)) (terpri)))
(princ "CURRENT_TIME_ORACLE_PASS\n")
