;;; agent-image-replay.el --- scaling probe  -*- lexical-binding: t -*-

;; Replaying N images of a transcript lays them out from their headers.
(scaling-probe agent-image-replay :n 400
  :setup (progn (scaling--agent)
                (require 'romacs-agent-ui)
                (let ((file (make-temp-file "scaling-image" nil ".png")))
                  (with-temp-file file
                    (set-buffer-multibyte nil)
                    (insert (unibyte-string 137 ?P ?N ?G 13 10 26 10 0 0 0 13 ?I ?H ?D ?R 0 0 3 192 0 0 2 28)
                            (make-string 9 ?x)
                            (unibyte-string 0 0 0 0 ?I ?E ?N ?D 174 66 96 130)))
                  (cons file n)))
  :body (cl-letf (((symbol-function 'image-size) (lambda (&rest _) (error "Replay decodes nothing"))))
          (dotimes (_ (cdr it))
            (or (romacs-agent-ui--image-strips (car it)) (error "No strips")))))
