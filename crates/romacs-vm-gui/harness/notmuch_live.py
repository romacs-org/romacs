#!/usr/bin/env python3
"""Private native Notmuch search/read/reply/send journey; no real mail or network.

Run against a supplied, already-built binary. Trace rows synchronize native input;
retained compositor PNGs still require visual review. Headless elapsed time is a
harness budget measurement, never an editor performance claim.
"""
import argparse
from email import policy
from email.parser import BytesParser
from email.utils import parsedate_to_datetime
import hashlib
import json
import re
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

import fileio_live as fio
import find_file_live as ff
import live_oracle as live


FROM = 'Reference User <reference@example.invalid>'
TO = 'Alex Example <alex@example.invalid>'
SUBJECT = 'Project notes for Friday'
BODY = ('Hello Reference,\n\nHere are the project notes for Friday.\n\n'
        '- Review the search results and thread view.\n'
        '- Keep the message headers readable.\n'
        '- Prepare a reply without sending it.\n\n'
        'Everything in this mailbox is synthetic.\n\nBest,\nAlex\n')
REPLY = ('Hi Alex,\n\nThanks for the notes. The search and message views look clear.\n'
         'This submission goes only to the private fake transport.\n\nBest,\nReference\n')
INITIAL = 'Ordinary sentinel\nsecond line\n'
QUESTIONS = (
    'Do we provide the user all the essential information properly formatted?',
    'Do we reduce noise/clutter?',
    'Is this pretty and easy on the eyes?',
)


