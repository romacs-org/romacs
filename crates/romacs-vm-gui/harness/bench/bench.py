#!/usr/bin/env python3
"""Paired-session bench: pinned GNU and Romacs side by side, driven by an agent.

Each engine runs a profile's session (owner.py: stock, no init on a generic
project, by default; or the owner's) in its own private HOME on
a headless labwc and serves romacs-mcp.el's endpoint with the session tools
(romacs-mcp-session.el): the same Lisp and the same tool surface in both.
The bench is itself an MCP server whose tools take an engine argument --
gnu, romacs or both -- so an agent can work in the two editors as a person
would, look at them and compare them:

  bench_start       start the pair (or one engine) on a profile's session;
                    Romacs from a world prepared first (world_seconds), as
                    GNU from its dump, so startup_seconds is a user's launch
  bench_send_keys   keys (kbd) or text through each command loop, or wtype
  bench_wait_idle   until each loop is idle and timers and processes settle
  bench_observe     what each frame shows (romacs_observe), paths normalised
  bench_eval        Lisp in each engine
  bench_screenshot  a PNG of each compositor output
  bench_compare     same / differs / romacs-error, with the differences
  bench_stop        stop the pair and write report.json and summary.txt

Every call is appended to the session's recording.jsonl (tool, arguments,
elapsed time, outcome) and its full result to results.jsonl.  `replay'
re-drives a recording against new binaries and reports each compared
step's verdict beside the recorded one, the GNU side re-run or taken from
a previous run's results.jsonl.

usage:
  bench.py mcp     [--control SOCK] [options]   MCP server on stdio
  bench.py daemon  --control SOCK [options]     the same tools on a socket
  bench.py call    --control SOCK TOOL [JSON]   one call to a daemon
  bench.py replay  RECORDING --output DIR [--gnu-cache RESULTS] [options]
options: --romacs BIN --gnu EMACS --output DIR --work DIR --profile stock|owner --gnosis-db COPY
         --project REPO --org FILE (ROMACS_BENCH_ORG) --tools DIR --timeout SECONDS

COMPARISON POLICY (bench_compare): the selected buffer, the buffer's name,
modes, point, size, flags, file and directory, global minor modes, the
window tree, each window's buffer, start, point, hscroll, body size,
dedication, mode and header lines and displayed rows (over the rows both
show: line heights follow each engine's font metrics), the minibuffer, the
echo area, *Messages* lines since the previous compare, the last
command-loop error and the buffer list.  Faces are compared separately and
reported as the `faces' category.  Normalised: each engine's private root
and package directory become <root> and <lisp>; clock times (HH:MM:SS)
become <clock>.  Ignored: window-end and pixel geometry.  A compare's
`ignore' names path prefixes the agent judged nondeterministic (a model's
reply), recorded with the call.  romacs-error: Romacs signalled where GNU
did not (a new command-loop error, or a new *Messages* line reporting an
error, or an error in the echo area GNU's does not show), or a Romacs
tool call failed.

DATA SAFETY: everything a session writes is inside its private trees.  The
gnosis database is a copy of a copy given with --gnosis-db, never the live
one.  A send_keys call that sends a hermes prompt must say
effect=hermes-prompt; the bench allows one per engine per session, and
replay skips such calls unless --allow-effect hermes-prompt.
"""
import argparse
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
sys.path.insert(0, str(HERE))
import parity_live  # noqa: E402
import fixture  # noqa: E402
import owner  # noqa: E402

ENGINES = ('gnu', 'romacs')
GUARDED = {'hermes-prompt': 1}  # effect -> calls allowed per engine per session
ERROR_LINE = re.compile(r'(?i)\berror\b|failed|wrong type|is void|void-function|void-variable|not implemented|unsupported'
                        r'|args out of range|invalid|debugger entered|\bVM2\b|not supported|wrong number of arguments'
                        # A condition's own name, as an unreported signal echoes it.
                        r'|wrong-type-argument|args-out-of-range|wrong-number-of-arguments|cl-no-applicable-method'
                        r'|setting-constant')
CLOCK = re.compile(r'\b\d\d:\d\d:\d\d\b')


class ToolError(Exception):
    pass


# --- one engine over its MCP socket ----------------------------------------

