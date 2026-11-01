#!/usr/bin/env python3
"""romacs FILE, end to end, headless: open a real .org file, see it, edit, C-x C-s.

Runs the REAL live binary exactly as a shell or the desktop entry would
(`romacs-vm-gui notes.org`, no --wayland test mode, no stdin protocol) in a
private headless labwc (WLR_BACKENDS=headless, WLR_RENDERER=pixman), and
types with wtype into that compositor only -- never the user's keyboard.

Assertions, each on an observable a user has:
  1. the file's text is in the frame the binary SUBMITS to the compositor
     (--trace-scene glyph dump: every glyph's character and row);
  2. after typing C-e " λ!" and C-x C-s, the file on disk is BYTE-IDENTICAL
     to the expected edited text (not "saving returned");
  3. modified/saved compositor pixels differ inside the mode line, not only
     at the blinking cursor;
  4. a compositor-native close asks before discarding, n cancels, y closes;
  5. native C-x C-s asks before overwriting an external writer's bytes;
     without confirmation, the external bytes remain unchanged.
Requires Pillow for image comparison. Keeps grim captures for visual review.
The CLI fixture exercises +LINE:COLUMN and -- with a leading-dash filename.

usage: fileio_live.py --binary PATH --output DIR
Run it against a pre-lane binary to see it fail at assertion 1 (that
binary exits 1 with "unknown option notes.org").
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import live_oracle as live

ORIGINAL = "* Heading\nλ body\n- [ ] item\n".encode()
EXPECTED = "* Heading λ!\nλ body\n- [ ] item\n".encode()


def last_scene(stderr_text):
    """Use the last complete submitted JSON record, not a partial append."""
    scene = None
    for line in stderr_text.split('\n'):
        if line.startswith("{") and '"submitted-scene"' in line:
            try:
                candidate = json.loads(line)
            except json.JSONDecodeError:
                continue
            if candidate.get("diagnostic") == "submitted-scene":
                scene = candidate
    return scene


def scene_rows(scene, clip=None):
    by_y = {}
    for g in scene["glyphs"]:
        if g["role"] == "Text" and (clip is None or g["clip"] == clip):
            by_y.setdefault((json.dumps(g["clip"], sort_keys=True), g["y"]), []).append((g["x"], g["char"]))
    return ["".join(c for _, c in sorted(r)) for _, r in sorted(by_y.items())]


def rows(stderr_text):
    """All Text rows, including echo/prompts, of the last submitted scene."""
    scene = last_scene(stderr_text)
    return [] if scene is None else scene_rows(scene)


def document_rows(stderr_text, clip):
    """All document rows at the exact clip pinned before editing; never filter text."""
    scene = last_scene(stderr_text)
    assert scene is not None, "no submitted scene for document observation"
    result = scene_rows(scene, clip)
    assert result, f"pinned document clip disappeared: {clip}"
    return result


def document_clip(stderr_text, expected_rows):
    """Identify the owned single document by its complete opened rowset once."""
    scene = last_scene(stderr_text)
    assert scene is not None, "no submitted scene to pin document clip"
    clips = {json.dumps(g["clip"], sort_keys=True) for g in scene["glyphs"] if g["role"] == "Text"}
    matches = [json.loads(clip) for clip in clips
               if scene_rows(scene, json.loads(clip)) == expected_rows]
    assert len(matches) == 1, f"expected one document clip, got {matches}"
    return matches[0]


def shot(path, env, report):
    """Capture the private compositor for pixel comparison and visual review."""
    try:
        time.sleep(0.4)
        subprocess.run(["grim", str(path)], env=env, check=True, timeout=10)
    except Exception as e:  # noqa: BLE001
        report.setdefault("screenshot_errors", []).append(repr(e))


def wait(pred, timeout, what, proc):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if pred():
            return
        if proc is not None and proc.poll() is not None:
            raise AssertionError(f"{what}: process exited {proc.returncode}")
        time.sleep(0.1)
    raise AssertionError(f"timed out waiting for {what}")


def main():
    from PIL import Image, ImageChops

    p = argparse.ArgumentParser()
    p.add_argument("--binary", required=True, type=Path)
    p.add_argument("--output", required=True, type=Path)
    p.add_argument("--timeout", type=float, default=60)
    p.add_argument("--labwc", default=live.LABWC)
    p.add_argument("--wlr-randr", default=live.RANDR)
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    report: dict = {"binary": str(a.binary.resolve())}
    with tempfile.TemporaryDirectory(prefix="fio-live-") as temp:
        runtime = Path(temp)
        runtime.chmod(0o700)
        config = runtime / "config"
        config.mkdir()
        (config / "rc.xml").write_text(live.RC_XML.replace(
            "</labwc_config>",
            '<keyboard><keybind key="A-F4"><action name="Close"/></keybind></keyboard></labwc_config>'))
        home = runtime / "home"
        home.mkdir()
        work = runtime / "work"
        work.mkdir()
        notes = work / "-notes.org"
        notes.write_bytes(ORIGINAL)
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": str(home), "LANG": "C.UTF-8", "ROMACS_DIAGNOSTICS": "1",
               "XDG_RUNTIME_DIR": str(runtime), "XDG_CONFIG_HOME": str(home / "config"),
               "XDG_CACHE_HOME": str(home / "cache"), "XDG_DATA_HOME": str(home / "data"),
               "XDG_STATE_HOME": str(home / "state"), "TMPDIR": str(runtime), "WLR_BACKENDS": "headless",
               "WLR_HEADLESS_OUTPUTS": "1", "WLR_RENDERER": "pixman", "WLR_LIBINPUT_NO_DEVICES": "1",
               "LIBSEAT_BACKEND": "noop"}
        comp = keyboard = proc = None
        stderr_path = a.output / "binary.stderr"
        try:
            with (a.output / "compositor.log").open("w") as clog, stderr_path.open("w") as blog:
                comp = subprocess.Popen([a.labwc, "-C", str(config)], env=env, stdout=clog, stderr=clog)
                wait(lambda: any(s.is_socket() for s in runtime.glob("wayland-*")), a.timeout, "compositor", comp)
                env["WAYLAND_DISPLAY"] = next(s for s in runtime.glob("wayland-*") if s.is_socket()).name
                subprocess.run([a.wlr_randr, "--output", "HEADLESS-1", "--custom-mode", "1600x1000"], env=env, check=True)
                keyboard = subprocess.Popen(["/usr/bin/wtype", "-s", "120000", "-k", "Shift_L"], env=env,
                                            stdout=blog, stderr=blog)
                time.sleep(0.3)
                # Exactly what a user types: a RELATIVE name, from its directory.
                cmd = [str(a.binary.resolve()), "--trace-scene", "+1:1", "--", "-notes.org"]
                report["command"] = cmd
                proc = subprocess.Popen(cmd, env=env, cwd=work, stdin=subprocess.DEVNULL,
                                        stdout=blog, stderr=blog)

                scene_start = 0

                def trace_text():
                    return stderr_path.read_text(errors="replace")[scene_start:]

                def shown():
                    r = rows(trace_text())
                    return "* Heading" in r and "λ body" in r and "- [ ] item" in r

                wait(shown, a.timeout, "notes.org text in the submitted frame", proc)
                clip = document_clip(trace_text(), ORIGINAL.decode().splitlines())
                report["document_clip"] = clip
                report["opened_document_rows"] = document_rows(trace_text(), clip)
                report["opened_rows"] = rows(trace_text())[:6]
                shot(a.output / "opened.png", env, report)
                # C-e, then " λ!" at the end of the heading line.
                subprocess.run(["/usr/bin/wtype", "-d", "120", "-M", "ctrl", "e", "-m", "ctrl"], env=env, check=True, timeout=15)
                subprocess.run(["/usr/bin/wtype", "-d", "120", " λ!"], env=env, check=True, timeout=15)
                wait(lambda: "* Heading λ!" in rows(trace_text()), a.timeout,
                     "the edit in the submitted frame", proc)
                assert notes.read_bytes() == ORIGINAL, "the file changed before C-x C-s"
                report["modified_rows"] = rows(trace_text())[:6]
                report["modified_document_rows"] = document_rows(trace_text(), clip)
                shot(a.output / "modified.png", env, report)
                # C-x C-s
                # One wtype per chord: a single `wtype -M ctrl x s` delivered
                # both keys as undefined events here (two text keysyms in one
                # wtype virtual keymap); C-x then C-s as two chords, as a person
                # types them, is the journey being proved.
                for chord in ("x", "s"):
                    subprocess.run(["/usr/bin/wtype", "-M", "ctrl", chord, "-m", "ctrl"], env=env, check=True, timeout=15)
                    time.sleep(0.3)
                wait(lambda: notes.read_bytes() != ORIGINAL, a.timeout, "C-x C-s to write notes.org", proc)
                time.sleep(0.5)
                got = notes.read_bytes()
                report["saved_bytes_hex"] = got.hex()
                report["expected_bytes_hex"] = EXPECTED.hex()
                report["byte_identical"] = got == EXPECTED
                report["saved_rows"] = rows(trace_text())[:6]
                report["saved_document_rows"] = document_rows(trace_text(), clip)
                (a.output / "saved-file.org").write_bytes(got)
                shot(a.output / "saved.png", env, report)
                assert got == EXPECTED, f"saved bytes {got!r} != expected {EXPECTED!r}"
                modified = Image.open(a.output / "modified.png").convert("RGB")
                saved = Image.open(a.output / "saved.png").convert("RGB")
                difference = ImageChops.difference(modified, saved)
                report["modified_saved_difference_bbox"] = difference.getbbox()
                report["modified_saved_changed_pixels"] = sum(pixel != (0, 0, 0) for pixel in difference.getdata())
                assert report["modified_saved_changed_pixels"] > 0, "modified and saved screenshots have identical pixels"
                difference.save(a.output / "modified-v-saved-diff.png")
                mode_rows = [y for y in range(modified.height)
                             if modified.getpixel((modified.width - 1, y)) == (80, 38, 154)]
                assert mode_rows, "purple mode line missing from compositor pixels"
                mode_box = (0, min(mode_rows), modified.width, max(mode_rows) + 1)
                mode_difference = difference.crop(mode_box)
                report["mode_line_changed_pixels"] = sum(pixel != (0, 0, 0) for pixel in mode_difference.getdata())
                report["mode_line_difference_bbox"] = mode_difference.getbbox()
                assert report["mode_line_changed_pixels"] > 0, "only cursor blink changed, not modified status"
                modified.crop(mode_box).save(a.output / "modified-mode-line.png")
                saved.crop(mode_box).save(a.output / "saved-mode-line.png")
                assert report["modified_document_rows"] == report["saved_document_rows"], "saving changed the displayed text"

                def send_chord(modifier, key):
                    subprocess.run(["/usr/bin/wtype", "-M", modifier, "-k", key, "-m", modifier],
                                   env=env, check=True, timeout=15)
                    time.sleep(0.3)

                def reopen(expected, label):
                    nonlocal proc, scene_start
                    assert proc.poll() == 0, "reopen requires a successful native close"
                    assert notes.read_bytes() == expected, "bytes changed before reopen"
                    scene_start = len(stderr_path.read_text(errors="replace"))
                    proc = subprocess.Popen(cmd, env=env, cwd=work, stdin=subprocess.DEVNULL,
                                            stdout=blog, stderr=blog)
                    expected_rows = expected.decode().splitlines()
                    wait(lambda: all(row in rows(trace_text()) for row in expected_rows),
                         a.timeout, f"{label} document reappears after native reopen", proc)
                    reopened_clip = document_clip(trace_text(), expected_rows)
                    actual = document_rows(trace_text(), reopened_clip)
                    assert actual == expected_rows, f"{label}: reopened document differs"
                    assert notes.read_bytes() == expected, "reopen changed file bytes"
                    report[label + "_rows"] = actual
                    report[label + "_bytes_hex"] = notes.read_bytes().hex()
                    shot(a.output / (label + ".png"), env, report)

                # Reload the actual C-x C-s bytes before testing the external writer.
                send_chord("alt", "F4")
                proc.wait(timeout=a.timeout)
                assert proc.returncode == 0, f"saved native shutdown exited {proc.returncode}"
                report["saved_native_close_exit"] = proc.returncode
                reopen(EXPECTED, "reopened-saved")
                send_chord("ctrl", "e")
                subprocess.run(["/usr/bin/wtype", "x"], env=env, check=True, timeout=15)
                wait(lambda: "* Heading λ!x" in rows(trace_text()),
                     a.timeout, "unsaved native-close fixture", proc)
                send_chord("alt", "F4")
                wait(lambda: any("(y/n; C-g cancels)" in row for row in rows(trace_text())),
                     a.timeout, "native compositor close confirmation", proc)
                shot(a.output / "native-close-prompt.png", env, report)
                assert proc.poll() is None, "native close destroyed unsaved text"
                subprocess.run(["/usr/bin/wtype", "n"], env=env, check=True, timeout=15)
                wait(lambda: "Cancelled" in rows(trace_text()),
                     a.timeout, "native close cancellation", proc)
                assert notes.read_bytes() == EXPECTED
                report["native_close_cancelled"] = proc.poll() is None
                assert report["native_close_cancelled"], "cancelled native close exited"
                shot(a.output / "native-close-cancelled.png", env, report)

                external = b"external writer must survive\n"
                notes.write_bytes(external)
                for key in ("x", "s"):
                    send_chord("ctrl", key)
                wait(lambda: any("File changed on disk" in row for row in rows(trace_text())),
                     a.timeout, "native save supersession refusal", proc)
                assert notes.read_bytes() == external, "native C-x C-s overwrote external bytes"
                report["native_supersession_preserved"] = True
                shot(a.output / "native-supersession-refused.png", env, report)
                send_chord("alt", "F4")
                wait(lambda: any("(y/n; C-g cancels)" in row for row in rows(trace_text())),
                     a.timeout, "second native close prompt", proc)
                subprocess.run(["/usr/bin/wtype", "y"], env=env, check=True, timeout=15)
                proc.wait(timeout=a.timeout)
                assert proc.returncode == 0, f"confirmed native shutdown exited {proc.returncode}"
                assert notes.read_bytes() == external
                report["native_close_confirmed"] = True
                report["confirmed_native_close_exit"] = proc.returncode
                # The superseding writer remains authoritative after confirmed discard.
                reopen(external, "reopened-external")
                send_chord("alt", "F4")
                proc.wait(timeout=a.timeout)
                assert proc.returncode == 0, f"reopened native shutdown exited {proc.returncode}"
                assert notes.read_bytes() == external
                (a.output / "external-file.org").write_bytes(notes.read_bytes())
                report["reopened_clean_exit"] = proc.returncode
                report["verdict"] = "PASS"
        except AssertionError as e:
            report["verdict"] = f"FAIL: {e}"
        finally:
            for x in (proc, keyboard, comp):
                if x is not None and x.poll() is None:
                    x.terminate()
                    try:
                        x.wait(5)
                    except subprocess.TimeoutExpired:
                        x.kill()
            if proc is not None:
                report["binary_exit"] = proc.returncode
    (a.output / "report.json").write_text(json.dumps(report, indent=1, ensure_ascii=False) + "\n")
    print(json.dumps({k: report.get(k) for k in ("verdict", "byte_identical", "binary_exit")}, ensure_ascii=False))
    print("FILEIO-LIVE", report["verdict"])
    return 0 if report["verdict"] == "PASS" else 1


if __name__ == "__main__":
    sys.exit(main())
