;;; romacs-agent-layout-tests.el --- A session record's layout changed under live sessions -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))

(ert-deftest romacs-agent-layout-a-change-under-live-records-is-refused ()
  ;; An older layout, with a record alive in the sessions table.
  (eval '(romacs-agent--defstruct (romacs-agent-layout-probe (:constructor romacs-agent-layout-probe--make))
           "A probe." id state)
        t)
  (let* ((romacs-agent--sessions (make-hash-table :test #'equal))
         (record (funcall 'romacs-agent-layout-probe--make :id "a" :state 'idle)))
    (puthash "a" record romacs-agent--sessions)
    ;; The new layout loaded live: refused, naming what changed.
    (let ((err (should-error (eval '(romacs-agent--defstruct (romacs-agent-layout-probe (:constructor romacs-agent-layout-probe--make))
                                      "A probe." id state stopped)
                                   t))))
      (should (string-match-p "fields of .romacs-agent-layout-probe. (added: stopped; removed: ) while 1 live records" (cadr err)))
      (should (string-match-p "restart Romacs" (cadr err))))
    ;; The live record still reads as it did, and its class is unchanged
    ;; (the printer and the debugger read it).
    (should (eq (funcall 'romacs-agent-layout-probe-state record) 'idle))
    (should (equal (mapcar #'car (cdr (cl-struct-slot-info 'romacs-agent-layout-probe))) '(id state)))
    ;; Fields in another order: as breaking as a new one.
    (should-error (eval '(romacs-agent--defstruct (romacs-agent-layout-probe (:constructor romacs-agent-layout-probe--make))
                           "A probe." state id)
                        t))
    ;; The same layout loads again.
    (eval '(romacs-agent--defstruct (romacs-agent-layout-probe (:constructor romacs-agent-layout-probe--make))
             "A probe." id state)
          t)
    ;; With no live record of the old layout, the new one loads.
    (clrhash romacs-agent--sessions)
    (eval '(romacs-agent--defstruct (romacs-agent-layout-probe (:constructor romacs-agent-layout-probe--make))
             "A probe." id state stopped)
          t)
    (should (eq (funcall 'romacs-agent-layout-probe-stopped (funcall 'romacs-agent-layout-probe--make :stopped t)) t))))

(ert-deftest romacs-agent-layout-a-record-a-view-keeps-counts ()
  ;; A history view keeps its own session record, in no table.
  (eval '(romacs-agent--defstruct (romacs-agent-layout-view (:constructor romacs-agent-layout-view--make))
           "A probe." id)
        t)
  (let ((romacs-agent--sessions (make-hash-table :test #'equal)))
    (with-temp-buffer
      (setq-local romacs-agent-layout-tests--session (funcall 'romacs-agent-layout-view--make :id "v"))
      (should-error (eval '(romacs-agent--defstruct (romacs-agent-layout-view (:constructor romacs-agent-layout-view--make))
                             "A probe." id stopped)
                          t)))))

(defvar romacs-agent-layout-tests--session nil)

(ert-deftest romacs-agent-layout-reloading-the-same-file-is-allowed ()
  ;; romacs-agent.el loaded again over live sessions of its own layout.
  (let ((romacs-agent--sessions (make-hash-table :test #'equal))
        (romacs-mcp-call-function romacs-mcp-call-function)
        (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
    (puthash "s" (romacs-agent--make :id "s" :harness 'claude) romacs-agent--sessions)
    (load "romacs-agent" nil t)
    (should (equal (romacs-agent-id (gethash "s" romacs-agent--sessions)) "s"))))

(provide 'romacs-agent-layout-tests)
;;; romacs-agent-layout-tests.el ends here
