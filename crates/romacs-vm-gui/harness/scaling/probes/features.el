;;; features.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe features :n 300
  :known "ticket features: featurep and provide are GNU's memq over the features list"
  :setup (mapcar #'intern (scaling--names "scaling-feature" n))
  :body (dolist (feature it) (provide feature) (featurep feature) (require feature)))