class McpEngine(parity_live.Engine):
    """parity_live's engine, spoken to through romacs-mcp.el's endpoint."""

    def start(self, tools):
        try:
            return super().start(tools)
        except BaseException:
            self.stop()
            raise

    def connect(self, token):
        port = int((self.root / 'mcp/port').read_text())
        self.sock = socket.create_connection(('127.0.0.1', port), timeout=30)
        self.stream = self.sock.makefile('rwb')
        self.stream.write(token.encode() + b'\n')
        self.lock = threading.Lock()
        self.ids = 0
        self.rpc('initialize', {'protocolVersion': '2025-11-25', 'capabilities': {},
                                'clientInfo': {'name': 'romacs-bench', 'version': '1'}})
        self.stream.write(b'{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}\n')
        self.stream.flush()
        self.instance = self.tool('romacs_identity')['instance']
        return self

    def rpc(self, method, params, deadline=30):
        with self.lock:
            self.ids += 1
            self.sock.settimeout(deadline)
            self.stream.write((json.dumps({'jsonrpc': '2.0', 'id': self.ids, 'method': method,
                                           'params': params}, ensure_ascii=False) + '\n').encode())
            self.stream.flush()
            while True:
                line = self.stream.readline()
                if not line:
                    raise ToolError(f'{self.name}: endpoint closed (engine exit {self.process.poll()})')
                reply = json.loads(line)
                if reply.get('id') == self.ids:
                    break
        if 'error' in reply:
            raise ToolError(f'{self.name}: {reply["error"].get("message")}')
        return reply['result']

    def tool(self, name, deadline=30, **arguments):
        """Call tool NAME, waiting up to DEADLINE seconds for its answer."""
        if name != 'romacs_identity':
            arguments['instance'] = self.instance
        result = self.rpc('tools/call', {'name': name, 'arguments': arguments}, deadline)
        text = result['content'][0]['text']
        if result.get('isError'):
            raise ToolError(f'{self.name}: {name}: {text}')
        return text if name == 'romacs_eval' else json.loads(text)

    def ask(self, form, timeout=None):
        return parity_live.read_sexp(self.tool('romacs_eval', timeout or 60, code=form))

    def normalise(self, value):
        if isinstance(value, dict):
            return {k: self.normalise(v) for k, v in value.items()}
        if isinstance(value, list):
            return [self.normalise(v) for v in value]
        if isinstance(value, str):
            value = value.replace(str(self.root), '<root>')
            if self.lisp is not None:
                value = value.replace(str(self.lisp), '<lisp>')
            value = value.replace(str(parity_live.ROOT), '<repo>').replace(str(Path.home()), '~')
            return CLOCK.sub('<clock>', value)
        return value


# --- keys for wtype ---------------------------------------------------------

WTYPE_KEYS = {'RET': 'Return', 'TAB': 'Tab', 'SPC': 'space', 'DEL': 'BackSpace', 'ESC': 'Escape',
              'up': 'Up', 'down': 'Down', 'left': 'Left', 'right': 'Right', 'home': 'Home', 'end': 'End',
              'prior': 'Prior', 'next': 'Next', 'tab': 'Tab', 'return': 'Return', 'backspace': 'BackSpace',
              'delete': 'Delete', 'escape': 'Escape', 'backtab': 'Tab'}
WTYPE_MODS = {'C': 'ctrl', 'M': 'alt', 'S': 'shift', 's': 'logo'}


def wtype_groups(keys):
    """`kbd' notation as wtype argument groups, one per key."""
    groups = []
    for token in keys.split():
        mods = []
        while len(token) > 2 and token[1] == '-' and token[0] in WTYPE_MODS:
            mods.append(WTYPE_MODS[token[0]])
            token = token[2:]
        name = token[1:-1] if token.startswith('<') and token.endswith('>') else token
        if name == 'backtab':
            mods.append('shift')
        key = ['-k', WTYPE_KEYS[name]] if name in WTYPE_KEYS else None
        if key is None and len(name) > 1 and not mods:
            groups.append([name])
            continue
        group = []
        for mod in mods:
            group += ['-M', mod]
        group += key or [name]
        for mod in reversed(mods):
            group += ['-m', mod]
        groups.append(group)
    return groups


# --- comparison ---------------------------------------------------------------

def diff(gnu, romacs, path, out):
    if isinstance(gnu, dict) and isinstance(romacs, dict):
        for key in sorted(set(gnu) | set(romacs)):
            diff(gnu.get(key), romacs.get(key), f'{path}.{key}' if path else key, out)
    elif isinstance(gnu, list) and isinstance(romacs, list) and len(gnu) == len(romacs):
        for i, (a, b) in enumerate(zip(gnu, romacs)):
            diff(a, b, f'{path}[{i}]', out)
    elif gnu != romacs:
        out.append({'path': path, 'gnu': gnu, 'romacs': romacs})


def rows_view(rows, span, faces):
    return [r.get('faces', []) if faces else r['text'] for r in rows[:span]]


