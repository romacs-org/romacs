;;; where-is-prefix-maps.el --- scaling probe  -*- lexical-binding: t -*-

;; A command's keys in a keymap of N prefix keymaps, each binding the
;; command and one keymap they all share, and each reached again as a meta
;; character under ESC; N keys of the command under one prefix of seven
;; events; and N menu items of it, each with a filter.  The reverse lookup
;; walks every binding once, tells a keymap
;; already reached under a prefix without a scan of those reached, places a
;; meta character's keymap after its ESC prefix without moving those
;; queued, and drops a repeated sequence without a scan of those found,
;; however long the events they share, and going on after a filter costs
;; nothing that grows with what was found.  After a keymap change, one walk
;; fills the cache that then answers each first-key lookup.  The keymaps
;; with N bindings are dense ones, so looking a found sequence up again
;; does not scan N bindings.
(scaling-probe where-is-prefix-maps :n 100
  :setup (let ((map (make-keymap)) (esc (make-keymap)) (deep (make-keymap)) (items (make-keymap)) (shared (make-sparse-keymap)))
           (define-key shared "s" #'scaling-where-shared)
           (define-key map [27] esc)
           (define-key map [f1 f2 f3 f4 f5 f6 f7] deep)
           (define-key map [f8] items)
           (dotimes (i n)
             (let ((prefix (make-sparse-keymap)))
               (define-key prefix "a" #'scaling-where-target)
               (define-key prefix "b" #'ignore)
               (define-key prefix "s" shared)
               (define-key map (vector (+ #x1000 i)) prefix)
               (define-key esc (vector (+ #x1000 i)) prefix)
               (define-key deep (vector (+ #x1000 i)) #'scaling-where-deep)
               (define-key items (vector (+ #x1000 i)) (list 'menu-item "Item" #'scaling-where-item :filter #'identity))))
           map)
  :body (dotimes (_ 3)
          (where-is-internal 'scaling-where-target (list it))
          (where-is-internal 'scaling-where-shared (list it))
          (where-is-internal 'scaling-where-deep (list it))
          (where-is-internal 'scaling-where-item (list it))
          (define-key it [f12] #'ignore)
          (dotimes (_ 20)
            (where-is-internal 'scaling-where-target (list it) t)
            (where-is-internal 'scaling-where-shared (list it) t))))
