;;; owner-window.el --- An owner's logical windows  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; An owner (lisp/vm/owner.el) lays out the master's windows that show
;; its buffers (ownership design O1b §2.1).  Each master window is one
;; real window here: the root window of a child frame of the owner's own
;; frame, F1, with that master window's id as the frame's `owner-window'
;; parameter.  The window is its master's: strongly dedicated, and
;; splitting or deleting it, or quitting it, signals `owner-window-fixed'.
;; So does every `display-buffer' in the owner that needs a window,
;; through `display-buffer-overriding-action': windows are the master's
;; to make.  F1's own window shows " *owner-root*" alone and is never
;; laid out.
;;
;; The master's requests (`owner-window-request') wait in the window's
;; record, the newest winning, and are applied when pictures are next
;; taken (`owner-window--refresh'); a delete (`owner-window-delete') acts
;; at once.  Taking pictures applies the requests, runs what a redisplay
;; runs before it lays windows out, then, window by window, rebuilds one
;; owner Lisp broke and lays it out as a frame of its own
;; (crates/romacs-vm/src/owner/display.rs); a window that fails is
;; logged, and has no picture this time.  `owner--picture' returns one
;; window's picture.  In an owner the window change functions run only
;; buffer-locally (lisp/vm/window-change.el): its master runs the default
;; ones.  A worker with no window side takes all of this as nothing.
;;
;; Limits: owner Lisp that subverts a pane's window from display Lisp
;; (a mode line's `:eval', say) is caught only by the check after the
;; window's snapshot: that picture is refused and the next refresh
;; repairs the window.  Face specs for a graphic display can resolve
;; differently here, since an owner has no window system open.
;; Not in GNU.

;;; Code:

;; Window management of an owner's window, which is its master's.  Data:
;; what the refused call was given.
(define-error 'owner-window-fixed "An owner's windows are its master's to manage")

(defvar romacs--window-change-local-only)

(defvar owner-window--records (make-hash-table :test 'eql)
  "The owner's logical windows, by master window id: each a record
\[owner-window MASTER FRAME BUFFER REQUEST PENDING LAID FAILED].  FRAME
is the child frame whose root window shows BUFFER, nil until a request
is applied; REQUEST the request applied, a plist as
`owner-window-request' takes it; PENDING the newest not yet applied, or
nil; LAID whether the current refresh laid the window out; FAILED
whether the last request failed to apply, which leaves the window
without pictures until one applies.")

(defvar owner-window--root nil
  "The owner's own frame, F1, once `owner-window--start' started a
display; nil in an owner with no window side.")

(defvar owner-window--publishing nil
  "Non-nil while `owner-window--refresh' runs: a nested call does nothing.")

(defvar owner-window--logged nil
  "The messages `owner-window--log' already wrote.")

(defconst owner-window--root-buffer " *owner-root*"
  "The buffer F1's window shows: no buffer of the owner's is laid out there.")

(defun owner-window--master (record) "RECORD's master window id." (aref record 1))
(defun owner-window--frame (record) "RECORD's child frame, or nil." (aref record 2))
(defun owner-window--buffer (record) "The buffer RECORD shows, or nil." (aref record 3))
(defun owner-window--request (record) "RECORD's applied request, or nil." (aref record 4))
(defun owner-window--pending (record) "RECORD's request not yet applied, or nil." (aref record 5))
(defun owner-window--laid (record) "Whether this refresh laid RECORD out." (aref record 6))
(defun owner-window--failed (record) "Whether RECORD's last request failed to apply." (aref record 7))

(defun owner-window--window (record)
  "RECORD's window, the root window of its frame, or nil without a live frame."
  (let ((frame (owner-window--frame record)))
    (and (frame-live-p frame) (frame-root-window frame))))

(defun owner-window--log (text)
  "Write TEXT as a message the first time it comes, and never again."
  (unless (member text owner-window--logged)
    (push text owner-window--logged)
    (message "%s" text)))

(defun owner-window--fixed (&rest args)
  "Refuse window management on an owner's window, which is its master's:
signal `owner-window-fixed' with ARGS."
  (signal 'owner-window-fixed args))

(defun owner-window--display-buffer (buffer alist)
  "The owner's `display-buffer' action for BUFFER: `fail' when ALIST
allows no window (`allow-no-window'), so `display-buffer' answers nil;
else signal `owner-window-fixed'."
  (if (cdr (assq 'allow-no-window alist))
      'fail
    (owner-window--fixed buffer alist)))

(defun owner-window--guard (window)
  "Fix WINDOW as its master's: strongly dedicated to its buffer, with
splitting and deleting it refused."
  (set-window-dedicated-p window t)
  (dolist (parameter '(split-window delete-window delete-other-windows))
    (set-window-parameter window parameter #'owner-window--fixed)))

(defun owner-window--start (slot)
  "Make this owner lay out its master's windows as owner slot SLOT:
from here its fonts are the slot's, its window change functions run
only buffer-locally, F1's window shows only `owner-window--root-buffer',
and a `display-buffer' that needs a window signals `owner-window-fixed'.
t; nil, SLOT unread and nothing changed, in a worker whose host attached
no display: it has no window side, and takes no requests."
  (when (romacs--owner-display-start slot)
    (setq owner-window--root (selected-frame)
          romacs--window-change-local-only t
          display-buffer-overriding-action '(owner-window--display-buffer))
    (set-frame-parameter owner-window--root 'owner-window 'root)
    (owner-window--make-root)
    t))

(defun owner-window--root-intact-p ()
  "Whether F1's window is alone in F1 and shows `owner-window--root-buffer'."
  (let ((window (frame-root-window owner-window--root)))
    (and (window-live-p window)
         (equal (buffer-name (window-buffer window)) owner-window--root-buffer))))

(defun owner-window--make-root ()
  "Leave F1 one window, showing `owner-window--root-buffer', fixed."
  (let ((ignore-window-parameters t)
        (window (frame-first-window owner-window--root)))
    (delete-other-windows window)
    (set-window-dedicated-p window nil)
    (set-window-buffer window (get-buffer-create owner-window--root-buffer))
    (owner-window--guard window)))

;;;; Requests

(defun owner-window-request (&rest request)
  "Take the master's REQUEST for its window, a plist: :window, the
master window's id; :view, the master's view generation; :buffer, the
buffer to show, by name; :revision, the request's own; :box, the
window's (WIDTH . HEIGHT) in px; :fringes, (LEFT . RIGHT) px; :cell,
the canonical character (LINE . COLUMN) px; :catalog, the master's font
catalog; :scale, its device scale; :selected, whether it is the
selected window; :bar, its status bar columns, or nil (:scale and :bar
are kept with the request, for owner images and the status bar).  It
waits in the window's record, replacing one not yet applied, until
pictures are next taken (`owner-window--refresh').  t; nil, REQUEST
unread, before `owner-window--start' started a display."
  (when owner-window--root
    (let ((master (plist-get request :window)))
      (unless (natnump master) (signal 'wrong-type-argument (list 'natnump master)))
      (aset (or (gethash master owner-window--records)
                (puthash master (record 'owner-window master nil nil nil nil nil nil)
                         owner-window--records))
            5 request)
      t)))

(defun owner-window-delete (&rest args)
  "Delete the owner's window for the master window ARGS' :window names, at
once, with its frame and any request not yet applied (`owner-window--forget').
t; nil, ARGS unread, before `owner-window--start' started a display."
  (when owner-window--root
    (let ((record (gethash (plist-get args :window) owner-window--records)))
      (when record
        (aset record 5 nil)
        (owner-window--forget record))
      t)))

(defun owner-window--discard (record)
  "Delete RECORD's frame past its guards, if it is live.  RECORD keeps
the frame while it lives, as when a deletion hook signals first."
  (let ((frame (owner-window--frame record)))
    (unwind-protect
        (when (frame-live-p frame)
          (let ((ignore-window-parameters t)) (delete-frame frame t)))
      (unless (frame-live-p frame) (aset record 2 nil)))))

(defun owner-window--forget (record)
  "Delete RECORD's frame and forget RECORD: kept while its frame lives,
and when a hook gave it a newer request meanwhile."
  (unwind-protect (owner-window--discard record)
    (unless (or (owner-window--frame record) (owner-window--pending record))
      (remhash (owner-window--master record) owner-window--records))))

(defun owner-window--drop (record why)
  "Forget RECORD with its frame, logging WHY once: the master's window has
no window here.  RECORD holds no buffer from here, even while a deletion
hook keeps its frame."
  (aset record 3 nil)
  (owner-window--forget record)
  (owner-window--log why))

;;;; Applying requests and repairing

(defun owner-window--apply (record)
  "Apply RECORD's pending request: show the buffer it names in RECORD's
window, made if there is none, at the request's cell, box and fringes,
then hold the buffer and the request as RECORD's; select the window if
the request does, and select F1's when it clears the selection of the
selected window.  Until the request applies RECORD has failed, and keeps
its applied request: a request this owner cannot apply leaves it so,
and one naming a buffer it does not hold drops RECORD."
  (let ((request (owner-window--pending record)))
    (when request
      (aset record 5 nil)
      (aset record 7 t)
      (let ((buffer (get-buffer (plist-get request :buffer))))
        (if (not (buffer-live-p buffer))
            (owner-window--drop record (format "Owner holds no buffer %S for window %s"
                                               (plist-get request :buffer) (owner-window--master record)))
          (owner-window--conform record request buffer)
          (aset record 3 buffer)
          (aset record 4 request)
          (aset record 7 nil)
          (owner-window--select record))))))

(defun owner-window--make (record request buffer)
  "A child frame of F1 for RECORD, showing BUFFER, at REQUEST's box and
fringes, its window fixed."
  (let* ((box (plist-get request :box))
         (fringes (plist-get request :fringes))
         ;; `make-frame' would make a terminal frame in a batch owner.
         (frame (with-current-buffer buffer
                  (romacs--create-frame
                   `((parent-frame . ,owner-window--root) (minibuffer . nil) (visibility . t)
                     (unsplittable . t) (no-other-frame . t)
                     (auto-hide-function . owner-window--fixed)
                     (owner-window . ,(owner-window--master record))
                     (left . 0) (top . 0)
                     ;; The text area: the box less the fringes.
                     (width . (text-pixels . ,(- (car box) (car fringes) (cdr fringes))))
                     (height . (text-pixels . ,(cdr box)))
                     (left-fringe . ,(car fringes)) (right-fringe . ,(cdr fringes)))))))
    (owner-window--guard (frame-root-window frame))
    frame))

(defun owner-window--sound-p (record buffer)
  "Whether RECORD's window is live, alone in its frame, on BUFFER."
  (let ((window (owner-window--window record)))
    (and (window-live-p window) (eq (window-buffer window) buffer))))

(defun owner-window--intact-p (record)
  "Whether RECORD's window is live, alone in its frame, on RECORD's buffer,
at its request's box."
  (let ((window (owner-window--window record))
        (box (plist-get (owner-window--request record) :box)))
    (and (owner-window--sound-p record (owner-window--buffer record))
         (eql (window-pixel-width window) (car box))
         (eql (window-pixel-height window) (cdr box)))))

(defun owner-window--conform (record request buffer)
  "Make RECORD's window what REQUEST says for BUFFER: made again when it
is not live, alone in its frame and on BUFFER, then at REQUEST's cell,
the frame's fringes, its box, and the window's own fringes, set once the
box gives them room.  An error when the window's fringes still differ
from REQUEST's: the request has not applied."
  (let ((box (plist-get request :box))
        (fringes (plist-get request :fringes))
        (cell (plist-get request :cell)))
    (unless (and (eql (car cell) (frame-char-height owner-window--root))
                 (eql (cdr cell) (frame-char-width owner-window--root)))
      (romacs--owner-set-cell (car cell) (cdr cell)))
    (unless (owner-window--sound-p record buffer)
      (owner-window--discard record)
      (aset record 2 (owner-window--make record request buffer)))
    (let* ((frame (owner-window--frame record))
           (window (frame-root-window frame)))
      (unless (and (eql (frame-parameter frame 'left-fringe) (car fringes))
                   (eql (frame-parameter frame 'right-fringe) (cdr fringes)))
        (modify-frame-parameters frame `((left-fringe . ,(car fringes)) (right-fringe . ,(cdr fringes)))))
      (unless (and (eql (window-pixel-width window) (car box))
                   (eql (window-pixel-height window) (cdr box)))
        (set-frame-size frame (- (car box) (car fringes) (cdr fringes)) (cdr box) t))
      (let ((wanted (list (car fringes) (cdr fringes))))
        (unless (equal (take 2 (window-fringes window)) wanted)
          (set-window-fringes window (car fringes) (cdr fringes)))
        (unless (equal (take 2 (window-fringes window)) wanted)
          (error "Owner window %s cannot take fringes %S" (owner-window--master record) fringes))))))

(defun owner-window--select (record)
  "Select RECORD's window when its request does; when it does not and
the window is selected, select F1's: never another logical window."
  (let ((window (owner-window--window record)))
    (cond ((plist-get (owner-window--request record) :selected)
           (unless (eq (selected-window) window) (select-window window)))
          ((eq (selected-window) window)
           (select-window (frame-root-window owner-window--root))))))

(defun owner-window--repair (record)
  "Repair RECORD's window from its record when owner Lisp left it dead,
split or on another buffer (made again), or at another box (resized),
logged once, and select it again when its request selects it.  RECORD
is dropped when its buffer is gone."
  (cond ((not (buffer-live-p (owner-window--buffer record)))
         (owner-window--drop record (format "Owner's buffer for window %s is gone"
                                            (owner-window--master record))))
        ((not (owner-window--intact-p record))
         (owner-window--log (format "Owner window %s repaired" (owner-window--master record)))
         (owner-window--conform record (owner-window--request record) (owner-window--buffer record))
         (when (plist-get (owner-window--request record) :selected)
           (select-window (owner-window--window record))))))

;;;; Pictures

(defun owner-window--all ()
  "The owner's window records, in no order."
  (let (records)
    (maphash (lambda (_ record) (push record records)) owner-window--records)
    records))

(defun owner-window--lay-out (record)
  "Repair RECORD's window (`owner-window--repair'), then lay it out for
its applied request, unless its last request failed.  RECORD is laid
once that succeeds and the window still shows RECORD's buffer: display
Lisp its snapshot ran may have changed it, and the next refresh repairs."
  (let ((request (owner-window--request record)))
    (when (and request (not (owner-window--failed record)))
      (owner-window--repair record)
      (let ((window (owner-window--window record)))
        (when window
          (romacs--owner-pane-lay-out window (owner-window--master record) (plist-get request :view)
                                      (plist-get request :revision) (plist-get request :catalog)
                                      (plist-get request :selected))
          (when (and (window-live-p window) (eq (window-buffer window) (owner-window--buffer record)))
            (aset record 6 t)))))))

(defun owner-window--each (step)
  "STEP on each window record, an error logged once and ending STEP for
that window alone."
  (dolist (record (owner-window--all))
    (condition-case err
        (funcall step record)
      (error (owner-window--log (format "Owner window %s: %S" (owner-window--master record) err))))))

(defun owner-window--refresh ()
  "Bring every window up to date and lay it out: apply each pending
request, heal F1, run what a redisplay runs before it lays windows out,
then repair and lay out each window.  A window that fails is not laid
this time, and the others go on.  t; nil inside another refresh, whose
hooks get no picture."
  (unless owner-window--publishing
    (let ((owner-window--publishing t))
      (maphash (lambda (_ record) (aset record 6 nil)) owner-window--records)
      (condition-case err
          (progn
            (owner-window--each #'owner-window--apply)
            (unless (owner-window--root-intact-p)
              (owner-window--log "Owner's own window healed")
              (owner-window--make-root))
            (romacs--owner-panes-prepare)
            (owner-window--each #'owner-window--lay-out))
        (error (owner-window--log (format "Owner windows not laid out: %S" err))))
      t)))

(defun owner--picture (master)
  "Refresh the owner's windows (`owner-window--refresh') and return master
window MASTER's picture, a unibyte string; nil when the refresh did not
lay it out, inside another refresh, and before `owner-window--start'
started a display."
  (when (and owner-window--root (owner-window--refresh))
    (let* ((record (gethash master owner-window--records))
           (window (and record (owner-window--laid record) (owner-window--window record))))
      (and window (romacs--owner-pane-picture window)))))

(provide 'owner-window)
;;; owner-window.el ends here
