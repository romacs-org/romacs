#!/usr/bin/env python3
"""Write the primh specs for read-from-minibuffer and read-string (lane vm2/minibuf).

The specs only DECLARE argument domains and probes; every answer comes from
live GNU through `primh.py record`.  Input is GNU's own batch regime for
prompts, `ert-simulate-keys` (ert-x.el:80): events in `unread-command-events'
and `executing-kbd-macro' non-nil, so read_minibuf (minibuf.c:653) runs the
real minibuffer instead of reading stdin.  A prompt-position value queues the
keys, then evaluates to the prompt; arguments are evaluated left to right
before the call, so the keys are queued before the primitive runs.

Domains are `extra' values only: the tag `none' matches no catalog value.
Usage: python3 minibuf_specs.py  (rewrites minibuf-specs/*.json)
"""
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
OUT = HERE / "minibuf-specs"


def keys(ks, prompt='"P: "'):
    # `append' of a string gives its characters; copy the prompt so an alias
    # test compares against a fresh object, never a literal shared with code.
    return f"(progn (setq unread-command-events (append {json.dumps(ks)} nil) executing-kbd-macro t) (copy-sequence {prompt}))"


PROMPTS = [keys("abc\r"), keys("\r"), keys("(a b)\r"), keys("1 2\r"), keys("ab"),
           keys("ab\x7fc\r"), keys("x\n"), keys("q\r", '""'), "nil", "'sym"]

# Declared GNU anchors (re-read before any edit): minibuf.c:1309 Fread_from_minibuffer,
# minibuf.c:1420 Fread_string.  Rust spans are those of src/prims/minibuf.rs.
SPY = ("(progn (defvar romacs-spy-v nil) "
       "(defun romacs-spy () (interactive) (setq romacs-spy-v (list (buffer-substring-no-properties (point-min) (point-max)) (point) "
       "(eq (selected-window) (minibuffer-window)) (eq (current-buffer) (window-buffer (minibuffer-window))) "
       "(get-text-property 1 'field) (get-text-property 1 'read-only) (get-text-property 1 'front-sticky) "
       "(get-text-property 1 'rear-nonsticky) (get-text-property 1 'face)))) "
       "(defvar romacs-spy-map (let ((m (make-sparse-keymap))) (set-keymap-parent m minibuffer-local-map) (define-key m [f5] 'romacs-spy) m)) ")

def sk(events, body):
    return f"(let ((unread-command-events (append {events} nil)) (executing-kbd-macro t)) {body})"

QUIT = "(condition-case q {} (quit (list 'quit q)))"

