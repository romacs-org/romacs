#!/usr/bin/env python3
"""A hermes keymap popup sized by fit-window-to-buffer, in a private compositor.

A native --loadup session with a private HOME and init.el (hermes pointed at
a loopback port nothing listens on) runs M-x hermes, then the dashboard's
popup key (the dashboard opens the popup itself first; that one is
dismissed with q).  keymap-popup displays its buffer in a bottom side window and
fits it with `fit-window-to-buffer', which measures with
`window-text-pixel-size' in the window the host has laid out.  An advice
around the fit records, in the popup's window, what the fit asked and got
and `posn-at-point' at the popup's first and last positions, and in the
dashboard's window `posn-at-point' at point.

The judge is the painted scene, not those answers: the popup window's
painted text rows must be exactly its buffer's lines, each row one line
high, with its mode line directly below the last, and each recorded posn's
pixel position must be where the painter drew that glyph.  labwc,
wlr-randr, wtype and grim come from PATH (the gate's headless tools).

With --child-frame, keymap-popup shows its buffer in a child frame
(`keymap-popup-backend-child-frame', fitted by `fit-frame-to-buffer'), a
layer over the frame: the popup's painted rows must be exactly its buffer's
lines, in the child frame's rect, and the root frame's windows and mode
lines must not move while it shows or after it goes.

With --which-key, which-key's `custom' popup type shows its keys in a
layer (romacs-layer.el) after C-x: a strip directly above the minibuffer
(the frame's bottom edge while romacs-complete's minibuffer layer is
hidden), with the root frame's windows unmoved while it shows and after it
goes.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

import fileio_live as fio

HERE = Path(__file__).resolve().parent
import find_file_live as ff
import live_oracle as live

INIT = '''(setq hermes-dashboard-transport-url "http://127.0.0.1:9"
      hermes-dashboard-transport-start-mode 'remote
      hermes-dashboard-transport-command "/nonexistent/hermes")
(setq keymap-popup-backend #'keymap-popup-backend-side-window)
(defun popup-live--posn (window pos)
  (let ((posn (posn-at-point pos window)))
    (and posn (list (posn-point posn) (posn-x-y posn) (posn-actual-col-row posn)
                    (posn-object-width-height posn)))))
(defun popup-live--record (fit buffer)
  (let* ((before (get-buffer-window buffer))
         (old (and before (window-pixel-height before)))
         (outcome (condition-case error (progn (funcall fit buffer) 'fitted)
                    (error (list 'error (error-message-string error)))))
         (window (get-buffer-window buffer))
         (dashboard (get-buffer-window "*Hermes Dashboard*")))
    (with-current-buffer buffer
      (write-region
       (prin1-to-string
        (list :outcome outcome :height-before old
              :lines (count-lines (point-min) (point-max))
              :window-pixel-height (window-pixel-height window)
              :window-body-pixel-height (window-body-height window t)
              :mode-line-height (window-mode-line-height window)
              ;; Its top as painted: past the frame's internal border.
              :window-pixel-top (nth 1 (window-edges window nil nil t))
              :text-left (car (window-inside-pixel-edges window))
              :frame-char-height (frame-char-height)
              :text-size (condition-case error (window-text-pixel-size window nil t nil nil t)
                           (error (error-message-string error)))
              :buffer-text-size (condition-case error (buffer-text-pixel-size buffer window)
                                  (error (error-message-string error)))
              :posn-first (condition-case error (popup-live--posn window (point-min))
                            (error (error-message-string error)))
              :posn-last (condition-case error
                             (popup-live--posn window (save-excursion (goto-char (point-max)) (skip-chars-backward "\\n") (1- (point))))
                           (error (error-message-string error)))
              :dashboard-posn (condition-case error (popup-live--posn dashboard (window-point dashboard))
                                (error (error-message-string error)))
              :dashboard-top (nth 1 (window-edges dashboard nil nil t))
              :dashboard-text-left (car (window-inside-pixel-edges dashboard))
              :line-starts (condition-case error
                               (save-excursion
                                 (goto-char (point-min))
                                 (let (out)
                                   (while (not (eobp))
                                     (push (list (buffer-substring-no-properties (point) (line-end-position))
                                                 (cdr (posn-x-y (posn-at-point (point) window))))
                                           out)
                                     (forward-line 1))
                                   (nreverse out)))
                             (error (error-message-string error)))))
       nil {state}))
    (if (eq outcome 'fitted) outcome (error "%s" (cadr outcome)))))
(with-eval-after-load 'keymap-popup
  (advice-add 'keymap-popup--fit-side-window :around #'popup-live--record))
'''


CHILD_INIT = '''(setq hermes-dashboard-transport-url "http://127.0.0.1:9"
      hermes-dashboard-transport-start-mode 'remote
      hermes-dashboard-transport-command "/nonexistent/hermes")
(setq keymap-popup-backend #'keymap-popup-backend-child-frame)
(defun popup-live--root-windows ()
  (mapcar (lambda (w) (list (buffer-name (window-buffer w)) (window-pixel-top w) (window-pixel-height w)))
          (window-list (frame-parent (selected-frame)) 'nomini)))
(defun popup-live--record (buffer)
  (let* ((frame (keymap-popup--session-get buffer :frame))
         (window (frame-root-window frame)))
    (with-current-buffer buffer
      (write-region
       (prin1-to-string
        (list :position (list (car (frame-position frame)) (cdr (frame-position frame)))
              :native (list (frame-native-width frame) (frame-native-height frame))
              :border (frame-parameter frame 'child-frame-border-width)
              :visible (frame-visible-p frame)
              :window-pixel-top (window-pixel-top window)
              :window-pixel-left (window-pixel-left window)
              :text-left (car (window-inside-pixel-edges window))
              :frame-char-height (frame-char-height)
              :mode-line (and (buffer-local-value 'mode-line-format buffer) t)
              :lines (save-excursion
                       (goto-char (point-min))
                       (let (out)
                         (while (not (eobp))
                           (push (buffer-substring-no-properties (point) (line-end-position)) out)
                           (forward-line 1))
                         (nreverse out)))
              :root-windows (popup-live--root-windows)
              :frames (length (frame-list))))
       nil {state} nil (quote quiet)))))
(with-eval-after-load 'keymap-popup
  (advice-add 'keymap-popup--show-child-frame :after #'popup-live--record))
'''


LAYER_INIT = '''(setq hermes-dashboard-transport-url "http://127.0.0.1:9"
      hermes-dashboard-transport-start-mode 'remote
      hermes-dashboard-transport-command "/nonexistent/hermes")
(setq keymap-popup-backend #'keymap-popup-backend-layer)
(defun popup-live--root-windows ()
  (mapcar (lambda (w) (list (buffer-name (window-buffer w)) (window-pixel-top w) (window-pixel-height w)))
          (window-list (frame-parent (selected-frame)) 'nomini)))
(defun popup-live--record (buffer)
  (let* ((frame (keymap-popup--session-get buffer :frame))
         (window (frame-root-window frame)))
    (with-current-buffer buffer
      (write-region
       (prin1-to-string
        (list :position (list (car (frame-position frame)) (cdr (frame-position frame)))
              :native (list (frame-native-width frame) (frame-native-height frame))
              :border (frame-parameter frame 'child-frame-border-width)
              :visible (frame-visible-p frame)
              :window-pixel-top (window-pixel-top window)
              :window-pixel-left (window-pixel-left window)
              :text-left (car (window-inside-pixel-edges window))
              :frame-char-height (frame-char-height)
              :mode-line (and (buffer-local-value 'mode-line-format buffer) t)
              :lines (save-excursion
                       (goto-char (point-min))
                       (let (out)
                         (while (not (eobp))
                           (push (buffer-substring-no-properties (point) (line-end-position)) out)
                           (forward-line 1))
                         (nreverse out)))
              :root-windows (popup-live--root-windows)
              :frames (length (frame-list))
              :chrome-bottom (romacs-chrome-bottom (frame-parent frame))
              :inset (frame-internal-border-width (frame-parent frame))
              :parent-text-width (frame-text-width (frame-parent frame))
              :kind (frame-parameter frame 'romacs-popup-kind)))
       nil {state} nil (quote quiet)))))
(with-eval-after-load 'keymap-popup
  (advice-add 'keymap-popup--fit-layer :after
            ;; Its first fit runs as the popup renders, before :show made the panel.
            (lambda (buffer) (when (keymap-popup--session-get buffer :frame) (popup-live--record buffer))))
(load {fixture} nil t)
(defun popup-live--fixture ()
  (interactive)
  (keymap-popup keymap-popup-fixture-map)
  (let* ((buffer (keymap-popup--popup-buffer)) (frame (keymap-popup--session-get buffer :frame))
         (window (frame-root-window frame)))
    (redisplay t)
    (write-region
     (prin1-to-string
      (list :runs (keymap-popup-fixture-placed-runs buffer)
            :position (list (car (frame-position frame)) (cdr (frame-position frame)))
            :text-left (car (window-inside-pixel-edges window)) :text-top (cadr (window-inside-pixel-edges window))
            :column (frame-char-width frame) :line (frame-char-height frame)))
     nil {faces} nil 'quiet)))
(global-set-key (kbd "C-c k") #'popup-live--fixture))
'''


REPEAT_INIT = '''(repeat-mode 1)
(split-window-right)
(defun popup-live--state ()
  (write-region (prin1-to-string (list :command (and (symbolp this-command) (symbol-name this-command))
                                       :window (format "%S" (selected-window))
                                       :repeat (and repeat-in-progress (symbol-name repeat-in-progress))
                                       :hints (and (bound-and-true-p romacs-popup--hints-shown) t)
                                       :bar-repeat (bound-and-true-p romacs-mode-line-repeat)
                                       :hint-window (and (bound-and-true-p romacs-popup--hints-place)
                                                         (format "%S" (car romacs-popup--hints-place)))))
                nil {state} nil 'quiet))
(add-hook 'post-command-hook #'popup-live--state 90)
(advice-add 'romacs-popup--hints-show :after (lambda (&rest _) (popup-live--state)))
'''


PROMPTS_INIT = '''(load {scenes} nil t)
(defun popup-live--prompt ()
  (interactive)
  (let* ((root (romacs-layer--root-frame)) (host (romacs--minibuffer-host)) (window (active-minibuffer-window)))
    (write-region
     (prin1-to-string
      (with-current-buffer (window-buffer window)
        (let ((lines (split-string (buffer-substring-no-properties (point-min) (point-max)) "\\n")))
          (list :command (and (symbolp this-command) (symbol-name this-command))
                :host (and host (list (car (frame-position host)) (cdr (frame-position host)) (frame-pixel-width host) (frame-pixel-height host)))
                :body (window-body-width window) :longest (seq-max (mapcar #'string-width lines)) :lines (length lines)
                :rows (/ (cdr (window-text-pixel-size window)) (frame-char-height root))
                :room (/ (frame-text-width root) (frame-char-width root))
                :text-pixels (romacs-complete--measure-prompt (current-buffer) (frame-char-width root) 100000)
                :layer-text (and host (frame-text-width host))
                :depth (minibuffer-depth)))))
     nil {state} nil 'quiet)))
(add-hook 'minibuffer-setup-hook (lambda () (run-at-time 0.6 nil #'popup-live--prompt)))
(global-set-key (kbd "<f12>") #'popup-live--prompt)
(define-key minibuffer-local-map (kbd "<f12>") #'popup-live--prompt)
;; The mode turned off in a prompt, then back on outside it.
(define-key minibuffer-local-map (kbd "<f4>") (lambda () (interactive) (romacs-complete-mode -1)))
(global-set-key (kbd "<f3>") (lambda () (interactive) (romacs-complete-mode 1)))
(define-key minibuffer-local-map (kbd "<f2>")
  (lambda () (interactive)
    (write-region (prin1-to-string (list :layer (and (frame-live-p romacs-complete--layer) (frame-visible-p romacs-complete--layer) t)
                                         :host (and (romacs--minibuffer-host) t)
                                         :root (eq (window-frame (minibuffer-window)) (romacs-layer--root-frame))
                                         :text (substring-no-properties (minibuffer-contents))))
                  nil {state} nil 'quiet)))
'''


TOASTS_INIT = '''(split-window-right)
(defun popup-live--toasts ()
  (write-region
   (prin1-to-string
    (list :toasts (mapcar (lambda (toast)
                            (let ((f (romacs-layer--frame (car toast))))
                              (list (substring-no-properties (nth 1 toast)) (frame-visible-p f)
                                    (car (frame-position f)) (cdr (frame-position f)) (frame-pixel-width f) (frame-pixel-height f)
                                    (and (frame-parameter f 'romacs-layer-glow-face) t))))
                          romacs-layer--toasts)
          :host (let ((host (romacs--minibuffer-host)))
                  (and host (frame-visible-p host)
                       (list (car (frame-position host)) (cdr (frame-position host)) (frame-pixel-width host) (frame-pixel-height host))))
          :pane (window-edges (frame-selected-window (romacs-layer--root-frame)) t nil t)
          :command (and (symbolp last-command) (symbol-name last-command))))
   nil {state} nil 'quiet))
(defun popup-live-path () (interactive)
  (message "Wrote %s" (expand-file-name "Projects/rust/romacs/rlisp/romacs-defaults.el" "~")))
(global-set-key (kbd "<f5>") #'popup-live-path)
(add-hook 'post-command-hook (lambda () (run-at-time 0.3 nil #'popup-live--toasts)) 90)
;; A toast while the M-x panel shows (a message there shows in the panel's
;; own echo, as GNU's echo area does over a minibuffer): placed below it.
(add-hook 'minibuffer-setup-hook (lambda () (run-at-time 0.4 nil (lambda () (romacs-layer-toast "Saved frame.rs (41234 bytes)") (run-at-time 0.4 nil #'popup-live--toasts)))))
'''


INDICATORS_INIT = '''(run-at-time 1 nil (lambda ()
  (switch-to-buffer (get-buffer-create "w.el"))
  (emacs-lisp-mode)
  (insert "(defun alpha ()\n  (list 1\n        2))\n\n(defun beta () 3)\n")
  (goto-char 20)
  (which-function-mode 1)
  (display-time-mode 1)
  (narrow-to-defun)
  ;; From the source buffer, which stays selected; *compilation* shows
  ;; in another window.
  (let ((default-directory (expand-file-name "~/"))) (compile "sleep 120"))
  (run-at-time 1 nil (lambda ()
    (write-region (prin1-to-string (list :time (substring-no-properties (string-trim display-time-string)) :selected (buffer-name (window-buffer)))) nil {state} nil 'quiet)))))
'''


WHICH_KEY_INIT = '''(require 'which-key)
(setq which-key-popup-type 'custom
      which-key-custom-show-popup-function #'romacs-popup-which-key-show
      which-key-custom-hide-popup-function #'romacs-popup-which-key-hide
      which-key-custom-popup-max-dimensions-function #'romacs-popup-which-key-max-dimensions
      ;; Not 0.3: the busy indicator's check falls 300 ms after a key, and
      ;; Lisp running then (the card's build) reads as busy since the key.
      which-key-idle-delay 0.6 which-key-idle-secondary-delay 0.05)
(which-key-mode 1)
(defun popup-live--record (&rest _)
  (let* ((frame (and (buffer-live-p which-key--buffer) (romacs-layer--frame-of which-key--buffer)))
         (window (romacs-layer--toast-window)))
    (write-region
     (prin1-to-string
      (list :shown (and frame (frame-visible-p frame) t)
            :kind (and frame (frame-parameter frame 'romacs-popup-kind))
            :position (and frame (list (car (frame-position frame)) (cdr (frame-position frame))))
            :size (and frame (list (frame-pixel-width frame) (frame-pixel-height frame)))
            :title (and frame (with-current-buffer which-key--buffer
                                (buffer-substring-no-properties (point-min) (save-excursion (goto-char (point-min)) (line-end-position)))))
            :page (nth 1 romacs-popup--card)
            :point (let ((xy (posn-x-y (posn-at-point (window-point window) window))) (body (window-edges window t nil t)))
                     (and xy (list (+ (nth 0 body) (car xy)) (+ (nth 1 body) (cdr xy)) (frame-char-width) (frame-char-height))))
            :pane (window-edges window t nil t)
            :chrome-bottom (romacs-chrome-bottom (romacs-layer--root-frame))
            :command (and (symbolp last-command) (symbol-name last-command))))
     nil {state} nil (quote quiet))))
(advice-add 'romacs-popup--card-render :after #'popup-live--record)
(advice-add 'romacs-popup--card-hide :after #'popup-live--record)
(add-hook 'post-command-hook (lambda () (run-at-time 0.2 nil #'popup-live--record)) 90)
;; Scenes: an org buffer, a narrow pane, point at the bottom right.
(defun popup-live-org () (interactive)
  (delete-other-windows) (switch-to-buffer (get-buffer-create "notes.org")) (org-mode)
  (erase-buffer) (insert "* Notes\nSome text.\n") (goto-char (point-min)))
(defun popup-live-narrow () (interactive)
  (popup-live-org) (switch-to-buffer "*scratch*") (split-window-right) (split-window-right) (balance-windows)
  (select-window (frame-first-window)))
(defun popup-live-flip () (interactive)
  (delete-other-windows) (switch-to-buffer (get-buffer-create "flip.txt"))
  ;; Lines that fit the window (a truncated line's posn is another matter).
  (erase-buffer) (dotimes (_ 200) (insert (make-string (- (window-body-width) 6) ?x) "\n"))
  (goto-char (point-min)) (forward-line (- (window-body-height) 3)) (end-of-line))
(global-set-key (kbd "<f5>") #'popup-live-org)
(global-set-key (kbd "<f6>") #'popup-live-narrow)
(global-set-key (kbd "<f7>") #'popup-live-flip)
;; Point past a truncated row's right edge: its posn is the edge's.
(defun popup-live-flip-truncated () (interactive)
  (popup-live-flip) (setq truncate-lines t) (setq-local auto-hscroll-mode nil)
  (erase-buffer) (dotimes (_ 200) (insert (make-string 300 ?x) "\n"))
  (goto-char (point-min)) (forward-line (- (window-body-height) 3)) (move-to-column 200))
(global-set-key (kbd "<f10>") #'popup-live-flip-truncated)
(global-set-key (kbd "<f8>") (lambda () (interactive) (which-key-mode (if which-key-mode -1 1))))
'''


def check_repeat(args, output, report, env, process, state, keys, glyphs):
    """C-x o in a split frame: repeat-mode's map shows as hints of its keys
    in a layer (and not also in the status bar); a bare o repeats
    other-window with the hints still shown; another key ends the repeat
    and the hints go."""
    def hint_rows():
        return [text for _, text, _ in rows([g for g in glyphs() if g.get('frame', 1) != 1], 'Text') if 'other-window' in text]

    def read():
        return read_plist(state.read_text()) if state.exists() else {}

    keys('-M', 'ctrl', 'x', '-m', 'ctrl', 'o')
    fio.wait(lambda: read().get('repeat') == 'other-window-repeat-map', args.timeout, 'repeat after C-x o', process)
    first = read()
    fio.wait(lambda: hint_rows(), args.timeout, 'hints after C-x o', process)
    time.sleep(.5)
    fio.shot(output / 'hints.png', env, report)
    shown = read()
    report['captures'].append({'name': 'hints', 'rows': hint_rows(), 'state': shown})
    if shown.get('bar-repeat'):
        raise AssertionError(f"the status bar shows the repeat too: {shown['bar-repeat']!r}")
    state.unlink()
    keys('o')
    fio.wait(lambda: read().get('command') == 'other-window', args.timeout, 'o repeats other-window', process)
    again = read()
    if again['window'] == first['window'] or again.get('repeat') != 'other-window-repeat-map':
        raise AssertionError(f'o did not repeat: {first} then {again}')
    fio.wait(lambda: hint_rows(), args.timeout, 'hints after the repeat', process)
    # The hints follow the selection into the window o selected.
    fio.wait(lambda: read().get('hint-window') == again['window'], args.timeout, 'hints in the selected window', process)
    state.unlink()
    keys('-M', 'ctrl', 'f', '-m', 'ctrl')
    fio.wait(lambda: read().get('command') is not None, args.timeout, 'C-f', process)
    fio.wait(lambda: not hint_rows(), args.timeout, 'hints gone after C-f', process)
    done = read()
    if done.get('repeat'):
        raise AssertionError(f'repeat still in progress after C-f: {done}')
    report['checks'].append({'hints': report['captures'][-1]['rows'], 'repeated': again['window'], 'ended': done.get('command')})


def check_prompts(args, output, report, env, process, state, keys, glyphs):
    """read-from-minibuffer menus laid out for a frame-wide minibuffer, as
    GNU's is (notmuch-jump, read-multiple-choice, a long y-or-n-p): the
    minibuffer's layer widens to the prompt's widest line, centred, so no
    row wraps; where the frame is narrower than that line, the layer takes
    the frame's room and the rows wrap at words."""
    for key, scene in (('F5', 'jump'), ('F6', 'rmc'), ('F7', 'yn')):
        if state.exists():
            state.unlink()
        keys('-k', key)
        fio.wait(state.exists, args.timeout, f'{scene} prompt', process)
        time.sleep(.6)
        probe = read_plist(state.read_text())
        fio.shot(output / f'prompt-{scene}.png', env, report)
        report['captures'].append({'name': scene, 'state': probe})
        x, y, width, height = probe['host']
        fits = probe['longest'] + 2 <= probe['room']
        if fits and (probe['rows'] != probe['lines'] or probe['body'] < probe['longest']):
            raise AssertionError(f'{scene}: {probe["lines"]} lines show in {probe["rows"]} rows, body {probe["body"]} for {probe["longest"]}')
        if not fits and probe['body'] + 4 < probe['room']:
            raise AssertionError(f'{scene}: a prompt wider than the frame in a layer {probe["body"]} wide of {probe["room"]}')
        if abs((x + width / 2) - args.width / 2) > 12:
            raise AssertionError(f'{scene}: layer at {x} {width} wide, not centred in {args.width}')
        keys('-M', 'ctrl', 'g', '-m', 'ctrl')
        time.sleep(.6)
    def probe_now(what):
        if state.exists():
            state.unlink()
        keys('-k', 'F12')
        fio.wait(state.exists, args.timeout, what, process)
        return read_plist(state.read_text())

    def wraps_needlessly(probe):
        # A layer narrower than its text while the room holds it.
        return probe['layer-text'] < min(probe['text-pixels'], (probe['room'] - 2) * 10) - 1

    # Typing into a prompt-only minibuffer widens its layer; deleting narrows it.
    keys('-k', 'F8')
    time.sleep(1)
    keys('q' * 40)
    typed = probe_now('typed')
    keys(*(['-k', 'BackSpace'] * 35))
    deleted = probe_now('deleted')
    fio.shot(output / 'prompt-typed.png', env, report)
    report['captures'].append({'name': 'typed', 'state': typed})
    report['captures'].append({'name': 'deleted', 'state': deleted})
    def fits(probe):
        return probe['text-pixels'] + 20 <= probe['room'] * 10

    if (wraps_needlessly(typed) or wraps_needlessly(deleted) or (fits(typed) and typed['rows'] != 1)
            or deleted['layer-text'] > typed['layer-text'] or (fits(deleted) and deleted['rows'] != 1)):
        raise AssertionError(f'typing {typed}, deleting {deleted}')
    keys('-M', 'ctrl', 'g', '-m', 'ctrl')
    time.sleep(.6)
    # A nested short prompt returns to its long outer one at the outer's width,
    # after RET and after C-g.
    keys('-k', 'F9')
    time.sleep(1)
    outer = probe_now('outer')
    for leave in (['-k', 'Return'], ['-M', 'ctrl', 'g', '-m', 'ctrl']):
        keys('-k', 'F11')
        time.sleep(1)
        keys(*leave)
        time.sleep(.8)
        back = probe_now('back in the outer prompt')
        report['captures'].append({'name': f'nested {leave[-1]}', 'state': back})
        if back['depth'] != 1 or back['layer-text'] != outer['layer-text'] or wraps_needlessly(back):
            raise AssertionError(f'outer {outer}, back {back}')
    keys('-M', 'ctrl', 'g', '-m', 'ctrl')
    time.sleep(.6)
    # A prompt at twice the height is as wide as it is painted.
    keys('-k', 'F10')
    time.sleep(1)
    tall = probe_now('tall')
    fio.shot(output / 'prompt-tall.png', env, report)
    report['captures'].append({'name': 'tall', 'state': tall})
    if wraps_needlessly(tall):
        raise AssertionError(f'tall prompt {tall}')
    keys('-M', 'ctrl', 'g', '-m', 'ctrl')
    time.sleep(.6)
    # A prompt in a small face, more characters than the room's columns: the
    # layer takes the room rather than wrap what fits it.
    keys('-M', 'ctrl', 'c', '-m', 'ctrl', '9')
    time.sleep(1)
    small = probe_now('small')
    fio.shot(output / 'prompt-small.png', env, report)
    report['captures'].append({'name': 'small', 'state': small})
    if small['layer-text'] + 4 * 10 < small['room'] * 10 and small['rows'] > 1:
        raise AssertionError(f'small-face prompt {small}')
    keys('-M', 'ctrl', 'g', '-m', 'ctrl')
    time.sleep(.6)
    # Turned off in a prompt, the mode makes no layer as the user types: the
    # input shows in the frame's own minibuffer line.
    keys('-k', 'F8')
    time.sleep(1)
    keys('-k', 'F4')
    time.sleep(.5)
    keys('abc')
    if state.exists():
        state.unlink()
    keys('-k', 'F2')
    fio.wait(state.exists, args.timeout, 'the prompt with the mode off', process)
    off = read_plist(state.read_text())
    fio.shot(output / 'prompt-mode-off.png', env, report)
    report['captures'].append({'name': 'mode off', 'state': off})
    if off['layer'] or off['host'] or not off['root'] or off['text'] != 'abc':
        raise AssertionError(f'mode off in a prompt: {off}')
    keys('-M', 'ctrl', 'g', '-m', 'ctrl')
    time.sleep(.5)
    keys('-k', 'F3')
    time.sleep(.5)
    report['checks'].append({c['name']: [c['state'].get('body'), c['state'].get('longest'), c['state'].get('rows'), c['state'].get('lines')] for c in report['captures']})


