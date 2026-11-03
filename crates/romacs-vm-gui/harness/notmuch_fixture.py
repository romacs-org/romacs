#!/usr/bin/env python3
"""A private synthetic mail store for the real notmuch package.

`build(root, anchor)' writes ROOT/mail, a maildir of about 200 generated
messages (threads, attachments, HTML, UTF-8 and RFC 2047 headers) dated
relative to ANCHOR (epoch seconds), ROOT/notmuch-config naming it, and its
notmuch database, then tags it as a user's mailbox looks after a sync.
The same SEED and ANCHOR give the same messages, files and thread order,
so two engines on two copies see the same store.  Nothing here reads the
caller's mail, ~/.notmuch-config or the network.

usage: notmuch_fixture.py ROOT [--anchor EPOCH]   (prints the environment)
"""
import argparse
from email.headerregistry import Address
from email.message import EmailMessage
from email.utils import format_datetime
import datetime
import json
import os
from pathlib import Path
import random
import subprocess
import sys

SEED = 'romacs-real-notmuch-1'
OWNER = Address('Owner Example', 'owner', 'example.org')
STUDENT = Address('Owner Student', 'owner', 'students.example.org')
PEOPLE = [
    Address('Alex Rivera', 'alex', 'example.org'),
    Address('Βασίλης Παπαδόπουλος', 'vasilis', 'example.org'),
    Address('Zoë Müller', 'zoe', 'example.org'),
    Address('Dan Example', 'dan', 'example.org'),
    Address('Carol Example', 'carol', 'example.org'),
    Address('李小龍', 'li', 'example.org'),
    Address('Renée Dubois', 'renee', 'example.org'),
    Address('Lists Bot', 'bot', 'lists.example.org'),
]
TOPICS = ['Project notes for Friday', 'Lab rotation schedule', 'Ελληνικά: πρόγραμμα εξετάσεων',
          'Café meeting — agenda', 'Release 0.4 checklist', 'Fwd: conference tickets 🎟',
          'Weekly digest', 'Pathology slides', 'Re: grant budget', 'Übersicht der Änderungen',
          '会议记录', 'Patch review: fix the parser', 'Bike repair', 'Invoice 2026-10',
          'Reading list', 'Garden plan', 'Server maintenance window', 'Thesis draft comments',
          'Hiking on Saturday?', 'Lunch?']
WORDS = ('alpha beta gamma delta notes plan review draft schedule meeting result patch '
         'thread search index value table figure slide budget ticket weekend coffee').split()
GREEK = 'Καλημέρα σε όλους, το πρόγραμμα αλλάζει την Πέμπτη.'
PNG = bytes.fromhex('89504e470d0a1a0a0000000d4948445200000001000000010806000000'
                    '1f15c4890000000d49444154789c6360f8cf00000301010018dd8db00000000049454e44ae426082')


def paragraph(rng, words=40):
    text = ' '.join(rng.choice(WORDS) for _ in range(words))
    return text[0].upper() + text[1:] + '.'


def html_body(rng, subject):
    items = ''.join(f'<li>{rng.choice(WORDS)} <b>{rng.choice(WORDS)}</b></li>' for _ in range(3))
    return (f'<html><body><h1>{subject}</h1><p>{paragraph(rng, 25)}</p>'
            f'<p><a href="https://example.org/{rng.randrange(1000)}">a link</a> and '
            f'<i>emphasis</i>.</p><ul>{items}</ul><table><tr><td>one</td><td>two</td></tr></table>'
            '</body></html>')


def message(rng, index, thread, depth, parent, date, sender, recipients, subject):
    """One message's EmailMessage; its kind follows INDEX deterministically."""
    m = EmailMessage()
    m['From'] = sender
    m['To'] = recipients
    if index % 7 == 3:
        m['Cc'] = PEOPLE[(index + 2) % len(PEOPLE)]
    m['Subject'] = ('Re: ' if depth else '') + subject
    m['Date'] = format_datetime(date)
    m['Message-ID'] = f'<fixture-{index:03d}@example.org>'
    if parent:
        m['In-Reply-To'] = parent
        m['References'] = parent
    kind = index % 10
    text = f'Hello,\n\n{paragraph(rng)}\n\n{paragraph(rng, 20)}\n'
    if index % 11 == 5:
        text += f'\n{GREEK}\nZoë’s café: naïve façade — “quotes” ✓\n'
    if depth:
        text += '\n> ' + paragraph(rng, 12) + '\n'
    text += f'\n-- \n{sender.display_name}\n'
    if kind == 2:
        m.set_content(html_body(rng, subject), subtype='html')
    elif kind == 4:
        m.set_content(text)
        m.add_alternative(html_body(rng, subject), subtype='html')
    elif kind == 6:
        m.set_content(text)
        m.add_attachment(f'col1,col2\n{index},{depth}\n', filename=f'data-{index}.csv')
        m.add_attachment(PNG, maintype='image', subtype='png', filename='dot.png')
    elif kind == 8:
        m.set_content(text)
        m.add_attachment(b'%PDF-1.4\n% synthetic\n' + bytes(range(256)), maintype='application',
                         subtype='pdf', filename='Übersicht ü.pdf')
    else:
        m.set_content(text, cte='quoted-printable' if index % 3 == 0 else None)
    return m


