#!/usr/bin/env python3
"""Bounded private-compositor C-x C-f journey through the real native GUI."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

import fileio_live as fio
import live_oracle as live


def current_rows(text):
    """Track the latest prompt snapshot or submitted scene, not a stale frame.

    Prompt text is pre-layout state; compositor captures qualify its visibility.
    """
    result = []
    for line in text.split('\n'):
        if not line.startswith('{'):
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if record.get("diagnostic") == "prompt-paint":
            result = [record["minibuffer"]]
        elif record.get("diagnostic") == "submitted-scene":
            result = fio.scene_rows(record)
    return result


def check_prompt_close(proc, keys, control, wait_visible, capture, trace, work, timeout, report):
    """Native close must protect both visible and hidden modified visits."""
    original = work / "original.txt"
    control("x")
    keys("1")
    control("x")
    control("f")
    wait_visible("Find file:")
    keys("original.txt", "-k", "Return")
    wait_visible("Original sentinel")
    control("e")
    keys(" DIRTY")
    expected = "Original sentinel DIRTY"
    wait_visible(expected)
    clip = fio.document_clip(trace(), [expected, "second line"])

    def switch(name):
        control("x")
        keys("b")
        wait_visible("Switch to buffer")
        keys(name, "-k", "Return")
        fio.wait(lambda: not any("Switch to buffer" in row for row in current_rows(trace())),
                 timeout, "buffer switch completed", proc)

    switch("other")
    keys("Other sentinel")
    wait_visible("Other sentinel")
    switch("original.txt")
    wait_visible(expected)
    cases = [("outside", None, False), ("find-visible", "f", False),
             ("switch-visible", "b", False), ("find-hidden", "f", True),
             ("switch-hidden", "b", True), ("mx-visible", "x", False),
             ("mx-hidden", "x", True)]
    for index, (name, prompt, hidden) in enumerate(cases):
        if hidden:
            switch("other")
        before = fio.document_rows(trace(), clip)
        if prompt:
            if prompt == "x":
                keys("-M", "alt", "x", "-m", "alt")
                wait_visible("M-x ")
            else:
                control("x")
                control("f") if prompt == "f" else keys("b")
                wait_visible("Find file:" if prompt == "f" else "Switch to buffer")
        keys("-M", "alt", "-k", "F4", "-m", "alt")
        wait_visible("discard unsaved changes?")
        assert proc.poll() is None, f"CLOSE_PROMPT_CONSENT_REQUIRED: {name}"
        capture(name + "-confirmation")
        if index % 2:
            control("g")
        else:
            keys("n")
        wait_visible("Cancelled")
        assert fio.document_rows(trace(), clip) == before, f"CLOSE_CANCEL_DOCUMENT: {name}"
        assert original.read_text() == "Original sentinel\nsecond line\n", f"CLOSE_CANCEL_DISK: {name}"
        control("x")
        control("f")
        wait_visible("Find file:")
        control("g")
        fio.wait(lambda: not any("Find file:" in row for row in current_rows(trace())),
                 timeout, "subsequent prompt cancelled", proc)
        if hidden:
            switch("original.txt")
        keys("!")
        expected += "!"
        wait_visible(expected)
        assert fio.document_rows(trace(), clip) == [expected, "second line"], f"CLOSE_CANCEL_POINT: {name}"
        capture(name + "-continued")
    control("x")
    keys("b")
    wait_visible("Switch to buffer")
    keys("-M", "alt", "-k", "F4", "-m", "alt")
    wait_visible("discard unsaved changes?")
    capture("explicit-discard")
    keys("y")
    proc.wait(timeout=timeout)
    assert proc.returncode == 0, "CLOSE_EXPLICIT_DISCARD_EXIT"
    assert original.read_text() == "Original sentinel\nsecond line\n", "CLOSE_DISCARD_DISK"
    report.update(status="PASS", close_cases=[name for name, _, _ in cases],
                  cancelled_close_preserves_text_and_point=True,
                  subsequent_prompts_usable=True, explicit_discard_exit=proc.returncode)


def check_dired(proc, keys, control, wait_visible, capture, trace, work, timeout, report):
    """Exercise local browsing through native input, preserving dirty file visits."""
    control("x")
    keys("1")
    control("x")
    control("f")
    wait_visible("Find file:")
    keys("original.txt", "-k", "Return")
    wait_visible("Original sentinel")
    control("e")
    keys(" DIRTY")
    wait_visible("Original sentinel DIRTY")
    keys("-M", "alt", "x", "-m", "alt")
    wait_visible("M-x ")
    keys("dir", "-k", "Tab")
    wait_visible("M-x dired")
    capture("mx-dired-completed")
    keys("-k", "Return")
    wait_visible("Dired (directory):")
    capture("dired-directory-prompt")
    keys("-k", "Return")
    wait_visible(str(work) + ":")
    capture("dired-listing")
    for label in ["00 child", "01 open λ.txt", "line\\nname.txt", "tab\\tname.txt", "back\\\\name.txt"]:
        assert any(label in row for row in current_rows(trace())), f"DIRED_ESCAPED_LABEL: {label}"
    for movement, index in [(5, 0), (2, 1), (-4, 2)]:
        keys(*(["n" if movement > 0 else "p"] * abs(movement)), "-k", "Return")
        wait_visible(f"Escaped entry {index}")
        capture(f"dired-escaped-open-{index}")
        control("x")
        control("f")
        wait_visible("Find file:")
        keys("./", "-k", "Return")
        wait_visible(str(work) + ":")
    keys("p", "p", "p", "-k", "Return")
    wait_visible("child sentinel.txt")
    capture("dired-child")
    keys("^")
    wait_visible("01 open λ.txt")
    keys("n", "-k", "Return")
    wait_visible("Dired opened λ")
    control("e")
    keys(" retained")
    wait_visible("Dired opened λ retained")
    control("x")
    control("f")
    wait_visible("Find file:")
    keys("./", "-k", "Return")
    wait_visible(str(work) + ":")
    before = current_rows(trace())
    keys("x")
    wait_visible("buffer-read-only")
    after = current_rows(trace())
    assert all(row in after for row in before), "DIRED_TYPING_CHANGED_LISTING"
    capture("dired-read-only-refusal")
    keys("-k", "Return")
    wait_visible("Dired opened λ retained")
    capture("dired-dirty-revisit")
    keys("!")
    wait_visible("Dired opened λ retained!")
    control("x")
    control("f")
    wait_visible("Find file:")
    keys("./", "-k", "Return")
    wait_visible(str(work) + ":")
    (work / "03 added.txt").write_text("added")
    keys("g")
    wait_visible("03 added.txt")
    capture("dired-refreshed")
    keys("n")
    (work / "02 removed.txt").unlink()
    keys("-k", "Return")
    wait_visible("Directory entry no longer exists")
    assert not (work / "02 removed.txt").exists(), "DIRED_STALE_ENTRY_CREATED_FILE"
    capture("dired-stale-entry-refused")
    keys("g")
    fio.wait(lambda: not any("02 removed.txt" in row for row in current_rows(trace())),
             timeout, "Dired removed row after refresh", proc)
    keys("q")
    wait_visible("Original sentinel DIRTY")
    keys("!")
    wait_visible("Original sentinel DIRTY!")
    capture("dired-quit-restored-point")
    assert (work / "original.txt").read_text() == "Original sentinel\nsecond line\n"
    assert (work / "01 open λ.txt").read_text() == "Dired opened λ\n"
    control("x")
    control("s")
    fio.wait(lambda: (work / "original.txt").read_text() == "Original sentinel DIRTY!\nsecond line\n",
             timeout, "Dired return saved original", proc)
    control("x")
    control("c")
    wait_visible("discard unsaved changes?")
    capture("dired-hidden-dirty-protected")
    keys("y")
    proc.wait(timeout=timeout)
    assert proc.returncode == 0, "DIRED_EXPLICIT_DISCARD_EXIT"
    assert (work / "01 open λ.txt").read_text() == "Dired opened λ\n"
    report.update(status="PASS", directory_browse=True, exact_labels=True,
                  dirty_visit_point_preserved=True, stale_entry_refused=True,
                  hidden_dirty_consent=True, clean_exit=True)


def check_scroll(proc, keys, control, wait_visible, capture, trace, work, timeout, report):
    """Native motion reveals minimally and refused boundary motion is inert."""
    control("x")
    keys("1")
    control("x")
    control("f")
    wait_visible("Find file:")
    keys("scroll.txt", "-k", "Return")
    wait_visible("ROW 001 λ")

    def state():
        publication = None
        for line in trace().split('\n'):
            try:
                record = json.loads(line)
            except json.JSONDecodeError:
                continue
            if record.get("diagnostic") == "cursor-publication":
                publication = record
        assert publication is not None, "SCROLL_CURSOR_PUBLICATION_REQUIRED"
        cursors = [c for c in publication["cursors"] if c["active"]]
        assert len(cursors) == 1, "SCROLL_ONE_ACTIVE_CURSOR"
        cursor = cursors[0]
        point = next(p["point_byte"] for p in publication["points"] if p["id"] == cursor["window"])
        rows = [r for r in current_rows(trace()) if r.startswith("ROW ")]
        assert rows, "SCROLL_DOCUMENT_ROWS_REQUIRED"
        return {"top": int(rows[0][4:7]), "cursor": [cursor["x"], cursor["y"]], "point_byte": point}

    previous = state()
    capture("scroll-opened")
    for _ in range(150):
        control("n")
        current = state()
        if current["top"] != previous["top"]:
            assert current["top"] == previous["top"] + 1, "NATIVE_MINIMAL_SCROLL_REVEAL"
            assert current["cursor"] == previous["cursor"], "NATIVE_SCROLL_CARET_STABLE_AT_EDGE"
            capture("scroll-first-edge-reveal")
            report["edge_transition"] = [previous, current]
            break
        previous = current
    else:
        raise AssertionError("SCROLL_BOUNDARY_NOT_EXERCISED")
    keys("-M", "ctrl", "n" * 160, "-m", "ctrl")
    wait_visible("End of buffer")
    stable = state()
    capture("scroll-eof")
    for index in range(3):
        control("n")
        assert state() == stable, "NATIVE_EOF_POINT_VIEWPORT_AND_CARET_STABLE"
        capture(f"scroll-eof-repeat-{index}")
    control("p")
    control("n")
    assert state() == stable, "NATIVE_REVERSE_FROM_EOF_RETURNS_TO_SAME_POINT"
    assert (work / "scroll.txt").read_text() == report["scroll_source"], "SCROLL_NAVIGATION_CHANGED_DISK"
    control("x")
    control("c")
    proc.wait(timeout=timeout)
    assert proc.returncode == 0
    del report["scroll_source"]
    report.update(status="PASS", minimal_reveal=True, eof_stable=True, clean_exit=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--timeout", type=float, default=45)
    parser.add_argument("--close-prompts", action="store_true")
    parser.add_argument("--extended-commands", action="store_true")
    parser.add_argument("--scroll", action="store_true")
    parser.add_argument("--dired", action="store_true")
    parser.add_argument("--width", type=int, default=1600)
    parser.add_argument("--height", type=int, default=1000)
    parser.add_argument("--font")
    parser.add_argument("--font-size", type=int, default=14)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    report = {"binary": str(args.binary.resolve()), "steps": [],
              "prompt_observation": "pre-layout snapshot; visibility qualified by compositor captures"}
    comp = keyboard = proc = None
    with tempfile.TemporaryDirectory(prefix="find-file-live-") as temp:
        runtime = Path(temp)
        runtime.chmod(0o700)
        config = runtime / "config"
        config.mkdir()
        (config / "rc.xml").write_text(live.RC_XML)
        home = runtime / "home"
        home.mkdir()
        work = runtime / "work"
        work.mkdir()
        original = work / "original.txt"
        original.write_text("Original sentinel\nsecond line\n")
        (work / "completion λ.txt").write_text("Completion sentinel λ\n")
        if args.scroll:
            report["scroll_source"] = '\n'.join(f"ROW {i:03d} λ" for i in range(1, 151))
            (work / "scroll.txt").write_text(report["scroll_source"])
        if args.dired:
            (work / "00 child").mkdir()
            (work / "00 child" / "child sentinel.txt").write_text("child")
            (work / "01 open λ.txt").write_text("Dired opened λ\n")
            (work / "02 removed.txt").write_text("removed")
            for index, name in enumerate(["line\nname.txt", "tab\tname.txt", "back\\name.txt"]):
                (work / name).write_text(f"Escaped entry {index}\n")
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": str(home), "LANG": "C.UTF-8", "ROMACS_DIAGNOSTICS": "1",
               "XDG_RUNTIME_DIR": str(runtime), "XDG_CONFIG_HOME": str(home / "config"),
               "XDG_CACHE_HOME": str(home / "cache"), "XDG_DATA_HOME": str(home / "data"),
               "XDG_STATE_HOME": str(home / "state"), "TMPDIR": str(runtime), "WLR_BACKENDS": "headless",
               "WLR_HEADLESS_OUTPUTS": "1", "WLR_RENDERER": "pixman", "WLR_LIBINPUT_NO_DEVICES": "1",
               "LIBSEAT_BACKEND": "noop"}
        if args.font:
            env.update(ROMACS_GUI_FONT=args.font, ROMACS_GUI_FONT_SIZE=str(args.font_size))
        stderr_path = args.output / "binary.stderr"
        try:
            with (args.output / "compositor.log").open("w") as clog, stderr_path.open("w") as blog:
                comp = subprocess.Popen([live.LABWC, "-C", str(config)], env=env, stdout=clog, stderr=clog)
                fio.wait(lambda: any(s.is_socket() for s in runtime.glob("wayland-*")), args.timeout, "private compositor", comp)
                env["WAYLAND_DISPLAY"] = next(s for s in runtime.glob("wayland-*") if s.is_socket()).name
                subprocess.run([live.RANDR, "--output", "HEADLESS-1", "--custom-mode", f"{args.width}x{args.height}"], env=env, check=True, timeout=10)
                keyboard = subprocess.Popen(["/usr/bin/wtype", "-s", "120000", "-k", "Shift_L"], env=env, stdout=blog, stderr=blog)
                time.sleep(0.3)
                proc = subprocess.Popen([str(args.binary.resolve()), "--trace-scene"], env=env, cwd=work, stdin=subprocess.DEVNULL, stdout=blog, stderr=blog)

                def trace():
                    return stderr_path.read_text(errors="replace")

                def visible(text):
                    return any(text in row for row in current_rows(trace()))

                def wait_visible(text):
                    fio.wait(lambda: visible(text), args.timeout, f"observed {text!r}", proc)

                def keys(*parts):
                    subprocess.run(["/usr/bin/wtype", "-d", "75", *parts], env=env, check=True, timeout=15)
                    time.sleep(0.2)

                def control(key):
                    keys("-M", "ctrl", key, "-m", "ctrl")

                def extended(command):
                    keys("-M", "alt", "x", "-m", "alt")
                    wait_visible("M-x ")
                    capture(f"mx-{len(report['steps'])}-{command}-prompt")
                    keys(command)
                    keys("-k", "Tab")
                    keys("-k", "Return")

                def find():
                    if args.extended_commands:
                        extended("find-file")
                    else:
                        control("x")
                        control("f")
                    wait_visible("Find file:")

                def switch_prompt():
                    if args.extended_commands:
                        extended("switch-to-buffer")
                    else:
                        control("x")
                        keys("b")
                    wait_visible("Switch to buffer")

                def capture(name):
                    subprocess.run(["grim", str(args.output / f"{name}.png")], env=env, check=True, timeout=10)
                    report["steps"].append({"name": name, "rows": current_rows(trace())})

                fio.wait(lambda: '"ready":true' in trace(), args.timeout, "native GUI ready", proc)
                if args.scroll:
                    check_scroll(proc, keys, control, wait_visible, capture, trace,
                                 work, args.timeout, report)
                    return
                if args.dired:
                    check_dired(proc, keys, control, wait_visible, capture, trace,
                                work, args.timeout, report)
                    return
                if args.close_prompts:
                    check_prompt_close(proc, keys, control, wait_visible, capture, trace,
                                       work, args.timeout, report)
                    return
                control("x")
                keys("1")
                find()
                keys("orig")
                keys("-k", "Tab")
                keys("-k", "Return")
                wait_visible("Original sentinel")
                clip = fio.document_clip(trace(), ["Original sentinel", "second line"])
                control("e")
                keys(" dirty")
                wait_visible("Original sentinel dirty")
                find()
                capture("prompt")
                keys("comp")
                keys("-k", "Tab")
                wait_visible("/completion λ.txt")
                capture("completed")
                keys("-k", "Return")
                wait_visible("Completion sentinel λ")
                capture("visited")
                switch_prompt()
                capture("switch-prompt")
                keys("orig")
                keys("-k", "Tab")
                capture("switch-completed")
                keys("-k", "Return")
                wait_visible("Original sentinel dirty")
                keys("!")
                wait_visible("Original sentinel dirty!")
                assert original.read_text() == "Original sentinel\nsecond line\n", "revisit wrote dirty text"
                capture("dirty-revisit-point")
                before = fio.document_rows(trace(), clip)
                find()
                keys("aborted.txt")
                capture("abort-prompt")
                control("g")
                fio.wait(lambda: not visible("Find file:"), args.timeout, "cancelled filename prompt", proc)
                assert fio.document_rows(trace(), clip) == before, "C-g changed document"
                keys("?")
                wait_visible("Original sentinel dirty!?")
                assert not (work / "aborted.txt").exists(), "C-g created a file"
                capture("cancelled-point")
                control("x")
                control("s")
                fio.wait(lambda: original.read_text() == "Original sentinel dirty!?\nsecond line\n", args.timeout, "saved original via C-x C-s", proc)
                capture("original-saved")
                switch_prompt()
                keys("comp")
                keys("-k", "Tab")
                keys("-k", "Return")
                wait_visible("Completion sentinel λ")
                capture("switch-second-buffer")
                # Switching to the buffer already shown changes nothing but
                # the prompt; the prompt must still leave the screen.
                switch_prompt()
                keys("comp")
                keys("-k", "Tab")
                keys("-k", "Return")
                fio.wait(lambda: not visible("Switch to buffer"), args.timeout, "prompt gone after switching to the shown buffer", proc)
                assert fio.document_rows(trace(), clip) == ["Completion sentinel λ"], "switch to the shown buffer changed document"
                capture("switch-same-buffer")
                switch_prompt()
                keys("cancelled buffer")
                control("g")
                fio.wait(lambda: not visible("Switch to buffer"), args.timeout, "cancelled buffer switch", proc)
                assert fio.document_rows(trace(), clip) == ["Completion sentinel λ"], "buffer cancel changed document"
                capture("switch-cancelled")
                find()
                keys("empty.txt")
                keys("-k", "Return")
                fio.wait(lambda: not visible("Find file:"), args.timeout, "empty new visited buffer", proc)
                assert not (work / "empty.txt").exists(), "empty visit wrote before save"
                control("x")
                control("s")
                fio.wait(lambda: (work / "empty.txt").exists(), args.timeout, "explicit empty save", proc)
                assert (work / "empty.txt").read_bytes() == b"", "empty save was not empty"
                capture("empty-saved")
                find()
                keys("new λ.txt")
                keys("-k", "Return")
                fio.wait(lambda: not visible("Find file:"), args.timeout, "new visited buffer", proc)
                assert not (work / "new λ.txt").exists(), "new visit created file before save"
                keys("New sentinel λ")
                wait_visible("New sentinel λ")
                capture("new-unsaved")
                control("x")
                control("s")
                fio.wait(lambda: (work / "new λ.txt").exists(), args.timeout, "C-x C-s created new file", proc)
                assert (work / "new λ.txt").read_text() == "New sentinel λ", "new save bytes differ"
                capture("new-saved")
                find()
                keys("-M", "alt", "p", "-m", "alt")
                wait_visible("/new λ.txt")
                capture("history-recall")
                control("g")
                find()
                keys("./")
                keys("-k", "Return")
                wait_visible(str(work) + ":")
                capture("directory-visited")
                keys("q")
                wait_visible("New sentinel λ")
                assert fio.document_rows(trace(), clip) == ["New sentinel λ"], "Dired return replaced document"
                capture("directory-returned")
                switch_prompt()
                keys("-M", "alt", "p", "-m", "alt")
                wait_visible("completion λ.txt")
                capture("buffer-history-recall")
                keys("-k", "Return")
                wait_visible("Completion sentinel λ")
                control("x")
                control("c")
                proc.wait(timeout=15)
                assert proc.returncode == 0, f"clean exit failed: {proc.returncode}"
                report["clean_exit"] = True
                report["status"] = "PASS"
                report["new_file_bytes_hex"] = (work / "new λ.txt").read_bytes().hex()
                report["original_saved_bytes_exact"] = original.read_text() == "Original sentinel dirty!?\nsecond line\n"
        except Exception as error:
            report["status"] = "FAIL"
            report["error"] = repr(error)
            if proc is not None and proc.poll() is None:
                fio.shot(args.output / "failure.png", env, report)
            raise
        finally:
            for child in (proc, keyboard, comp):
                if child is not None and child.poll() is None:
                    child.terminate()
                    try:
                        child.wait(timeout=8)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.wait(timeout=8)
            (args.output / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
            (args.output / "INDEX.txt").write_text("\n".join(f"{s['name']}.png: {s['name']}" for s in report["steps"]) + "\n")
    print(json.dumps(report, ensure_ascii=False))


if __name__ == "__main__":
    main()
