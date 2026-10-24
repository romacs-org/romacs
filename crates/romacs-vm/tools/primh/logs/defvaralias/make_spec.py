#!/usr/bin/env python3
"""Augment the from-manifest defvaralias spec with its domain and probes."""
import json, sys
from pathlib import Path
p = Path(sys.argv[1])
spec = json.loads(p.read_text())
spec["args"] = ["any", "symbol", "string"]
spec["extra"] = {"1": ["1", "(copy-sequence \"abc\")", "'primh--base"], "2": ["nil", "1"]}
P = "primh-va"
spec["probes"] = [
 f"(progn (defvar {P}1-b 5) (defvaralias '{P}1-a '{P}1-b) (list (symbol-value '{P}1-a) (boundp '{P}1-a)))",
 f"(progn (defvar {P}2-b 1) (defvaralias '{P}2-a '{P}2-b) (set '{P}2-a 8) (list (symbol-value '{P}2-b)))",
 f"(progn (defvar {P}3-b 1) (defvaralias '{P}3-a '{P}3-b) (set '{P}3-b 9) (symbol-value '{P}3-a))",
 f"(progn (defvar {P}4-a 3) (list (defvaralias '{P}4-a '{P}4-b) (symbol-value '{P}4-b) (symbol-value '{P}4-a)))",
 f"(progn (defvaralias '{P}5-a '{P}5-b) (list (boundp '{P}5-a) (boundp '{P}5-b) (progn (set '{P}5-b 2) (symbol-value '{P}5-a))))",
 f"(progn (defvar {P}6-a 1) (defvar {P}6-b 2) (defvaralias '{P}6-a '{P}6-b) (list (symbol-value '{P}6-a) (symbol-value '{P}6-b)))",
 f"(progn (defvaralias '{P}7-a '{P}7-b) (defvaralias '{P}7-b '{P}7-a))",
 f"(progn (defvaralias '{P}8-a '{P}8-b) (defvaralias '{P}8-b '{P}8-c) (defvaralias '{P}8-c '{P}8-a))",
 f"(progn (defvar {P}9-c 9) (defvaralias '{P}9-b '{P}9-c) (defvaralias '{P}9-a '{P}9-b) (set '{P}9-a 10) (list (symbol-value '{P}9-a) (symbol-value '{P}9-b) (symbol-value '{P}9-c)))",
 f"(progn (defvar {P}10-b 1) (defvaralias '{P}10-a '{P}10-b) (eval '(list (let (({P}10-a 5)) (list {P}10-b (symbol-value '{P}10-a))) {P}10-b) t))",
 f"(progn (defvar {P}11-a 1) (defvar {P}11-b 2) (eval '(let (({P}11-a 3)) (defvaralias '{P}11-a '{P}11-b)) t))",
 f"(progn (defvar {P}12-a 1) (make-variable-buffer-local '{P}12-a) (defvaralias '{P}12-a '{P}12-b))",
 f"(with-temp-buffer (defvar {P}13-a 1) (make-local-variable '{P}13-a) (defvaralias '{P}13-a '{P}13-b))",
 f"(progn (defvaralias '{P}14-a '{P}14-b \"Doc.\") (get '{P}14-a 'variable-documentation))",
 f"(progn (put '{P}15-a 'variable-documentation \"Old\") (defvaralias '{P}15-a '{P}15-b) (get '{P}15-a 'variable-documentation))",
 f"(progn (defvaralias '{P}16-a '{P}16-b) (list (special-variable-p '{P}16-a) (special-variable-p '{P}16-b)))",
 f"(progn (defvar {P}17-b 1) (defvaralias '{P}17-a '{P}17-b) (makunbound '{P}17-a) (list (boundp '{P}17-b) (boundp '{P}17-a)))",
 f"(progn (defvar {P}18-x 1) (defvar {P}18-y 2) (defvaralias '{P}18-a '{P}18-x) (defvaralias '{P}18-a '{P}18-y) (list (symbol-value '{P}18-a) (symbol-value '{P}18-x)))",
 f"(defvaralias 'most-positive-fixnum '{P}19)",
 f"(progn (defvaralias '{P}20-a :{P}-kw) (list (symbol-value '{P}20-a) (condition-case e (set '{P}20-a 1) (error e))))",
 f"(progn (defvaralias '{P}21-a nil) (list (symbol-value '{P}21-a) (condition-case e (set '{P}21-a 1) (error e)) (symbol-value '{P}21-a)))",
 f"(progn (defvar {P}22-b 1) (make-variable-buffer-local '{P}22-b) (defvaralias '{P}22-a '{P}22-b) (list (with-temp-buffer (set '{P}22-a 5) (list (symbol-value '{P}22-b) (local-variable-if-set-p '{P}22-a))) (symbol-value '{P}22-b)))",
 f"(let ((b (make-symbol \"{P}-uninterned\"))) (eq b (defvaralias '{P}23 b)))",
 f"(progn (defvar {P}24-b 0) (defvaralias '{P}24-a '{P}24-b) (eval '(progn (setq {P}24-a 4) (list {P}24-b {P}24-a)) t))",
 f"(progn (defvaralias '{P}25-a '{P}25-b) (eval '(defvar {P}25-a 6) t) (list (symbol-value '{P}25-b)))",
 f"(progn (defvaralias '{P}27-a '{P}27-b) (condition-case e (symbol-value '{P}27-a) (error e)))",
 f"(progn (defvar {P}28-b nil) (defvaralias '{P}28-a '{P}28-b) (set '{P}28-b (list 1 (list 2 3) (copy-sequence \"s\"))) (let ((i 0)) (while (< i 5000) (list i i) (setq i (1+ i)))) (symbol-value '{P}28-a))",
 f"(defvaralias '{P}29 '{P}29)",
 f"(progn (defvar {P}30-b 1) (list (eval '(let (({P}30-b 2)) (defvaralias '{P}30-a '{P}30-b) (symbol-value '{P}30-a)) t) (symbol-value '{P}30-a)))",
 f"(progn (defvaralias '{P}31-a '{P}31-b) (eval '(list (boundp '{P}31-a) (condition-case e {P}31-a (error e))) t))",
 f"(progn (defvar {P}32-b 1) (defvaralias '{P}32-a '{P}32-b) (eval '(progn (defvar {P}32-a 7) {P}32-b) t))",
]
p.write_text(json.dumps(spec) + "\n")
print(f"{p}: {len(spec['probes'])} probes")
