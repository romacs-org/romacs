;;; agent-tray-keys.el --- scaling probe  -*- lexical-binding: t -*-

;; N keys in a chat whose draft carries N images: the tray is drawn once.
(scaling-probe agent-tray-keys :n 2000
  :setup (progn (scaling--agent)
                (require 'romacs-agent-images)
                (scaling--names "scaling-image" n))
  :body (let ((romacs-agent-images--tray-shown nil))
          (cl-letf (((symbol-function 'romacs-agent-images--tray-files) (lambda (_) it))
                    ((symbol-function 'romacs-agent-images--tray-anchor) (lambda (&rest _) '(10 . 20)))
                    ((symbol-function 'romacs-agent-images--tray-draw) #'ignore)
                    ((symbol-function 'romacs-popup-show) #'ignore))
            (dotimes (_ (length it)) (romacs-agent-images-tray-sync)))))
