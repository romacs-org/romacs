;;; buffer-order.el --- scaling probe  -*- lexical-binding: t -*-

;; The buffer list's order (buffer_order.rs) among N live buffers: twenty
;; rounds of each step that changes it.  A round is the birth and kill of an
;; inhibited buffer and of a named one, a record (`select-window' on a
;; window showing the newest buffer) and a bury.  Each step is O(1) in the
;; order, so N costs nothing.  Held by its slope: instructions the twenty
;; rounds grow by per live buffer.  Copying or walking the order once a
;; step, as the Lisp list it replaced was copied, costs at least about 100
;; instructions per buffer a step: 8,000 or more a buffer for these four
;; steps over twenty rounds.  Measured 2026-11-20: under 20 in one run and
;; 34.2 in the next on the same code, the noise of an 8.5M body (0.24%).  A
;; budget of 200 holds that noise six times over and fails any walk by
;; forty times.
(scaling-probe buffer-order :n 200 :k 4 :slope 200
  :setup (car (last (mapcar #'get-buffer-create (scaling--names "scaling-order" n))))
  :body (dotimes (_ 20)
          (kill-buffer (generate-new-buffer " scaling-order-temp" t))
          (kill-buffer (get-buffer-create "scaling-order-named"))
          (set-window-buffer nil it)
          (select-window (selected-window))
          (bury-buffer-internal it)))