def messages(anchor):
    """(MESSAGE, FOLDER, FLAGS) for every message of the store."""
    rng = random.Random(SEED)
    now = datetime.datetime.fromtimestamp(anchor, datetime.timezone.utc).replace(microsecond=0)
    out, index = [], 0
    for thread in range(48):
        subject = TOPICS[thread % len(TOPICS)] + (f' #{thread // len(TOPICS)}' if thread >= len(TOPICS) else '')
        size = 1 + rng.randrange(7)
        # Newest threads first in thread number: the first few fall today and
        # this week, the rest over two months.
        start = now - datetime.timedelta(hours=1 + thread * 30 + rng.randrange(20), minutes=rng.randrange(60))
        parent = None
        for depth in range(size):
            if index >= 200:
                break
            sender = OWNER if depth % 3 == 2 else PEOPLE[(thread + depth) % len(PEOPLE)]
            to = PEOPLE[(thread + 1) % len(PEOPLE)] if sender == OWNER else (STUDENT if thread % 9 == 4 else OWNER)
            date = min(now - datetime.timedelta(minutes=5), start + datetime.timedelta(minutes=37 * depth))
            m = message(rng, index, thread, depth, parent, date, sender, to, subject)
            parent = m['Message-ID']
            folder = 'sent' if sender == OWNER else 'INBOX'
            flags = 'S' if sender == OWNER or thread >= 6 or depth < size - 1 else ''
            if index % 13 == 0:
                flags = 'F' + flags
            out.append((m, folder, flags))
            index += 1
    return out


# Subjects and authors as real mail carries them, raw: a form feed and a
# TAB (notmuch-sanitize turns them into spaces), a C1 control, a raw byte,
# no-break spaces and emoji with a variation selector and zero-width
# joiners, which GNU shows as escapes, no-break glyphs and thin spaces.
CONTROL_SUBJECTS = [b'Form\x0cfeed and\ttab', 'Next\u0085line control'.encode(), b'Raw \xff byte in the header',
                    'Heart \u2764\ufe0f family \U0001f468\u200d\U0001f469\u200d\U0001f467 grin \U0001f600'.encode(),
                    'No\u00a0break\u00a0space'.encode(), 'Mixed \u0085\u00a0\u200b end \U0001f3df'.encode()]
CONTROL_AUTHORS = ['Zoë\u00a0Müller'.encode(), b'Raw \xe9 Sender', 'Plain Sender'.encode()]


def control_messages(anchor):
    """Raw messages whose headers carry CONTROL_SUBJECTS, tagged `controls'."""
    out = []
    for n in range(36):
        date = format_datetime(datetime.datetime.fromtimestamp(anchor - 3600 * (n + 1), datetime.timezone.utc))
        subject = CONTROL_SUBJECTS[n % len(CONTROL_SUBJECTS)] + f' {n}'.encode()
        author = CONTROL_AUTHORS[n % len(CONTROL_AUTHORS)]
        out.append(b'From: ' + author + b' <sender' + str(n).encode() + b'@example.org>\n'
                   b'To: owner@example.org\nSubject: ' + subject + b'\nDate: ' + date.encode() +
                   b'\nMessage-ID: <controls-' + str(n).encode() + b'@example.org>\nMIME-Version: 1.0\n'
                   b'Content-Type: text/plain; charset=utf-8\n\nA message whose headers carry controls.\n')
    return out


def notmuch(env, *arguments, input=None):
    return subprocess.run(['notmuch', *arguments], env=env, check=True, capture_output=True,
                          timeout=120, input=input).stdout


def config_text(mail):
    return (f'[database]\npath={mail}\n\n[user]\nname={OWNER.display_name}\n'
            f'primary_email={OWNER.addr_spec}\nother_email={STUDENT.addr_spec}\n\n'
            '[new]\ntags=unread;inbox;\nignore=\n\n[search]\nexclude_tags=deleted;spam;\n\n'
            '[maildir]\nsynchronize_flags=true\n')


def environment(root):
    """The variables a process needs to see only this store."""
    return {'NOTMUCH_CONFIG': str(root / 'notmuch-config'), 'MAILDIR': str(root / 'mail')}


def build(root, anchor):
    root = Path(root).resolve()
    mail = root / 'mail'
    for folder in ('INBOX', 'sent', 'drafts', 'mail-vps/Sent', 'controls'):
        for sub in ('cur', 'new', 'tmp'):
            (mail / folder / sub).mkdir(parents=True, exist_ok=True)
    (root / 'notmuch-config').write_text(config_text(mail))
    for number, (m, folder, flags) in enumerate(messages(anchor)):
        name = f'{anchor}.{number:03d}.fixture'
        path = mail / folder / ('cur' if flags or folder == 'sent' else 'new') / (f'{name}:2,{flags}' if flags or folder == 'sent' else name)
        path.write_bytes(m.as_bytes())
        stamp = datetime.datetime.strptime(m['Date'], '%a, %d %b %Y %H:%M:%S %z').timestamp()
        os.utime(path, (stamp, stamp))
    for number, raw in enumerate(control_messages(anchor)):
        (mail / 'controls' / 'new' / f'{anchor}.c{number:03d}.fixture').write_bytes(raw)
    env = dict(os.environ, HOME=str(root), **environment(root))
    notmuch(env, 'new', '--quiet')
    # As after a user's sync: their sent mail is tagged sent and out of the
    # inbox, a few threads archived, one list's mail tagged.
    batch = ('-inbox -unread +sent -- folder:sent\n'
             '-inbox +archived -- date:..21d\n'
             '+lists -- from:bot@lists.example.org\n'
             '+emacs-pm -- subject:"Patch review"\n'
             '+controls -- folder:controls\n')
    notmuch(env, 'tag', '--batch', input=batch.encode())
    return environment(root)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('root')
    parser.add_argument('--anchor', type=int, default=None)
    args = parser.parse_args()
    anchor = args.anchor or int(datetime.datetime.now().timestamp())
    print(json.dumps(build(args.root, anchor)))
    sys.exit(0)
