#!/usr/bin/env python3
"""Regenerate a probe's .gnu file: pinned GNU's answers to its forms.

Child frames and window dividers need a window system, which GNU batch
does not have, so this runs the pinned GNU X11 build
($ROMACS_GNU_GUI_BINARY) on a private Xvfb ($ROMACS_DISPLAY_TOOLS) over
loopback TCP, with a private HOME, and writes one printed value per probe
form beside the probe.  tests/child_frames.rs compares Romacs's answers
with child_frames.gnu, tests/frame_chrome.rs with frame_chrome.gnu.

usage: child_frames_oracle.py [--probe FILE.el] [--output FILE]
"""
import argparse
import os
import pathlib
import secrets
import socket
import subprocess
import tempfile
import time

HERE = pathlib.Path(__file__).resolve().parent

# Each probe line in one session, as tests/child_frames.rs evaluates it.
DRIVER = r"""
(let ((lines (with-temp-buffer (insert-file-contents (getenv "PROBE"))
                               (split-string (buffer-string) "\n")))
      (out nil))
  (set-buffer "*scratch*")
  (dolist (line lines)
    (unless (or (string-empty-p line) (string-prefix-p ";" line))
      (let ((form (car (read-from-string line))))
        (push (if (eq (car-safe form) 'setq)
                  (progn (eval form t) "-")
                (prin1-to-string (condition-case e (eval form t) (error (list 'signal e)))))
              out))))
  (with-temp-file (getenv "OUT") (insert (mapconcat #'identity (nreverse out) "\n") "\n"))
  (kill-emacs 0))
"""


def free_display():
    for number in range(140, 200):
        with socket.socket() as s:
            if s.connect_ex(("127.0.0.1", 6000 + number)) != 0:
                return number
    raise SystemExit("no free X display")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--probe", default=str(HERE / "child_frames.el"))
    parser.add_argument("--output")
    args = parser.parse_args()
    probe = pathlib.Path(args.probe).resolve()
    tools = pathlib.Path(os.environ["ROMACS_DISPLAY_TOOLS"])
    emacs = os.environ["ROMACS_GNU_GUI_BINARY"]
    scratch = pathlib.Path(tempfile.mkdtemp(prefix="child-frames-", dir=os.environ.get("TMPDIR")))
    number = free_display()
    display = f"127.0.0.1:{number}"
    auth = scratch / "Xauthority"
    subprocess.run([tools / "xauth", "-f", auth, "add", display, "MIT-MAGIC-COOKIE-1", secrets.token_hex(16)], check=True)
    server = subprocess.Popen([tools / "Xvfb", f":{number}", "-screen", "0", "1280x800x24", "-nolisten", "unix",
                               "-listen", "tcp", "-nolock", "-noreset", "-auth", auth],
                              cwd=scratch, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        for _ in range(50):
            with socket.socket() as s:
                if s.connect_ex(("127.0.0.1", 6000 + number)) == 0:
                    break
            time.sleep(0.1)
        home = scratch / "home"
        home.mkdir()
        out = scratch / "answers"
        env = {"PATH": os.environ["PATH"], "HOME": str(home), "DISPLAY": display, "XAUTHORITY": str(auth),
               "GDK_BACKEND": "x11", "LANG": "C.UTF-8", "PROBE": str(probe), "OUT": str(out)}
        if "ROMACS_GATE_FONTCONFIG" in os.environ:
            env["FONTCONFIG_FILE"] = os.environ["ROMACS_GATE_FONTCONFIG"]
        subprocess.run([emacs, "-Q", "--eval", DRIVER], env=env, check=True, timeout=120)
        answers = out.read_text()
    finally:
        server.terminate()
        server.wait()
    header = f";; pinned GNU {os.path.basename(os.path.dirname(os.path.dirname(emacs)))} X11 on Xvfb: child_frames_oracle.py\n"
    pathlib.Path(args.output or probe.with_suffix(".gnu")).write_text(header + answers)


if __name__ == "__main__":
    main()
