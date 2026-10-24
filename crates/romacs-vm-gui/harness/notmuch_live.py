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
TO = 'Dan Example <dan@example.invalid>'
CC = 'Carol Example <carol@example.invalid>'
CONFIGURED_FROM = 'Configured Override <override@example.invalid>'
SELECTED_ID = 'fixture-grandchild@example.invalid'
QUERY = 'id:' + SELECTED_ID
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


def notmuch(command, env, *arguments):
    """Read or mutate only the caller's owned synthetic index."""
    return subprocess.run(command + list(arguments), env=env, check=True,
                          capture_output=True, timeout=20).stdout


def prepare_fixture(root, env):
    """Index a five-message branching thread and an independent search thread."""
    maildir = root / 'Maildir'
    for name in ('cur', 'new', 'tmp'):
        (maildir / name).mkdir(parents=True)
    root_id = 'fixture-root@example.invalid'
    child_id = 'fixture-child@example.invalid'
    messages = [
        (root_id, 'Alex Example <alex@example.invalid>', SUBJECT, '', '',
         'The root is context, never the selected reply source.\n'),
        (child_id, 'Bob Example <bob@example.invalid>', SUBJECT,
         root_id, f'<{root_id}>', 'The intermediate child is context.\n'),
        (SELECTED_ID, TO, SUBJECT, child_id, f'<{root_id}> <{child_id}>', BODY),
        ('fixture-excluded@example.invalid', CC, SUBJECT, root_id,
         f'<{root_id}>', 'This excluded branch remains visible as context.\n'),
        ('fixture-orphan@example.invalid', 'Eve Example <eve@example.invalid>', SUBJECT,
         'missing-parent@example.invalid', f'<{root_id}> <missing-parent@example.invalid>',
         'A descendant whose direct parent is absent.\n'),
        ('fixture-independent@example.invalid', 'Sam Example <sam@example.invalid>',
         'A small reading list', '', '', 'A short synthetic reading list.\n'),
    ]
    for index, (identifier, sender, subject, parent, references, body) in enumerate(messages):
        threading = (f'In-Reply-To: <{parent}>\nReferences: {references}\n' if parent else '')
        cc = f'Cc: {CC}\n' if identifier == SELECTED_ID else ''
        (maildir / 'new' / str(index)).write_text(
            f'From: {sender}\nTo: {FROM}\n{cc}Subject: {subject}\n'
            f'Date: Fri, 23 Oct 2026 09:{15 + index:02d}:00 +0000\n'
            f'Message-ID: <{identifier}>\n{threading}MIME-Version: 1.0\n'
            f'Content-Type: text/plain; charset=utf-8\n\n{body}', encoding='utf-8')
    config = root / 'notmuch-config'
    config.write_text(f'[database]\npath={maildir}\n[user]\nname=Reference User\n'
                      'primary_email=reference@example.invalid\n[new]\ntags=inbox;unread;\n'
                      '[search]\nexclude_tags=deleted;\n'
                      '[maildir]\nsynchronize_flags=false\n', encoding='utf-8')
    command = ['/usr/bin/notmuch', '--config=' + str(config)]
    notmuch(command, env, 'new')
    notmuch(command, env, 'tag', '+deleted', '--', 'id:fixture-excluded@example.invalid')
    rows = json.loads(notmuch(command, env, 'search', '--format=json', '--', 'tag:inbox'))
    assert len(rows) == 2, 'PRIVATE_NOTMUCH_FIXTURE_HAS_EXACTLY_TWO_THREADS'
    assert sorted(row['total'] for row in rows) == [1, 5], 'PRIVATE_BRANCH_THREAD_HAS_FIVE_MESSAGES'
    selected = json.loads(notmuch(command, env, 'search', '--format=json', '--', QUERY))
    assert len(selected) == 1 and selected[0]['matched'] == 1 and selected[0]['total'] == 5
    transport = root / 'capture_mail.py'
    captured = root / 'captured.eml'
    transport.write_text('from pathlib import Path\nimport sys\n'
                         'Path(sys.argv[1]).write_bytes(sys.stdin.buffer.read())\n', encoding='utf-8')
    return command, [sys.executable, str(transport), str(captured)], captured


