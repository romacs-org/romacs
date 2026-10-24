#!/usr/bin/env python3
"""Write cases.json: the hand-designed varh case list.

Every template below is written by hand; this script only instantiates each
template over the variables it is meaningful for, in a fixed order, so ids
are stable.  It never writes an expected value: outcomes come from GNU
(varh.py record).

  gen_cases.py            # rewrite cases.json
  gen_cases.py --check    # exit 1 if cases.json is not what this script writes

Variables (GNU's forwarded kinds, as VM2 defines them, plus controls):
  object (DEFVAR_LISP): help-char system-type standard-display-table
      scroll-preserve-screen-position emacs-version minibuffer-local-map
      window-persistent-parameters; added later (see LOAD below):
      after-load-alist current-load-list
  bool (DEFVAR_BOOL): enable-recursive-minibuffers
  int (DEFVAR_INT): undo-limit
  per-buffer (DEFVAR_PER_BUFFER, integerp predicate): fill-column
  control (ordinary Lisp symbols, set up by SETUP): varh-v (defvar'd),
      varh-l (defvar'd + make-variable-buffer-local)

GNU's -Q session is a dumped image: Lisp ran at dump time, so the initial
values of emacs-version, minibuffer-local-map and window-persistent-parameters
differ from a bare VM2.  Those are never printed -- SHOW() replaces them
with a predicate (stringp / keymapp / consp).  Values the case itself stored
are printed as they are.
"""
import json, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent

# name -> (kind, predicate used instead of printing the dumped initial value)
VARS = {
    "help-char": ("object", None),
    "system-type": ("object", None),
    "standard-display-table": ("object", None),
    "scroll-preserve-screen-position": ("object", None),
    "emacs-version": ("object", "stringp"),
    "minibuffer-local-map": ("object", "keymapp"),
    "window-persistent-parameters": ("object", "consp"),
    "enable-recursive-minibuffers": ("bool", None),
    "undo-limit": ("int", None),
    "fill-column": ("per-buffer", None),
    "varh-v": ("control", None),
    "varh-l": ("control-local", None),
}
ALL = list(VARS)
# CORE: one printable and one opaque object variable, and the bool, int and
# per-buffer built-ins, plus both controls.  REP (the deeper templates): the
# int and per-buffer built-ins -- the kinds with checks and locality -- and
# both controls; CORE templates already cover every kind in every family.
CORE = ["help-char", "minibuffer-local-map", "enable-recursive-minibuffers", "undo-limit", "fill-column", "varh-v", "varh-l"]
REP = ["undo-limit", "fill-column", "varh-v", "varh-l"]

SETUP = {
    "varh-v": "(defvar varh-v 1)",
    "varh-l": "(progn (defvar varh-l 1) (make-variable-buffer-local 'varh-l))",
}


def show(v, expr):
    pred = VARS[v][1]
    return f"({pred} {expr})" if pred else expr


def setup_for(v, dyn=False):
    parts = [SETUP[v]] if v in SETUP else []
    if dyn:
        parts.append(f"(defun varh-read () {v})")
        parts.append(f"(defun varh-write (x) (setq {v} x))")
    if not parts:
        return None
    return parts[0] if len(parts) == 1 else "(progn " + " ".join(parts) + ")"


# (family, template, variables, needs-dynamic-setup).  {v} = variable,
# {S:EXPR} is expanded by show(); see expand().
T = []


def fam(family, vars_, *templates, dyn=False):
    for t in templates:
        T.append((family, t, vars_, dyn))


fam("boundp", ALL,
    "(boundp '{v})")
fam("boundp", CORE,
    "(let (({v} {v})) (boundp '{v}))",
    "(with-temp-buffer (boundp '{v}))")
fam("symbol-value", ALL,
    "{S:(symbol-value '{v})}")
fam("symbol-value", CORE,
    "(eq (symbol-value '{v}) {v})",
    "(with-temp-buffer {S:(symbol-value '{v})})")
