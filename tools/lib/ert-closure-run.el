;;; ert-closure-run.el --- Strict scoped ERT result -*- lexical-binding: t; -*-
(let* ((stats (ert-run-tests-batch t))
       (executed (- (ert-stats-completed stats) (ert-stats-skipped stats)))
       (passed (+ (ert--stats-passed-expected stats)
                  (ert--stats-passed-unexpected stats))))
  (unless (and (> executed 0)
               (= (ert-stats-completed-unexpected stats) 0)
               (= (ert-stats-skipped stats) 0)
               (= (ert-stats-completed stats) (ert-stats-total stats))
               (not (ert--stats-aborted-p stats)))
    (error "ERT run incomplete or unsuccessful: %s executed, %s passed"
           executed passed))
  (list :executed executed :passed passed))
