;;; gnosis-fsrs-fit.el --- scaling probe  -*- lexical-binding: t -*-

;; FSRS weights fitted to the review history of N cards, twelve reviews
;; each, interleaved in time order as Gnosis reads them: each card's
;; history is kept apart, and every delayed review becomes a training item
;; of its card's reviews so far.  That copy grows with the reviews of one
;; card (as Anki's items do), not with the number of cards.
(scaling-probe gnosis-fsrs-fit :n 500
  :setup (let (reviews)
           (dotimes (step 12)
             (dotimes (card n)
               (push (list card (if (zerop (% (+ card step) 7)) 1 3)
                           (if (zerop step) 0 (1+ (% (+ card step) 9))))
                     reviews)))
           (nreverse reviews))
  :body (romacs-fsrs-compute-parameters it))
