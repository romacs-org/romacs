#!/usr/bin/env python3
"""Real private-Wayland compositor capture smoke (not the G1a text oracle)."""
import argparse
import os
from pathlib import Path
import select
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser()
parser.add_argument("--labwc", required=True)
parser.add_argument("--binary", required=True)
parser.add_argument("--output", required=True)
args = parser.parse_args()
out = Path(args.output).resolve()
out.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory(prefix="s115-native-", dir=tempfile.gettempdir()) as work:
    runtime = Path(work)
    runtime.chmod(0o700)
    config = runtime / "config"
    config.mkdir()
    env = os.environ.copy()
    env.update(XDG_RUNTIME_DIR=work, WLR_BACKENDS="headless", WLR_HEADLESS_OUTPUTS="1",
               WLR_RENDERER="pixman", LIBSEAT_BACKEND="noop", XDG_CONFIG_HOME=str(config))
    env.pop("DISPLAY", None)
    env.pop("WAYLAND_DISPLAY", None)
    log = (out / "compositor.log").open("w")
    compositor = subprocess.Popen([args.labwc, "-C", str(config)], env=env, stdout=log, stderr=log)
    probe = None
    try:
        deadline = time.monotonic() + 15
        while True:
            sockets = list(runtime.glob("wayland-*"))
            sockets = [p for p in sockets if p.is_socket()]
            if sockets:
                break
            if compositor.poll() is not None or time.monotonic() >= deadline:
                raise RuntimeError("private compositor failed to publish a socket")
            time.sleep(0.05)
        socket = sockets[0]
        env["WAYLAND_DISPLAY"] = socket.name
        probe = subprocess.Popen([args.binary, str(socket)], env=env, text=True,
                                 stdout=subprocess.PIPE, stderr=(out / "probe.stderr").open("w"))
        for revision, color in [(1, (255, 0, 0)), (2, (255, 255, 255))]:
            deadline = time.monotonic() + 30
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not select.select([probe.stdout], [], [], remaining)[0]:
                    raise RuntimeError(f"no activation for frame {revision}")
                line = probe.stdout.readline()
                if not line:
                    raise RuntimeError(f"probe exited {probe.poll()} before activation")
                print(line.strip(), flush=True)
                if line.strip() == f"activated {revision}":
                    break
            time.sleep(0.5)
            capture = out / f"frame-{revision}.png"
            subprocess.run(["grim", str(capture)], env=env, check=True, timeout=10)
            rgb = subprocess.check_output(["magick", str(capture), "-alpha", "off", "-depth", "8", "RGB:-"])
            expected = bytes(color)
            count = sum(rgb[index:index + 3] == expected for index in range(0, len(rgb), 3))
            assert count > 10000, f"frame {revision}: native capture has only {count} {color} pixels"
            print(f"CAPTURE frame={revision} rgb={color} pixels={count}", flush=True)
        result = probe.wait(timeout=10)
        assert result == 0, f"probe exited {result}"
    finally:
        if probe is not None and probe.poll() is None:
            probe.terminate()
            probe.wait(timeout=10)
        compositor.terminate()
        compositor.wait(timeout=10)
        log.close()
