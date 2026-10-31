#!/usr/bin/env python3
"""The project commands over a 5,000-file project, in a worker isolate.

Each session is a fresh private headless labwc and a fresh launch with
private HOME/XDG/TMPDIR (perkey_live.Session), visiting a text file inside
a git project of three copies of gnu-lisp/ (about 5,100 files).  Hooks
logged from Lisp into a file stamp, with `float-time', the start of each
command (`pre-command-hook'), each command's end (`post-command-hook'),
the prompt's appearance (`minibuffer-setup-hook') and the xref buffer's
update (`xref-after-update-hook'), which also writes the buffer's text and,
for the prompt, the sorted candidates: what the user would see, compared
across variants byte for byte.

Variants: `isolate' (romacs, the worker path, traced) and `sync' (romacs,
`romacs-project-isolate' nil: GNU's definitions run in the session,
untraced: the tracer follows one presentation and a blocked session
drains queued keys into several).
During each command the harness types --keys C-f keys at --gap intervals
and records when it sent each and when its command ran (the post-command
stamp): the key's latency while the job runs.  Romacs sessions run with
--trace-latency, whose `latency-key' samples give key enqueue to render
observed in the isolate variant.  `project-find-regexp' runs twice: cold (the worker starts) and
warm (the pooled worker is reused).  Pinned GNU (--gnu-batch, the oracle)
times the same enumeration and search in batch: it blocks for that long
in a session, so a key typed meanwhile waits the rest of the job.

usage: isolates_live.py --binary PATH --output DIR [--gnu-batch PATH] [--project DIR]
                        [--keys 12] [--gap 0.15] [--sessions 1] [--tools DIR:DIR]
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import tempfile
import time

import live_oracle as live
import perkey_live as perkey

# The deliver stamp is the isolate path's delivery: the result has arrived
# and the command continues in the session, as GNU's does after its search.
LOG_HOOKS = """(progn
  (defvar romacs-journey-log "%s")
  (defun romacs-journey-stamp (kind &optional text)
    (let ((inhibit-message t))
      (write-region (format "%%s %%.6f %%S\\n" kind (float-time) (or text "")) nil romacs-journey-log t 'silent)))
  (add-hook 'pre-command-hook (lambda () (romacs-journey-stamp 'pre (format "%%s" this-command))))
  (add-hook 'post-command-hook (lambda () (romacs-journey-stamp 'post (format "%%s" this-command))))
  (add-hook 'minibuffer-setup-hook
            (lambda ()
              (romacs-journey-stamp 'prompt (minibuffer-prompt))
              (when (and minibuffer-completion-table (string-prefix-p "Find file" (or (minibuffer-prompt) "")))
                (let ((names (sort (copy-sequence (all-completions "" minibuffer-completion-table)) #'string<)))
                  (with-temp-file "%s" (insert (mapconcat #'identity names "\\n") "\\n"))
                  (romacs-journey-stamp 'candidates (number-to-string (length names)))))))
  (with-eval-after-load 'xref
    (add-hook 'xref-after-update-hook
              (lambda ()
                (write-region (point-min) (point-max) "%s" nil 'silent)
                (romacs-journey-stamp 'xref (number-to-string (count-lines (point-min) (point-max)))))))
  (dolist (fn '(romacs-project--show-matches romacs-project--find-file-continue))
    (advice-add fn :before (lambda (&rest _) (romacs-journey-stamp 'deliver))))
  t)"""

CF = ['-M', 'ctrl', 'f', '-m', 'ctrl']


class Invalid(Exception):
    pass


def stamps(path):
    """(kind, time, text) per line of the Lisp log."""
    found = []
    if not path.exists():
        return found
    for line in path.read_text(errors='replace').splitlines():
        parts = line.split(' ', 2)
        if len(parts) == 3:
            found.append((parts[0], float(parts[1]), parts[2]))
    return found


def make_project(root, gnu_lisp):
    """Three copies of gnu-lisp/ and a notes file, committed in one git repository."""
    project = root / 'project'
    for name in ('a', 'b', 'c'):
        shutil.copytree(gnu_lisp, project / name)
    (project / 'notes.txt').write_text(''.join(f'line {i} of the notes, typed and retyped\n' for i in range(200)))
    env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(root), 'GIT_CONFIG_GLOBAL': '/dev/null',
           'GIT_AUTHOR_NAME': 'journey', 'GIT_AUTHOR_EMAIL': 'j@j', 'GIT_COMMITTER_NAME': 'journey', 'GIT_COMMITTER_EMAIL': 'j@j'}
    for command in (['git', 'init', '-q'], ['git', 'add', '-A'], ['git', '-c', 'commit.gpgsign=false', 'commit', '-q', '-m', 'journey']):
        subprocess.run(command, cwd=project, env=env, check=True, capture_output=True)
    return project


class Session(perkey.Session):
    """A perkey session visiting a file of the project, with the hooks logged."""

    def __init__(self, args, root, variant, project):
        # The latency tracer follows one presentation at a time; the blocked
        # variant drains queued keys into several, so it runs untraced.
        super().__init__(args, root, latency=(variant == 'isolate'))
        self.variant, self.project = variant, project
        self.log = root / 'journey.log'
        self.candidates = root / 'candidates.txt'
        self.xref = root / 'xref.txt'

    def start(self):
        work, config = self.root / 'work', self.root / 'config'
        for directory in (self.root / 'home', work, config):
            directory.mkdir()
        (config / 'rc.xml').write_text(live.RC_XML)
        self.file = self.project / 'notes.txt'
        self.stdout, self.stderr = self.root / 'binary.stdout', self.root / 'binary.stderr'
        self.environ = env = self.env()
        log = (self.root / 'compositor.log').open('w')
        self.compositor = subprocess.Popen([shutil.which('labwc', path=env['PATH']) or 'labwc', '-C', str(config)],
                                           env=env, stdout=log, stderr=log)
        self.wait(lambda: any(p.is_socket() for p in self.root.glob('wayland-*')), 20, 'compositor')
        env['WAYLAND_DISPLAY'] = next(p.name for p in self.root.glob('wayland-*') if p.is_socket())
        subprocess.run([shutil.which('wlr-randr', path=env['PATH']) or 'wlr-randr', '--output', 'HEADLESS-1',
                        '--custom-mode', '1600x1000'], env=env, check=True, timeout=15)
        self.keyboard = subprocess.Popen(['wtype', '-s', '600000', '-k', 'Shift_L'], env=env, stdout=log, stderr=log)
        time.sleep(.3)
        hooks = LOG_HOOKS % (self.log, self.candidates, self.xref)
        if self.variant == 'gnu':
            argv = [str(self.args.gnu), '-Q', '--eval', hooks, str(self.file)]
        else:
            argv = [str(self.args.binary), '-Q', '--loadup'] + (['--trace-latency'] if self.latency else []) + [str(self.file)]
        self.process = subprocess.Popen(argv, env=env, cwd=str(self.project), stdin=subprocess.DEVNULL,
                                        stdout=self.stdout.open('w'), stderr=self.stderr.open('w'))
        if self.variant == 'gnu':
            time.sleep(4)
        else:
            self.wait(lambda: any(r.get('ready') for r in records(self.stdout)), self.args.timeout, 'ready')
            self.tid = self.evaluator()
            self.eval_form(hooks)
            if self.variant == 'sync':
                self.eval_form('(setq romacs-project-isolate nil)')
            else:
                self.eval_form('(setq isolate-pool-size 1)')
        time.sleep(1)

    def eval_form(self, form):
        """M-: FORM, through the session's own command loop."""
        before = len(records(self.stdout))
        self.type(['-M', 'alt', ':', '-m', 'alt'])
        time.sleep(.2)
        self.type([form])
        self.type(['-k', 'Return'])
        done = lambda: [r for r in records(self.stdout)[before:] if r.get('native_command') == 'eval-expression']
        self.wait(done, self.args.timeout, f'M-: {form[:40]}')
        if any(r.get('signal') or r.get('native_error') for r in done()):
            raise Invalid(f'M-: {form[:60]} signalled: {done()}')

    def command(self, name, keys_before_prompt, finish):
        """Run NAME with KEYS_BEFORE_PROMPT typed at its prompt, then --keys C-f
        keys while it works, then FINISH keys once it is done: the stamps and
        the keys' send times."""
        before = stamps(self.log)
        self.type(['-M', 'alt', 'x', '-m', 'alt'])
        time.sleep(.3)
        self.type([name])
        started = time.time()
        self.type(['-k', 'Return'])
        if keys_before_prompt:
            # A secondary prompt (the regexp): the job starts at its RET.
            time.sleep(.5)
            for part in keys_before_prompt:
                self.type(part)
                time.sleep(.2)
            started = time.time()
            self.type(['-k', 'Return'])
        sent = []
        for _ in range(self.args.keys):
            time.sleep(self.args.gap)
            self.type(CF)
            sent.append(time.time())
        done = lambda: any(k in ('xref', 'candidates') for k, t, _ in stamps(self.log)[len(before):])
        self.wait(done, self.args.timeout, f'{name} to finish')
        time.sleep(1)
        for part in finish:
            self.type(part)
            time.sleep(.3)
        time.sleep(.5)
        return {'started': started, 'sent': sent, 'stamps': stamps(self.log)[len(before):]}

    def stop(self):
        super().stop()


def analyse(run, name):
    """The command's job time and the keys' latencies from RUN's stamps:
    the job runs from the harness's RET to the xref or candidates stamp; the
    command's own time ends at its post-command stamp. The search is the
    job up to the result's delivery (the deliver stamp, isolate path), after
    which the command continues in the session as GNU's does after its
    search: a key sent during the search is the latency measured; the
    delivery's own length is reported."""
    marks = run['stamps']
    start = run['started']
    end = next((t for k, t, _ in marks if k in ('xref', 'candidates')), None)
    deliver = next((t for k, t, _ in marks if k == 'deliver' and t > start), None)
    search_end = deliver if deliver is not None else end
    command_end = next((t for k, t, text in marks if k == 'post' and text == f'"{name}"' and t > start), None)
    posts = [t for k, t, text in marks if k == 'post' and text == '"forward-char"']
    latencies = []
    for sent, ran in zip(run['sent'], posts):
        latencies.append(max(0.0, ran - sent))
    during = [l for s, l in zip(run['sent'], latencies) if search_end is None or s < search_end]
    return {'job_s': None if end is None else end - start, 'command_s': None if command_end is None else command_end - start,
            'search_s': None if search_end is None else search_end - start,
            'delivery_s': None if deliver is None or end is None else end - deliver,
            'keys_sent': len(run['sent']), 'keys_ran': len(posts),
            'key_latency_s': latencies, 'during_job': during,
            'median_during_s': statistics.median(during) if during else None,
            'max_during_s': max(during) if during else None,
            'result_lines': next((text for k, _, text in marks if k in ('xref', 'candidates')), None)}


def render_latencies(stderr_path, since):
    """key enqueue to render observed, in ms, of latency-key samples after SINCE records."""
    samples = [r for r in records(stderr_path)[since:] if r.get('diagnostic') == 'latency-key']
    return [s['key_to_render_observed_ns'] / 1e6 for s in samples if s.get('key_to_render_observed_ns') is not None]


def records(path):
    return perkey.records(path)


def gnu_batch(binary, project):
    """Pinned GNU's times, in batch, for the enumeration and the search the
    commands do over PROJECT: what a GNU session blocks for."""
    form = f"""(progn (require 'project) (require 'xref)
      (let* ((dir (expand-file-name {json.dumps(str(project) + '/')})) (pr (project-current nil dir)) (t0 (float-time))
             (files (project-files pr)) (t1 (float-time))
             (matches (let ((default-directory dir) (project-files-relative-names t)) (xref-matches-in-files "defcustom project-" (project-files pr))))
             (t2 (float-time)))
        (princ (format "%S" (list 'files (length files) 'enumerate_s (- t1 t0) 'matches (length matches) 'search_s (- t2 t1))))))"""
    run = subprocess.run([str(binary), '--batch', '-Q', '--eval', form], capture_output=True, text=True, timeout=300,
                         env={'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': os.environ.get('HOME', '/'), 'LANG': 'C.UTF-8'})
    return {'stdout': run.stdout.strip(), 'returncode': run.returncode}


def session(args, variant, project, index):
    runtime = Path(os.environ.get('XDG_RUNTIME_DIR', '/run/user/1000'))
    with tempfile.TemporaryDirectory(prefix='isol-', dir=runtime) as temporary:
        root = Path(temporary)
        root.chmod(0o700)
        live_session = Session(args, root, variant, project)
        result = {'variant': variant, 'index': index, 'valid': False}
        try:
            live_session.start()
            for name, key in (('find_regexp', 'find_regexp'), ('find_regexp_warm', 'find_regexp')):
                since = len(records(live_session.stderr))
                regexp = live_session.command('project-find-regexp', [['defcustom project-']], [['-k', 'q']])
                result[name] = analyse(regexp, 'project-find-regexp')
                result[name]['render_ms'] = render_latencies(live_session.stderr, since) if live_session.latency else None
                result['xref_text'] = live_session.xref.read_text() if live_session.xref.exists() else None
            since = len(records(live_session.stderr))
            find = live_session.command('project-find-file', [], [['-M', 'ctrl', 'g', '-m', 'ctrl']])
            result['find_file'] = analyse(find, 'project-find-file')
            result['find_file']['render_ms'] = render_latencies(live_session.stderr, since) if live_session.latency else None
            result['candidates_text'] = live_session.candidates.read_text() if live_session.candidates.exists() else None
            result['valid'] = True
        except (Invalid, perkey.Invalid, subprocess.CalledProcessError) as error:
            result['invalid'] = str(error)
        finally:
            kept = args.output / f'{variant}-{index}'
            live_session.stop()
            shutil.copytree(root, kept, ignore=shutil.ignore_patterns('project', 'home'))
        return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--gnu-batch', type=Path, default=os.environ.get('ROMACS_GNU_ORACLE'),
                        help='pinned GNU, run in batch over the project for the job times it blocks for')
    parser.add_argument('--project', type=Path, help='an existing project; else three copies of gnu-lisp/ are made')
    parser.add_argument('--gnu-lisp', type=Path, default=Path(__file__).resolve().parents[3] / 'gnu-lisp')
    parser.add_argument('--keys', type=int, default=12)
    parser.add_argument('--gap', type=float, default=.15)
    parser.add_argument('--sessions', type=int, default=1)
    parser.add_argument('--settle', type=float, default=3)
    parser.add_argument('--timeout', type=float, default=240)
    parser.add_argument('--regime', default='loadup')
    parser.add_argument('--tools')
    args = parser.parse_args()
    args.binary = args.binary.resolve()
    args.output = args.output.resolve()
    args.output.mkdir(parents=True)
    with tempfile.TemporaryDirectory(prefix='isolates-project-', dir=args.output) as scratch:
        project = args.project.resolve() if args.project else make_project(Path(scratch), args.gnu_lisp)
        results = [session(args, variant, project, i) for variant in ('isolate', 'sync') for i in range(args.sessions)]
        gnu = gnu_batch(args.gnu_batch, project) if args.gnu_batch else None
    texts = {}
    for r in results:
        if r['valid']:
            texts.setdefault(r['variant'], {})['xref'] = r.pop('xref_text')
            texts[r['variant']]['candidates'] = r.pop('candidates_text')
    identical = {kind: len({texts[v][kind] for v in texts}) == 1 for kind in ('xref', 'candidates')} if texts else {}
    summary = {}
    for r in results:
        if not r['valid']:
            continue
        for command in ('find_regexp', 'find_regexp_warm', 'find_file'):
            a = r[command]
            summary.setdefault(r['variant'], {})[command] = {
                'job_s': a['job_s'], 'command_s': a['command_s'], 'search_s': a['search_s'], 'delivery_s': a['delivery_s'],
                'key_median_during_job_s': a['median_during_s'], 'key_max_during_job_s': a['max_during_s'],
                'render_median_ms': statistics.median(a['render_ms']) if a['render_ms'] else None,
                'render_max_ms': max(a['render_ms']) if a['render_ms'] else None, 'keys_ran': a['keys_ran'],
                'result_lines': a['result_lines']}
    isolate = summary.get('isolate', {})
    sync = summary.get('sync', {})
    # Responsive: while the search runs, a key's command runs within 50 ms
    # in the isolate path, against the hundreds of milliseconds it waits
    # when the session searches itself. A key sent once the result is being
    # delivered waits for that delivery, as it does in GNU for the whole
    # command; its length is delivery_s.
    responsive = all(isolate.get(c, {}).get('key_max_during_job_s') is not None and sync.get(c, {}).get('key_max_during_job_s') is not None
                     and isolate[c]['key_max_during_job_s'] < 0.05 and isolate[c]['key_max_during_job_s'] < 0.5 * sync[c]['key_max_during_job_s']
                     for c in ('find_regexp', 'find_regexp_warm'))
    status = 'PASS' if results and all(r['valid'] for r in results) and all(identical.values()) and responsive else 'FAIL'
    report = {'status': status, 'summary': summary, 'gnu_batch': gnu, 'results_identical': identical, 'responsive': responsive,
              'texts': {v: {k: (t or '')[:2000] for k, t in kinds.items()} for v, kinds in texts.items()},
              'sessions': results, 'binary': str(args.binary), 'gnu_batch_binary': str(args.gnu_batch) if args.gnu_batch else None,
              'keys': args.keys, 'gap_s': args.gap}
    (args.output / 'report.json').write_text(json.dumps(report, indent=1) + '\n')
    print(json.dumps({'status': status, 'summary': summary, 'gnu_batch': gnu, 'results_identical': identical, 'responsive': responsive}, indent=1))
    return 0 if status == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
