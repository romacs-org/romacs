;;; inotify.el --- GNU inotify.c: file notification on GNU/Linux  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; The `inotify' feature filenotify.el detects: `inotify-add-watch',
;; `inotify-rm-watch' and `inotify-valid-p', with GNU's watch list,
;; aspects, descriptors and event format, transcribed from src/inotify.c
;; over the descriptor primitives of prims/inotify.rs.  The kernel watches;
;; the instance's descriptor joins the keyboard waits
;; (`romacs--read-host-char', the GUI's idle loop, and batch `read-event'),
;; and each event runs, as read_char runs a special event, as (file-notify
;; EVENT CALLBACK) through `special-event-map', whose `file-notify' binding
;; is `file-notify-handle-event'.  The kernel queue stands for GNU's
;; keyboard buffer: events wait there until a keyboard wait reads them, so
;; `sleep-for' and `accept-process-output' leave them queued, as GNU's do
;; (DIVERGENCES.md VM2-INOTIFY).

;;; Code:

;; This file runs in a bare VM too (the process prelude), before subr.el:
;; only C-level forms and the prelude's macros.

(defconst romacs--inotify-masks
  '((access . #x1) (modify . #x2) (attrib . #x4) (close-write . #x8)
    (close-nowrite . #x10) (open . #x20) (moved-from . #x40)
    (moved-to . #x80) (create . #x100) (delete . #x200)
    (delete-self . #x400) (move-self . #x800) (unmount . #x2000)
    (q-overflow . #x4000) (ignored . #x8000) (isdir . #x40000000))
  "<sys/inotify.h>'s event bits, by GNU's aspect names.")

(defconst romacs--inotify-request-masks
  '((move . #xc0) (close . #x18) (onlydir . #x1000000)
    (dont-follow . #x2000000) (all-events . #xfff) (t . #xfff))
  "The bits a watch may also ask for: groups and watch flags.")

(defconst romacs--inotify-aspect-order
  '(access attrib close-write close-nowrite create delete delete-self modify
    move-self moved-from moved-to open ignored isdir q-overflow unmount)
  "mask_to_aspects's order, which conses each onto the last: an event's
aspects come in its reverse.")

(defvar romacs--inotify-watch-list nil
  "GNU inotify.c watch_list: ((WD . WATCHES) ...), WATCHES sorted by ID,
each (ID FILE-NAME CALLBACK MASK).  The kernel gives one WD per file, so
the ID tells apart the watches sharing it.")

(defvar romacs--special-events nil
  "GNU's keyboard buffer for special events: those `insert-special-event'
stored and file notification events read from the instance, not yet
handled, oldest first.  An event's error leaves the rest here.")

(defconst romacs--special-event-kinds
  '(delete-frame iconify-frame make-frame-visible save-session file-notify
    config-changed-event focus-in focus-out move-frame sleep-event)
  "The special events `insert-special-event' takes, as built with file
notification and without D-Bus or threads.")

(defun insert-special-event (event)
  "Insert the special EVENT into the input event queue.
Only 'input_event' slots KIND and ARG are set."
  (unless (consp event) (signal 'wrong-type-argument (list 'consp event)))
  (unless (memq (car event) romacs--special-event-kinds)
    (signal 'error (list "Invalid special event kind" (car event))))
  (setq romacs--special-events
        (nconc romacs--special-events (list (cons (car event) (cdr event)))))
  nil)

(defun romacs--inotify-error (string name)
  "report_file_notify_error for EINVAL: signal `file-notify-error' with
STRING, the error's text and NAME, spliced in when a list."
  (signal 'file-notify-error
          (cons string (cons "Invalid argument" (if (listp name) name (list name))))))

(defun romacs--inotify-symbol-mask (symbol)
  "symbol_to_inotifymask."
  (let ((bit (or (assq symbol romacs--inotify-request-masks)
                 (and (not (memq symbol '(isdir q-overflow)))
                      (assq symbol romacs--inotify-masks)))))
    (if bit (cdr bit) (romacs--inotify-error "Unknown aspect" symbol))))

(defun romacs--inotify-mask (aspect)
  "aspect_to_inotifymask: ASPECT, a symbol or a list of them, as bits."
  (if (listp aspect)
      (let ((mask 0) (tail aspect))
        (while (consp tail)
          (setq mask (logior mask (romacs--inotify-symbol-mask (car tail)))
                tail (cdr tail)))
        (when tail (signal 'wrong-type-argument (list 'listp aspect)))
        mask)
    (romacs--inotify-symbol-mask aspect)))

(defun romacs--inotify-aspects (mask)
  "mask_to_aspects: the aspect symbols of event MASK."
  (let (aspects)
    (dolist (aspect romacs--inotify-aspect-order aspects)
      (when (/= 0 (logand mask (cdr (assq aspect romacs--inotify-masks))))
        (setq aspects (cons aspect aspects))))))

(defun inotify-add-watch (file-name aspect callback)
  "Add a watch for FILE-NAME to inotify.
Return a watch descriptor.  The watch will look for ASPECT events and
invoke CALLBACK when an event occurs.

ASPECT might be one of the following symbols or a list of those symbols:

access
attrib
close-write
close-nowrite
create
delete
delete-self
modify
move-self
moved-from
moved-to
open

all-events or t
move
close

ASPECT can also contain the following symbols, which control whether
the watch descriptor will be created:

dont-follow
onlydir

Watching a directory is not recursive.  CALLBACK is passed a single argument
EVENT which contains an event structure of the format

\(WATCH-DESCRIPTOR ASPECTS NAME COOKIE)

WATCH-DESCRIPTOR is the same object that was returned by this function.  It can
be tested for equality using `equal'.  ASPECTS describes the event.  It is a
list of ASPECT symbols described above and can also contain one of the following
symbols

ignored
isdir
q-overflow
unmount

If a directory is watched then NAME is the name of file that caused the event.

COOKIE is an object that can be compared using `equal' to identify two matching
renames (moved-from and moved-to).

See inotify(7) and inotify_add_watch(2) for further information.  The
inotify fd is managed internally and there is no corresponding
inotify_init.  Use `inotify-rm-watch' to remove a watch.

The following inotify bit-masks cannot be used because descriptors are
shared across different callers.

IN_EXCL_UNLINK
IN_MASK_ADD
IN_ONESHOT"
  (let ((mask (romacs--inotify-mask aspect)))
    (unless (stringp file-name) (signal 'wrong-type-argument (list 'stringp file-name)))
    ;; IN_MASK_ADD and IN_EXCL_UNLINK: watches of one file share its WD.
    (romacs--inotify-add
     (romacs--inotify-add-watch file-name (logior mask #x20000000 #x4000000))
     file-name callback mask)))

(defun romacs--inotify-add (wd file-name callback mask)
  "add_watch: a watch of WD with the lowest free ID; its (WD . ID)."
  (let ((tail (assoc wd romacs--inotify-watch-list)) (id 0))
    (if (null tail)
        (setq tail (list wd)
              romacs--inotify-watch-list (cons tail romacs--inotify-watch-list))
      (while (and (cdr tail) (eql (car (car (cdr tail))) id))
        (setq tail (cdr tail) id (1+ id))))
    (setcdr tail (cons (list id file-name callback mask) (cdr tail)))
    (cons wd id)))

(defun romacs--inotify-remove-descriptor (element invalid)
  "remove_descriptor: drop ELEMENT, (WD . WATCHES), from the watch list;
unless INVALID (its IN_IGNORED came) the kernel's watch too.  The last
one gone closes the instance."
  (let ((error (and (not invalid)
                    (condition-case data
                        (progn (romacs--inotify-rm-watch (car element)) nil)
                      (file-notify-error data)))))
    (setq romacs--inotify-watch-list (delq element romacs--inotify-watch-list))
    (unless romacs--inotify-watch-list (romacs--inotify-close))
    (when error (signal (car error) (cdr error)))))

(defun romacs--inotify-valid-descriptor-p (descriptor)
  "valid_watch_descriptor."
  (and (consp descriptor)
       (fixnump (car descriptor)) (<= 0 (car descriptor) #x7fffffff)
       (natnump (cdr descriptor)) (fixnump (cdr descriptor))))

(defun inotify-rm-watch (watch-descriptor)
  "Remove an existing WATCH-DESCRIPTOR.

WATCH-DESCRIPTOR should be an object returned by `inotify-add-watch'.

See inotify_rm_watch(2) for more information."
  (unless (romacs--inotify-valid-descriptor-p watch-descriptor)
    (romacs--inotify-error "Invalid descriptor " watch-descriptor))
  (let* ((element (assoc (car watch-descriptor) romacs--inotify-watch-list))
         (prev element))
    (while (cdr prev)
      (if (not (eql (cdr watch-descriptor) (car (car (cdr prev)))))
          (setq prev (cdr prev))
        (setcdr prev (cdr (cdr prev)))
        (unless (cdr element) (romacs--inotify-remove-descriptor element nil))
        (setq prev nil))))
  t)

(defun inotify-valid-p (watch-descriptor)
  "Check a watch specified by its WATCH-DESCRIPTOR.

WATCH-DESCRIPTOR should be an object returned by `inotify-add-watch'.

A watch can become invalid if the file or directory it watches is
deleted, or if the watcher thread exits abnormally for any other
reason.  Removing the watch by calling `inotify-rm-watch' also makes
it invalid."
  (and (romacs--inotify-valid-descriptor-p watch-descriptor)
       (let ((element (assoc (car watch-descriptor) romacs--inotify-watch-list)))
         (and (assq (cdr watch-descriptor) (cdr element)) t))))

(defun romacs--inotify-event (watch record)
  "inotifyevent_to_event: RECORD, (WD MASK COOKIE NAME), as WATCH's
\(file-notify EVENT CALLBACK), or nil when WATCH did not ask for it."
  (let ((mask (nth 1 record)))
    ;; These come whether or not the watch asked for them.
    (when (or (/= 0 (logand mask (nth 3 watch)))
              (/= 0 (logand mask (logior #x8000 #x4000 #x40000000 #x2000))))
      (list 'file-notify
            (list (cons (car record) (car watch))
                  (romacs--inotify-aspects mask)
                  (or (nth 3 record) (nth 1 watch))
                  (nth 2 record))
            (nth 2 watch)))))

(defun romacs--inotify-events ()
  "inotify_callback: the instance's queued records as events, each
watch of its WD in turn; a WD's IN_IGNORED drops it from the list."
  (let (events)
    (dolist (record (romacs--inotify-read))
      (let ((element (assoc (car record) romacs--inotify-watch-list)))
        (when element
          (dolist (watch (cdr element))
            (let ((event (romacs--inotify-event watch record)))
              (when event (setq events (cons event events)))))
          (when (/= 0 (logand (nth 1 record) #x8000))
            (romacs--inotify-remove-descriptor element t)))))
    (nreverse events)))

(defun romacs--file-notify-watching-p ()
  "Whether a special event waits or a watch is live, so events may yet come."
  (and (or romacs--special-events romacs--inotify-watch-list) t))

(defun romacs--file-notify-dispatch ()
  "read_char for the queued special events, file notification events
read first: run each through its `special-event-map' binding.  An error
leaves the rest queued.  Non-nil if any ran."
  (when (or romacs--special-events romacs--inotify-watch-list)
    (setq romacs--special-events (nconc romacs--special-events (romacs--inotify-events)))
    (let ((ran nil))
      (while romacs--special-events
        (let ((event (car romacs--special-events)))
          (setq romacs--special-events (cdr romacs--special-events))
          (setq ran t)
          (romacs--special-event event)))
      ran)))

(defun romacs--file-notify-idle ()
  "`romacs--file-notify-dispatch' for the command loop's idle, where
read_key_sequence would read them: an error is reported as the command
loop reports one (cmd_error), and the loop goes on."
  (condition-case data
      (romacs--file-notify-dispatch)
    ((debug t) (romacs--cmd-error data) t)))

(provide 'inotify)

;;; inotify.el ends here