fam("set", CORE,
    "(list (set '{v} 42) (symbol-value '{v}) {v})",
    "(let ((old {v})) (list (set '{v} nil) {v} (progn (set '{v} old) (eq {v} old))))")
fam("set", REP,
    "(progn (set '{v} 42) (set '{v} 43) (list {v} (default-value '{v})))",
    "(list (set (intern \"{v}\") -7) {v})")
fam("setq", CORE,
    "(list (setq {v} 7) {v})")
fam("setq", REP,
    "(list (setq {v} 1 {v} 2) {v})",
    "(let ((old {v})) (setq {v} 3) (setq {v} old) (eq {v} old))")
fam("let", CORE,
    "(let ((old {v})) (list (let (({v} 11)) (list {v} (symbol-value '{v}))) (eq {v} old)))",
    "(let ((old {v})) (list (let* (({v} 12) (y {v})) y) (eq {v} old)))")
fam("let", REP,
    "(list (let (({v} 13)) (default-value '{v})) {v})",
    "(list (let (({v} 14)) (setq {v} 15) {v}) {v})",
    "(list (let (({v} nil)) {v}) {v})",
    "(list (let (({v})) {v}) {v})",
    "(list (let (({v} 16)) (let (({v} 17)) {v})) {v})",
    "(list (let (({v} 18)) (set '{v} 19) (symbol-value '{v})) (symbol-value '{v}))")
fam("let-signal", CORE,
    "(let ((old {v})) (list (condition-case nil (let (({v} 21)) (error \"boom\")) (error 'caught)) (eq {v} old)))",
    "(let ((old {v})) (list (catch 'k (let (({v} 22)) (throw 'k {v}))) (eq {v} old)))")
fam("let-signal", REP,
    "(let ((old {v}) seen) (list (catch 'k (unwind-protect (let (({v} 23)) (throw 'k {v})) (setq seen {v}))) (eq seen old) (eq {v} old)))",
    "(let (seen) (list (catch 'k (let (({v} 24)) (unwind-protect (throw 'k 0) (setq seen {v})))) seen {v}))",
    "(list (condition-case nil (let (({v} 25)) (setq {v} 26) (signal 'arith-error nil)) (arith-error {v})) {v})",
    "(list (condition-case nil (let (({v} 27)) (let (({v} 28)) (error \"x\"))) (error {v})) {v})",
    "(list (catch 'k (let (({v} 29)) (catch 'j (let (({v} 30)) (throw 'j nil))) (throw 'k {v}))) {v})")
fam("dynamic", CORE,
    "(let ((old {v})) (list (let (({v} 31)) (varh-read)) (eq (varh-read) old)))",
    dyn=True)
fam("dynamic", REP,
    "(list (let (({v} 32)) (let (({v} 33)) (varh-read))) (varh-read))",
    "(let (({v} 34)) (funcall #'varh-read))",
    "(let (({v} 35)) (funcall (lambda () {v})))",
    "(list (let (({v} 36)) (varh-write 37) {v}) {v})",
    "(list (let (({v} 38)) (varh-write 39) (varh-read)) (varh-read))",
    "(progn (varh-write 40) (list {v} (varh-read)))",
    "(list (let (({v} 41)) (apply #'varh-read nil)) (apply #'varh-read nil))",
    dyn=True)
fam("makunbound", ALL,
    "(let ((old {v})) (list (condition-case e (makunbound '{v}) (error (list 'err e))) (boundp '{v}) (and (boundp '{v}) (eq {v} old))))")
fam("makunbound", REP,
    "(list (let (({v} 3)) (condition-case e (makunbound '{v}) (error (list 'err e))) (boundp '{v})) (boundp '{v}))",
    "(progn (condition-case nil (makunbound '{v}) (error nil)) (condition-case e {v} (error (list 'err e))))",
    "(progn (condition-case nil (makunbound '{v}) (error nil)) (list (default-boundp '{v}) (set '{v} 5) {v}))")
