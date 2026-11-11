;;; vm-bench-tests.el --- V0 checked rows -*- lexical-binding: t; -*-
(require 'ert)
;; A runner may preload the byte- or native-compiled harness first.
(unless (featurep 'core-bench)
  (load (expand-file-name "core-bench.el" (file-name-directory load-file-name)) nil t))

(ert-deftest vm-bench-seven-oracles-reject-wrong-engine-results ()
  (dolist (name '("bc-funcall-dynamic" "bc-seq-filter" "bc-make-closure-1e6"
                  "bc-condition-case-loop" "bc-catch-throw-1e5"
                  "bc-varref-global" "bc-call-optional-rest"))
    (should-error (core-bench--check name :wrong-vm-result)
                  :type 'error)))

(ert-deftest vm-bench-seven-oracles-accept-real-work ()
  (dolist (name '("bc-funcall-dynamic" "bc-seq-filter" "bc-make-closure-1e6"
                  "bc-condition-case-loop" "bc-catch-throw-1e5"
                  "bc-varref-global" "bc-call-optional-rest"))
    (let ((answer (funcall (intern (concat "core-bench-row-" name)))))
      (should-not (core-bench--check name answer)))))

;; V0b amendment rows: lexical capture of a MUTATED variable (amendment 2/3)
;; and errors-as-data with a backtrace captured at signal time (amendment 4/5).
(ert-deftest vm-bench-amendment-rows-accept-real-work ()
  (should (= (core-bench-row-bc-closure-counter) 10000007))
  (should-not (core-bench--check "bc-closure-counter" (core-bench-row-bc-closure-counter)))
  (should (= (core-bench-row-bc-error-backtrace-1e4) 90000))
  (should-not (core-bench--check "bc-error-backtrace-1e4" (core-bench-row-bc-error-backtrace-1e4))))

(ert-deftest vm-bench-amendment-rows-reject-wrong-engine-results ()
  ;; 10000000: lost the captured initial value; 1: the closure copied N
  ;; instead of sharing its box; 80000/90009/0: a frame lost, one extra
  ;; frame per error, no backtrace at signal time.
  (dolist (wrong '(:wrong-vm-result 10000000 1 10000006 nil))
    (should-error (core-bench--check "bc-closure-counter" wrong) :type 'error))
  (dolist (wrong '(:wrong-vm-result 80000 90009 0 nil))
    (should-error (core-bench--check "bc-error-backtrace-1e4" wrong) :type 'error)))

(ert-deftest vm-bench-counter-closure-shares-its-mutated-box ()
  (let ((a (core-bench--make-counter 0)) (b (core-bench--make-counter 100)))
    (funcall a) (funcall a) (funcall b)
    (should (= (funcall a) 3))
    (should (= (funcall b) 102))))

(ert-deftest vm-bench-backtrace-captured-at-signal-time ()
  (let ((frames nil))
    (condition-case nil
        (handler-bind ((core-bench-signal
                        (lambda (_) (setq frames (core-bench--bt-count (backtrace-frames))))))
          (core-bench--bt-recur 8))
      (core-bench-signal nil))
    (should (= frames 9))))
