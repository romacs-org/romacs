#!/usr/bin/env python3
"""Native 3D models with Gnosis, in a private headless labwc, HOME, XDG and
TMPDIR (server_live.py), on a synthetic scene and a private Gnosis database.
Real pointer and key input; nothing reads or writes a user's data.

  journey  the scene imports; the agent tools see the renderer and the
           scene; `gnosis_model_show' opens a viewer whose picture is
           drawn (screenshot); a real click is observed as a pick on the
           left cube, never an answer; a drag orbits and the old pick is no
           longer current; the wheel zooms; a Find card previews, saves,
           reads back, and its retried save returns the same thema; the
           closed viewer refuses and its surface is freed; native authoring
           makes a region on the right cube by click and `g', and saves a
           Find card on it with `F'; the saved left-cube card is reviewed
           by a click and RET, a correct answer recorded once; a text card
           beside them keeps its ordinary fields.
  lifetime a model whose handle is dropped is freed by its finalizer after
           a collection; a stale backend claim never detaches its
           successor; requests to a destroyed model are refused.

usage: model_live.py --romacs BIN --output DIR [--workflow NAME]... [--tools DIR]
"""
import argparse
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import time

import parity_live
from server_live import Server, VirtualPointer, until

INIT = ''';; -*- lexical-binding: t -*-
(setq inhibit-startup-screen t initial-scratch-message nil)
(setq gnosis-dir (expand-file-name "gnosis" (getenv "MODEL_LIVE_ROOT")))
(defun model-live-tool (name &rest pairs)
  "Call MCP tool NAME as an agent would: PAIRS are its JSON arguments."
  (require 'romacs-agent-gnosis)
  (let* ((tool (alist-get name romacs-mcp-tools nil nil #'equal))
         (arguments (apply #'romacs-mcp--object pairs)))
    (romacs-mcp--arguments (plist-get tool :schema) arguments)
    (condition-case err
        (json-parse-string (funcall (plist-get tool :handler) arguments)
                           :object-type 'plist :false-object :false :null-object nil)
      (error (list :error (error-message-string err))))))
(defmacro model-live-shown (&rest body)
  "Run BODY in the buffer the selected window shows."
  `(with-current-buffer (window-buffer (selected-window)) ,@body))
(defun model-live-glyph ()
  "The shown model glyph: frame pixel x, y and its square size."
  (model-live-shown
   (let* ((w (selected-window)) (edges (window-inside-pixel-edges w))
          (glyph (text-property-not-all (point-min) (point-max) 'romacs-model nil))
          (at (posn-x-y (posn-at-point glyph w))))
     (vector (+ (nth 0 edges) (car at)) (+ (nth 1 edges) (cdr at))
             (plist-get (gnosis-model-backend-snapshot) :size)))))
(defvar model-live-resource nil)
(defvar model-live-review nil)
(defun model-live-review ()
  "Review the thema `model-live-review' names, natively."
  (interactive)
  (gnosis-review-loop (list model-live-review)))
(defun model-live-author ()
  "Author targets and cards on `model-live-resource', natively."
  (interactive)
  (gnosis-model-author-scene model-live-resource '("anatomy")))
'''

BUTTON_LEFT = 272


def cube(x0):
    vertices = '\n'.join(f'v {x0 + x} {y} {z}' for x in (0, 1.5) for y in (-.75, .75) for z in (-.75, .75))
    return vertices + '\nf 1 2 4 3\nf 5 7 8 6\nf 1 5 6 2\nf 3 4 8 7\nf 1 3 7 5\nf 2 6 8 4\n'


def scene(directory):
    directory.mkdir()
    (directory / 'left.obj').write_text(cube(-2.0))
    (directory / 'right.obj').write_text(cube(.5))
    (directory / 'scene.json').write_text(json.dumps({
        'objects': [{'id': 'left', 'label': 'Left cube', 'path': 'left.obj'},
                    {'id': 'right', 'label': 'Right cube', 'path': 'right.obj'}],
        'initial_view': [0, 0, 1], 'license': 'CC0; synthetic test geometry', 'source': 'Romacs model_live fixture'}))
    return directory / 'scene.json'


