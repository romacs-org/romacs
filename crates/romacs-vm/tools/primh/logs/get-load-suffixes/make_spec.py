#!/usr/bin/env python3
"""Augment the from-manifest get-load-suffixes spec with probes.

get-load-suffixes takes no arguments, so its behaviour lives in the two
variables it reads (lread.c:961). Every probe let-binds them (both are
special built-ins, so the binding is dynamic) except the bare call, which
reads the session's own values: GNU's are the dumped image's, where
jka-cmpr-hook.el has added ".gz" to load-file-rep-suffixes.
"""
import json, sys
from pathlib import Path

def bind(suffixes, reps, body="(get-load-suffixes)"):
    return f"(let ((load-suffixes {suffixes}) (load-file-rep-suffixes {reps})) {body})"

PROBES = [
    # The C initial values (lread.c:5586-5629, GNU built with modules).
    bind("(list \".so\" \".elc\" \".el\")", "(list \"\")"),
    bind("(list \".elc\" \".el\")", "(list \"\")"),
    bind("(list \".elc\" \".el\")", "(list \"\" \".gz\")"),
    bind("nil", "(list \"\")"),
    bind("(list \".elc\")", "nil"),
    bind("nil", "nil"),
    # Order: suffix-major, extension-minor.
    bind("(list \"a\" \"b\")", "(list \"1\" \"2\" \"3\")"),
    # The module rule: a module suffix with a non-empty extension that is a
    # member of jka-compr-load-suffixes is skipped.
    # jka-compr-load-suffixes is a Lisp defcustom (jka-cmpr-hook.el), special
    # in GNU's dumped image and unknown to a bare VM2, so a `let' would bind
    # it lexically in VM2 only: set its global value instead (one process
    # per case, so the store cannot leak).
    "(progn (set 'jka-compr-load-suffixes (list \".gz\")) " + bind("(list \".so\" \".elc\")", "(list \"\" \".gz\" \".bz2\")") + ")",
    "(progn (set 'jka-compr-load-suffixes nil) " + bind("(list \".so\" \".elc\")", "(list \"\" \".gz\")") + ")",
    "(progn (set 'jka-compr-load-suffixes (list \".gz\")) " + bind("(list \"x.so\" \".so.1\")", "(list \".gz\")") + ")",
    # A void jka-compr-load-suffixes signals once consulted (Fsymbol_value).
    "(progn (makunbound 'jka-compr-load-suffixes) " + bind("(list \".so\")", "(list \"\" \".gz\")") + ")",
    "(progn (makunbound 'jka-compr-load-suffixes) " + bind("(list \".elc\")", "(list \"\" \".gz\")") + ")",
    # Empty extension never consults jka-compr-load-suffixes.
    bind("(list \".so\")", "(list \"\")"),
    # Improper tails end the walk (FOR_EACH_TAIL).
    bind("(cons \".a\" \".b\")", "(list \"\" \".gz\")"),
    bind("(list \".a\")", "(cons \"\" \".z\")"),
    bind("\".elc\"", "(list \"\")"),
    # Non-string elements reach concat.
    bind("(list \".a\" 1)", "(list \"\" \".gz\")"),
    bind("(list \".a\" 'sym)", "(list \"\")"),
    bind("(list (list ?a ?b))", "(list \"\")"),
    bind("(list \".a\")", "(list (vector ?x))"),
    # NOT recorded: (list 7) as load-file-rep-suffixes. GNU 32.0.50 reads
    # SCHARS (7) unchecked under HAVE_MODULES and dies with SIGSEGV in both
    # regimes (gnu-crash.log); VM2 signals (wrong-type-argument sequencep 7)
    # from concat.
    # Multibyte text survives the join.
    bind("(list \".λ\")", "(list \"\" \".ж\")"),
    bind("(list \".a\")", "(list \"\")", "(let ((r (get-load-suffixes))) (list (multibyte-string-p (car r)) (length r)))"),
    # Fresh strings each call, not shared with the variables.
    bind("(list \".a\")", "(list \"\")", "(let ((x (get-load-suffixes))) (list (eq (car x) (car load-suffixes)) (equal (car x) (car load-suffixes)) (eq x (get-load-suffixes))))"),
    # Circular lists.
    bind("(let ((x (list \".a\"))) (setcdr x x) x)", "(list \"\")"),
    bind("(list \".a\")", "(let ((x (list \"\"))) (setcdr x x) x)"),
    # setq inside a let, then the global restored.
    "(progn (let ((load-suffixes (list \".q\"))) (setq load-file-rep-suffixes (list \"\"))) (car (get-load-suffixes)))",
    # The variables are special and boundp built-ins.
    "(list (special-variable-p 'load-suffixes) (special-variable-p 'load-file-rep-suffixes) (boundp 'load-suffixes))",
    # Call through funcall/apply and with GC pressure.
    bind("(list \".x\")", "(list \"\" \".y\")", "(list (funcall #'get-load-suffixes) (apply #'get-load-suffixes nil))"),
    bind("(list (copy-sequence \".x\"))", "(list \"\" (copy-sequence \".y\"))",
         "(let ((r (get-load-suffixes))) (let ((i 0)) (while (< i 5000) (list i i) (setq i (1+ i)))) r)"),
    # Wrong number of arguments.
    "(condition-case e (funcall #'get-load-suffixes 1) (error e))",
]
p = Path(sys.argv[1])
spec = json.loads(p.read_text())
spec["probes"] = PROBES
p.write_text(json.dumps(spec) + "\n")
print(f"{p}: {len(PROBES)} probes")
