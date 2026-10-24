#!/usr/bin/env python3
"""Official legacy MCP SDK proof against native Lisp through the byte relay.

Requires mcp==1.30.0. ROMACS and ROMACS_MCP_RELAY are explicit executables.
This is legacy 2025-11-25 interoperability, not current-protocol conformance.
"""
import argparse
import asyncio
from importlib.metadata import version
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client


async def exercise(relay, socket, env, pid, rows, output):
    parameters = StdioServerParameters(command=str(relay), args=["--socket", str(socket)], env=env)
    with (output / "relay-stderr.log").open("w") as errors:
        async with stdio_client(parameters, errlog=errors) as (read, write):
            async with ClientSession(read, write) as session:
                initialized = await session.initialize()
                rows.append({"name": "sdk-legacy-initialize", "pass": initialized.protocolVersion == "2025-11-25", "observed": initialized.model_dump(mode="json", by_alias=True)})
                listed = await session.list_tools()
                rows.append({"name": "sdk-discovery", "pass": len(listed.tools) == 8, "observed": listed.model_dump(mode="json", by_alias=True)})

                async def tool(name, args, data=False):
                    result = await session.call_tool(name, args)
                    rows.append({"name": name, "pass": not result.isError, "observed": result.model_dump(mode="json", by_alias=True)})
                    assert not result.isError, result
                    return json.loads(result.content[0].text) if data else result.content[0].text

                identity = await tool("romacs_identity", {}, True)
                assert identity["pid"] == pid
                instance = identity["instance"]
                assert await tool("romacs_eval", {"instance": instance, "code": "(+ 20 22)"}) == "42"
                async def progress(progress: float, total: float | None, message: str | None) -> None:
                    pass  # A server need not emit optional progress notifications.
                progressed = await session.call_tool("romacs_eval", {"instance": instance, "code": "(+ 20 22)"},
                                                     progress_callback=progress)
                progress_result = progressed.model_dump(mode="json", by_alias=True)
                rows.append({"name": "sdk-legacy-progress-eval", "pass": not progress_result["isError"]
                             and progress_result["content"][0]["text"] == "42", "observed": progress_result})
                assert rows[-1]["pass"], progressed
                await tool("romacs_eval", {"instance": instance, "code": '(switch-to-buffer (get-buffer-create "*SDK study*")) (insert "learner") (goto-char 2) (setq sdk-study (list (selected-window) (window-buffer) (point) (window-start) (buffer-string) buffer-undo-list))'})
                claim = await tool("romacs_companion_claim", {"instance": instance}, True)
                edit = {"instance": instance, "operationId": "sdk-edit", "handle": claim["handle"], "tick": claim["tick"], "start": 1, "end": 1, "text": "SDK Ελλάδα\n"}
                receipt = await tool("romacs_companion_edit", edit, True)
                assert receipt["status"] == "succeeded"
                assert receipt == await tool("romacs_companion_receipt", {"instance": instance, "operationId": "sdk-edit"}, True)
                state = await tool("romacs_companion_read", {"instance": instance, "handle": claim["handle"]}, True)
                assert state["text"] == edit["text"]
                await tool("romacs_companion_undo", {"instance": instance, "handle": claim["handle"], "tick": state["tick"]}, True)
                after = await tool("romacs_companion_read", {"instance": instance, "handle": claim["handle"]}, True)
                assert after["text"] == ""
                preserved = await tool("romacs_eval", {"instance": instance, "code": '(with-current-buffer (window-buffer (selected-window)) (equal sdk-study (list (selected-window) (window-buffer) (point) (window-start) (buffer-string) buffer-undo-list)))'})
                assert preserved == "t"
                rows.append({"name": "sdk-native-companion-study-journey", "pass": True})
    return instance


def run(output):
    assert version("mcp") == "1.30.0", version("mcp")
    executable = Path(os.environ["ROMACS"]).resolve()
    relay = Path(os.environ["ROMACS_MCP_RELAY"]).resolve()
    repo = Path(__file__).resolve().parents[2]
    root = Path(tempfile.mkdtemp(prefix="nmcp-sdk-", dir=os.environ["TMPDIR"]))
    for name in ("home", "run", "cache", "config", "state", "data"):
        (root / name).mkdir(mode=0o700)
    env = {"HOME": str(root/"home"), "XDG_RUNTIME_DIR": str(root/"run"), "TMPDIR": str(root), "PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "RUST_LOG": "error"}
    env.update({"XDG_"+name.upper()+"_HOME": str(root/name) for name in ("cache", "config", "state", "data")})
    socket = root/"run/mcp"
    command = [str(executable), "-Q", "--fg-daemon=native-mcp-sdk", "-L", str(repo/"rlisp"), "--eval", f'(setq server-socket-dir {json.dumps(str(root/"run"))})', "--eval", '(progn (require \'romacs-mcp-companion) (romacs-mcp-enable-companion-tools))', "--eval", f'(romacs-mcp-start {json.dumps(str(socket))})']
    output.mkdir(parents=True, exist_ok=True)
    log = (output/"daemon.log").open("w")
    rows = []
    process = subprocess.Popen(command, env=env, cwd=root, stdout=log, stderr=subprocess.STDOUT)
    try:
        deadline = time.monotonic()+20
        while not socket.exists():
            if process.poll() is not None:
                raise RuntimeError("Owned daemon exited")
            if time.monotonic() > deadline:
                raise TimeoutError("Owned daemon readiness")
            time.sleep(.01)
        asyncio.run(exercise(relay, socket, env, process.pid, rows, output))
        assert all(row["pass"] for row in rows)
        assert process.poll() is None, "Relay EOF killed editor"
        client = executable.with_name("romacsclient")
        peer_count = subprocess.run([str(client), "-s", str(root/"run/native-mcp-sdk"), "--eval", "(length romacs-mcp--peers)"], env=env, capture_output=True, text=True, timeout=5)
        assert peer_count.stdout.strip() == "0", peer_count
        rows.append({"name": "sdk-relay-eof-only-retires-connection", "pass": True})
        subprocess.run([str(client), "-s", str(root/"run/native-mcp-sdk"), "--eval", "(kill-emacs 0)"], env=env, capture_output=True, timeout=5)
        assert process.wait(timeout=5) == 0 and not socket.exists()
        rows.append({"name": "sdk-normal-owned-cleanup", "pass": True})
    finally:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill(); process.wait(timeout=5)
        log.close()
        shutil.rmtree(root)
        receipt = {"sdk": version("mcp"), "executable": str(executable), "relay": str(relay), "checks": rows, "count": len(rows), "passed": sum(row["pass"] for row in rows), "fixtureRemoved": not root.exists()}
        (output/"sdk.json").write_text(json.dumps(receipt, indent=2, ensure_ascii=False)+"\n")
        print(json.dumps({key: receipt[key] for key in ("count", "passed", "fixtureRemoved")}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    run(parser.parse_args().output)