def comparable(obs, other, faces):
    """OBS's compared parts under COMPARISON POLICY, rows clipped to OTHER's."""
    view = {k: obs.get(k) for k in ('selectedBuffer', 'buffer', 'globalMinorModes', 'tree', 'echo', 'buffers')}
    view['minibuffer'] = dict(obs.get('minibuffer') or {})
    if faces:
        view = {}
    windows = []
    for i, window in enumerate(obs.get('windows') or []):
        twin = (other.get('windows') or [])[i] if i < len(other.get('windows') or []) else None
        span = min(len(window['rows']), len(twin['rows'])) if twin else len(window['rows'])
        entry = {} if faces else {k: window.get(k) for k in ('buffer', 'selected', 'start', 'point', 'hscroll', 'body',
                                                               'dedicated', 'modeLine', 'headerLine')}
        entry['rows'] = rows_view(window['rows'], span, faces)
        windows.append(entry)
    view['windows'] = windows
    mini = obs.get('minibuffer') or {}
    if 'rows' in mini:
        twin = (other.get('minibuffer') or {}).get('rows') or []
        view['minibuffer'] = dict(view.get('minibuffer') or {}, rows=rows_view(mini['rows'], min(len(mini['rows']), len(twin)), faces))
    if not faces:
        view['messages'] = obs['messages']['lines']
    return view


def ignored(path, prefixes):
    return any(path == p or path.startswith(p + '.') or path.startswith(p + '[') for p in prefixes)


def verdict(gnu, romacs, ignore, gnu_error, romacs_error):
    """The compare verdict for two normalised observations."""
    found, faces = [], []
    diff(comparable(gnu, romacs, False), comparable(romacs, gnu, False), '', found)
    diff(comparable(gnu, romacs, True), comparable(romacs, gnu, True), '', faces)
    if gnu_error != romacs_error:
        found.append({'path': 'lastError', 'gnu': gnu_error, 'romacs': romacs_error})
    found = [d for d in found if not ignored(d['path'], ignore)]
    faces = [d for d in faces if not ignored(d['path'], ignore)]
    gnu_lines = set(gnu['messages']['lines'])
    errors = [line for line in romacs['messages']['lines'] if ERROR_LINE.search(line) and line not in gnu_lines]
    # A command error shows in the echo area even where the command loop
    # neither logs it nor calls `command-error-function'.
    echo = romacs.get('echo')
    if echo and echo != gnu.get('echo') and ERROR_LINE.search(echo) and echo not in errors:
        errors.append(f'echo: {echo}')
    if romacs_error and romacs_error != gnu_error:
        errors.insert(0, f'command error: {romacs_error}')
    categories = sorted({re.split(r'[.\[]', d['path'])[0] for d in found} | ({'faces'} if faces else set()))
    kind = 'romacs-error' if errors else 'differs' if found or faces else 'same'
    return {'verdict': kind, 'categories': categories, 'romacsErrors': errors,
            'differences': found[:60], 'differenceCount': len(found),
            'faceDifferences': faces[:20], 'faceDifferenceCount': len(faces)}


# --- the bench ----------------------------------------------------------------

def schema(properties, required=()):
    return {'type': 'object', 'properties': properties, 'required': list(required), 'additionalProperties': False}


ENGINE = {'type': 'string', 'enum': ['gnu', 'romacs', 'both'], 'description': 'default both'}
STEP = {'type': 'string', 'description': 'the journey step this call belongs to, for the recording'}

TOOLS = {
    'bench_start': ('Start GNU and Romacs (or one engine) in private HOMEs: profile stock (no init, a generic '
                    'project at ~/src/tally, the default) or owner (the owner\'s packages and data copies).',
                    schema({'engines': {'type': 'array', 'items': {'type': 'string', 'enum': list(ENGINES)}},
                            'profile': {'type': 'string', 'enum': ['stock', 'owner']},
                            'hermes': {'type': 'boolean'}, 'step': STEP})),
    'bench_send_keys': ('Type keys (kbd notation, e.g. "C-x C-f") or literal text into each engine through its '
                        'command loop (via events, paced like typing) or the compositor (via wtype). '
                        'wait=SECONDS then waits for idle. effect=hermes-prompt marks the one allowed hermes send.',
                        schema({'engine': ENGINE, 'keys': {'type': 'string'}, 'text': {'type': 'string'},
                                'via': {'type': 'string', 'enum': ['events', 'wtype']}, 'burst': {'type': 'boolean'},
                                'wait': {'type': 'number'}, 'effect': {'type': 'string'}, 'step': STEP})),
    'bench_wait_idle': ('Wait until each command loop is idle and timers and process output settled, or timeout.',
                        schema({'engine': ENGINE, 'timeout': {'type': 'number'}, 'settle': {'type': 'number'},
                                'horizon': {'type': 'number'}, 'step': STEP})),
    'bench_observe': ('What each frame shows: windows, rows, mode lines, minibuffer, echo, new *Messages*, '
                      'last error, modes.',
                      schema({'engine': ENGINE, 'faces': {'type': 'boolean'}, 'maxRows': {'type': 'integer'},
                              'step': STEP})),
    'bench_eval': ('Evaluate Lisp in each engine; returns the printed value.',
                   schema({'engine': ENGINE, 'code': {'type': 'string'}, 'step': STEP}, ['code'])),
    'bench_screenshot': ('Save a PNG of each engine output; returns the paths.',
                         schema({'engine': ENGINE, 'name': {'type': 'string'}, 'step': STEP})),
    'bench_compare': ('Observe both engines and compare: same, differs (with the differences) or romacs-error.',
                      schema({'step': STEP, 'ignore': {'type': 'array', 'items': {'type': 'string'}},
                              'note': {'type': 'string'}})),
    'bench_stop': ('Stop the engines; write report.json and summary.txt; return the step table.',
                   schema({'step': STEP})),
}