def check_toasts(args, output, report, env, process, state, keys, glyphs):
    """One toast at a time, in the selected pane's top right: a long file
    name abbreviated and cut in its directories; an undefined key's message
    replacing it with the error glow; C-g toasting nothing; a message while
    the M-x panel shows placed below the panel; the next command clearing it."""
    def read():
        return read_plist(state.read_text()) if state.exists() else {}

    def wait_for(pred, what):
        def holds():
            try:
                return pred(read())
            except Exception as error:
                report['state_error'] = repr(error)
                return False
        try:
            fio.wait(holds, args.timeout, what, process)
        except AssertionError:
            report['last_state'] = state.read_text() if state.exists() else None
            raise
        return read()

    keys('-k', 'F5')
    shown = wait_for(lambda s: s.get('command') == 'popup-live-path' and s.get('toasts'), 'path toast')
    time.sleep(.6)
    fio.shot(output / 'toast-path.png', env, report)
    report['captures'].append({'name': 'path', 'state': shown})
    [(text, visible, x, y, width, height, glow)] = shown['toasts']
    left, top, right, bottom = shown['pane']
    if not (visible and text.startswith('Wrote ~/') and '…/' in text and text.endswith('romacs-defaults.el')):
        raise AssertionError(f'path toast {text!r} visible {visible}')
    if not (left <= x and x + width <= right and top <= y) or width > 420 or glow:
        raise AssertionError(f'path toast at {x},{y} {width}x{height} in pane {shown["pane"]}, glow {glow}')
    keys('-k', 'F9')
    error = wait_for(lambda s: s.get('toasts') and 'undefined' in s['toasts'][0][0], 'undefined-key toast')
    time.sleep(.6)
    fio.shot(output / 'toast-error.png', env, report)
    report['captures'].append({'name': 'error', 'state': error})
    if len(error['toasts']) != 1 or not error['toasts'][0][6]:
        raise AssertionError(f'error toast {error["toasts"]}')
    keys('-M', 'ctrl', 'g', '-m', 'ctrl')
    quiet = wait_for(lambda s: s.get('command') == 'keyboard-quit', 'C-g')
    if quiet.get('toasts'):
        raise AssertionError(f'C-g toasted {quiet["toasts"]}')
    keys('-M', 'alt', 'x', '-m', 'alt')
    panel = wait_for(lambda s: s.get('host') and s.get('toasts') and s['toasts'][0][0].startswith('Saved'), 'toast over the panel')
    time.sleep(.6)
    fio.shot(output / 'toast-panel.png', env, report)
    report['captures'].append({'name': 'panel', 'state': panel})
    hx, hy, hw, hh = panel['host']
    _, _, x, y, width, height, _ = panel['toasts'][0]
    if x < hx + hw and hx < x + width and y < hy + hh and hy < y + height:
        raise AssertionError(f'toast at {x},{y} {width}x{height} over the panel {panel["host"]}')
    keys('-M', 'ctrl', 'g', '-m', 'ctrl')
    keys('-M', 'ctrl', 'f', '-m', 'ctrl')
    cleared = wait_for(lambda s: s.get('command') == 'forward-char', 'C-f')
    if any(t[1] for t in cleared.get('toasts') or []):
        raise AssertionError(f'toasts after the next command {cleared["toasts"]}')
    report['checks'].append({'path': text, 'error': error['toasts'][0][0], 'panel': [panel['host'], panel['toasts'][0][2:6]]})