def prepare_fixture(root, env):
    """Create/index exactly two synthetic messages under the owned private root."""
    maildir = root / 'Maildir'
    for name in ('cur', 'new', 'tmp'):
        (maildir / name).mkdir(parents=True)
    messages = [
        ('fixture-1@example.invalid', TO, SUBJECT, 'Fri, 23 Oct 2026 09:15:00 +0000', BODY),
        ('fixture-2@example.invalid', 'Sam Example <sam@example.invalid>', 'A small reading list',
         'Sat, 24 Oct 2026 10:30:00 +0000', 'Hello Reference,\n\nA short synthetic reading list.\n'),
    ]
    for index, (identifier, sender, subject, date, body) in enumerate(messages):
        (maildir / 'new' / str(index)).write_text(
            f'From: {sender}\nTo: {FROM}\nSubject: {subject}\nDate: {date}\n'
            f'Message-ID: <{identifier}>\nMIME-Version: 1.0\n'
            f'Content-Type: text/plain; charset=utf-8\n\n{body}', encoding='utf-8')
    config = root / 'notmuch-config'
    config.write_text(f'[database]\npath={maildir}\n[user]\nname=Reference User\n'
                      'primary_email=reference@example.invalid\n[new]\ntags=inbox;unread;\n'
                      '[maildir]\nsynchronize_flags=false\n', encoding='utf-8')
    command = ['/usr/bin/notmuch', '--config=' + str(config)]
    subprocess.run(command + ['new'], env=env, check=True, capture_output=True, timeout=20)
    result = subprocess.run(command + ['search', '--format=json', '--', 'tag:inbox'],
                            env=env, check=True, capture_output=True, text=True, timeout=20)
    rows = json.loads(result.stdout)
    assert len(rows) == 2, 'PRIVATE_NOTMUCH_FIXTURE_HAS_EXACTLY_TWO_THREADS'
    transport = root / 'capture_mail.py'
    captured = root / 'captured.eml'
    transport.write_text('from pathlib import Path\nimport sys\n'
                         'Path(sys.argv[1]).write_bytes(sys.stdin.buffer.read())\n', encoding='utf-8')
    return command, [sys.executable, str(transport), str(captured)], captured


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--timeout', type=float, default=25)
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {'status': 'FAIL', 'binary': str(binary), 'checks': [], 'captures': [],
              'geometry': [2532, 1347], 'font': 'Iosevka', 'font_size': 20,
              'visual_review': {question: 'PENDING: inspect actual rendered captures' for question in QUESTIONS},
              'timing_scope': 'private headless harness budget only; not editor performance'}
    compositor = keyboard = process = None
    started = time.monotonic()
    try:
        report['binary_sha256'] = hashlib.sha256(binary.read_bytes()).hexdigest()
        with tempfile.TemporaryDirectory(prefix='notmuch-live-') as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            home, work, config = [root / name for name in ('home', 'work', 'config')]
            for directory in (home, work, config):
                directory.mkdir()
            env = {'PATH': '/usr/bin:/bin', 'HOME': str(home), 'LANG': 'C.UTF-8',
                   'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
                   'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
                   'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
                   'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
                   'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop',
                   'ROMACS_GUI_FONT': 'Iosevka', 'ROMACS_GUI_FONT_SIZE': '20', 'SHELL': '/usr/bin/bash'}
            command, transport, captured = prepare_fixture(root, env)
            report['checks'].append('PRIVATE_REAL_NOTMUCH_INDEX_TWO_SYNTHETIC_MESSAGES')
            original = work / 'ordinary.txt'
            original.write_text(INITIAL, encoding='utf-8')
            (config / 'rc.xml').write_text(live.RC_XML.replace('</labwc_config>',
                '<keyboard><keybind key="A-F4"><action name="Close"/></keybind></keyboard></labwc_config>'))
            log_path = output / 'binary.stderr'
            with (output / 'compositor.log').open('w') as compositor_log, log_path.open('w') as log:
                compositor = subprocess.Popen([live.LABWC, '-C', str(config)], env=env,
                                              stdout=compositor_log, stderr=compositor_log)
                fio.wait(lambda: any(p.is_socket() for p in root.glob('wayland-*')), args.timeout,
                         'private compositor', compositor)
                env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
                subprocess.run([live.RANDR, '--output', 'HEADLESS-1', '--custom-mode', '2532x1347'],
                               env=env, check=True, timeout=15)
                keyboard = subprocess.Popen(['/usr/bin/wtype', '-s', '120000', '-k', 'Shift_L'],
                                            env=env, stdout=log, stderr=log)
                time.sleep(.3)
                process = subprocess.Popen([str(binary), '--trace-scene', str(original)], env=env, cwd=work,
                                           stdin=subprocess.DEVNULL, stdout=log, stderr=log)

                def trace():
                    return log_path.read_text(errors='replace')

                def keys(*parts):
                    subprocess.run(['/usr/bin/wtype', '-d', '40', *parts], env=env, check=True, timeout=30)
                    time.sleep(.15)

                def control(key):
                    keys('-M', 'ctrl', key, '-m', 'ctrl')

                def meta(key):
                    keys('-M', 'alt', key, '-m', 'alt')

                def visible(text):
                    fio.wait(lambda: any(text in row for row in ff.current_rows(trace())),
                             args.timeout, text, process)

                def capture(name):
                    path = output / (name + '.png')
                    fio.shot(path, env, report)
                    assert path.is_file(), 'COMPOSITOR_CAPTURE_REQUIRED: ' + name
                    report['captures'].append({'name': name, 'rows': ff.current_rows(trace())})

                def extended(name):
                    meta('x')
                    visible('M-x ')
                    keys(name, '-k', 'Return')

                def switch(name):
                    control('x')
                    keys('b')
                    visible('Switch to buffer')
                    keys(name, '-k', 'Return')
                    fio.wait(lambda: not any('Switch to buffer' in row for row in ff.current_rows(trace())),
                             args.timeout, 'buffer switch completed', process)

                visible('Ordinary sentinel')
                control('x')
                keys('1')
                control('e')
                keys(' DIRTY')
                visible('Ordinary sentinel DIRTY')
                # Configure only the isolated index and stdin-only fake transport via real M-:.
                meta(':')
                visible('Eval: ')
                lisp_list = lambda values: "'(" + ' '.join(json.dumps(v) for v in values) + ')'
                expression = ('(progn (setq romacs-notmuch-command ' + lisp_list(command)
                              + ' romacs-notmuch-from ' + json.dumps(FROM)
                              + ' romacs-notmuch-send-command ' + lisp_list(transport)
                              + ') (message "PRIVATE_MAIL_CONFIGURED"))')
                keys(expression, '-k', 'Return')
                visible('PRIVATE_MAIL_CONFIGURED')
                report['checks'].append('NATIVE_META_COLON_CONFIGURES_PRIVATE_INDEX_AND_FAKE_STDIN_TRANSPORT')
                extended('notmuch')
                visible('End of search results.')
                visible(SUBJECT)
                visible('A small reading list')
                capture('search')
                report['checks'].append('NATIVE_M_X_NOTMUCH_REAL_INDEX_SEARCH_TWO_THREADS')
                # A second real asynchronous search, followed by ordinary buffer switching.
                extended('notmuch-search')
                visible('Notmuch search: ')
                keys('tag:inbox', '-k', 'Return')
                switch('ordinary.txt')
                visible('Ordinary sentinel DIRTY')
                keys('!')
                visible('Ordinary sentinel DIRTY!')
                capture('ordinary-edit-after-search-dispatch')
                assert original.read_text() == INITIAL, 'ORDINARY_INITIAL_FILE_UNCHANGED'
                report['checks'].append('NATIVE_ORDINARY_EDIT_AND_POINT_SURVIVE_ASYNC_SEARCH_DISPATCH')
                switch('*notmuch-search*<2>')
                visible('End of search results.')
                visible(SUBJECT)
                # Newest-first fixture places Sam first and Alex second.
                meta('<')
                keys('n', '-k', 'Return')
                visible('Subject: ' + SUBJECT)
                visible('Everything in this mailbox is synthetic.')
                visible('To: ' + FROM)
                capture('message')
                report['checks'].append('NATIVE_RET_READS_EXACT_REAL_NOTMUCH_THREAD_WITH_HEADERS_AND_BODY')
                keys('r')
                visible('Subject: Re: ' + SUBJECT)
                visible('From: ' + FROM)
                visible('To: ' + TO)
                meta('>')
                keys(REPLY)
                visible('This submission goes only to the private fake transport.')
                capture('compose')
                report['checks'].append('NATIVE_R_REPLY_EDITABLE_HEADERS_QUOTED_BODY_AND_TYPED_REPLY')
                control('c')
                control('c')
                visible('Submit this message? Type yes to send: ')
                capture('fake-send-confirmation')
                assert not captured.exists(), 'NO_TRANSPORT_BEFORE_EXPLICIT_YES'
                keys('yes', '-k', 'Return')
                fio.wait(lambda: 'Mail sent' in ''.join(
                    g['char'] for g in (fio.last_scene(trace()) or {}).get('glyphs', [])
                    if g['role'] == 'ModeLine'), args.timeout, 'visible Mail sent mode line', process)
                fio.wait(captured.exists, args.timeout, 'fake RFC stdin captured', process)
                data = captured.read_bytes()
                mail = BytesParser(policy=policy.default).parsebytes(data)
                expected_body = '\n> ' + BODY.replace('\n', '\n> ') + '\n' + REPLY
                expected_headers = {'From': FROM, 'To': TO, 'Subject': 'Re: ' + SUBJECT,
                                    'In-Reply-To': '<fixture-1@example.invalid>',
                                    'User-Agent': 'Romacs notmuch 0.1'}
                for header, expected in expected_headers.items():
                    assert str(mail[header]) == expected, 'EXACT_CAPTURED_RFC_' + header.upper()
                payload = mail.get_payload(decode=True)
                assert isinstance(payload, bytes), 'CAPTURED_RFC_HAS_PLAIN_BYTE_BODY'
                assert payload.decode('utf-8') == expected_body, 'EXACT_CAPTURED_RFC_BODY'
                raw_headers = dict(mail.raw_items())
                date = raw_headers.get('Date', '')
                identifier = raw_headers.get('Message-ID', '')
                parsed_date = parsedate_to_datetime(date)
                offset = parsed_date.utcoffset()
                assert offset is not None and offset.total_seconds() == 0, 'RFC_DATE_IS_UTC'
                assert re.fullmatch(r'<romacs-[^<>\s]+@localhost>', identifier), 'RFC_MESSAGE_ID_VALID'
                expected_rfc = (f'From: {FROM}\nTo: {TO}\nSubject: Re: {SUBJECT}\n'
                                f'Date: {date}\nMessage-ID: {identifier}\n'
                                'In-Reply-To: <fixture-1@example.invalid>\nUser-Agent: Romacs notmuch 0.1\nMIME-Version: 1.0\n'
                                'Content-Type: text/plain; charset=utf-8\nContent-Transfer-Encoding: 8bit\n\n'
                                + expected_body).encode('utf-8')
                assert data == expected_rfc, 'EXACT_CAPTURED_RFC_BYTES'
                (output / 'fake-transport-captured.eml').write_bytes(data)
                scene = fio.last_scene(trace())
                assert scene and 'Mail sent' in ''.join(g['char'] for g in scene['glyphs']), 'VISIBLE_MAIL_SENT_STATE'
                capture('sent')
                report['checks'].append('NATIVE_C_C_C_C_YES_FAKE_TRANSPORT_EXACT_RFC_AND_VISIBLE_SENT_STATE')
                # Reading/replying must never mark read or mutate tags.
                result = subprocess.run(command + ['search', '--format=json', '--', 'tag:inbox AND tag:unread'],
                                        env=env, capture_output=True, text=True, check=True, timeout=15)
                assert len(json.loads(result.stdout)) == 2, 'READ_REPLY_SEND_DO_NOT_MUTATE_FIXTURE_TAGS'
                switch('ordinary.txt')
                visible('Ordinary sentinel DIRTY!')
                keys('?')
                visible('Ordinary sentinel DIRTY!?')
                assert original.read_text() == INITIAL, 'INITIAL_DISK_FILE_PRESERVED_AFTER_MAIL'
                capture('ordinary-buffer-preserved-after-mail')
                report['checks'].append('ORDINARY_BUFFER_TEXT_POINT_AND_DISK_PRESERVED_AFTER_MAIL')
                keys('-M', 'alt', '-k', 'F4', '-m', 'alt')
                visible('discard unsaved changes?')
                capture('explicit-discard-confirmation')
                keys('y')
                process.wait(timeout=args.timeout)
                assert process.returncode == 0, 'NATIVE_EXPLICIT_DISCARD_CLEAN_EXIT'
                assert original.read_text() == INITIAL, 'EXPLICIT_DISCARD_PRESERVES_INITIAL_FILE'
                report['checks'].append('EXPLICIT_DISCARD_PROTECTS_DIRTY_ORDINARY_BUFFER_AND_EXITS_ZERO')
                report.update(status='PASS', exit=0)
                return 0
    except Exception as error:
        report['error'] = repr(error)
        return 1
    finally:
        for child in (process, keyboard, compositor):
            live.stop(child)
        report['headless_harness_elapsed_seconds'] = time.monotonic() - started
        (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
        (output / 'INDEX.txt').write_text('\n'.join(c['name'] + '.png' for c in report['captures']) + '\n')


if __name__ == '__main__':
    raise SystemExit(main())