class Bench:
    def __init__(self, options, gnu_cache=None, allow_effects=()):
        self.options, self.engines, self.session = options, {}, None
        self.gnu_cache, self.allow_effects = gnu_cache, set(allow_effects)
        self.lock = threading.Lock()
        parity_live.GNU = options.gnu

    # recording

    def record(self, tool, arguments, started, outcome, result):
        self.seq += 1
        entry = {'seq': self.seq, 'tool': tool, 'args': arguments, 'elapsed': round(time.time() - started, 3)}
        entry.update(outcome)
        with (self.session / 'recording.jsonl').open('a') as stream:
            stream.write(json.dumps(entry, ensure_ascii=False) + '\n')
        with (self.session / 'results.jsonl').open('a') as stream:
            stream.write(json.dumps({'seq': self.seq, 'tool': tool, 'result': result}, ensure_ascii=False) + '\n')

    def call(self, tool, arguments):
        with self.lock:
            return self.call_unlocked(tool, arguments)

    def call_unlocked(self, tool, arguments):
        if tool not in TOOLS:
            raise ToolError(f'unknown tool {tool}')
        if tool != 'bench_start' and self.session is None:
            raise ToolError('no session: call bench_start first')
        if tool == 'bench_start':
            # The recording names the profile, whatever default supplied it.
            arguments = {**arguments, 'profile': arguments.get('profile') or self.options.profile}
        started = time.time()
        try:
            result, outcome = getattr(self, tool)(dict(arguments))
            outcome = dict(outcome, ok=True)
        except Exception as error:  # noqa: BLE001
            result, outcome = {'error': str(error)}, {'ok': False, 'error': str(error)}
            if self.session is not None:
                self.record(tool, arguments, started, outcome, result)
            raise ToolError(str(error)) from error
        self.record(tool, arguments, started, outcome, result)
        # The observations behind a compare stay in results.jsonl.
        return {k: v for k, v in result.items() if k != 'observations'}

    def targets(self, arguments):
        names = ENGINES if arguments.get('engine', 'both') == 'both' else [arguments['engine']]
        return [self.engines[n] for n in names if n in self.engines]

    def each(self, arguments, function):
        """FUNCTION per target engine, concurrently: {name: value or {'error'}} and timings."""
        results, seconds = {}, {}

        def run(engine):
            started = time.time()
            try:
                results[engine.name] = function(engine)
            except Exception as error:  # noqa: BLE001
                results[engine.name] = {'error': str(error)}
            seconds[engine.name] = round(time.time() - started, 3)
        threads = [threading.Thread(target=run, args=(e,)) for e in self.targets(arguments)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
        return results, seconds

    # tools

    def bench_start(self, arguments):
        if self.engines:
            raise ToolError('a session is running: bench_stop first')
        options = self.options
        names = arguments.get('engines') or list(ENGINES)
        if self.gnu_cache is not None:
            names = [n for n in names if n != 'gnu']
        out = Path(options.output)
        out.mkdir(parents=True, exist_ok=True)
        self.session = Path(tempfile.mkdtemp(prefix=time.strftime('session-%Y%m%d-%H%M%S-'), dir=out))
        self.seq, self.effects, self.steps = 0, {}, []
        self.cursors = {}
        work = Path(options.work)
        work.mkdir(parents=True, exist_ok=True)
        profile = arguments.get('profile') or options.profile
        stock = profile == 'stock'
        project = Path(options.project) if options.project else None if stock else owner.default_project()
        org = Path(options.org) if options.org and not stock else None
        started = {}

        def launch(name):
            t = time.time()
            root, home, env = parity_live.private_tree(name, work, options.tools)
            (root / 'mcp').mkdir(mode=0o700)
            token = secrets.token_hex(24)
            used = owner.populate(home, project, org, None if stock else options.gnosis_db)
            if stock and not project:
                fixture.make_repo(home / 'src/tally')
                used['project'] = '~/src/tally'
            lisp = None if stock else owner.lisp_dir(work, name)
            init = owner.init_source(name, lisp, root / 'mcp/port', token, profile)
            # A stock session loads the bench's init as `emacs -Q -l' does,
            # in both engines; an owner session, as the user's init.
            if stock:
                (root / 'bench-init.el').write_text(init)
                quick = ['-Q', '-l', str(root / 'bench-init.el')]
            if name == 'gnu' and stock:
                engine = McpEngine('gnu', [options.gnu, '--font', 'Iosevka-14', *quick], root, None, env, options.timeout)
                engine.start(options.tools)
            elif name == 'gnu':
                engine = parity_live.gnu_engine(root, home, env, init, lisp, options.timeout, options.tools, McpEngine)
            elif stock:
                engine = parity_live.romacs_engine(options.romacs, root, home, env, None, lisp, options.timeout,
                                                   options.tools, McpEngine, quick)
            else:
                engine = parity_live.romacs_engine(options.romacs, root, home, env, init, lisp, options.timeout,
                                                   options.tools, McpEngine)
            self.engines[name] = engine
            engine.connect(token)
            engine.fit_grid()
            errors = engine.ask('bench-init-errors')
            # Startup's own steps finish after the init; let them.
            engine.tool('romacs_wait_idle', 70, timeout=60)
            started[name] = {'profile': profile, 'startup_seconds': round(engine.startup_seconds, 3),
                             'world_seconds': getattr(engine, 'world_seconds', None),
                             'ready_seconds': round(time.time() - t, 3), 'data': used,
                             'init_errors': errors or [], 'runtime': engine.ask('emacs-version'),
                             'startup_hook_ran': engine.ask('bench-startup-hook-ran') is not None}
        failures = {}

        def guarded(name):
            try:
                launch(name)
            except Exception as error:  # noqa: BLE001
                failures[name] = repr(error)
        threads = [threading.Thread(target=guarded, args=(n,)) for n in names]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
        if failures:
            self.stop_engines()
            raise ToolError(f'start failed: {failures}')
        if self.gnu_cache is not None:
            self.cursors['gnu'] = {'error': None}
        for engine in self.engines.values():
            self.cursors[engine.name] = {'observe': None, 'compare': None, 'error': None}
            engine.ask('(progn (setq romacs-mcp-session--last-error nil) t)')
            # The compare cursor starts after startup's messages.
            self.cursors[engine.name]['compare'] = self.cursors[engine.name]['observe'] = \
                engine.tool('romacs_observe', maxRows=1)['messages']['next']
        result = {'session': str(self.session), 'engines': started}
        return result, {'summary': {n: {k: v for k, v in s.items() if k in ('startup_seconds', 'world_seconds', 'init_errors', 'startup_hook_ran')}
                                    for n, s in started.items()}}

    def bench_send_keys(self, arguments):
        effect = arguments.get('effect')
        if effect in GUARDED:
            for engine in self.targets(arguments):
                used = self.effects.get((effect, engine.name), 0)
                if used >= GUARDED[effect]:
                    raise ToolError(f'{effect} already used {used} time(s) in {engine.name} this session')
            if self.replaying and effect not in self.allow_effects:
                return {'skipped': f'effect {effect} needs --allow-effect'}, {'summary': 'skipped'}
            for engine in self.targets(arguments):
                self.effects[(effect, engine.name)] = self.effects.get((effect, engine.name), 0) + 1
        via = arguments.get('via', 'events')
        keys, text = arguments.get('keys'), arguments.get('text')

        def send(engine):
            if via == 'wtype':
                groups = wtype_groups(keys) if keys else [[text]]
                engine.keys(*groups)
                sent = {'groups': len(groups)}
            else:
                payload = {'keys': keys} if keys is not None else {'text': text}
                if arguments.get('burst'):
                    payload['burst'] = True
                sent = engine.tool('romacs_send_keys', 60, **payload)
            if arguments.get('wait'):
                sent['idle'] = engine.tool('romacs_wait_idle', arguments['wait'] + 10, timeout=arguments['wait'])
            return sent
        results, seconds = self.each(arguments, send)
        return {'engines': results, 'seconds': seconds}, {'seconds': seconds, 'summary': summarise(results)}

    def bench_wait_idle(self, arguments):
        timeout = arguments.get('timeout', 10)
        extra = {k: arguments[k] for k in ('settle', 'horizon') if k in arguments}
        results, seconds = self.each(arguments, lambda e: e.tool('romacs_wait_idle', timeout + 10,
                                                                 timeout=timeout, **extra))
        return {'engines': results, 'seconds': seconds}, {'seconds': seconds, 'summary': summarise(results)}

    def observe(self, engine, cursor, faces=False, rows=None):
        extra = {'maxRows': rows} if rows else {}
        if self.cursors[engine.name][cursor] is not None:
            extra['messagesFrom'] = self.cursors[engine.name][cursor]
        obs = engine.tool('romacs_observe', 60, faces=faces, **extra)
        self.cursors[engine.name][cursor] = obs['messages']['next']
        return engine.normalise(obs)

    def bench_observe(self, arguments):
        results, seconds = self.each(arguments, lambda e: self.observe(e, 'observe', arguments.get('faces', False),
                                                                       arguments.get('maxRows')))
        return {'engines': results, 'seconds': seconds}, {'seconds': seconds}

    def bench_eval(self, arguments):
        results, seconds = self.each(arguments, lambda e: e.normalise(e.tool('romacs_eval', 120, code=arguments['code'])))
        return {'engines': results, 'seconds': seconds}, {'seconds': seconds, 'summary': summarise(results)}

    def bench_screenshot(self, arguments):
        name = re.sub(r'[^A-Za-z0-9_.-]', '-', arguments.get('name') or f'shot-{self.seq + 1}')

        def shoot(engine):
            path = self.session / f'{name}-{engine.name}.png'
            subprocess.run([f'{engine.tools}/grim', str(path)], env=engine.env, check=True, timeout=30,
                           capture_output=True)
            return str(path)
        results, seconds = self.each(arguments, shoot)
        return {'engines': results}, {'summary': results}

    def bench_compare(self, arguments):
        step = arguments.get('step') or f'step-{len(self.steps) + 1}'
        results, seconds = self.each({'engine': 'both'}, lambda e: self.observe(e, 'compare', True))
        cached_exit = None
        if self.gnu_cache is not None:
            results['gnu'], cached_exit = (self.gnu_cache.pop(0) if self.gnu_cache
                                           else ({'error': 'no cached GNU observation left'}, None))
        gnu, romacs = results.get('gnu'), results.get('romacs')
        errors = {}
        for name, obs in (('gnu', gnu), ('romacs', romacs)):
            if obs and 'error' not in obs:
                last = obs.get('lastError') or {}
                previous = self.cursors.get(name, {}).get('error')
                errors[name] = last.get('message') if last.get('serial') != previous else None
                if name in self.cursors:
                    self.cursors[name]['error'] = last.get('serial')
        time.sleep(.5 if any('error' in (v or {}) for v in results.values()) else 0)
        exits = {n: e.process.poll() for n, e in self.engines.items()}
        if self.gnu_cache is not None:
            exits['gnu'] = cached_exit
        if any(code is not None for code in exits.values()):
            # An editor exited (the journey's C-x C-c): compare how.
            kind = ('same' if len(set(exits.values())) == 1 else
                    'romacs-error' if exits.get('romacs') not in (None, 0) else 'differs')
            outcome = {'verdict': kind, 'categories': ['exit'], 'exitCodes': exits,
                       'romacsErrors': [f'romacs exited {exits["romacs"]}'] if kind == 'romacs-error' else []}
        elif not romacs or 'error' in romacs:
            outcome = {'verdict': 'romacs-error', 'categories': ['bench'],
                       'romacsErrors': [romacs.get('error') if romacs else 'no Romacs engine']}
        elif not gnu or 'error' in gnu:
            outcome = {'verdict': 'gnu-error', 'categories': ['bench'], 'gnuError': gnu.get('error') if gnu else None}
        else:
            outcome = verdict(gnu, romacs, arguments.get('ignore') or [], errors.get('gnu'), errors.get('romacs'))
        self.steps.append({'step': step, 'verdict': outcome['verdict'], 'categories': outcome['categories'],
                           'romacsErrors': outcome.get('romacsErrors', []), 'note': arguments.get('note')})
        self.write_report()
        result = dict(outcome, step=step, observations={'gnu': gnu, 'romacs': romacs})
        return result, {'summary': {'verdict': outcome['verdict'], 'categories': outcome['categories'],
                                    'romacsErrors': outcome.get('romacsErrors', [])[:3]}}

    def write_report(self):
        report = {'romacs': self.options.romacs, 'gnu': self.options.gnu, 'steps': self.steps}
        (self.session / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
        lines = [f'{s["verdict"]:13} {s["step"]}  {",".join(s["categories"])}'
                 + (f'  [{s["romacsErrors"][0][:100]}]' if s['romacsErrors'] else '') for s in self.steps]
        (self.session / 'summary.txt').write_text('\n'.join(lines) + '\n')

    def stop_engines(self):
        for engine in self.engines.values():
            for log in ('engine.log', 'compositor.log'):
                if (engine.root / log).exists() and self.session:
                    shutil.copy(engine.root / log, self.session / f'{engine.name}-{log}')
            engine.stop()
            if not self.options.keep:
                shutil.rmtree(engine.root, ignore_errors=True)
        self.engines = {}

    def bench_stop(self, arguments):
        self.write_report()
        self.stop_engines()
        counts = {}
        for step in self.steps:
            counts[step['verdict']] = counts.get(step['verdict'], 0) + 1
        return {'session': str(self.session), 'counts': counts, 'steps': self.steps}, {'summary': counts}

    replaying = False


def summarise(results):
    """A compact outcome per engine for the recording."""
    out = {}
    for name, value in results.items():
        if isinstance(value, dict):
            if 'error' in value:
                out[name] = {'error': value['error'][:200]}
            else:
                keep = {k: value[k] for k in ('undelivered', 'timedOut', 'settled', 'dueTimers', 'skipped') if k in value}
                if 'idle' in value and isinstance(value['idle'], dict):
                    keep['idle'] = {k: value['idle'][k] for k in ('settled', 'dueTimers') if k in value['idle']}
                out[name] = keep
        else:
            out[name] = str(value)[:200]
    return out


# --- front ends -------------------------------------------------------------

def tool_list():
    return [{'name': n, 'description': d, 'inputSchema': s} for n, (d, s) in TOOLS.items()]


def serve_control(bench, path):
    """Answer {"tool", "arguments"} lines on a Unix socket, one client at a time."""
    if os.path.exists(path):
        os.unlink(path)
    server = socket.socket(socket.AF_UNIX)
    server.bind(path)
    os.chmod(path, 0o600)
    server.listen(1)
    while True:
        connection, _ = server.accept()
        with connection, connection.makefile('rwb') as stream:
            for line in stream:
                request = json.loads(line)
                try:
                    reply = {'ok': True, 'result': bench.call(request['tool'], request.get('arguments') or {})}
                except Exception as error:  # noqa: BLE001
                    reply = {'ok': False, 'error': str(error)}
                stream.write((json.dumps(reply, ensure_ascii=False) + '\n').encode())
                stream.flush()


def serve_mcp(bench):
    """A minimal MCP server on stdio: initialize, tools/list, tools/call."""
    for line in sys.stdin:
        if not line.strip():
            continue
        message = json.loads(line)
        method, mid = message.get('method'), message.get('id')
        if mid is None:
            continue
        params = message.get('params') or {}
        try:
            if method == 'initialize':
                result = {'protocolVersion': params.get('protocolVersion', '2025-06-18'),
                          'capabilities': {'tools': {}}, 'serverInfo': {'name': 'romacs-bench', 'version': '1'}}
            elif method == 'ping':
                result = {}
            elif method == 'tools/list':
                result = {'tools': tool_list()}
            elif method == 'tools/call':
                try:
                    value = bench.call(params['name'], params.get('arguments') or {})
                    result = {'content': [{'type': 'text', 'text': json.dumps(value, ensure_ascii=False)}],
                              'isError': False}
                except ToolError as error:
                    result = {'content': [{'type': 'text', 'text': str(error)}], 'isError': True}
            else:
                raise ValueError(f'method not found: {method}')
            reply = {'jsonrpc': '2.0', 'id': mid, 'result': result}
        except Exception as error:  # noqa: BLE001
            reply = {'jsonrpc': '2.0', 'id': mid, 'error': {'code': -32601, 'message': str(error)}}
        sys.stdout.write(json.dumps(reply, ensure_ascii=False) + '\n')
        sys.stdout.flush()


def control_call(path, tool, arguments):
    connection = socket.socket(socket.AF_UNIX)
    connection.connect(path)
    with connection, connection.makefile('rwb') as stream:
        stream.write((json.dumps({'tool': tool, 'arguments': arguments}) + '\n').encode())
        stream.flush()
        return json.loads(stream.readline())


def replay(bench, recording, output):
    """Re-drive RECORDING; return per-step verdicts beside the recorded ones."""
    bench.replaying = True
    entries = [json.loads(line) for line in Path(recording).read_text().splitlines() if line.strip()]
    recorded = [e for e in entries if e['tool'] == 'bench_compare']
    started, rows, index = time.time(), [], 0
    try:
        for entry in entries:
            if entry['tool'] == 'bench_stop':
                continue
            arguments = entry['args']
            if bench.gnu_cache is not None and arguments.get('engine') == 'gnu':
                continue
            try:
                value = bench.call(entry['tool'], arguments)
            except ToolError as error:
                value = {'error': str(error)}
                if entry['tool'] == 'bench_start':
                    raise
            if entry['tool'] == 'bench_compare':
                before = recorded[index].get('summary', {}).get('verdict')
                now = value.get('verdict', 'romacs-error')
                change = ('same' if before == now else 'fixed' if now == 'same'
                          else 'regressed' if before == 'same' else 'changed')
                rows.append({'step': value.get('step', arguments.get('step')), 'recorded': before, 'now': now,
                             'change': change, 'categories': value.get('categories', []),
                             'romacsErrors': value.get('romacsErrors', [])[:3]})
                index += 1
    finally:
        if bench.engines:
            bench.call('bench_stop', {})
    report = {'recording': str(recording), 'romacs': bench.options.romacs, 'gnu_cached': bench.gnu_cache is not None,
              'seconds': round(time.time() - started, 1), 'steps': rows, 'session': str(bench.session)}
    Path(output).mkdir(parents=True, exist_ok=True)
    (Path(output) / 'replay.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    for row in rows:
        print(f'{row["now"]:13} (was {row["recorded"]:13}) {row["change"]:9} {row["step"]}')
    print(f'replayed {len(rows)} compared steps in {report["seconds"]}s')
    return 1 if any(r['change'] == 'regressed' for r in rows) else 0


def gnu_observations(results):
    """The GNU side of each compare in a previous run's results.jsonl: its
    observation and exit code (None while it ran)."""
    out = []
    for line in Path(results).read_text().splitlines():
        entry = json.loads(line)
        if entry['tool'] == 'bench_compare' and 'observations' in entry['result']:
            out.append((entry['result']['observations']['gnu'],
                        (entry['result'].get('exitCodes') or {}).get('gnu')))
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('mode', choices=['mcp', 'daemon', 'call', 'replay'])
    parser.add_argument('rest', nargs='*')
    parser.add_argument('--control')
    parser.add_argument('--romacs', default=os.environ.get('ROMACS_BENCH_ROMACS'))
    parser.add_argument('--gnu', default=os.environ.get('ROMACS_GNU_ORACLE'))
    parser.add_argument('--output', default=os.environ.get('ROMACS_BENCH_OUTPUT'))
    parser.add_argument('--work', default=os.environ.get('ROMACS_BENCH_WORK'))
    parser.add_argument('--gnosis-db', default=os.environ.get('ROMACS_BENCH_GNOSIS_DB'),
                        help='a copy of gnosis.db; never the live one')
    parser.add_argument('--profile', choices=['stock', 'owner'], default='stock',
                        help="bench_start's default profile")
    parser.add_argument('--project')
    parser.add_argument('--org', default=os.environ.get('ROMACS_BENCH_ORG'), help='an org file to copy')
    parser.add_argument('--tools', default=parity_live.TOOLS)
    parser.add_argument('--timeout', type=float, default=120)
    parser.add_argument('--keep', action='store_true', help='keep the private trees')
    parser.add_argument('--gnu-cache', help="replay: a previous run's results.jsonl for the GNU side")
    parser.add_argument('--allow-effect', action='append', default=[])
    args = parser.parse_args()
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    if args.mode == 'call':
        tool = args.rest[0]
        arguments = json.loads(args.rest[1]) if len(args.rest) > 1 else {}
        reply = control_call(args.control, tool, arguments)
        print(json.dumps(reply, indent=1, ensure_ascii=False))
        return 0 if reply['ok'] else 1
    if not (args.romacs and args.gnu and args.output):
        parser.error('--romacs, --gnu and --output (or their ROMACS_BENCH_* variables) are required')
    args.work = args.work or str(Path(args.output) / 'work')
    cache = gnu_observations(args.gnu_cache) if args.gnu_cache else None
    bench = Bench(args, cache, args.allow_effect)
    try:
        if args.mode == 'replay':
            return replay(bench, args.rest[0], args.output)
        if args.mode == 'daemon':
            serve_control(bench, args.control)
        else:
            if args.control:
                threading.Thread(target=serve_control, args=(bench, args.control), daemon=True).start()
            serve_mcp(bench)
    finally:
        bench.stop_engines()
    return 0


if __name__ == '__main__':
    sys.exit(main())