def check_indicators(args, output, report, env, process, state, keys, glyphs):
    """What packages and GNU features show through the standard places
    reaches the status bar of the selected window: display-time's time and
    which-function's name (`mode-line-misc-info'), compile's `[Compiling]'
    (`mode-line-modes') from the source buffer, the narrowing glyph; with
    the compilation buffer selected, its process state
    (`mode-line-process').  Minor-mode lighters stay out."""
    def bar():
        return ' '.join(text for _, text, _ in rows(glyphs(), 'StatusBar'))

    fio.wait(state.exists, args.timeout, 'indicators set up', process)
    setup = read_plist(state.read_text())
    if setup['selected'] != 'w.el':
        raise AssertionError(f'selected {setup}')
    narrow = chr(0xFE005)
    wanted = [setup['time'], '[alpha]', '[Compiling]', narrow]
    fio.wait(lambda: all(w in bar() for w in wanted), args.timeout, f'bar holds {wanted}', process)
    time.sleep(.5)
    fio.shot(output / 'indicators.png', env, report)
    source = bar()
    report['captures'].append({'name': 'indicators', 'bar': source})
    for lighter in ('ElDoc', 'WK', 'Narrow'):
        if lighter in source:
            raise AssertionError(f'the bar shows {lighter!r}: {source!r}')
    # The compilation buffer selected: its process state.
    keys('-M', 'ctrl', 'x', '-m', 'ctrl', 'o')
    fio.wait(lambda: ':run' in bar(), args.timeout, 'process state with *compilation* selected', process)
    time.sleep(.5)
    fio.shot(output / 'process.png', env, report)
    report['captures'].append({'name': 'process', 'bar': bar()})
    report['checks'].append({'source_bar': source, 'process_bar': report['captures'][-1]['bar']})