def reply_expectation(command, env, all_recipients):
    """Retain the real CLI envelope as the independent compose/wire oracle."""
    result = json.loads(notmuch(command, env, 'reply', '--format=json', '--format-version=5',
                               '--reply-to=' + ('all' if all_recipients else 'sender'), '--', QUERY))
    headers = result['reply-headers']
    assert headers['From'] == FROM and headers['To'] == TO, 'BACKEND_INFERRED_REPLY_IDENTITY'
    assert headers.get('Cc', '') == (CC if all_recipients else ''), 'BACKEND_REPLY_RECIPIENTS'
    assert headers['In-reply-to'] == '<' + SELECTED_ID + '>', 'BACKEND_SELECTED_GRANDCHILD'
    assert headers['References'] == ('<fixture-root@example.invalid> '
                                     '<fixture-child@example.invalid> <' + SELECTED_ID + '>')
    original = result['original']['headers']
    attribution = 'On ' + original['Date'] + ', ' + original['From'] + ' wrote:'
    quoted = attribution + '\n> ' + BODY.replace('\n', '\n> ') + '\n\n'
    return headers, quoted


def validate_wire(data, headers, expected_body):
    """Require exact RFC bytes, allowing only generated Date and Message-ID."""
    mail = BytesParser(policy=policy.default).parsebytes(data)
    expected = {name: headers.get(name, '') for name in ('From', 'To', 'Cc', 'Subject')}
    expected.update({'In-Reply-To': headers['In-reply-to'], 'References': headers['References'],
                     'User-Agent': 'Romacs notmuch 0.1'})
    for name, value in expected.items():
        assert str(mail.get(name, '')) == value, 'EXACT_CAPTURED_RFC_' + name.upper()
    payload = mail.get_payload(decode=True)
    assert isinstance(payload, bytes) and payload.decode('utf-8') == expected_body, 'EXACT_CAPTURED_RFC_BODY'
    raw = dict(mail.raw_items())
    date, identifier = raw.get('Date', ''), raw.get('Message-ID', '')
    offset = parsedate_to_datetime(date).utcoffset()
    assert offset is not None and offset.total_seconds() == 0, 'RFC_DATE_IS_UTC'
    assert re.fullmatch(r'<romacs-[^<>\s]+@localhost>', identifier), 'RFC_MESSAGE_ID_VALID'
    initial = ''.join(name + ': ' + headers.get(name, '') + '\n'
                      for name in ('From', 'To', 'Cc', 'Subject'))
    expected_rfc = (initial + f'Date: {date}\nMessage-ID: {identifier}\n'
                    + 'In-Reply-To: ' + headers['In-reply-to'] + '\n'
                    + 'References: ' + headers['References'] + '\n'
                    + 'User-Agent: Romacs notmuch 0.1\nMIME-Version: 1.0\n'
                    + 'Content-Type: text/plain; charset=utf-8\nContent-Transfer-Encoding: 8bit\n\n'
                    + expected_body).encode('utf-8')
    assert data == expected_rfc, 'EXACT_CAPTURED_RFC_BYTES'



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
            tags_before = notmuch(command, env, 'dump', '--format=batch-tag', '--', '*')
            (output / 'tags-before.dump').write_bytes(tags_before)
            all_headers, quote = reply_expectation(command, env, True)
            sender_headers, _ = reply_expectation(command, env, False)
            report['checks'].append('PRIVATE_REAL_NOTMUCH_SIX_MESSAGES_TWO_THREADS_BRANCH_EXCLUDED_AND_MISSING_PARENT')
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

                def evaluate(expression, marker):
                    meta(':')
                    visible('Eval: ')
                    keys('(progn ' + expression + ' (message "' + marker + '"))', '-k', 'Return')
                    visible(marker)

                def require(expression, marker):
                    evaluate('(unless ' + expression + ' (error "' + marker + '_FAILED"))', marker)
                    assert not any(marker + '_FAILED' in row for row in ff.current_rows(trace())), marker

                def selected(identifier, marker):
                    require('(equal (cdr (plist-get (romacs-mail-row) :key)) '
                            + json.dumps(identifier) + ')', marker)

                def discard_draft():
                    control('c')
                    control('k')
                    visible('Discard this mail draft? Type yes to discard: ')
                    keys('yes', '-k', 'Return')

                visible('Ordinary sentinel')
                control('x')
                keys('1')
                control('e')
                keys(' DIRTY')
                visible('Ordinary sentinel DIRTY')
                meta(':')
                visible('Eval: ')
                lisp_list = lambda values: "'(" + ' '.join(json.dumps(v) for v in values) + ')'
                expression = ('(progn (setq romacs-notmuch-command ' + lisp_list(command)
                              + ' romacs-notmuch-from ' + json.dumps(CONFIGURED_FROM)
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
                extended('notmuch-search')
                visible('Notmuch search: ')
                keys(QUERY, '-k', 'Return')
                switch('ordinary.txt')
                visible('Ordinary sentinel DIRTY')
                keys('!')
                visible('Ordinary sentinel DIRTY!')
                capture('ordinary-edit-after-search-dispatch')
                assert original.read_text() == INITIAL, 'ORDINARY_INITIAL_FILE_UNCHANGED'
                report['checks'].append('NATIVE_ORDINARY_EDIT_AND_POINT_SURVIVE_ASYNC_SEARCH_DISPATCH')
                switch('*mail-search*<2>')
                visible('End of search results.')
                visible('[1/5]')
                meta('<')
                evaluate('(setq mail-harness-search (current-buffer) mail-harness-search-point (point))',
                         'SEARCH_POINT_SAVED')
                keys('-k', 'Return')
                visible('End of thread.')
                visible('[excluded]')
                selected(SELECTED_ID, 'INITIAL_MATCHING_GRANDCHILD')
                require("(and (eq major-mode 'romacs-mail-tree-mode) (not (get-buffer \"*mail-message*\")) "
                        '(= (length romacs-mail-rows) 5))', 'TREE_ONLY_FIVE_CONTEXT_ROWS')
                capture('thread-tree')
                keys('n', 'p')
                selected(SELECTED_ID, 'MATCHING_NAVIGATION_SKIPS_CONTEXT')
                keys('P')
                selected('fixture-child@example.invalid', 'ALL_PREVIOUS_INCLUDES_CONTEXT')
                keys('n')
                selected(SELECTED_ID, 'MATCHING_NEXT_RETURNS_GRANDCHILD')
                evaluate('(setq mail-harness-tree (current-buffer) mail-harness-tree-point (point))',
                         'TREE_POINT_SAVED')
                keys('-k', 'Return')
                visible('Everything in this mailbox is synthetic.')
                visible('Subject: ' + SUBJECT)
                visible('To: ' + FROM)
                visible('Cc: ' + CC)
                visible(TO)
                visible('Date: Fri, 23 Oct 2026 09:17:00 +0000')
                require("(and (eq major-mode 'romacs-mail-message-mode) "
                        '(eq (window-buffer (selected-window)) (current-buffer)) '
                        '(equal (cdr romacs-mail-active-key) ' + json.dumps(SELECTED_ID) + '))',
                        'EXACT_MESSAGE_SELECTED')
                evaluate('(setq mail-harness-pane (selected-window) mail-harness-message (current-buffer))',
                         'MESSAGE_PAIR_SAVED')
                require('(let ((tree (get-buffer-window mail-harness-tree))) '
                        '(and tree (not (eq tree (selected-window))) '
                        '(< (abs (- (* 4 (window-pixel-height tree)) '
                        '(+ (window-pixel-height tree) (window-pixel-height (selected-window))))) 160)))',
                        'TREE_TOP_QUARTER_MESSAGE_LOWER_SELECTED')
                visible('▶ ')
                capture('selected-grandchild-message-pane')
                report['checks'].append('NATIVE_TWO_RET_TREE_MATCHING_AND_ALL_NAVIGATION_EXACT_SELECTED_MESSAGE_QUARTER_PAIR')
                control('x')
                keys('o')
                require('(eq (current-buffer) mail-harness-tree)', 'OTHER_WINDOW_IS_TREE')
                keys('N')
                selected('fixture-excluded@example.invalid', 'ALL_NEXT_INCLUDES_EXCLUDED')
                keys('-k', 'Return')
                visible('This excluded branch remains visible as context.')
                require('(and (eq (selected-window) mail-harness-pane) '
                        '(eq (current-buffer) mail-harness-message) '
                        '(equal (cdr romacs-mail-active-key) "fixture-excluded@example.invalid"))',
                        'SECOND_RET_REUSES_EXACT_PAIR')
                capture('reused-message-pane-excluded-row')
                keys('q')
                visible('End of thread.')
                selected('fixture-excluded@example.invalid', 'MESSAGE_Q_RETURNS_EXACT_TREE_ROW')
                require('(and (eq (current-buffer) mail-harness-tree) '
                        '(not (get-buffer-window mail-harness-message)))', 'MESSAGE_Q_CLOSES_OWNED_PAIR')
                keys('p')
                selected(SELECTED_ID, 'MATCHING_PREVIOUS_SKIPS_CONTEXT')
                keys('q')
                visible('End of search results.')
                require('(and (eq (current-buffer) mail-harness-search) '
                        '(= (point) mail-harness-search-point))', 'TREE_Q_RESTORES_SEARCH_POINT')
                capture('q-chain-search-restored')
                report['checks'].append('NATIVE_PAIR_REUSE_AND_Q_CHAIN_PRESERVE_ROW_AND_SEARCH_POINT')
                keys('-k', 'Return')
                visible('End of thread.')
                selected(SELECTED_ID, 'REOPEN_TREE_SELECTS_GRANDCHILD')
                keys('r')
                visible('Subject: Re: ' + SUBJECT)
                visible('From: ' + sender_headers['From'])
                visible('To: ' + sender_headers['To'])
                require('(and (not (string-match "carol@example.invalid" (buffer-string))) '
                        '(equal (plist-get romacs-mail-envelope :in-reply-to) '
                        + json.dumps(sender_headers['In-reply-to']) + '))', 'SENDER_REPLY_EXCLUDES_CC_AND_THREADS_EXACT_MESSAGE')
                capture('reply-sender-from-tree')
                discard_draft()
                switch('*mail-tree*')
                selected(SELECTED_ID, 'TREE_ROW_AFTER_DISCARD')
                keys('-k', 'Return')
                visible('Everything in this mailbox is synthetic.')
                extended('notmuch-show-reply-sender')
                visible('To: ' + sender_headers['To'])
                require('(not (string-match "carol@example.invalid" (buffer-string)))', 'PUBLIC_SENDER_WRAPPER')
                discard_draft()
                switch('*mail-message*')
                keys('R')
                visible('Subject: Re: ' + SUBJECT)
                visible('From: ' + all_headers['From'])
                visible('To: ' + all_headers['To'])
                visible('Cc: ' + all_headers['Cc'])
                require('(and (equal (plist-get romacs-mail-envelope :in-reply-to) '
                        + json.dumps(all_headers['In-reply-to']) + ') '
                        '(equal (plist-get romacs-mail-envelope :references) '
                        + json.dumps(all_headers['References']) + '))', 'REPLY_ALL_EXACT_BACKEND_THREADING')
                meta('>')
                keys(REPLY)
                visible('This submission goes only to the private fake transport.')
                meta('<')
                capture('compose-reply-all')
                report['checks'].append('NATIVE_R_SENDER_PUBLIC_WRAPPER_AND_R_ALL_BACKEND_INFERRED_FROM_CC_QUOTED_GRANDCHILD')
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
                validate_wire(data, all_headers, quote + REPLY)
                (output / 'fake-transport-captured.eml').write_bytes(data)
                scene = fio.last_scene(trace())
                assert scene and 'Mail sent' in ''.join(g['char'] for g in scene['glyphs']), 'VISIBLE_MAIL_SENT_STATE'
                capture('sent')
                report['checks'].append('NATIVE_C_C_C_C_YES_FAKE_TRANSPORT_EXACT_RFC_AND_VISIBLE_SENT_STATE')
                broken = root / 'failing_notmuch.py'
                broken.write_text('import sys\nsys.stderr.write("PRIVATE_NOTMUCH_FAILURE\\n")\nsys.exit(23)\n',
                                  encoding='utf-8')
                evaluate('(setq romacs-notmuch-command ' + lisp_list([sys.executable, str(broken)]) + ')',
                         'BROKEN_COMMAND_CONFIGURED_FOR_NEW_ACCOUNT')
                extended('notmuch')
                visible('Mail Search failed:')
                visible('PRIVATE_NOTMUCH_FAILURE')
                capture('truthful-new-account-search-failure')
                tags_after = notmuch(command, env, 'dump', '--format=batch-tag', '--', '*')
                (output / 'tags-after.dump').write_bytes(tags_after)
                assert tags_after == tags_before, 'ALL_FIXTURE_TAGS_BYTE_IDENTICAL_AFTER_MAIL_AND_FAILURE'
                report['checks'].append('NEW_ACCOUNT_BACKEND_FAILURE_TRUTHFUL_AND_ALL_TAGS_UNCHANGED')
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