fam("default-value", ALL,
    "(eq (default-value '{v}) {v})")
fam("default-value", CORE,
    "{S:(default-value '{v})}")
fam("default-value", REP,
    "(list (let (({v} 51)) (default-value '{v})) (default-value '{v}))",
    "(with-temp-buffer (set '{v} 52) (list {v} (default-value '{v})))",
    "(list (with-temp-buffer (set '{v} 53) (default-value '{v})) (default-value '{v}) {v})")
fam("set-default", CORE,
    "(let ((old {v})) (list (set-default '{v} 54) {v} (default-value '{v}) (progn (set-default '{v} old) (eq {v} old))))")
fam("set-default", REP,
    "(list (let (({v} 55)) (set-default '{v} 56) (list {v} (default-value '{v}))) {v} (default-value '{v}))",
    "(with-temp-buffer (set-default '{v} 57) (list {v} (default-value '{v})))",
    "(list (with-temp-buffer (set '{v} 58) (set-default '{v} 59) (list {v} (default-value '{v}))) {v})")
fam("default-boundp", ALL,
    "(default-boundp '{v})")
fam("default-boundp", CORE,
    "(let (({v} 1)) (default-boundp '{v}))")
fam("default-boundp", REP,
    "(with-temp-buffer (set '{v} 2) (default-boundp '{v}))")
fam("special-variable-p", ALL,
    "(special-variable-p '{v})")
fam("special-variable-p", REP,
    "(let (({v} 1)) (special-variable-p '{v}))")
fam("defvar", ALL,
    "(let ((old {v})) (list (defvar {v} 99) (eq {v} old)))")
fam("defvar", CORE,
    "(let ((old {v})) (defvar {v} 'other) (eq (default-value '{v}) old))")
fam("defvar", REP,
    "(progn (setq {v} 5) (defvar {v} 99) {v})",
    "(list (let (({v} 7)) (defvar {v} 99) {v}) {v})",
    "(progn (defvar {v}) (list (special-variable-p '{v}) (let (({v} 8)) (funcall (lambda () {v})))))",
    "(with-temp-buffer (set '{v} 9) (defvar {v} 99) (list {v} (default-value '{v})))")

# per-buffer: the subject is buffer-locality.  Applied to the per-buffer
# built-in, both controls, and one global representative of every other kind.
PB = ["help-char", "enable-recursive-minibuffers", "undo-limit", "fill-column", "varh-v", "varh-l"]
fam("per-buffer", PB,
    "(with-temp-buffer (setq {v} 61) (list {v} (default-value '{v})))",
    "(progn (with-temp-buffer (setq {v} 62)) (list {v} (default-value '{v})))",
    "(let ((b (get-buffer-create \"varh-b\"))) (with-current-buffer b (setq {v} 63)) (list {v} (with-current-buffer b {v}) (default-value '{v})))",
    "(let ((b (get-buffer-create \"varh-b\"))) (setq {v} 64) (list {v} (with-current-buffer b {v}) (with-temp-buffer {v})))",
    "(let ((b (get-buffer-create \"varh-b\"))) (list (let (({v} 65)) (list {v} (with-current-buffer b {v}) (default-value '{v}))) {v} (with-current-buffer b {v})))",
    "(let ((b (get-buffer-create \"varh-b\"))) (with-temp-buffer (setq {v} 66) (list (let (({v} 67)) (list {v} (default-value '{v}) (with-current-buffer b {v}))) {v} (default-value '{v}))))",
    "(let ((a (get-buffer-create \"varh-a\")) (c (get-buffer-create \"varh-c\"))) (with-current-buffer a (setq {v} 68)) (set-default '{v} 69) (list (with-current-buffer a {v}) (with-current-buffer c {v}) {v} (default-value '{v})))",
    "(with-temp-buffer (list (make-local-variable '{v}) {v} (progn (setq {v} 70) {v}) (default-value '{v})))",
    "(let ((b (get-buffer-create \"varh-b\"))) (with-current-buffer b (make-local-variable '{v}) (set-default '{v} 71)) (list (with-current-buffer b {v}) {v}))",
    "(let ((orig (current-buffer)) (b (get-buffer-create \"varh-b\"))) (set-buffer b) (setq {v} 72) (set-buffer orig) (list {v} (with-current-buffer b {v})))",
    "(let ((b (get-buffer-create \"varh-b\"))) (with-temp-buffer (setq {v} 73) (let (({v} 74)) (with-current-buffer b {v}))))",
    "(let ((b (get-buffer-create \"varh-b\")) (orig (current-buffer))) (with-current-buffer b (setq {v} 75)) (with-current-buffer b (let (({v} 76)) (set-buffer orig) {v})) (list {v} (with-current-buffer b {v})))",
    "(let ((b (get-buffer-create \"varh-b\"))) (let (({v} 77)) (with-current-buffer b (setq {v} 78))) (list {v} (with-current-buffer b {v}) (default-value '{v})))",
    "(let ((b (get-buffer-create \"varh-b\"))) (with-current-buffer b (let (({v} 79)) (set-default '{v} 80) (list {v} (default-value '{v})))))",
    "(with-temp-buffer (setq {v} 81) (list (local-variable-if-set-p '{v}) (with-temp-buffer {v})))")