RFM_PROBES = [
    # C-g: abort-minibuffers -> minibuffer-quit-recursive-edit -> signal minibuffer-quit (a quit, not an error)
    QUIT.format(sk('"ab\\C-g"', '(read-from-minibuffer "P: ")')),
    QUIT.format(sk('"\\C-g"', '(read-from-minibuffer "P: " "init" nil t nil "(d)")')),
    # after a C-g the next prompt works and the caller's buffer and window are back
    sk('"a\\C-gb\\r"', "(let ((b (current-buffer)) (w (selected-window))) (list (condition-case q (read-from-minibuffer \"P: \") (quit q)) (eq b (current-buffer)) (eq w (selected-window)) (read-from-minibuffer \"Q: \") (eq b (current-buffer)) (eq w (selected-window))))"),
    # the prompt really is in the minibuffer window, selected, with GNU's properties
    SPY + sk('[?a f5 ?\\r]', '(list (read-from-minibuffer "P: " "in" romacs-spy-map) romacs-spy-v)') + ")",
    SPY + sk('[?a f5 ?\\r]', "(let ((minibuffer-prompt-properties '(read-only t face minibuffer-prompt))) (list (read-from-minibuffer \"P: \" nil romacs-spy-map) romacs-spy-v))") + ")",
    SPY + sk('[?a f5 ?\\r]', "(let ((minibuffer-prompt-properties '(read-only t face minibuffer-prompt)) (p (copy-sequence \"P: \"))) (put-text-property 0 2 'face 'bold p) (list (read-from-minibuffer p nil romacs-spy-map) romacs-spy-v (get-text-property 0 'face p) (get-text-property 2 'face p)))") + ")",
    SPY + sk('[?a f5 ?\\r]', '(list (read-from-minibuffer "" nil romacs-spy-map) romacs-spy-v)') + ")",
    SPY + sk('[?a f5 ?\\r]', "(list (read-from-minibuffer \"P: \" '(\"in\" . 2) romacs-spy-map) romacs-spy-v)") + ")",
    SPY + sk('[f5 ?\\r]', "(list (read-from-minibuffer \"P: \" '(\"init\" . 1) romacs-spy-map) romacs-spy-v)") + ")",
    # after the prompt: caller's buffer/window restored, minibuffer emptied
    sk('"abc\\r"', '(let ((b (current-buffer)) (w (selected-window))) (list (read-from-minibuffer "P: ") (eq b (current-buffer)) (eq w (selected-window)) (with-current-buffer (window-buffer (minibuffer-window)) (buffer-size))))'),
    # text properties: stripped unless minibuffer-allow-text-properties
    sk('"a\\r"', "(let ((i (copy-sequence \"x\"))) (put-text-property 0 1 'q 1 i) (let ((r (read-from-minibuffer \"P: \" i))) (list r (get-text-property 0 'q r))))"),
    sk('"a\\r"', "(let ((i (copy-sequence \"x\")) (minibuffer-allow-text-properties t)) (put-text-property 0 1 'q 1 i) (let ((r (read-from-minibuffer \"P: \" i))) (list r (get-text-property 0 'q r) (get-text-property 1 'q r))))"),
    # the prompt string is never the result, never mutated
    sk('"\\r"', '(let ((p (copy-sequence "P: "))) (list (read-from-minibuffer p) p))'),
    # INITIAL-CONTENTS shapes the dom family does not cover
    sk('"Z\\r"', "(read-from-minibuffer \"P: \" '(5 . 1))"),
    sk('"Z\\r"', "(read-from-minibuffer \"P: \" '(\"init\" . 99))"),
    sk('"Z\\r"', "(read-from-minibuffer \"P: \" '(\"init\" . -4))"),
    sk('"Z\\r"', "(read-from-minibuffer \"P: \" '(\"init\" . 5))"),
    sk('"Z\\r"', "(read-from-minibuffer \"P: \" '(\"\" . 1))"),
    sk('"Z\\r"', "(read-from-minibuffer \"P: \" (cons \"init\" 1.5))"),
    # check order: keymap is checked in Fread_from_minibuffer, INITIAL in read_minibuf
    sk('"Z\\r"', "(read-from-minibuffer \"P: \" 5 'sym)"),
    sk('"Z\\r"', "(read-from-minibuffer 5 5 'sym)"),
    sk('"Z\\r"', "(read-from-minibuffer \"P: \" nil '(keymap (90 . exit-minibuffer)))"),
    # READ: default forms and trailing garbage
    sk('"\\r"', "(read-from-minibuffer \"P: \" nil nil t nil 7)"),
    sk('"\\r"', "(read-from-minibuffer \"P: \" nil nil t nil '(7))"),
    sk('"\\r"', "(read-from-minibuffer \"P: \" nil nil t nil \"\")"),
    sk('"\\r"', "(read-from-minibuffer \"P: \" nil nil t nil \"x y\")"),
    sk('"\\r"', "(read-from-minibuffer \"P: \" nil nil t nil \"x \\t\\n\")"),
    sk('"x\\r"', "(read-from-minibuffer \"P: \" nil nil t nil \"(d)\")"),
    sk('"\\"s\\"\\r"', "(read-from-minibuffer \"P: \" nil nil t)"),
    sk('"?a\\r"', "(read-from-minibuffer \"P: \" nil nil 'x)"),
    sk('"\\r"', "(read-from-minibuffer \"P: \" \"(i)\" nil t)"),
    # HIST: an unbound history variable is set to nil before reading (minibuf.c:794)
    sk('"\\r"', "(progn (read-from-minibuffer \"P: \" nil nil nil 'romacs-unbound-hist) (boundp 'romacs-unbound-hist))"),
    sk('"a\\r"', "(read-from-minibuffer \"P: \" nil nil nil '(romacs-h . 2))"),
    sk('"a\\r"', "(read-from-minibuffer \"P: \" nil nil nil t)"),
    # history recording: GNU calls subr.el add-to-history (minibuf.c:1095)
    sk('"a\\r"', "(progn (setq minibuffer-history nil) (read-from-minibuffer \"P: \") minibuffer-history)"),
    # recursion refused unless enable-recursive-minibuffers (minibuf.c:641)
    "(progn (defun romacs-nest () (interactive) (setq romacs-nest-v (condition-case e (read-from-minibuffer \"Q: \") (error e)))) (defvar romacs-nest-v nil) "
    + sk('[?a f6 ?\\r]', "(let ((m (make-sparse-keymap))) (set-keymap-parent m minibuffer-local-map) (define-key m [f6] 'romacs-nest) (list (read-from-minibuffer \"P: \" nil m) romacs-nest-v))") + ")",
    # inhibit-interaction is checked first (minibuf.c:1373)
    sk('"a\\r"', "(let ((inhibit-interaction t)) (read-from-minibuffer 5))"),
    # outside a keyboard macro, batch GNU reads stdin (minibuf.c:659); primh's stdin is empty
    "(let ((executing-kbd-macro nil) (unread-command-events (list ?a ?\\r))) (read-from-minibuffer \"P: \"))",
    # end of the keyboard macro ends the prompt with what was typed
    sk('""', '(read-from-minibuffer "P: " "init")'),
    # unread-command-events consumed exactly to the RET
    sk('"a\\rb\\r"', '(list (read-from-minibuffer "P: ") unread-command-events)'),
    # a collection during the prompt keeps every argument and the result's bytes
    "(progn (defun romacs-churn () (interactive) (let ((i 0)) (while (< i 5000) (list i (make-string 3 ?z)) (setq i (1+ i)))) (garbage-collect)) "
    + sk('[?a f7 ?b ?\\r]', "(let ((m (make-sparse-keymap)) (p (copy-sequence \"Prompt: \")) (i (copy-sequence \"in\")) (d (copy-sequence \"(dflt)\"))) (set-keymap-parent m minibuffer-local-map) (define-key m [f7] 'romacs-churn) (list (read-from-minibuffer p i m nil nil d) p i d (read-from-minibuffer p nil m t nil d)))") + ")",
]