class Pointer(VirtualPointer):
    """Buttons and the wheel as well as motion."""

    def button(self, pressed):
        self.send(5, 2, struct.pack('<III', int(time.time() * 1000) & 0xffffffff, BUTTON_LEFT, int(pressed)))
        self.send(5, 4)

    def click(self, x, y):
        self.move(x, y)
        time.sleep(.15)
        self.button(True)
        time.sleep(.08)
        self.button(False)

    def drag(self, x0, y0, x1, y1, steps=8):
        self.move(x0, y0)
        time.sleep(.15)
        self.button(True)
        for i in range(1, steps + 1):
            time.sleep(.03)
            self.move(int(x0 + (x1 - x0) * i / steps), int(y0 + (y1 - y0) * i / steps))
        time.sleep(.08)
        self.button(False)

    def wheel(self, notches):
        self.send(5, 5, struct.pack('<I', 0))  # axis_source: wheel
        self.send(5, 7, struct.pack('<IIii', int(time.time() * 1000) & 0xffffffff, 0, int(15 * 256 * notches), notches))
        self.send(5, 4)


class Session(Server):
    def __init__(self, args, base, name):
        super().__init__(args, base, name, init=INIT)
        self.env['MODEL_LIVE_ROOT'] = str(self.root)
        self.client('-n', str(self.file('a.txt')))
        self.pid = int(self.eval('(emacs-pid)'))
        self.keyboard_ready()
        self.eval("(progn (require 'gnosis) (require 'romacs-agent-gnosis) t)")
        self.pointer = Pointer(self.env)

    def value(self, form):
        """FORM's value read back as JSON (`json-serialize' of FORM)."""
        text = self.eval(f'(json-serialize {form} :false-object :false :null-object nil)')
        return json.loads(parity_live.read_sexp(text))

    def tool(self, name, **arguments):
        pairs = ' '.join(f'{json.dumps(key.replace("_", "-"))} {lisp_json(value)}' for key, value in arguments.items())
        return self.value(f'(model-live-tool {json.dumps(name)} {pairs})')

    def glyph(self):
        """The selected window's model glyph: frame pixel (x, y, size)."""
        return self.value('(model-live-glyph)')

    def shot(self, path):
        subprocess.run([f'{self.args.tools}/grim', str(path)], env=self.env, check=True, timeout=30)
        return str(path)

    def keys_text(self, text):
        self.keys(text)

    def key(self, name):
        self.keys('-k', name)


def lisp_json(value):
    if value is True:
        return 't'
    if value is False:
        return ':false'
    if isinstance(value, list):
        return '(vector ' + ' '.join(lisp_json(v) for v in value) + ')'
    return json.dumps(value)


def centres(x, y, size):
    """The two cubes' centres: the renderer draws the scene radius across
    0.4 * size, and the cubes' centres are 1.25 / 2.26 of that radius out."""
    offset = 1.25 / ((2 ** 2 + .75 ** 2 + .75 ** 2) ** .5) * .4 * size
    return (int(x + size / 2 - offset), int(y + size / 2)), (int(x + size / 2 + offset), int(y + size / 2))