# wrong-type: DEFVAR_INT checks integerp; DEFVAR_BOOL stores !NILP; the
# per-buffer predicate applies to set but not set-default / let of default.
TY = ["undo-limit", "enable-recursive-minibuffers", "fill-column", "help-char", "varh-v", "varh-l"]
fam("wrong-type", TY,
    "(list (condition-case e (set '{v} 'varh-sym) (error (list 'err e))) {v})",
    "(list (condition-case e (setq {v} \"str\") (error (list 'err e))) {v})",
    "(list (condition-case e (setq {v} 1.5) (error (list 'err e))) {v})",
    "(list (condition-case e (let (({v} 'varh-sym)) {v}) (error (list 'err e))) {v})",
    "(list (condition-case e (set-default '{v} 'varh-sym) (error (list 'err e))) (default-value '{v}))",
    "(list (condition-case e (setq {v} nil) (error (list 'err e))) {v})",
    "(list (condition-case e (setq {v} 0) (error (list 'err e))) {v})",
    "(list (condition-case e (setq {v} -1) (error (list 'err e))) {v})",
    "(with-temp-buffer (list (condition-case e (setq {v} 'varh-sym) (error (list 'err e))) {v} (default-value '{v})))",
    "(with-temp-buffer (condition-case nil (setq {v} 'varh-sym) (error nil)) (set-default '{v} 82) {v})",
    "(with-temp-buffer (list (condition-case e (let (({v} 'varh-sym)) (list {v} (default-value '{v}))) (error (list 'err e))) {v}))",
    "(with-temp-buffer (setq {v} 83) (list (condition-case e (let (({v} 'varh-sym)) {v}) (error (list 'err e))) {v}))",
    "(list (condition-case e (let (({v} 84)) (setq {v} 'varh-sym)) (error (list 'err e))) {v})")

fam("defvaralias", ALL,
    "(condition-case e (defvaralias '{v} 'varh-base) (error (list 'err e)))")
fam("defvaralias", CORE,
    "(progn (condition-case nil (defvaralias '{v} 'varh-base) (error nil)) (list (boundp 'varh-base) (special-variable-p '{v})))",
    "(progn (defvaralias 'varh-alias '{v}) (list (eq varh-alias {v}) (boundp 'varh-alias) (special-variable-p 'varh-alias)))")
