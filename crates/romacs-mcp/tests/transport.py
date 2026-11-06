"""Real OS pipes/sockets; fixture peer is transport-only, not MCP proof.
Run: python3 transport.py /absolute/path/to/romacs-mcp
"""
import json
import os
import pathlib
import select
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest

BINARY = pathlib.Path(sys.argv.pop(1)).resolve()
BUDGET = 300


class Transport(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="mcp-relay-", dir=os.environ["TMPDIR"])
        self.root = pathlib.Path(self.tmp.name)
        self.path = str(self.root / "peer")
        self.listener = socket.socket(socket.AF_UNIX)
        self.listener.bind(self.path)
        self.listener.listen(4)
        self.listener.settimeout(3)
        self.children = []
        self.sockets = []
        self.threads = []
        self.errors = []

    def tearDown(self):
        for p in self.children:
            if p.poll() is None:
                p.kill()
            p.wait(timeout=3)
            for f in [p.stdin, p.stdout, p.stderr]:
                if f and not f.closed:
                    f.close()
        for s in self.sockets:
            s.close()
        self.listener.close()
        for t in self.threads:
            t.join(3)
            self.assertFalse(t.is_alive(), "fixture worker leaked")
        self.tmp.cleanup()
        if self.errors:
            raise self.errors[0]

    def launch(self, path=None, **kwargs):
        p = subprocess.Popen([str(BINARY), "--socket", path or self.path,
                              "--timeout-ms", str(BUDGET)], stdin=subprocess.PIPE,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, **kwargs)
        self.children.append(p)
        return p

    def accept(self):
        peer, _ = self.listener.accept()
        peer.settimeout(3)
        self.sockets.append(peer)
        return peer

    def thread(self, fn):
        def wrapped():
            try:
                fn()
            except Exception as e:
                self.errors.append(e)
        t = threading.Thread(target=wrapped)
        t.start()
        self.threads.append(t)
        return t

    def finish(self, p, success=True, needle=None):
        start = time.monotonic()
        code = p.wait(timeout=2)
        self.assertLess(time.monotonic() - start, 1.2)
        self.assertEqual(code, 0 if success else 1)
        err = p.stderr.read()
        if needle:
            self.assertIn(needle, err)
        elif success:
            self.assertEqual(err, b"")
        return p.stdout.read()

    def test_utf8_ndjson_fragmented_and_coalesced_blind(self):
        payload = ('{"x":"γειά 👋","escaped":"a\\nb"}\n'
                   '{"not":"a protocol request"}\n\n').encode() + b'\xff\x00not-json\r\n'
        p = self.launch()
        peer = self.accept()
        received = bytearray()
        def server():
            while True:
                data = peer.recv(7)
                if not data:
                    break
                received.extend(data)
                # Deliberately split UTF8 sequences and framing both ways.
                for byte in data:
                    peer.sendall(bytes([byte]))
            peer.shutdown(socket.SHUT_WR)
        self.thread(server)
        for chunk in [payload[:1], payload[1:8], payload[8:]]:
            p.stdin.write(chunk)
            p.stdin.flush()
        p.stdin.close()
        self.assertEqual(self.finish(p), payload)
        self.assertEqual(bytes(received), payload)

    def test_large_duplex_exact_bytes(self):
        payload = bytes(range(256)) * 8192
        p = self.launch()
        peer = self.accept()
        def server():
            while data := peer.recv(16384):
                peer.sendall(data)
            peer.shutdown(socket.SHUT_WR)
        self.thread(server)
        out, err = p.communicate(payload, timeout=6)
        self.assertEqual(p.returncode, 0)
        self.assertEqual(out, payload)
        self.assertEqual(err, b"")

    def test_eof_drains_delayed_response_and_keeps_listener(self):
        # A client that writes its requests and closes its input at once (a
        # shell pipe) still gets the replies: the relay does not pass the
        # EOF on, which would end the server's side before it answered, and
        # closes once the server has been quiet for its timeout.
        p = self.launch()
        peer = self.accept()
        def server():
            request = b""
            while not request.endswith(b"second\n"):
                request += peer.recv(1024)
            self.assertEqual(request, b"request\nsecond\n")
            time.sleep(0.2)
            peer.sendall(b"final outstanding\n")
            time.sleep(0.2)
            peer.sendall(b"and the second\n")
            self.assertEqual(peer.recv(1), b"")
        self.thread(server)
        p.stdin.write(b"request\nsecond\n")
        p.stdin.close()
        start = time.monotonic()
        code = p.wait(timeout=3)
        self.assertEqual(code, 0)
        self.assertGreater(time.monotonic() - start, 0.4 + BUDGET / 1000 * 0.9)
        self.assertEqual(p.stdout.read(), b"final outstanding\nand the second\n")
        # Relay closes only its connection; it neither unlinks nor kills server.
        self.assertTrue(pathlib.Path(self.path).exists())
        next_client = socket.socket(socket.AF_UNIX)
        self.sockets.append(next_client)
        next_client.connect(self.path)
        self.accept()

    def test_eof_no_peer_eof_deadline_and_peer_recovers(self):
        p = self.launch()
        peer = self.accept()
        p.stdin.close()
        # Quiet for the timeout after the input ended: the relay closes.
        self.assertEqual(self.finish(p), b"")
        self.assertEqual(peer.recv(1), b"")
        q = self.launch()
        successor = self.accept()
        successor.sendall(b"recovered\n")
        successor.close()
        self.assertEqual(self.finish(q), b"recovered\n")

    def test_peer_close_exits_with_stdin_still_open_blocked(self):
        p = self.launch()
        self.accept().close()
        self.assertEqual(self.finish(p), b"")
        self.assertFalse(p.stdin.closed)

    def test_stdout_broken_cleanup(self):
        p = self.launch()
        peer = self.accept()
        p.stdout.close()
        peer.sendall(b"response\n")
        p.wait(timeout=2)
        self.assertEqual(p.returncode, 1)
        self.assertIn(b"socket to stdout", p.stderr.read())
        self.assertEqual(peer.recv(1), b"")

    def test_unread_stdout_backpressure_bounded_and_exit(self):
        p = self.launch()
        peer = self.accept()
        peer.setblocking(False)
        sent = 0
        deadline = time.monotonic() + 2
        block = b"x" * 16384
        while time.monotonic() < deadline and p.poll() is None:
            try:
                sent += peer.send(block)
            except BlockingIOError:
                select.select([], [peer], [], 0.03)
            except (BrokenPipeError, ConnectionResetError):
                break
        self.assertGreater(sent, 16384)
        # No stdout reader until after deadline: this genuinely blocks its worker.
        out = self.finish(p, False, b"stdout write deadline")
        self.assertLess(len(out), sent)
        self.assertLess(sent, 2 * 1024 * 1024, "transport did not backpressure")

    def test_unread_upstream_deadline_closes_own_connection(self):
        p = self.launch()
        peer = self.accept()
        os.set_blocking(p.stdin.fileno(), False)
        sent = 0
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline and p.poll() is None:
            try:
                sent += os.write(p.stdin.fileno(), b"x" * 16384)
            except BlockingIOError:
                select.select([], [p.stdin], [], 0.03)
            except BrokenPipeError:
                break
        self.assertGreater(sent, 16384)
        self.assertEqual(self.finish(p, False, b"upstream write deadline"), b"")
        self.assertLess(sent, 2 * 1024 * 1024)
        total = 0
        while data := peer.recv(65536):
            total += len(data)
        self.assertLess(total, sent)

    def test_saturated_stderr_cannot_hang_diagnostic_exit(self):
        read, write = os.pipe()
        try:
            os.set_blocking(write, False)
            while True:
                try:
                    os.write(write, b"x" * 4096)
                except BlockingIOError:
                    break
            os.set_blocking(write, True)
            p = subprocess.Popen([str(BINARY), "--socket", str(self.root / "absent")],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=write)
            self.children.append(p)
            p.wait(timeout=1)
            self.assertEqual(p.returncode, 1)
            self.assertEqual(p.stdout.read(), b"")
        finally:
            os.close(write)
            os.close(read)

    def test_explicit_selection_no_fallback_or_automatic_reconnect(self):
        p = self.launch()
        original = self.accept()
        # Replace the pathname while connection is live: original inode stays bound.
        os.unlink(self.path)
        successor = socket.socket(socket.AF_UNIX)
        self.sockets.append(successor)
        successor.bind(self.path)
        successor.listen(4)
        successor.settimeout(0.15)
        original.sendall(b"original\n")
        self.assertEqual(p.stdout.readline(), b"original\n")
        original.close()
        self.assertEqual(self.finish(p), b"")
        with self.assertRaises(TimeoutError):
            successor.accept()
        q = self.launch()
        new_peer, _ = successor.accept()
        self.sockets.append(new_peer)
        new_peer.sendall(b"successor\n")
        new_peer.close()
        self.assertEqual(self.finish(q), b"successor\n")
        missing = self.launch(str(self.root / "missing"))
        self.assertEqual(self.finish(missing, False, b"connect:"), b"")
        self.assertTrue(pathlib.Path(self.path).exists())

    def test_full_connect_backlog_deadline(self):
        self.listener.close()
        os.unlink(self.path)
        self.listener = socket.socket(socket.AF_UNIX)
        self.listener.bind(self.path)
        self.listener.listen(0)
        filler = socket.socket(socket.AF_UNIX)
        self.sockets.append(filler)
        filler.connect(self.path)
        p = self.launch()
        self.assertEqual(self.finish(p, False, b"connect deadline/worker"), b"")
        self.assertTrue(pathlib.Path(self.path).exists())

    # The reconnect relay, against a session that accepts and goes silent.
    def relay(self, timeout="30"):
        child = subprocess.Popen([str(BINARY), "--socket", self.path, "--reconnect", "--session", "s",
                                  "--reply-timeout", timeout],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        self.children.append(child)
        peer, _ = self.listener.accept()
        self.sockets.append(peer)
        return child, peer

    def say(self, child, message):
        child.stdin.write((json.dumps(message) + "\n").encode())
        child.stdin.flush()

    def hear(self, child, wanted, seconds=5):
        deadline = time.monotonic() + seconds
        buffer = b""
        while time.monotonic() < deadline:
            ready, _, _ = select.select([child.stdout], [], [], .1)
            if not ready:
                continue
            buffer += os.read(child.stdout.fileno(), 1 << 16)
            while b"\n" in buffer:
                line, buffer = buffer.split(b"\n", 1)
                message = json.loads(line)
                if message.get("id") == wanted:
                    return message
        self.fail(f"no reply to {wanted}")

    def endpoint(self, answer=b'{"status":"ok"}\n'):
        server = socket.socket(socket.AF_UNIX)
        server.bind(str(self.root / "inspect.sock"))
        server.listen(4)
        self.sockets.append(server)

        def serve():
            while True:
                try:
                    peer, _ = server.accept()
                except OSError:
                    return
                peer.recv(16)
                peer.sendall(answer)
                peer.close()
        threading.Thread(target=serve, daemon=True).start()

    def test_reconnect_discovery_and_inspection_with_a_silent_session(self):
        self.endpoint()
        child, _ = self.relay()
        self.say(child, {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "v"}})
        reply = self.hear(child, 1)
        self.assertEqual(reply["result"]["protocolVersion"], "v")
        self.say(child, {"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
        names = [tool["name"] for tool in self.hear(child, 2)["result"]["tools"]]
        self.assertEqual(names, ["romacs_debug_inspect"])
        self.say(child, {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "romacs_debug_inspect"}})
        reply = self.hear(child, 3, 2)
        self.assertIs(reply["result"]["isError"], False)
        self.assertIn("ok", reply["result"]["content"][0]["text"])

    def test_reconnect_inspection_past_a_full_session_socket(self):
        self.endpoint()
        child, _ = self.relay()
        big = "x" * (1 << 20)
        for n in range(100, 180):
            self.say(child, {"jsonrpc": "2.0", "id": n, "method": "tools/call", "params": {"name": "romacs_eval", "arguments": {"code": big}}})
        self.say(child, {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "romacs_debug_inspect"}})
        self.assertIs(self.hear(child, 3, 2)["result"]["isError"], False)

    def test_reconnect_outcomes_of_written_and_queued_calls(self):
        child, peer = self.relay("1")
        self.say(child, {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "t"}})
        peer.settimeout(3)
        received = b""
        while b'"id":1' not in received:
            received += peer.recv(1 << 16)
        reply = self.hear(child, 1, 15)
        self.assertIn("outcome unknown", reply["result"]["content"][0]["text"])

    def test_reconnect_a_call_queued_behind_a_stuck_socket_is_withdrawn(self):
        child, _ = self.relay("1")
        big = "x" * (1 << 20)
        for n in range(100, 110):
            self.say(child, {"jsonrpc": "2.0", "id": n, "method": "tools/call", "params": {"name": "t", "arguments": {"code": big}}})
        reply = self.hear(child, 109)
        self.assertIn("not executed", reply["result"]["content"][0]["text"])

    def test_reconnect_a_session_that_answers_lists_both(self):
        child, peer = self.relay()
        self.say(child, {"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
        peer.settimeout(3)
        received = b""
        while b"tools/list" not in received:
            received += peer.recv(1 << 16)
        peer.sendall(b'{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"romacs_eval"}]}}\n')
        names = [tool["name"] for tool in self.hear(child, 2)["result"]["tools"]]
        self.assertEqual(names, ["romacs_eval", "romacs_debug_inspect"])

    def hear_method(self, child, method, seconds=5):
        deadline = time.monotonic() + seconds
        buffer = b""
        while time.monotonic() < deadline:
            ready, _, _ = select.select([child.stdout], [], [], .1)
            if not ready:
                continue
            buffer += os.read(child.stdout.fileno(), 1 << 16)
            if method.encode() in buffer:
                return
        self.fail(f"no {method}")

    def test_reconnect_inspects_through_a_route(self):
        self.endpoint()
        route = self.root / "agents"
        route.mkdir()
        (route / "route").symlink_to(self.path)
        self.path, real = str(route / "route"), self.path
        child, _ = self.relay()
        self.path = real
        self.say(child, {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "romacs_debug_inspect"}})
        self.assertIs(self.hear(child, 3, 2)["result"]["isError"], False)

    def test_reconnect_a_session_lost_during_discovery_settles_it(self):
        child, peer = self.relay()
        self.say(child, {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "v"}})
        time.sleep(.2)
        peer.close()
        self.assertEqual(self.hear(child, 1, 2)["result"]["protocolVersion"], "v")

    def test_reconnect_the_list_changes_when_the_session_returns(self):
        child, peer = self.relay()
        peer.close()
        time.sleep(.2)
        self.say(child, {"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
        self.assertEqual(len(self.hear(child, 2)["result"]["tools"]), 1)
        back, _ = self.listener.accept()
        self.sockets.append(back)
        back.settimeout(5)
        received = b""
        while b"romacs-relay-probe" not in received:
            received += back.recv(1 << 16)
        line = next(l for l in received.split(b"\n") if b"romacs-relay-probe" in l)
        back.sendall(json.dumps({"jsonrpc": "2.0", "id": json.loads(line)["id"], "result": {"tools": []}}).encode() + b"\n")
        self.hear_method(child, "notifications/tools/list_changed")

    def test_reconnect_a_withdrawn_call_stays_unsent_when_its_id_returns(self):
        child, peer = self.relay("1")
        big = "x" * (1 << 20)
        for n in range(100, 109):
            self.say(child, {"jsonrpc": "2.0", "id": n, "method": "tools/call", "params": {"name": "t", "arguments": {"code": big}}})
        self.say(child, {"jsonrpc": "2.0", "id": 109, "method": "tools/call", "params": {"name": "first"}})
        self.assertIn("not executed", self.hear(child, 109)["result"]["content"][0]["text"])
        self.say(child, {"jsonrpc": "2.0", "id": 109, "method": "tools/call", "params": {"name": "second"}})
        peer.settimeout(.5)
        received = b""
        try:
            while True:
                part = peer.recv(1 << 20)
                if not part:
                    break
                received += part
        except socket.timeout:
            pass
        self.assertIn(b'"second"', received)
        self.assertNotIn(b'"first"', received)

    def test_reconnect_inspections_are_bounded(self):
        silent = socket.socket(socket.AF_UNIX)
        silent.bind(str(self.root / "inspect.sock"))
        silent.listen(32)
        self.sockets.append(silent)
        child, _ = self.relay()
        for n in range(1, 21):
            self.say(child, {"jsonrpc": "2.0", "id": n, "method": "tools/call", "params": {"name": "romacs_debug_inspect"}})
        reply = self.hear(child, 20, 2)
        self.assertIn("too many", reply["result"]["content"][0]["text"])

    def answer_probes(self, peer, seconds=6):
        """Read what the relay sends, and answer each probe at once."""
        peer.settimeout(.1)
        deadline = time.monotonic() + seconds
        buffer = b""
        while time.monotonic() < deadline:
            try:
                part = peer.recv(1 << 20)
            except socket.timeout:
                continue
            if not part:
                return
            buffer += part
            while b"\n" in buffer:
                line, buffer = buffer.split(b"\n", 1)
                if b"romacs-relay-probe" in line:
                    peer.sendall(json.dumps({"jsonrpc": "2.0", "id": json.loads(line)["id"], "result": {"tools": []}}).encode() + b"\n")
                    return

    def test_reconnect_the_change_follows_the_list_it_replaces(self):
        child, peer = self.relay()
        self.say(child, {"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
        threading.Thread(target=self.answer_probes, args=(peer,), daemon=True).start()
        lines = []
        buffer = b""
        deadline = time.monotonic() + 6
        while time.monotonic() < deadline and not any(b"list_changed" in line for line in lines):
            ready, _, _ = select.select([child.stdout], [], [], .1)
            if ready:
                buffer += os.read(child.stdout.fileno(), 1 << 16)
                *complete, buffer = buffer.split(b"\n")
                lines += complete
        self.assertEqual(len(lines), 2, lines)
        self.assertEqual(json.loads(lines[0])["id"], 2)
        self.assertIn(b"list_changed", lines[1])

    def test_reconnect_a_probe_refused_by_a_full_queue_is_retried(self):
        child, peer = self.relay("60")
        big = "x" * (1 << 20)
        for n in range(100, 180):
            self.say(child, {"jsonrpc": "2.0", "id": n, "method": "tools/call", "params": {"name": "t", "arguments": {"code": big}}})
        self.say(child, {"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
        self.assertEqual(len(self.hear(child, 2)["result"]["tools"]), 1)
        threading.Thread(target=self.answer_probes, args=(peer, 20), daemon=True).start()
        self.hear_method(child, "notifications/tools/list_changed", 20)

    def test_cli_requires_one_absolute_socket(self):
        for args in [["--socket", "relative"], ["--socket"], ["--server"], ["--server", "a/b"],
                     ["--socket", self.path, "--socket", self.path],
                     ["--socket", self.path, "--timeout-ms", "0"],
                     ["--socket", self.path, "--timeout-ms", "60001"],
                     ["--unknown"], ["--socket", self.path, "--start-editor"]]:
            p = subprocess.run([str(BINARY), *args], capture_output=True, timeout=2)
            self.assertEqual(p.returncode, 1)
            self.assertEqual(p.stdout, b"")
            self.assertIn(b"usage:", p.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