def check_which_key(args, output, report, env, process, state, keys, glyphs):
    """which-key's automatic hint is the key-hint card: after C-x, at the
    selected pane's bottom right 12 px in and above the status bar, titled
    by the prefix, prefixes first; C-h pages it in place and the command
    still runs; a sub-prefix key hides it at once and a card for the longer
    prefix follows; C-g ends it.  Then C-c in org, C-h as a prefix, a narrow
    pane, and point at the bottom right (the card flips to the bottom left)."""
    def card_rows():
        return [text for _, text, _ in rows([g for g in glyphs() if g.get('frame', 1) != 1], 'Text')]

    def read():
        return read_plist(state.read_text()) if state.exists() else {}

    def card(title):
        fio.wait(lambda: read().get('shown') and read().get('kind') == 'card' and read().get('title') == title and card_rows(),
                 args.timeout, f'card {title}', process)
        time.sleep(.6)
        return read()

    def gone(what):
        fio.wait(lambda: not card_rows(), args.timeout, f'card gone after {what}', process)

    def placed(probe, corner='right'):
        (x, y), (width, height) = probe['position'], probe['size']
        left, top, right, bottom = probe['pane']
        inside = left + 12 <= x and x + width <= right - 12 and top + 12 <= y and y + height <= min(bottom, probe['chrome-bottom']) - 12
        at = (x + width == right - 12) if corner == 'right' else (x == left + 12)
        if not (inside and at and y + height == bottom - 12):
            raise AssertionError(f"card at {x},{y} {width}x{height} in pane {probe['pane']} (bar {probe['chrome-bottom']}), want bottom {corner}")

    time.sleep(1)
    # A prefix pending with which-key off, for the busy indicator's sake.
    keys('-k', 'F8')
    time.sleep(.5)
    keys('-M', 'ctrl', 'x', '-m', 'ctrl')
    time.sleep(1.2)
    fio.shot(output / 'pending-no-card.png', env, report)
    keys('-M', 'ctrl', 'g', '-m', 'ctrl')
    keys('-k', 'F8')
    time.sleep(.5)
    keys('-M', 'ctrl', 'x', '-m', 'ctrl')
    cx = card('C-x')
    fio.shot(output / 'card-cx.png', env, report)
    shown = card_rows()
    report['captures'].append({'name': 'C-x', 'rows': shown, 'state': cx})
    placed(cx)
    if not any('+' in row for row in shown[1:3]) or not any('find-file' in row for row in shown):
        raise AssertionError(f'C-x card rows {shown}')
    keys('-M', 'ctrl', 'h', '-m', 'ctrl')
    fio.wait(lambda: read().get('page') == 1 and card_rows(), args.timeout, 'C-h pages the card', process)
    time.sleep(.6)
    fio.shot(output / 'card-cx-page.png', env, report)
    report['captures'].append({'name': 'C-x page 2', 'rows': card_rows()})
    keys('h')
    fio.wait(lambda: read().get('command') == 'mark-whole-buffer', args.timeout, 'C-x C-h h runs C-x h', process)
    gone('C-x C-h h')
    keys('-M', 'ctrl', 'g', '-m', 'ctrl')
    keys('-M', 'ctrl', 'x', '-m', 'ctrl')
    card('C-x')
    keys('4')
    gone('C-x 4')
    card('C-x 4')
    keys('-M', 'ctrl', 'g', '-m', 'ctrl')
    gone('C-g')
    for key, title, name in (('F5', 'C-c', 'card-org-cc'), (None, 'C-h', 'card-ch')):
        if key:
            keys('-k', key)
            time.sleep(.5)
        keys('-M', 'ctrl', title[-1], '-m', 'ctrl')
        probe = card(title)
        fio.shot(output / f'{name}.png', env, report)
        report['captures'].append({'name': title, 'rows': card_rows(), 'state': probe})
        placed(probe)
        keys('-M', 'ctrl', 'g', '-m', 'ctrl')
        gone(title)
    keys('-k', 'F6')
    time.sleep(.5)
    keys('-M', 'ctrl', 'x', '-m', 'ctrl')
    narrow = card('C-x')
    fio.shot(output / 'card-narrow.png', env, report)
    report['captures'].append({'name': 'narrow', 'rows': card_rows(), 'state': narrow})
    (x, _), (width, _) = narrow['position'], narrow['size']
    if x < 0 or width > narrow['pane'][2] - narrow['pane'][0] and x + width > 1280:
        raise AssertionError(f'narrow card {narrow}')
    keys('-M', 'ctrl', 'g', '-m', 'ctrl')
    gone('narrow C-g')
    # C-x 4's card is narrow enough for a side: over point at the bottom
    # right, it takes the bottom left.
    keys('-k', 'F7')
    time.sleep(.5)
    keys('-M', 'ctrl', 'x', '-m', 'ctrl')
    card('C-x')
    keys('4')
    flip = card('C-x 4')
    fio.shot(output / 'card-flip.png', env, report)
    report['captures'].append({'name': 'flip', 'state': flip})
    placed(flip, 'left')
    keys('-M', 'ctrl', 'g', '-m', 'ctrl')
    gone('flip C-g')
    keys('-k', 'F10')
    time.sleep(.5)
    keys('-M', 'ctrl', 'x', '-m', 'ctrl')
    card('C-x')
    keys('4')
    flip_truncated = card('C-x 4')
    fio.shot(output / 'card-flip-truncated.png', env, report)
    report['captures'].append({'name': 'flip truncated', 'state': flip_truncated})
    # Point past the truncated row's edge is drawn at its last column, which
    # the card, inset in the pane, does not cover.
    (px, py, cw, ch), (x, y), (width, height) = flip_truncated['point'], flip_truncated['position'], flip_truncated['size']
    if px < x + width and x < px + cw and py < y + height and y < py + ch:
        raise AssertionError(f'card {flip_truncated} over point at {px},{py}')
    keys('-M', 'ctrl', 'g', '-m', 'ctrl')
    gone('flip truncated C-g')
    report['checks'].append({'cx': cx['position'], 'narrow': narrow['position'], 'flip': flip['position']})


