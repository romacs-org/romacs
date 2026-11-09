;;; jabber-nick-colors.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe jabber-nick-colors :n 1000
  :setup (progn (scaling--jabber) (clrhash jabber-muc--nick-colors) (scaling--names "nick" n))
  :body (dotimes (_ 2) (dolist (nick it) (jabber-muc--nick-face nick))))
