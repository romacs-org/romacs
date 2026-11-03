"""A small MCP client for live harnesses: drive a Romacs session through its
MCP tools (romacs_eval, romacs_buffer_read, romacs_observe, romacs_send_keys,
romacs_wait_idle, ...) instead of wtype keystrokes and ad-hoc diagnostics.

It runs the `romacs-mcp' relay beside the Romacs binary as its child and
speaks JSON-RPC over the relay's stdio, as a harness (Claude Code, Codex)
does, so a harness exercises the same path an agent takes.  Standard
library only.

    from romacs_mcp_client import RomacsMcp
    with RomacsMcp.for_server(server) as mcp:       # a server_live.Server
        mcp.call('romacs_eval', code='(+ 1 2)')     # -> '3'
        mcp.call('romacs_buffer_read', name='*scratch*', start=1, maxChars=200)
        mcp.call('romacs_observe')                  # the frame as rows of text

`call' fills `instance' (romacs_identity) and returns the tool's result,
parsed when it is JSON; a tool error raises `ToolError'.  `tool' returns
the raw (request id, text, is_error) for callers that judge errors
themselves.  `tools()' lists what the endpoint offers.

The agent tools (romacs_buffer_edit, romacs_file_write, romacs_visit,
romacs_call_outcome, and the editor and session tools) are registered when
`romacs-agent' is loaded; `for_server(..., agent_tools=True)' loads it.
"""
import json
import os
from pathlib import Path
import subprocess
import threading

PROTOCOL = '2025-11-25'


class ToolError(Exception):
    """A tool's error result (isError) or a JSON-RPC error."""


class RomacsMcp:
    def __init__(self, argv, env=None, name='romacs-harness'):
        self.process = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        text=True, bufsize=1, env=env)
        self.replies, self.next_id, self.lock = {}, 0, threading.Lock()
        self.arrived = threading.Condition(self.lock)
        threading.Thread(target=self._read, daemon=True).start()
        _, reply = self.request('initialize', {'protocolVersion': PROTOCOL, 'capabilities': {},
                                               'clientInfo': {'name': name, 'version': '1'}})
        if 'error' in reply:
            raise ToolError(f"initialize: {reply['error']}")
        self.send({'jsonrpc': '2.0', 'method': 'notifications/initialized'})
        self.instance, self._schemas = None, None

    @classmethod
    def connect(cls, socket, relay, env=None, extra=()):
        """A client of the endpoint at SOCKET through RELAY (romacs-mcp);
        EXTRA are further relay arguments (`--reconnect', `--session ID')."""
        return cls([str(relay), '--socket', str(socket), *extra], env)

    @classmethod
    def for_server(cls, server, agent_tools=False):
        """A client of a server_live.Server's session: its native socket,
        the relay beside its binary."""
        socket = Path(server.env['XDG_RUNTIME_DIR']) / 'romacs' / server.name / 'server.sock'
        client = cls.connect(socket, Path(server.args.romacs).with_name('romacs-mcp'), server.env)
        if agent_tools:
            client.call('romacs_eval', code="(progn (require 'romacs-agent) t)")
        return client

    def _read(self):
        for line in self.process.stdout:
            try:
                message = json.loads(line)
            except ValueError:
                continue
            with self.lock:
                self.replies[message.get('id')] = message
                self.arrived.notify_all()
        with self.lock:
            self.arrived.notify_all()

    def send(self, message):
        self.process.stdin.write(json.dumps(message) + '\n')
        self.process.stdin.flush()

    def request(self, method, params, timeout=120):
        """(request id, the JSON-RPC reply) of one request."""
        with self.lock:
            self.next_id += 1
            ident = self.next_id
        self.send({'jsonrpc': '2.0', 'id': ident, 'method': method, 'params': params})
        with self.lock:
            self.arrived.wait_for(lambda: ident in self.replies or self.process.poll() is not None, timeout)
            return ident, self.replies.pop(ident, {'error': {'message': 'no reply'}})

    def schemas(self):
        """Each offered tool's input schema, by name (read once)."""
        if self._schemas is None:
            _, reply = self.request('tools/list', {})
            self._schemas = {t['name']: t.get('inputSchema', {}) for t in reply.get('result', {}).get('tools', [])}
        return self._schemas

    def tools(self):
        """The names of the tools the endpoint offers."""
        self._schemas = None
        return sorted(self.schemas())

    def tool(self, name, arguments, timeout=120):
        """(request id, text, is_error) of one tools/call."""
        ident, reply = self.request('tools/call', {'name': name, 'arguments': arguments}, timeout)
        if 'error' in reply:
            return ident, reply['error'].get('message', ''), True
        result = reply['result']
        return ident, ''.join(c.get('text', '') for c in result.get('content', [])), result.get('isError', False)

    def identity(self):
        if not self.instance:
            self.instance = self.call('romacs_identity')['instance']
        return self.instance

    def call(self, name, timeout=120, **arguments):
        """The result of tool NAME with ARGUMENTS: JSON parsed, else text.
        `instance' is filled for tools whose schema takes it."""
        if 'instance' in self.schemas().get(name, {}).get('properties', {}) and 'instance' not in arguments:
            arguments['instance'] = self.identity()
        _, text, error = self.tool(name, arguments, timeout)
        if error:
            raise ToolError(f'{name}: {text}')
        try:
            return json.loads(text)
        except ValueError:
            return text

    def close(self):
        if self.process.poll() is None:
            self.process.stdin.close()
            try:
                self.process.wait(5)
            except subprocess.TimeoutExpired:
                self.process.kill()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