def journey(args, base):
    s = Session(args, base, 'mj')
    checks, shots = {}, base / 'journey'
    shots.mkdir()
    try:
        source = scene(s.root / 'work' / 'scene')
        resource = s.value(f'(setq model-live-resource (gnosis-model-import {json.dumps(str(source))}))')
        text_id = s.value('(gnosis-add-thema-fields "basic" "Capital of Greece?" nil (list "Athens") "" (list "geo") 0 nil)')
        status = s.tool('gnosis_model_status')
        checks['renderer available'] = status.get('available') is True
        scenes = s.tool('gnosis_model_scenes')['scenes']
        checks['scene listed'] = [x['resource'] for x in scenes] == [resource]
        described = s.tool('gnosis_model_describe', resource=resource)
        checks['targets described'] = [t['id'] for t in described['targets']] == ['left', 'right']

        shown = s.tool('gnosis_model_show', resource=resource)
        viewer = shown['viewer']
        until(lambda: s.tool('gnosis_model_observe', viewer=viewer)['ready'] is True, 30, 'the viewer ready')
        x, y, size = s.glyph()
        left, right = centres(x, y, size)
        checks['picture drawn'] = s.shot(shots / '1-shown.png')

        s.pointer.click(*left)
        picked = until(lambda: ((s.tool('gnosis_model_observe', viewer=viewer).get('pick') or {}).get('mesh') and
                                s.tool('gnosis_model_observe', viewer=viewer)['pick']), 15, 'a pick on a mesh')
        checks['click observed on the left cube'] = (picked['mesh'] == 'left' and picked['kind'] == 'observation'
                                                    and [t['id'] for t in picked['targets']] == ['left']
                                                    and picked['current'] is True)
        s.shot(shots / '2-picked.png')

        s.pointer.drag(x + size // 2, y + size - 20, x + size // 2 + 60, y + size - 20)
        orbited = until(lambda: (lambda o: o if o['view'][0] != 0 and o['ready'] is True else None)(
            s.tool('gnosis_model_observe', viewer=viewer)), 15, 'an orbit')
        checks['orbit moves the camera, old pick stale'] = orbited['pick']['current'] is False
        s.pointer.wheel(-2)
        zoomed = until(lambda: (lambda o: o if o['view'][2] > 1.0 else None)(
            s.tool('gnosis_model_observe', viewer=viewer)), 15, 'a zoom')
        checks['wheel zooms'] = zoomed['view'][2] > 1.0
        s.shot(shots / '3-orbited.png')

        plan = s.tool('gnosis_model_card_preview', resource=resource, type='find', target='left',
                      view=zoomed['view'], tags=['anatomy'])
        checks['preview writes nothing'] = s.value('(length (gnosis-select (quote id) (quote themata) nil t))') == 1
        saved = s.tool('gnosis_model_card_save', plan_token=plan['plan-token'])
        again = s.tool('gnosis_model_card_save', plan_token=plan['plan-token'])
        find_id = saved['thema-id']
        checks['card saved and read back'] = (saved['item']['type'] == 'model' and saved['item']['answers'] == ['left']
                                              and saved['item']['question'] == 'Click the left cube.')
        checks['retry returns the first receipt'] = again['thema-id'] == find_id
        refused = s.tool('gnosis_model_card_preview', resource=resource, type='name', target='left', cues=['right'])
        checks['invalid card refused'] = 'error' in refused

        surface = s.value('(with-current-buffer (gethash %s gnosis-model-explore--viewers)'
                          ' (romacs-model--id (gnosis-model-backend-get :owner)))' % json.dumps(viewer))
        closed = s.tool('gnosis_model_close', viewer=viewer)
        stale = s.tool('gnosis_model_observe', viewer=viewer)
        checks['closed viewer refuses'] = closed['closed'] is True and 'closed or replaced' in stale.get('error', '')
        checks['closed viewer surface freed'] = s.value(f'(if (romacs-surface-destroy {surface}) "live" "freed")') == 'freed'

        # Native authoring: a region on the right cube, then a Find card on it.
        s.keys('-M', 'alt', '-k', 'x', '-m', 'alt')
        s.keys_text('model-live-author')
        s.key('Return')
        until(lambda: s.value('(model-live-shown (and gnosis-model--author-context (gnosis-model-backend-get :ready) t))') is True,
              30, 'the authoring view')
        x, y, size = s.glyph()
        left, right = centres(x, y, size)
        s.pointer.click(*right)
        until(lambda: s.value('(model-live-shown (and (plist-get gnosis-model--author-context :target) t))') is True, 15, 'a selection')
        s.key('g')
        time.sleep(.5)
        s.keys_text('Right front')
        s.key('Return')
        until(lambda: s.value('(model-live-shown (equal "region" (alist-get (quote kind) (gnosis-model-target'
                              ' (plist-get gnosis-model--author-context :scene)'
                              ' (plist-get gnosis-model--author-context :target)))))') is True, 15, 'the region')
        until(lambda: s.value('(model-live-shown (and (gnosis-model-backend-get :ready) t))') is True, 15, 'the region painted')
        s.shot(shots / '4-region.png')
        s.keys('F')
        time.sleep(.5)
        s.key('Return')
        until(lambda: s.value('(model-live-shown (car (plist-get gnosis-model--author-context :saved)))') == 1, 15, 'the region card')
        s.key('q')
        until(lambda: s.value('(model-live-shown (null gnosis-model--author-context))') is True, 15, 'authoring done')
        # The authored card is the model thema other than the agent's Find card.
        region = s.value('(let ((row (seq-find (lambda (row) (not (equal (car row) %s)))'
                         ' (gnosis-select (quote [id hypothesis answer]) (quote themata) (quote (= type "model"))))))'
                         ' (vector (car row) (car (nth 1 row)) (car (nth 2 row))))' % find_id)
        region_target = s.value('(alist-get (quote kind) (gnosis-model-target (gnosis-model--scene (expand-file-name %s'
                                ' (gnosis-assets-root))) %s))' % (json.dumps(region[1]), json.dumps(region[2])))
        checks['region card saved on a new scene revision'] = region[1] != resource and region_target == 'region'

        # Review the left-cube Find card: a click on the left cube and RET.
        s.eval(f'(setq model-live-review {find_id})')
        s.keys('-M', 'alt', '-k', 'x', '-m', 'alt')
        s.keys_text('model-live-review')
        s.key('Return')
        until(lambda: s.value('(model-live-shown (and gnosis-review--model-context (gnosis-model-backend-get :ready) t))')
              is True, 30, 'the review model')
        s.shot(shots / '5-review.png')
        x, y, size = s.value('(model-live-glyph)')
        view = s.value('(model-live-shown (vconcat (gnosis-model-backend-get :view)))')
        before = s.value(f'(length (gnosis-select (quote event-id) (quote review-events) (quote (= thema-id {find_id})) t))')
        # Exploratory clicks never submit: aim along the centre row, as a
        # learner would, until the selection is on the left cube.
        def aimed():
            for step in range(2, 23):
                s.pointer.click(int(x + size * step / 24), int(y + size / 2))
                time.sleep(.4)
                if s.value('(model-live-shown (plist-get (plist-get gnosis-review--model-context :selection) :mesh))') == 'left':
                    return True
            return False
        checks['exploratory clicks never submit'] = aimed() and s.value(
            f'(length (gnosis-select (quote event-id) (quote review-events) (quote (= thema-id {find_id})) t))') == before
        s.key('Return')
        # Feedback shows the answer; `n' accepts it, as a learner does.
        until(lambda: s.value('(model-live-shown (and gnosis-review--feedback t))') is True, 20, 'the feedback')
        s.shot(shots / '6-feedback.png')
        checks['nothing recorded before acceptance'] = s.value(
            f'(length (gnosis-select (quote event-id) (quote review-events) (quote (= thema-id {find_id})) t))') == before
        s.key('n')
        until(lambda: s.value(f'(length (gnosis-select (quote event-id) (quote review-events)'
                              f' (quote (= thema-id {find_id})) t))') == before + 1, 20, 'the review recorded')
        checks['review recorded once'] = True
        checks['review camera'] = view
        checks['text card unchanged'] = s.value(f'(let ((row (car (gnosis-select (quote [type keimenon answer])'
                                                f' (quote themata) (quote (= id {text_id}))))))'
                                                ' (vector (nth 0 row) (nth 1 row) (vconcat (nth 2 row))))') == ['basic', 'Capital of Greece?', ['Athens']]
        return {'checks': checks, 'ok': all(v is not False for v in checks.values()),
                'resource': resource, 'find': find_id, 'region': region}
    finally:
        (base / 'journey.log').write_text(s.log())
        s.close()


def lifetime(args, base):
    s = Session(args, base, 'ml')
    checks = {}
    try:
        source = scene(s.root / 'work' / 'scene')
        meshes = f'(list (list "left" {json.dumps(str(source.parent / "left.obj"))}))'
        surface = s.value(f'(let ((m (romacs-model-create :meshes {meshes} :width 64 :height 64)))'
                          ' (romacs-model--id m))')
        # Only the finalizer frees it: a request to the id is refused once
        # it ran, and nothing here destroys the surface itself.
        refused = lambda: s.value(f'(progn (garbage-collect) (condition-case e (progn (romacs-model--set-view {surface} 1 0 0 1) "live")'
                                  ' (error (error-message-string e))))') == 'Model is destroyed'
        checks['dropped handle freed by its finalizer'] = until(refused, 15, 'the finalizer')
        # A callback's prompt reads the keys typed ahead of its event, in order.
        # The keys and the model's `ready' queue while the session sleeps,
        # so the loop takes them in one batch: event first, then the keys.
        busy = s.waiting('-e', f'(progn (setq model-live-read nil model-live-typeahead'
                         f' (romacs-model-create :meshes {meshes} :width 64 :height 64'
                         ' :callback (lambda (_ type _) (when (eq type (quote ready))'
                         ' (setq model-live-read (read-string "Typed: "))))))'
                         ' (sleep-for 3) t)')
        time.sleep(1)
        s.keys_text('ab')
        s.key('Return')
        busy.wait(timeout=30)
        typed = until(lambda: s.value('model-live-read'), 15, 'the callback prompt')
        checks['callback prompt reads typed-ahead keys'] = typed == 'ab' and s.value(
            '(with-current-buffer (window-buffer (selected-window)) (buffer-string))') == 'one\ntwo\nthree three\nfour\n'

        checks['destroyed model refuses requests'] = s.value(
            f'(let ((m (romacs-model-create :meshes {meshes} :width 64 :height 64)))'
            ' (romacs-model-destroy m) (condition-case e (progn (romacs-model-set-view m 1 2 1) "sent")'
            ' (error (error-message-string e))))') == 'Model is destroyed'
        checks['stale claim keeps successor'] = s.value(
            f'(with-current-buffer (progn (require (quote gnosis-model-backend-romacs)) (get-buffer-create "*stale*"))'
            f' (gnosis-model-backend-romacs-open {json.dumps(str(source))} (list 0 0 1) 128 t nil)'
            ' (let ((old (gnosis-model-backend-get :owner)))'
            f'  (gnosis-model-backend-romacs-open {json.dumps(str(source))} (list 0 0 1) 128 t nil)'
            '   (let ((new (gnosis-model-backend-get :owner)))'
            '    (vector (if (gnosis-model-backend-detach old old) t :false) (eq new (gnosis-model-backend-get :owner))'
            '            (eq (car (romacs-model-status old)) (quote destroyed))))))') == [False, True, True]
        return {'checks': checks, 'ok': all(v is not False for v in checks.values())}
    finally:
        (base / 'lifetime.log').write_text(s.log())
        s.close()


WORKFLOWS = {'journey': journey, 'lifetime': lifetime}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--tools', default=parity_live.TOOLS)
    parser.add_argument('--workflow', action='append', choices=sorted(WORKFLOWS))
    args = parser.parse_args()
    out = Path(args.output)
    out.mkdir(parents=True, exist_ok=False)
    results = {}
    for name in args.workflow or list(WORKFLOWS):
        base = out / name
        base.mkdir()
        try:
            results[name] = WORKFLOWS[name](args, base)
        except Exception as error:  # a workflow's failure is its result
            results[name] = {'ok': False, 'error': repr(error)}
        print(name, json.dumps(results[name]), flush=True)
    (out / 'summary.json').write_text(json.dumps(results, indent=2))
    return 0 if all(r.get('ok') is True for r in results.values()) else 1


if __name__ == '__main__':
    sys.exit(main())