fam("defvaralias", REP,
    "(progn (defvaralias 'varh-alias '{v}) (setq varh-alias 85) (list {v} varh-alias))",
    "(progn (defvaralias 'varh-alias '{v}) (list (let ((varh-alias 86)) {v}) {v}))",
    "(progn (defvaralias 'varh-alias '{v}) (list (let (({v} 87)) varh-alias) varh-alias))",
    "(progn (defvaralias 'varh-alias '{v}) (list (condition-case e (setq varh-alias 'varh-sym) (error (list 'err e))) {v}))",
    "(progn (defvaralias 'varh-alias '{v}) (with-temp-buffer (setq varh-alias 88) (list {v} (default-value '{v}) (default-value 'varh-alias))))",
    "(progn (defvaralias 'varh-alias '{v}) (set-default 'varh-alias 89) (list {v} (default-value '{v})))",
    "(progn (defvaralias 'varh-alias '{v}) (list (condition-case e (makunbound 'varh-alias) (error (list 'err e))) (boundp '{v})))",
    "(progn (defvaralias 'varh-alias '{v}) (list (local-variable-if-set-p 'varh-alias) (default-boundp 'varh-alias)))")
fam("local-variable-if-set-p", ALL,
    "(local-variable-if-set-p '{v})")
fam("local-variable-if-set-p", REP,
    "(local-variable-if-set-p '{v} (current-buffer))",
    "(with-temp-buffer (local-variable-if-set-p '{v}))",
    "(with-temp-buffer (setq {v} 91) (local-variable-if-set-p '{v}))",
    "(let (({v} 92)) (local-variable-if-set-p '{v}))",
    "(with-temp-buffer (condition-case e (make-local-variable '{v}) (error (list 'err e))) (local-variable-if-set-p '{v}))",
    "(progn (condition-case e (make-variable-buffer-local '{v}) (error (list 'err e))) (local-variable-if-set-p '{v}))")


# Load-machinery object variables (lread.c DEFVAR_LISP, both nil in C),
# added after the original twelve.  Their templates are appended to T, so
# every earlier id is unchanged and theirs continue each family's numbering.
# GNU maintains both while it runs: `load' specbinds current-load-list and
# LOADHIST_ATTACH conses onto it from defvar/defalias/provide/require, and
# `provide' runs after-load-alist entries.  So a case must leave them as it
# found them (no lasting set/setq, which would also break GNU's own load of
# the driver), and their dumped/loading values are shown as `listp' only.
# The templates are the ALL ones plus every restoring CORE one.
LOAD = ["after-load-alist", "current-load-list"]
for _v in LOAD:
    VARS[_v] = ("object", "listp")
_LEAVES_STATE = {"(list (set '{v} 42) (symbol-value '{v}) {v})", "(list (setq {v} 7) {v})"}
for _family, _t, _vars, _dyn in list(T):
    if (_vars is ALL or _vars is CORE) and _t not in _LEAVES_STATE:
        T.append((_family, _t, LOAD, _dyn))


def expand(template, v):
    out = template.replace("{v}", v)
    # {S:EXPR} -> show(v, EXPR); EXPR has balanced parens and no '}'.
    while "{S:" in out:
        i = out.index("{S:")
        j = out.index("}", i)
        out = out[:i] + show(v, out[i + 3:j]) + out[j + 1:]
    return out


def build():
    rows, seen = [], {}
    for family, template, vars_, dyn in T:
        for v in vars_:
            n = seen.get(family, 0)
            seen[family] = n + 1
            row = {"id": f"{family}/{n:03d}", "family": family, "kind": VARS[v][0],
                   "variable": v, "form": expand(template, v)}
            s = setup_for(v, dyn)
            if s:
                row["setup"] = s
            rows.append(row)
    return rows


if __name__ == "__main__":
    text = json.dumps(build(), indent=1) + "\n"
    path = HERE / "cases.json"
    if "--check" in sys.argv:
        sys.exit(0 if path.exists() and path.read_text() == text else 1)
    path.write_text(text)
    rows = build()
    fams = {}
    for r in rows:
        fams[r["family"]] = fams.get(r["family"], 0) + 1
    print(f"{len(rows)} cases -> {path}")
    for f, n in fams.items():
        print(f"  {f}: {n}")