def scene_glyphs(text):
    scene = fio.last_scene(text)
    return (scene or {}).get('glyphs', [])


def rows(glyphs, role):
    """ROLE's painted rows, top down: (y, text, [(x, char)])."""
    by_y = {}
    for glyph in glyphs:
        if glyph['role'] == role:
            by_y.setdefault(glyph['y'], []).append((glyph['x'], glyph['char']))
    return [(y, ''.join(c for _, c in sorted(row)), sorted(row)) for y, row in sorted(by_y.items())]


def read_plist(text):
    """The few Lisp shapes the init writes: lists, integers, strings, symbols."""
    tokens, i = [], 0
    while i < len(text):
        c = text[i]
        if c in '()':
            tokens.append(c); i += 1
        elif c.isspace():
            i += 1
        elif c == '"':
            j, out = i + 1, ''
            while text[j] != '"':
                if text[j] == '\\':
                    j += 1
                out += text[j]; j += 1
            tokens.append(('str', out)); i = j + 1
        else:
            j = i
            while j < len(text) and not text[j].isspace() and text[j] not in '()':
                j += 1
            tokens.append(('atom', text[i:j])); i = j

    def parse(at):
        token = tokens[at]
        if token == '(':
            items, at = [], at + 1
            while tokens[at] != ')':
                item, at = parse(at)
                items.append(item)
            # A dotted pair (A . B) reads as [A, B].
            return [x for x in items if x != '.'], at + 1
        kind, value = token
        if kind == 'str':
            return value, at + 1
        try:
            return int(value), at + 1
        except ValueError:
            return (None if value == 'nil' else value), at + 1

    value, _ = parse(0)
    return {key.lstrip(':'): item for key, item in zip(value[::2], value[1::2])}