RS_PROBES = [
    QUIT.format(sk('"ab\\C-g"', '(read-string "P: ")')),
    QUIT.format(sk('"\\C-g"', '(read-string "P: " nil nil "dflt")')),
    sk('"\\r"', '(let ((d (copy-sequence "dd"))) (eq d (read-string "P: " nil nil d)))'),
    sk('"\\r"', '(let ((d (list (copy-sequence "d1") "d2"))) (eq (car d) (read-string "P: " nil nil d)))'),
    sk('"\\r"', "(read-string \"P: \" nil nil '(nil \"x\"))"),
    sk('"\\r"', "(read-string \"P: \" nil nil 'sym)"),
    sk('"\\r"', "(read-string \"P: \" \"\" nil \"dflt\")"),
    sk('"a\\r"', "(read-string \"P: \" nil t \"dflt\")"),
    sk('"a\\rb\\r"', '(list (read-string "1: ") (read-string "2: "))'),
    sk('"a\\r"', '(list (read-string "1: ") (read-string "2: "))'),
    sk('"a\\r"', '(let ((inhibit-interaction t)) (read-string 5))'),
    "(let ((executing-kbd-macro nil) (unread-command-events (list ?a ?\\r))) (read-string \"P: \"))",
    "(progn (defun romacs-churn () (interactive) (let ((i 0)) (while (< i 5000) (list i (make-string 3 ?z)) (setq i (1+ i)))) (garbage-collect)) "
    + sk('[f7 ?\\r ?x f7 ?\\r]', "(let ((m (make-sparse-keymap)) (d (copy-sequence \"dflt\"))) (set-keymap-parent m minibuffer-local-map) (define-key m [f7] 'romacs-churn) (let ((minibuffer-local-map m)) (list (eq d (read-string \"P: \" nil nil d)) d (read-string \"Q: \" nil nil d))))") + ")",
]


def spec(name, arity, gnu_def, rust, extra, probes):
    return dict(name=name, arity=arity, effect="global", gnu_files=["minibuf.c"], gnu_def=gnu_def,
                rust=rust, args=["none"] * len(extra), extra={str(i): v for i, v in enumerate(extra)},
                probes=probes)


def main():
    import sys
    rfm_line, rs_line = sys.argv[1], sys.argv[2]
    OUT.mkdir(exist_ok=True)
    rfm = spec("read-from-minibuffer", [1, 7], "minibuf.c:1309", f"src/prims/minibuf.rs:{rfm_line}", [
        PROMPTS,
        ["nil", '(copy-sequence "init")', '(cons (copy-sequence "init") 1)', '(cons (copy-sequence "init") 3)',
         '(cons (copy-sequence "init") 0)', '(list (copy-sequence "init"))', "5", "(cons (copy-sequence \"init\") 'x)"],
        ["nil", "'sym"],
        ["nil", "t"],
        ["nil"],
        ["nil", '(copy-sequence "(d)")', '(list (copy-sequence "(d)") "e")', "7"],
        ["nil"],
    ], RFM_PROBES)
    rs = spec("read-string", [1, 5], "minibuf.c:1420", f"src/prims/minibuf.rs:{rs_line}", [
        PROMPTS,
        ["nil", '(copy-sequence "init")', '(cons (copy-sequence "init") 1)', "5"],
        ["nil"],
        ["nil", '(copy-sequence "dflt")', '(list (copy-sequence "d1") "d2")', "5", "(list 5 6)", '(copy-sequence "")'],
        ["nil", "t"],
    ], RS_PROBES)
    for s in (rfm, rs):
        (OUT / f"{s['name']}.json").write_text(json.dumps(s, indent=1, ensure_ascii=False) + "\n")
        print(s["name"], "written")


if __name__ == "__main__":
    main()
