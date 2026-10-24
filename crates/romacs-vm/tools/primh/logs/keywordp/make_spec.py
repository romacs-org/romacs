#!/usr/bin/env python3
"""Augment the from-manifest keywordp spec with domain values and probes.

Run from the repository root:  python3 crates/romacs-vm/tools/primh/logs/keywordp/make_spec.py
Each value/probe is Lisp SOURCE evaluated freshly per case on both sides.
GNU data.c:383 Fkeywordp: SYMBOLP, first name byte ':' and
SYMBOL_INTERNED_IN_INITIAL_OBARRAY_P. The values below separate each of the
three tests: interned vs `make-symbol' names starting with ':', a ':' only at
the end, the empty name, a multibyte name, and non-symbols whose printed form
starts with ':'.
"""
import json
from pathlib import Path

SPECS = Path(__file__).resolve().parents[2] / "specs"


def update(name, **fields):
    p = SPECS / f"{name}.json"
    spec = json.loads(p.read_text())
    spec.update(fields)
    p.write_text(json.dumps(spec) + "\n")
    print(f"{p}: {len(spec.get('probes', []))} probes")


update(
    "keywordp",
    args=[],
    extra={"0": [
        ":a", "':", "'a", "'a:", "(intern \":x\")", "(make-symbol \":x\")",
        "(make-symbol \":\")", "(intern \"\")", "(make-symbol \"\")",
        ":\u03bb", "(intern \":\u03bb\u00e9\")", "(make-symbol \":\u03bb\")",
        "(intern \"\u03bb:\")", "\":a\"", "(list :a)", "(vector :a)",
        "(symbol-function 'car)", "?:",
    ]},
    probes=[
        "(list (keywordp :a) (keywordp ':) (keywordp 'a) (keywordp nil) (keywordp t) (keywordp 42) (keywordp \"str\") (keywordp (list :a)))",
        "(let ((s (make-symbol \":x\"))) (list (keywordp s) (eq s :x) (keywordp (intern \":x\")) (eq (intern \":x\") :x)))",
        "(let ((s (make-symbol \":fresh-kw\"))) (list (keywordp s) (keywordp (intern (symbol-name s)))))",
        "(keywordp (car '(:k 1)))",
        "(keywordp (intern (concat \":\" \"made\")))",
        # Through funcall of the symbol (VM2 has no `mapcar' yet).
        "(let ((xs (list :a 'b :c \":d\" (make-symbol \":e\") nil)) r) (while xs (setq r (cons (funcall #'keywordp (car xs)) r) xs (cdr xs))) (nreverse r))",
        "(keywordp (intern \":\u03bb\"))",
        "(eq (keywordp :\u03bb) t)",
    ],
)