def lisp_string(text):
    """A printed Lisp string's value (prin1 of an ASCII string)."""
    return json.loads(text)


def multiset(runs):
    return sorted((text, tuple(faces or ())) for text, faces in runs)


def check_layer_faces(args, output, report, env, process, faces_state, keys, glyphs):
    """keymap_popup_fixture.el's menu in Romacs's panel shows every key,
    heading, entry, exit row and title in the faces pinned GNU X11 shows
    them in (keymap_popup_faces.gnu): the same runs, as multisets (the
    panel's width may lay columns out differently), the title its first
    row; and each run is painted in its face's colour."""
    gnu = json.loads(lisp_string([l for l in (HERE / 'keymap_popup_faces.gnu').read_text().splitlines() if l and not l.startswith(';')][-1]))
    keys('-M', 'ctrl', 'c', '-m', 'ctrl', 'k')
    fio.wait(faces_state.exists, args.timeout, 'fixture popup', process)
    time.sleep(1.5)
    fio.shot(output / 'fixture.png', env, report)
    probe = read_plist(faces_state.read_text())
    runs = [tuple(run) for run in probe['runs']]
    title = [(t, f) for t, f, line, _, _ in runs if line == 0]
    body = [(t, f) for t, f, line, _, _ in runs if line > 0]
    report['checks'].append({'fixture_title': title, 'fixture_runs': len(body)})
    if multiset(title) != multiset(gnu['title']):
        raise AssertionError(f"title runs {title}, GNU's {gnu['title']}")
    if multiset(body) != multiset(gnu['body']):
        missing = set(multiset(gnu['body'])) - set(multiset(body))
        extra = set(multiset(body)) - set(multiset(gnu['body']))
        raise AssertionError(f'panel runs differ from GNU: missing {sorted(missing)}, extra {sorted(extra)}')
    # Painted: the glyph at each run's first cell draws in its face's colour.
    left, top = probe['position']
    painted = {(round(g['x']), round(g['y'])): g for g in glyphs() if g.get('frame', 1) != 1 and g['role'] == 'Text'}
    wrong = []
    for text, faces, line, column, color in runs:
        x, y = left + probe['text-left'] + column * probe['column'], top + probe['text-top'] + line * probe['line']
        glyph = painted.get((x, y))
        if glyph is None or glyph.get('char') != text[0] or glyph.get('foreground', '').lower() != color:
            wrong.append((text, faces, color, glyph and (glyph.get('char'), glyph.get('foreground'))))
    if wrong:
        raise AssertionError(f'runs painted in other colours: {wrong[:6]}')
    keys('q')


