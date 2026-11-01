#!/usr/bin/env python3
"""A loopback SMTP submission server that captures what it accepts.

It listens on 127.0.0.1 only: one port speaks plain SMTP and offers
STARTTLS, another speaks SMTP inside implicit TLS (as port 465 does), both
with DIR/cert.pem, a certificate for localhost and 127.0.0.1 that
DIR/ca.pem, a fixture authority, issues on first use.  AUTH PLAIN and LOGIN accept USER and PASSWORD
only, and only inside TLS.  Each accepted message becomes DIR/NNN.eml with
DIR/NNN.json recording its envelope, port kind, TLS version and AUTH user.
It writes DIR/ports.json once listening; SIGTERM stops it.  Nothing it
receives leaves the machine.

usage: fake_smtp.py DIR [--user USER] [--password PASSWORD]
"""
import argparse
import base64
import itertools
import json
from pathlib import Path
import signal
import socketserver
import ssl
import subprocess
import sys
import threading


def certificate(directory):
    """DIR/cert.pem for localhost and 127.0.0.1, issued by DIR/ca.pem, a
    fixture authority a client trusts as its trustfile (a self-signed
    server certificate draws GnuTLS's :self-signed warning even when
    trusted, which GNU's Network Security Manager asks about)."""
    ca, ca_key = directory / 'ca.pem', directory / 'ca-key.pem'
    cert, key, request = directory / 'cert.pem', directory / 'key.pem', directory / 'cert.csr'
    ec = ['-newkey', 'ec', '-pkeyopt', 'ec_paramgen_curve:prime256v1', '-nodes']
    run = lambda *argv: subprocess.run(['openssl', *argv], check=True, capture_output=True, timeout=60)
    if not cert.exists():
        run('req', '-x509', *ec, '-days', '3650', '-subj', '/CN=fake-smtp fixture CA', '-keyout', str(ca_key),
            '-out', str(ca), '-addext', 'basicConstraints=critical,CA:TRUE', '-addext', 'keyUsage=critical,keyCertSign')
        run('req', *ec, '-subj', '/CN=localhost', '-keyout', str(key), '-out', str(request))
        extensions = directory / 'cert.ext'
        extensions.write_text('basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\n'
                              'extendedKeyUsage=serverAuth\nsubjectAltName=DNS:localhost,IP:127.0.0.1\n')
        run('x509', '-req', '-in', str(request), '-CA', str(ca), '-CAkey', str(ca_key), '-CAcreateserial',
            '-days', '3650', '-extfile', str(extensions), '-out', str(cert))
    return cert, key


