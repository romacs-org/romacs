;;; gc-collection-log.el --- scaling probe  -*- lexical-binding: t -*-

;; The collections a usage log reads (gc.rs `Vm::note_collection'): N
;; collections, past what the log keeps at 4N, then its read.  A record
;; costs the same however many were kept, and past the limit only a count.
(scaling-probe gc-collection-log :n 100
  :setup (romacs--take-collections)
  :body (progn (dotimes (_ n) (garbage-collect))
               (romacs--take-collections)))
