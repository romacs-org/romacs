;; Scrolling latency over VISUALLY WRAPPED lines, in a real TTY session.
;;
;; The companion to tty-typing.el, and it exists because that fixture cannot
;; see the path this measures. tty-typing stays on ONE screen and edits in
;; place, so it exercises the cursor row and the line-start row acquisition;
;; it barely touches the mid-line resume path that the row route takes on the
;; continuation rows of a wrapped line. A change that makes every mid-line
;; walk position do more work is nearly invisible there and dominant here.
;;
;; Shape, and why:
;;   * The buffer is generated, not a repo file, so the wrap ratio is a
;;     property of the fixture rather than of whatever seq.el happens to look
;;     like. Each logical line is ~6 screen rows wide at 80 columns, so almost
;;     every row laid out is a CONTINUATION row entered mid-line.
;;   * Character wrap, truncate-lines nil, word-wrap off: the plain
;;     continuation class, which is what the route covers.
;;   * scroll-conservatively 0 and a scroll step of a whole window keeps each
;;     redisplay a full re-layout of a fresh screenful rather than a shift of
;;     mostly-reusable rows, so the measurement is layout work and not the
;;     terminal update planner.
;;   * It scrolls DOWN then back UP the same distance, so the run returns to
;;     where it started and every iteration lays out the same total content.
;;
;; Like tty-typing it records the iteration count it ACTUALLY completed and
;; the runner exits non-zero if the sentinel never appears: a run that stops
;; early still burns a believable number of instructions.
(run-at-time 0 nil (lambda ()
  (let* ((root (expand-file-name default-directory))
         (log  (expand-file-name "tmp/reval/wrapped-scroll.out" root))
         (done (expand-file-name "tmp/reval/wrapped-scroll.done" root))
         (buf (get-buffer-create "*wrapped-scroll-bench*"))
         (lines (string-to-number (or (getenv "WRAP_LINES") "400")))
         (n (string-to-number (or (getenv "SCROLL_ITERS") "300"))))
    (setq scroll-conservatively 0
          scroll-step 0
          auto-window-vscroll nil)
    (set-window-buffer (selected-window) buf)
    (with-current-buffer buf
      (setq truncate-lines nil
            word-wrap nil
            buffer-read-only nil)
      (erase-buffer)
      ;; ~480 columns per logical line: about 6 screen rows at 80 columns.
      (dotimes (i lines)
        (insert (format "%04d " i))
        (dotimes (j 24)
          (insert (format "wrapped-token-%02d-%03d " j i)))
        (insert "\n"))
      (goto-char (point-min))
      (redisplay t)
      (let ((t0 (car (current-cpu-time))))
        (dotimes (_ n)
          (scroll-up)
          (redisplay t)
          (scroll-down)
          (redisplay t))
        (write-region (format "scroll_us=%d iters=%d per_scroll_us=%.1f\n"
                              (- (car (current-cpu-time)) t0) n
                              (/ (float (- (car (current-cpu-time)) t0)) (max 1 n)))
                      nil log t 'silent)))
    (write-region "done\n" nil done nil 'silent)
    (kill-emacs 0))))