class Session(socketserver.StreamRequestHandler):
    """One SMTP conversation (RFC 5321 submission, the commands smtpmail uses)."""

    def setup(self):
        server = self.server
        if server.implicit:
            self.request = server.context.wrap_socket(self.request, server_side=True)
        super().setup()
        self.tls = server.implicit
        self.auth = None
        self.envelope = None

    def reply(self, line):
        self.wfile.write(line.encode() + b'\r\n')
        self.wfile.flush()

    def line(self):
        raw = self.rfile.readline(65536)
        if not raw:
            raise EOFError
        return raw.decode('utf-8', 'replace').rstrip('\r\n')

    def handle(self):
        try:
            self.reply('220 localhost fake-smtp ready')
            while self.command(self.line()):
                pass
        except (EOFError, ConnectionError, ssl.SSLError):
            pass

    def ehlo(self):
        lines = ['localhost greets you', 'PIPELINING', '8BITMIME', 'SIZE 10485760']
        lines += ['AUTH PLAIN LOGIN'] if self.tls else ['STARTTLS']
        for i, text in enumerate(lines):
            self.reply(('250 ' if i == len(lines) - 1 else '250-') + text)

    def command(self, text):
        verb = text.split(' ', 1)[0].upper()
        argument = text[len(verb):].strip()
        if verb in ('EHLO', 'HELO'):
            self.envelope = None
            self.ehlo() if verb == 'EHLO' else self.reply('250 localhost')
        elif verb == 'STARTTLS' and not self.tls:
            self.reply('220 ready to start TLS')
            self.request = self.server.context.wrap_socket(self.request, server_side=True)
            self.rfile, self.wfile = self.request.makefile('rb'), self.request.makefile('wb')
            self.tls, self.envelope = True, None
        elif verb == 'AUTH' and self.tls:
            self.authenticate(argument)
        elif verb == 'MAIL':
            if self.server.user and not self.auth:
                self.reply('530 authentication required')
            else:
                self.envelope = {'from': argument[5:].strip(), 'to': []}
                self.reply('250 ok')
        elif verb == 'RCPT' and self.envelope is not None:
            self.envelope['to'].append(argument[3:].strip())
            self.reply('250 ok')
        elif verb == 'DATA' and self.envelope and self.envelope['to']:
            self.reply('354 end with .')
            self.store(self.data())
            self.reply('250 queued')
        elif verb == 'RSET':
            self.envelope = None
            self.reply('250 ok')
        elif verb == 'NOOP':
            self.reply('250 ok')
        elif verb == 'QUIT':
            self.reply('221 bye')
            return False
        else:
            self.reply('503 bad sequence')
        return True

    def authenticate(self, argument):
        mechanism, _, initial = argument.partition(' ')
        if mechanism.upper() == 'PLAIN':
            if not initial:
                self.reply('334 ')
                initial = self.line()
            _, user, password = base64.b64decode(initial).decode().split('\0')
        elif mechanism.upper() == 'LOGIN':
            self.reply('334 VXNlcm5hbWU6')
            user = base64.b64decode(self.line()).decode()
            self.reply('334 UGFzc3dvcmQ6')
            password = base64.b64decode(self.line()).decode()
        else:
            return self.reply('504 unknown mechanism')
        if (user, password) == (self.server.user, self.server.password):
            self.auth = user
            self.reply('235 authenticated')
        else:
            self.reply('535 bad credentials')

    def data(self):
        lines = []
        while True:
            raw = self.rfile.readline(1 << 20)
            if not raw or raw in (b'.\r\n', b'.\n'):
                return b''.join(lines)
            lines.append(raw[1:] if raw.startswith(b'..') else raw)

    def store(self, body):
        with self.server.lock:
            stem = self.server.directory / f'{next(self.server.counter):03d}'
        stem.with_suffix('.eml').write_bytes(body)
        version = self.request.version() if isinstance(self.request, ssl.SSLSocket) else None
        stem.with_suffix('.json').write_text(json.dumps(dict(self.envelope, port=self.server.kind, tls=version,
                                                             auth=self.auth)) + '\n')


class Server(socketserver.ThreadingTCPServer):
    daemon_threads = True
    allow_reuse_address = True

    def __init__(self, kind, context, args):
        super().__init__(('127.0.0.1', 0), Session)
        self.kind, self.implicit, self.context = kind, kind == 'tls', context
        self.directory, self.user, self.password = Path(args.dir), args.user, args.password
        self.lock, self.counter = args.lock, args.counter


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('dir')
    parser.add_argument('--user', default='owner@example.org')
    parser.add_argument('--password', default='fixture-password')
    args = parser.parse_args()
    directory = Path(args.dir)
    directory.mkdir(parents=True, exist_ok=True)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(*certificate(directory))
    # Both servers share one counter, so messages number in arrival order.
    args.lock, args.counter = threading.Lock(), itertools.count(1)
    servers = [Server(kind, context, args) for kind in ('starttls', 'tls')]
    for server in servers:
        threading.Thread(target=server.serve_forever, daemon=True).start()
    (directory / 'ports.json').write_text(json.dumps({s.kind: s.server_address[1] for s in servers}) + '\n')
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
    signal.pause()


if __name__ == '__main__':
    main()