def check_child_frame(args, output, report, env, process, state, keys, glyphs, log_path):
    """The popup is a child frame: its rows are its buffer's lines in its
    rect, painted over the root frame, whose windows never move."""
    def root(role):
        return rows([g for g in glyphs() if g.get('frame', 1) == 1], role)

    fio.wait(state.exists, args.timeout, 'first popup', process)
    state.unlink()
    keys('h')
    fio.wait(state.exists, args.timeout, 'popup shown', process)
    probe = read_plist(state.read_text())
    report['probe'] = probe

    def popup_rows():
        return rows([g for g in glyphs() if g.get('frame', 1) != 1], 'Text')

    fio.wait(lambda: any(text.strip() for _, text, _ in popup_rows()), args.timeout, 'popup painted', process)
    time.sleep(1.5)
    fio.shot(output / 'popup.png', env, report)
    # The root frame's mode line: its status bar (VM2-STATUS-BAR).
    shown = {'root_mode_lines': [(y, t) for y, t, _ in root('StatusBar') + root('ModeLine')], 'popup_rows': [(y, t) for y, t, _ in popup_rows()]}
    report['captures'].append({'name': 'popup', **shown})
    lines = [line.rstrip() for line in probe['lines'] if line.strip()]
    # A line as wide as the window continues on the next row, as GNU
    # shows it without fringes (the last column holds the `\\').
    painted, carry = [], ''
    for _, text in shown['popup_rows']:
        if text.endswith('\\'):
            carry += text[:-1]
            continue
        painted.append((carry + text).rstrip())
        carry = ''
    painted = [text for text in painted if text.strip()]
    if painted != lines:
        raise AssertionError(f'popup painted {painted} for its lines {lines}')
    # Its rows start at its window's top inside the frame's rect, a line apart.
    left, top = probe['position']
    first_y = shown['popup_rows'][0][0]
    line = probe['frame-char-height']
    if first_y != top + probe['window-pixel-top']:
        raise AssertionError(f"first popup row at {first_y}, frame top {top} + window top {probe['window-pixel-top']}")
    ys = [y for y, _ in shown['popup_rows']]
    if any(not line <= b - a <= 2 * line + 1 for a, b in zip(ys, ys[1:])):
        raise AssertionError(f'popup rows at {ys}, not a line ({line}px) apart')
    width, height = probe['native']
    xs = [x for _, _, row in popup_rows() for x, _ in row]
    if min(xs) < left + probe['text-left'] or max(xs) >= left + width:
        raise AssertionError(f'popup glyphs span {min(xs)}..{max(xs)}, frame at {left} width {width}')
    if ys[-1] + line > top + height:
        raise AssertionError(f'last popup row {ys[-1]} below the frame bottom {top + height}')
    if args.layer:
        # Romacs's panel: across the frame's bottom above the status bar,
        # its title (the popup's mode-line text) its first row, no mode
        # line of its own, and no context segment in the status bar.
        if probe['kind'] != 'panel' or probe['mode-line']:
            raise AssertionError(f"panel kind {probe['kind']}, mode line {probe['mode-line']}")
        if (left, top + height) != (probe['inset'], probe['chrome-bottom']):
            raise AssertionError(f"panel at {left},{top} {width}x{height}, chrome bottom {probe['chrome-bottom']}")
        if not lines or not lines[0].lstrip().startswith('q'):
            raise AssertionError(f'title row {lines[:1]}')
        bar = ' '.join(t for _, t in shown['root_mode_lines'])
        if 'Dismiss' in bar or lines[0].strip() in bar:
            raise AssertionError(f'status bar shows the popup: {bar!r}')
        report['checks'].append({'panel_title': lines[0], 'panel_rows': len(lines)})
    # The root frame's windows are as they were with the popup gone.
    keys('q')
    fio.wait(lambda: not popup_rows(), args.timeout, 'popup gone', process)
    time.sleep(1)
    fio.shot(output / 'gone.png', env, report)
    gone = [(y, t) for y, t, _ in root('StatusBar') + root('ModeLine')]
    report['captures'].append({'name': 'gone', 'root_mode_lines': gone})
    if [y for y, _ in gone] != [y for y, _ in shown['root_mode_lines']]:
        raise AssertionError(f"root mode lines at {[y for y, _ in gone]} after, {[y for y, _ in shown['root_mode_lines']]} while the popup showed")
    report['checks'].append({'popup_lines': len(lines), 'popup_rect': [left, top, width, height],
                             'root_windows': probe['root-windows'], 'root_mode_line_y': [y for y, _ in gone]})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--timeout', type=float, default=120)
    parser.add_argument('--font', default='Iosevka')
    parser.add_argument('--font-size', default='14')
    parser.add_argument('--child-frame', action='store_true', help="keymap-popup's child-frame backend")
    parser.add_argument('--which-key', action='store_true', help="which-key's popup in a layer")
    parser.add_argument('--layer', action='store_true', help="keymap-popup's Romacs panel backend")
    parser.add_argument('--width', type=int, default=1280, help='the output width: a narrow one reflows the menu')
    parser.add_argument('--repeat', action='store_true', help="repeat-mode's map as hints after C-x o")
    parser.add_argument('--indicators', action='store_true', help="packages' indicators in the status bar")
    parser.add_argument('--toasts', action='store_true', help='messages as toasts')
    parser.add_argument('--prompts', action='store_true', help='read-from-minibuffer menus in the minibuffer layer')
    parser.add_argument('--theme', help='a theme loaded first (catppuccin-latte: a light one)')
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    report = {'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'status': 'FAIL', 'captures': [], 'checks': []}
    compositor = keyboard = process = None
    try:
        with tempfile.TemporaryDirectory(prefix='popup-live-') as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            home, work, config = [root / name for name in ('home', 'work', 'config')]
            for directory in (home, work, config):
                directory.mkdir()
            state = work / 'popup-state.el'
            (home / 'config/romacs').mkdir(parents=True)
            init = (PROMPTS_INIT if args.prompts else TOASTS_INIT if args.toasts else INDICATORS_INIT if args.indicators else REPEAT_INIT if args.repeat else WHICH_KEY_INIT if args.which_key else CHILD_INIT if args.child_frame
                    else LAYER_INIT if args.layer else INIT)
            if args.theme:
                # Doubled braces survive the init's format below.
                init = f"(mapc #'disable-theme custom-enabled-themes) (load-theme '{args.theme} t)\n" + init
            faces_state = work / 'popup-faces.el'
            (home / 'config/romacs/init.el').write_text(init.format(state=json.dumps(str(state)), fixture=json.dumps(str(HERE / 'keymap_popup_fixture.el')),
                                                                   faces=json.dumps(str(faces_state))) if args.layer else init.format(state=json.dumps(str(state)), scenes=json.dumps(str(HERE / 'prompt_scenes.el'))) if args.prompts else init.format(state=json.dumps(str(state))))
            (config / 'rc.xml').write_text(live.RC_XML)
            env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home), 'LANG': 'C.UTF-8', 'ROMACS_DIAGNOSTICS': '1',
                   'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
                   'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
                   'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
                   'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
                   'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop',
                   'ROMACS_GUI_FONT': args.font, 'ROMACS_GUI_FONT_SIZE': args.font_size}
            for key in ('ROMACS_LISP_DIR', 'ROMACS_RUNTIME_ROOT', 'ROMACS_WORLD', 'ROMACS_GATE_FONTCONFIG'):
                if key in os.environ:
                    env[key] = os.environ[key]
            if 'ROMACS_GATE_FONTCONFIG' in env:
                env['FONTCONFIG_FILE'] = env['ROMACS_GATE_FONTCONFIG']
            log_path = output / 'binary.stderr'
            with (output / 'compositor.log').open('w') as compositor_log, log_path.open('w') as log:
                compositor = subprocess.Popen([shutil.which('labwc', path=env['PATH']) or live.LABWC, '-C', str(config)],
                                              env=env, stdout=compositor_log, stderr=compositor_log)
                fio.wait(lambda: any(p.is_socket() for p in root.glob('wayland-*')), 15, 'private compositor', compositor)
                env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
                # Every client below connects to XDG_RUNTIME_DIR/WAYLAND_DISPLAY:
                # the private compositor's socket, never the user's display.
                socket = Path(env['XDG_RUNTIME_DIR']) / env['WAYLAND_DISPLAY']
                assert socket.is_socket() and socket.parent == root, socket
                assert 'DISPLAY' not in env
                report['wayland_socket'] = str(socket)
                subprocess.run(['wlr-randr', '--output', 'HEADLESS-1', '--custom-mode', f'{args.width}x800'],
                               env=env, check=True, timeout=15)
                keyboard = subprocess.Popen(['wtype', '-s', '120000', '-k', 'Shift_L'], env=env, stdout=log, stderr=log)
                time.sleep(.3)
                process = subprocess.Popen([str(binary), '--loadup', '--trace-scene'], env=env, cwd=work,
                                           stdin=subprocess.DEVNULL, stdout=log, stderr=log)

                def glyphs():
                    return scene_glyphs(log_path.read_text(errors='replace'))

                def keys(*parts):
                    subprocess.run(['wtype', '-d', '40', *parts], env=env, check=True, timeout=20)
                    time.sleep(.15)

                def text_rows():
                    return [text for _, text, _ in rows(glyphs(), 'Text')]

                def mode_lines():
                    # The status bar is the mode line (VM2-STATUS-BAR), with
                    # the popup's own line folded into it.
                    return [text for _, text, _ in rows(glyphs(), 'StatusBar') + rows(glyphs(), 'ModeLine')]

                fio.wait(lambda: text_rows(), args.timeout, 'first frame', process)
                if args.prompts:
                    time.sleep(1)
                    check_prompts(args, output, report, env, process, state, keys, glyphs)
                    report['status'] = 'PASS'
                    return
                if args.toasts:
                    time.sleep(1)
                    check_toasts(args, output, report, env, process, state, keys, glyphs)
                    report['status'] = 'PASS'
                    return
                if args.indicators:
                    check_indicators(args, output, report, env, process, state, keys, glyphs)
                    report['status'] = 'PASS'
                    return
                if args.repeat:
                    time.sleep(1)
                    check_repeat(args, output, report, env, process, state, keys, glyphs)
                    report['status'] = 'PASS'
                    return
                if args.which_key:
                    time.sleep(1)
                    check_which_key(args, output, report, env, process, state, keys, glyphs)
                    report['status'] = 'PASS'
                    return
                keys('-M', 'alt', 'x', '-m', 'alt')
                fio.wait(lambda: ff.prompt_shown(ff.current_rows(log_path.read_text(errors='replace')), 'M-x '), args.timeout, 'M-x prompt', process)
                keys('hermes', '-k', 'Return')
                fio.wait(lambda: any('*Hermes Dashboard*' in row for row in mode_lines()), args.timeout, 'dashboard', process)
                time.sleep(1)
                fio.shot(output / 'dashboard.png', env, report)
                report['captures'].append({'name': 'dashboard', 'rows': text_rows(), 'mode_lines': mode_lines()})
                if args.child_frame or args.layer:
                    check_child_frame(args, output, report, env, process, state, keys, glyphs, log_path)
                    if args.layer:
                        check_layer_faces(args, output, report, env, process, faces_state, keys, glyphs)
                    report['status'] = 'PASS'
                    return
                # The dashboard opens its popup itself; the popup key shows
                # it again, refitting.
                fio.wait(state.exists, args.timeout, 'first popup fit', process)
                report['first_fit'] = state.read_text()
                state.unlink()
                keys('h')
                fio.wait(state.exists, args.timeout, 'popup fit', process)
                fio.wait(lambda: mode_lines() and any('Navigate' in row for row in text_rows()),
                         args.timeout, 'popup painted', process)
                time.sleep(1.5)
                fio.shot(output / 'popup.png', env, report)
                painted = glyphs()
                report['captures'].append({'name': 'popup', 'rows': text_rows(), 'mode_lines': mode_lines()})
                probe = read_plist(state.read_text())
                report['probe'] = probe
                report['probe_source'] = state.read_text()
                if probe['outcome'] != 'fitted':
                    raise AssertionError(f"fit-window-to-buffer: {probe['outcome']}")
                # The popup is the bottom window: its text is the rows between
                # its top and its body's bottom edge.  A GUI window has no
                # mode-line row (its line folds into the status bar), so the
                # edge comes from the window's geometry, not a painted band.
                text = rows(painted, 'Text')
                line = probe['frame-char-height']
                top = probe['window-pixel-top']
                # The text-size counts the window's mode line (none under
                # the status bar); the body excludes it and the divider.
                band = probe['mode-line-height']
                edge = top + probe['window-body-pixel-height']
                popup_rows = [r for r in text if top <= r[0] < edge]
                report['checks'].append({'popup_rows': [t for _, t, _ in popup_rows], 'edge': edge, 'top': top})
                # Each line's painted row is where posn-at-point puts it,
                # every nonblank line is painted, and nothing is cut.
                starts = probe['line-starts']
                if not isinstance(starts, list):
                    raise AssertionError(f'line starts: {starts}')
                painted_at = {y: text for y, text, _ in popup_rows}
                for line_text, y in starts:
                    if line_text.strip() and painted_at.get(top + y, '').rstrip() != line_text.rstrip():
                        raise AssertionError(f'line {line_text!r} at {top + y}: painted {painted_at.get(top + y)!r}')
                if len([t for t, _ in starts if t.strip()]) != len(popup_rows):
                    raise AssertionError(f'{len(popup_rows)} painted rows for lines {starts}')
                width, height = probe['text-size']
                text_bottom = top + height - band
                if text_bottom > edge:
                    raise AssertionError(f'text ends at {text_bottom}, below the body edge at {edge}')
                # fit-window-to-buffer rounds to whole lines (pixelwise nil),
                # fitting the text and what the window has besides its body
                # (its bottom divider under the pane chrome), as GNU's does.
                needed = height - band + probe['window-pixel-height'] - probe['window-body-pixel-height']
                if probe['window-pixel-height'] != -(-needed // line) * line:
                    raise AssertionError(f"window {probe['window-pixel-height']}px for {height}px of text and {needed - height}px besides")
                if edge - text_bottom >= line:
                    raise AssertionError(f'{edge - text_bottom}px of blank rows above the body edge')
                widest = max(x for _, _, row in popup_rows for x, _ in row)
                report['checks'].append({'text_size': [width, height], 'painted_last_glyph_x': widest})
                # Each posn's X, Y is where the painter drew its glyph: X and
                # Y count from the window's text area, which starts after
                # its left margin (keymap-popup's buffer has one) and
                # includes its header line (posn-at-x-y's doc).
                for name, window_top, text_left in (('posn-first', top, probe['text-left']),
                                                    ('posn-last', top, probe['text-left']),
                                                    ('dashboard-posn', probe['dashboard-top'], probe['dashboard-text-left'])):
                    posn = probe[name]
                    if not isinstance(posn, list):
                        raise AssertionError(f'{name}: {posn}')
                    _, (x, y), _, _ = posn
                    at = [g for g in painted if g['role'] == 'Text' and g['x'] == text_left + x and g['y'] == window_top + y]
                    report['checks'].append({name: posn, 'text_left': text_left, 'painted': [g['char'] for g in at]})
                    if name != 'dashboard-posn' and not at:
                        raise AssertionError(f'{name} {posn}: no glyph painted at ({text_left + x}, {window_top + y})')
                report['status'] = 'PASS'
    except Exception as error:  # noqa: BLE001
        report['error'] = repr(error)
    finally:
        live.stop(process)
        live.stop(keyboard)
        live.stop(compositor)
        (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    print(report['status'])
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
