#!/usr/bin/env python3
"""Record INITIAL-terminal frame contracts with the pinned GNU oracle.

Run from the repository root:
  python3 crates/romacs-vm/tools/primh/frame_specs.py --gnu "$ROMACS_GNU_ORACLE" --manifest MANIFEST
Omit --manifest only before the production declarations exist (red-first).
Evidence is isolated from the shared primh evidence directory. Every stateful
probe gets a fresh GNU process. No TTY/display server or printed object IDs.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import primh

HERE = Path(__file__).resolve().parent
PIN = "a31ba7aa12a7e4bbdda6ab9e19dab8338340f81f8100c33ac3843d10e176c8d8"
BAD = ["nil", "t", "1", "'symbol", '"string"', "(selected-window)"]
SPLIT = "(let* ((a (selected-window)) (h (window-pixel-height a)) (half (/ h 2)) (b (progn (set-window-new-pixel a (- h half)) (split-window-internal a half nil 0.5))) (p (window-parent b))) BODY)"
def split(body):
    return SPLIT.replace("BODY", body)

PROBES = {
    "framep": ["(framep (selected-frame))", "(framep (frame-terminal))", *[f"(framep {x})" for x in BAD], "(funcall #'framep (selected-frame))"],
    "frame-list": ["(equal (frame-list) (list (selected-frame)))", "(eq (frame-list) (frame-list))", "(equal (funcall #'frame-list) (frame-list))"],
    "frame-terminal": ["(eq (frame-terminal) (frame-terminal nil))", "(eq (frame-terminal) (frame-terminal (selected-frame)))", "(eq (funcall #'frame-terminal) (frame-terminal))", "(terminal-live-p (frame-terminal))"],
    "terminal-live-p": ["(terminal-live-p nil)", "(terminal-live-p (selected-frame))", "(terminal-live-p (frame-terminal))", *[f"(terminal-live-p {x})" for x in BAD[1:]], "(funcall #'terminal-live-p (frame-terminal))"],
    "last-nonminibuffer-frame": ["(eq (last-nonminibuffer-frame) (selected-frame))", "(eq (funcall #'last-nonminibuffer-frame) (selected-frame))"],
    "terminal-list": ["(equal (terminal-list) (list (frame-terminal)))", "(eq (terminal-list) (terminal-list))", "(equal (funcall #'terminal-list) (terminal-list))"],
    "tty-top-frame": ["(tty-top-frame)", "(tty-top-frame (selected-frame))", "(tty-top-frame (frame-terminal))", "(funcall #'tty-top-frame nil)"],
    "window-list": ["(equal (window-list) (list (selected-window)))", "(equal (window-list nil t) (list (selected-window) (minibuffer-window)))", "(equal (window-list nil 'never) (list (selected-window)))", "(equal (window-list nil t (minibuffer-window)) (list (minibuffer-window) (selected-window)))", "(eq (window-list) (window-list))", "(equal (funcall #'window-list) (window-list))", split("(list (equal (window-list) (list a b)) (equal (window-list nil nil b) (list b a)) (equal (window-list nil t b) (list b (minibuffer-window) a)))"), split("(condition-case e (window-list nil nil p) (error (list (car e) (car (cdr e)) (eq (car (cdr (cdr e))) p))))")],
    "get-buffer-window": ["(eq (get-buffer-window) (selected-window))", "(eq (get-buffer-window (current-buffer)) (selected-window))", "(eq (get-buffer-window (buffer-name)) (selected-window))", "(get-buffer-window \"frame-probe-missing-buffer\")", "(get-buffer-window (get-buffer-create \"frame-probe-undisplayed\"))", "(eq (funcall #'get-buffer-window nil t) (selected-window))", *[f"(eq (get-buffer-window nil {x}) (selected-window))" for x in ["nil", "t", "0", "'visible", "(selected-frame)"]], split("(let ((buf (get-buffer-create \"frame-probe-other\"))) (set-window-buffer b buf) (list (eq (get-buffer-window buf) b) (eq (get-buffer-window nil) a)))")],
    "window-combination-limit": ["(window-combination-limit nil)", "(window-combination-limit (selected-window))", "(window-combination-limit (minibuffer-window))", split("(window-combination-limit p)"), split("(funcall #'window-combination-limit p)"), split("(progn (set-window-new-pixel p h) (set-window-new-pixel a h) (delete-window-internal b) (condition-case e (window-combination-limit p) (error (list (car e) (car (cdr e)) (eq (car (cdr (cdr e))) p)))))")],
}
PROBES["get-buffer-window"].append(split('(let* ((hb (window-pixel-height b)) (half-b (/ hb 2)) (c (progn (set-window-new-pixel b (- hb half-b)) (split-window-internal b half-b nil 0.5))) (x (get-buffer-create "frame-search-cyclic"))) (set-window-buffer a x) (set-window-buffer c x) (select-window b) (eq (get-buffer-window x) c))'))
PROBES["window-combination-limit"].append("(let ((window-combination-limit t)) " + split("(eq (window-combination-limit p) t)") + ")")
# Optional argument domains stay scalar: direct printed frame/terminal identity
# is process-specific and is instead tested by the probes above.
DOMAINS = {
    "framep": [BAD], "frame-list": [], "frame-terminal": [BAD[:5]],
    "terminal-live-p": [BAD[:5]], "last-nonminibuffer-frame": [],
    "terminal-list": [], "tty-top-frame": [BAD[:5]],
    "window-list": [["nil", "1", "'symbol"], ["nil", "t"], ["nil", "1"]],
    "get-buffer-window": [["nil", "1", "'symbol"], ["nil", "t", "0", "'visible"]],
    "window-combination-limit": [["nil", "1", "'symbol"]],
}

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--gnu", required=True)
    ap.add_argument("--gnu-src", type=Path, default=primh.GNU_SRC)
    ap.add_argument("--manifest", type=Path)
    args = ap.parse_args()
    assert hashlib.sha256(Path(args.gnu).resolve().read_bytes()).hexdigest() == PIN, "GNU binary pin mismatch"
    manifest = {r["name"]: r for r in json.loads(args.manifest.read_text())} if args.manifest else None
    out = HERE / "frame-specs"
    out.mkdir(exist_ok=True)
    primh.EVID = HERE / "frame-evidence"
    paths = []
    for name, probes in PROBES.items():
        hits = []
        for file in ["frame.c", "terminal.c", "term.c", "window.c"]:
            text = (args.gnu_src / file).read_text()
            match = re.search(r'DEFUN \("' + re.escape(name) + r'",\s*\w+,\s*\w+,\s*(\d+),\s*(\d+)', text)
            if match:
                hits.append((file, text[:match.start()].count("\n") + 1, [int(match[1]), int(match[2])]))
        assert len(hits) == 1, (name, hits)
        file, line, arity = hits[0]
        spec = dict(name=name, arity=arity, effect="reads", gnu_files=[file], gnu_def=f"{file}:{line}", args=["none"] * len(DOMAINS[name]), extra={str(i): v for i, v in enumerate(DOMAINS[name])}, probes=[f"(fboundp '{name})", *probes])
        if manifest is not None:
            m = manifest[name]
            assert arity == [m["min"], m["max"]], name
            spec.update(effect=m["effect"].lower(), gnu_def=m["gnu"], rust=m["rust"])
        path = out / f"{name}.json"
        path.write_text(json.dumps(spec, indent=1) + "\n")
        paths.append(str(path))
    primh.record(argparse.Namespace(spec=paths, gnu=args.gnu, gnu_src=args.gnu_src))
    rows = []
    for path in paths:
        spec = primh.load_spec(path)
        ev = primh.evidence(spec)
        # Process-specific frame/window/terminal printer IDs are retained in
        # full evidence, but stable Rust replay excludes those and printer rows.
        for row in ev["cases"]:
            opaque = any(token in row["expected"] for token in ("#<frame ", "#<window ", "#<terminal "))
            if row["family"] in ("probe", "arity"):
                assert not opaque, row
            if row["family"] != "printer" and not opaque:
                rows.append(dict(primitive=spec["name"], **row))
    primh.atomic_json(HERE / "frame-replay.json", dict(gnu_binary_sha256=PIN, cases=rows))
    print(f"{len(paths)} primitives; {len(rows)} stable Rust replay cases")

if __name__ == "__main__":
    main()
