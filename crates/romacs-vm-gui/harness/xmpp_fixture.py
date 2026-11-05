#!/usr/bin/env python3
"""A private XMPP server for jabber's journeys: synthetic accounts, rooms and archives.

One account (`lane@fixture.test', password `fixture') on 127.0.0.1, plain
TCP, SCRAM-SHA-1.  The server answers what a client's session asks for:
bind, session, roster (--contacts items, their available presence after
the client's own), disco#info and #items, private storage (no bookmarks),
vCard (empty), ping, and MAM (XEP-0313 with XEP-0059 paging) over a
personal archive and one archive per room.  Rooms (`--rooms', NAME:
MESSAGES:OCCUPANTS each) answer a join with every occupant's presence,
then the self-presence and the subject; after the join a busy room keeps
talking (--live-rate messages a second) and its occupants come and go
(--presence-rate a second).  Archives are generated from SEED and ANCHOR
(epoch seconds): the same arguments give the same stanzas, so two clients
see the same stream.  A page holds at most --max-page results whatever
the client asks for, as servers clamp it (Prosody's default is 50).

Nothing here touches the network beyond 127.0.0.1 or anyone's account.
Every request is appended to --log as a JSON line (time, kind, archive,
RSM max/after/before, results sent), the summary to --stats at exit.

usage: xmpp_fixture.py --port N [--rooms busy1:5000:800,...] [--log FILE]
                       [--stats FILE] [--anchor EPOCH] [--max-page 50]
"""
import argparse
import asyncio
import base64
import bisect
import hashlib
import hmac
import json
import os
import random
import signal
import sys
import time
import xml.etree.ElementTree as ET
from xml.sax.saxutils import escape, quoteattr

DOMAIN = 'fixture.test'
MUC = 'conference.fixture.test'
USER, PASSWORD = 'lane', 'fixture'
SEED = 'romacs-xmpp-fixture-1'
WORDS = ('the build is green again after the merge but the perf numbers moved a little '
         'has anyone tried the new release on aarch64 yet I see a crash in the tls handshake '
         'patch sent to the list please review when you have a minute thanks '
         'Ελληνικά καλημέρα σε όλους — café naïve über 日本語 テスト 🎉 👍 '
         'the archive query returns fine here but the client stalls while it redraws').split()
NS_MAM, NS_RSM = 'urn:xmpp:mam:2', 'http://jabber.org/protocol/rsm'


def q(value):
    return quoteattr(str(value))


def stamp(epoch):
    return time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime(epoch))


def sentence(rng, low=4, high=40):
    return ' '.join(rng.choice(WORDS) for _ in range(rng.randint(low, high)))


class Archive:
    """One archive: entries (uid, epoch, stanza builder args), oldest first.
    Indexed: a page costs its own size, not the archive's, so the client's
    work dominates what a journey measures."""

    def __init__(self, jid, entries):
        self.jid, self.entries = jid, []
        self.index, self.times, self.by_peer = {}, [], {}
        for entry in entries:
            self.append(entry)

    def append(self, entry):
        uid, t, args = entry
        self.index[uid] = len(self.entries)
        self.entries.append(entry)
        self.times.append(t)
        self.by_peer.setdefault(peer_of(self.jid, args), []).append(len(self.entries) - 1)

    def page(self, start, after, before, limit, with_=None):
        """The results a query selects (None: an unknown cursor), whether
        the page reaches the archive's end in its direction, and the count
        of the filtered archive.  WITH_ keeps one peer's messages."""
        if with_ is not None:
            positions = self.by_peer.get(with_, [])
        else:
            positions = range(len(self.entries))
        low = bisect.bisect_left(self.times, start) if start is not None else 0
        lo = bisect.bisect_left(positions, low)
        hi = len(positions)
        if before is not None and before is not True:
            if before not in self.index:
                return None, False, 0
            hi = bisect.bisect_left(positions, self.index[before])
        if after is not None:
            if after not in self.index:
                return None, False, 0
            lo = max(lo, bisect.bisect_right(positions, self.index[after]))
        count = len(positions) - bisect.bisect_left(positions, low)
        if before is not None:
            chosen = [self.entries[positions[i]] for i in range(max(lo, hi - limit), hi)] if limit else []
            return chosen, hi - lo <= limit, count
        chosen = [self.entries[positions[i]] for i in range(lo, min(hi, lo + limit))]
        return chosen, lo + limit >= hi, count


def peer_of(archive_jid, args):
    """The bare JID a personal archive's message is with (the room for a room's)."""
    mtype, frm, _origin, _body, _extra, mto, _nick = args
    if mtype == 'groupchat':
        return archive_jid
    me = f'{USER}@{DOMAIN}'
    bare = lambda jid: jid.split('/')[0]
    return bare(mto) if bare(frm) == me else bare(frm)


def room_archive(name, messages, occupants, anchor):
    rng = random.Random(f'{SEED}:{name}')
    jid = f'{name}@{MUC}'
    nicks = [f'nick{i}' for i in range(max(1, occupants))]
    entries, t = [], anchor - messages * 30
    for i in range(messages):
        t += rng.randint(1, 59)
        uid = f'{name}-{i:06d}'
        kind = rng.random()
        nick = 'lane' if rng.random() < 0.02 else rng.choice(nicks)
        extra = ''
        if kind < 0.03 and i:
            extra = (f"<reply xmlns='urn:xmpp:reply:0' to={q(jid + '/' + rng.choice(nicks))} id={q(f'{name}-orig-{rng.randrange(i)}')}/>")
        elif kind < 0.05:
            extra = (f"<x xmlns='jabber:x:oob'><url>https://files.fixture.test/{uid}.png</url></x>")
        body = sentence(rng)
        if kind < 0.05 and not extra.startswith('<reply'):
            body = f'https://files.fixture.test/{uid}.png'
        entries.append((uid, t, ('groupchat', f'{jid}/{nick}', f'{name}-orig-{i}', body, extra, jid, nick)))
    return Archive(jid, entries)


def personal_archive(messages, contacts, anchor):
    rng = random.Random(f'{SEED}:personal')
    me = f'{USER}@{DOMAIN}'
    entries, t = [], anchor - messages * 120
    for i in range(messages):
        t += rng.randint(1, 239)
        peer = f'contact{rng.randrange(max(1, contacts // 5))}@{DOMAIN}'
        out = rng.random() < 0.4
        frm, to = (f'{me}/phone', peer) if out else (f'{peer}/laptop', me)
        entries.append((f'p-{i:06d}', t, ('chat', frm, f'p-orig-{i}', sentence(rng), '', to, None)))
    return Archive(me, entries)


def result_stanza(archive, queryid, entry, to):
    uid, t, (mtype, frm, origin, body, extra, mto, nick) = entry
    occupant = (f"<occupant-id xmlns='urn:xmpp:occupant-id:0' id={q('occ-' + nick)}/>" if nick else '')
    return (f"<message to={q(to)} from={q(archive.jid)}><result xmlns='{NS_MAM}' queryid={q(queryid)} id={q(uid)}>"
            f"<forwarded xmlns='urn:xmpp:forward:0'><delay xmlns='urn:xmpp:delay' stamp={q(stamp(t))}/>"
            f"<message xmlns='jabber:client' type={q(mtype)} from={q(frm)} to={q(mto)} id={q(origin)}>"
            f"<body>{escape(body)}</body>{extra}{occupant}"
            f"<stanza-id xmlns='urn:xmpp:sid:0' by={q(archive.jid)} id={q(uid)}/>"
            f"<origin-id xmlns='urn:xmpp:sid:0' id={q(origin)}/><markable xmlns='urn:xmpp:chat-markers:0'/>"
            f"</message></forwarded></result></message>")


def scram_keys(password, salt, iterations):
    salted = hashlib.pbkdf2_hmac('sha1', password.encode(), salt, iterations)
    client_key = hmac.new(salted, b'Client Key', 'sha1').digest()
    return salted, client_key, hashlib.sha1(client_key).digest()


class Server:
    def __init__(self, args):
        self.args = args
        anchor = args.anchor or int(time.time())
        self.anchor = anchor
        self.rooms = {}
        for spec in filter(None, args.rooms.split(',')):
            name, messages, occupants = spec.split(':')
            self.rooms[f'{name}@{MUC}'] = (room_archive(name, int(messages), int(occupants), anchor), int(occupants))
        self.personal = personal_archive(args.personal, args.contacts, anchor)
        self.stats = {'iq': {}, 'mam_queries': 0, 'mam_results': 0, 'presence_sent': 0,
                      'live_messages': 0, 'bytes_sent': 0, 'sessions': 0}
        self.log = open(args.log, 'a') if args.log else None

    def note(self, **record):
        if self.log:
            record['t'] = round(time.time(), 6)
            self.log.write(json.dumps(record) + '\n')
            self.log.flush()

    async def serve(self):
        server = await asyncio.start_server(self.session, '127.0.0.1', self.args.port)
        print(f'xmpp_fixture listening on 127.0.0.1:{self.args.port}', flush=True)
        async with server:
            await server.serve_forever()

    async def session(self, reader, writer):
        self.stats['sessions'] += 1
        await Session(self, reader, writer).run()

    def write_stats(self):
        if self.args.stats:
            with open(self.args.stats, 'w') as f:
                json.dump(self.stats, f, indent=1)


class Session:
    def __init__(self, server, reader, writer):
        self.server, self.reader, self.writer = server, reader, writer
        self.authed, self.jid, self.tasks = False, None, []
        self.joined = {}

    def send(self, text):
        data = text.encode()
        self.server.stats['bytes_sent'] += len(data)
        self.writer.write(data)

    def new_parser(self):
        self.parser = ET.XMLPullParser(('start', 'end'))
        self.depth = 0

    async def run(self):
        self.new_parser()
        try:
            while True:
                data = await self.reader.read(65536)
                if not data:
                    break
                self.parser.feed(data)
                for event, element in self.parser.read_events():
                    if event == 'start':
                        self.depth += 1
                        if self.depth == 1:
                            self.open_stream()
                    else:
                        self.depth -= 1
                        if self.depth == 1:
                            restart = self.handle(element)
                            await self.writer.drain()
                            if restart:
                                self.new_parser()
                                break
                        elif self.depth == 0:
                            self.send('</stream:stream>')
                            return
        except (ConnectionError, ET.ParseError) as error:
            self.server.note(kind='session-error', error=str(error))
        finally:
            for task in self.tasks:
                task.cancel()
            self.writer.close()

    def open_stream(self):
        self.send("<?xml version='1.0'?><stream:stream xmlns='jabber:client' "
                  "xmlns:stream='http://etherx.jabber.org/streams' "
                  f"from='{DOMAIN}' id='fixture-{os.getpid()}-{time.time_ns()}' version='1.0' xml:lang='en'>")
        if not self.authed:
            self.send("<stream:features><mechanisms xmlns='urn:ietf:params:xml:ns:xmpp-sasl'>"
                      "<mechanism>SCRAM-SHA-1</mechanism></mechanisms></stream:features>")
        else:
            self.send("<stream:features><bind xmlns='urn:ietf:params:xml:ns:xmpp-bind'/>"
                      "<session xmlns='urn:ietf:params:xml:ns:xmpp-session'><optional/></session>"
                      "</stream:features>")

    def handle(self, element):
        tag = element.tag.split('}')[-1]
        if tag == 'auth':
            self.scram_first(base64.b64decode(element.text or ''))
        elif tag == 'response':
            return self.scram_final(base64.b64decode(element.text or ''))
        elif tag == 'iq':
            self.iq(element)
        elif tag == 'presence':
            self.presence(element)
        elif tag == 'message':
            self.server.stats['client_messages'] = self.server.stats.get('client_messages', 0) + 1
            self.server.note(kind='client-message', to=element.get('to'))
        return False

    # SCRAM-SHA-1 (RFC 5802), the account's one password.
    def scram_first(self, message):
        text = message.decode()
        self.client_first_bare = text.split(',', 2)[2]
        attrs = dict(part.split('=', 1) for part in self.client_first_bare.split(','))
        self.nonce = attrs['r'] + base64.b64encode(os.urandom(12)).decode()
        self.salt = b'fixture-salt'
        self.server_first = f"r={self.nonce},s={base64.b64encode(self.salt).decode()},i=4096"
        self.send("<challenge xmlns='urn:ietf:params:xml:ns:xmpp-sasl'>"
                  f"{base64.b64encode(self.server_first.encode()).decode()}</challenge>")

    def scram_final(self, message):
        text = message.decode()
        without_proof, proof = text.rsplit(',p=', 1)
        salted, client_key, stored_key = scram_keys(PASSWORD, self.salt, 4096)
        auth = ','.join((self.client_first_bare, self.server_first, without_proof)).encode()
        signature = hmac.new(stored_key, auth, 'sha1').digest()
        expected = bytes(a ^ b for a, b in zip(client_key, signature))
        if base64.b64decode(proof) != expected:
            self.send("<failure xmlns='urn:ietf:params:xml:ns:xmpp-sasl'><not-authorized/></failure>")
            return False
        server_key = hmac.new(salted, b'Server Key', 'sha1').digest()
        verifier = base64.b64encode(hmac.new(server_key, auth, 'sha1').digest()).decode()
        self.send("<success xmlns='urn:ietf:params:xml:ns:xmpp-sasl'>"
                  f"{base64.b64encode(('v=' + verifier).encode()).decode()}</success>")
        self.authed = True
        return True

    def origin(self, iq):
        """Who answers IQ: its addressee, or the account itself (RFC 6120 10.3)."""
        return iq.get('to') or (f'{USER}@{DOMAIN}' if self.jid else DOMAIN)

    def result(self, iq, payload=''):
        self.send(f"<iq type='result' id={q(iq.get('id'))} from={q(self.origin(iq))} to={q(self.jid)}>{payload}</iq>")

    def error(self, iq, condition='service-unavailable', kind='cancel'):
        self.send(f"<iq type='error' id={q(iq.get('id'))} from={q(self.origin(iq))} to={q(self.jid or '')}>"
                  f"<error type='{kind}'><{condition} xmlns='urn:ietf:params:xml:ns:xmpp-stanzas'/></error></iq>")

    def iq(self, iq):
        child = next(iter(iq), None)
        ns = child.tag.split('}')[0][1:] if child is not None and '}' in child.tag else ''
        local = child.tag.split('}')[-1] if child is not None else ''
        counts = self.server.stats['iq']
        counts[ns] = counts.get(ns, 0) + 1
        to = iq.get('to')
        if ns == 'urn:ietf:params:xml:ns:xmpp-bind':
            resource = child.findtext('{urn:ietf:params:xml:ns:xmpp-bind}resource') or 'fixture'
            self.jid = f'{USER}@{DOMAIN}/{resource}'
            self.result(iq, f"<bind xmlns='{ns}'><jid>{escape(self.jid)}</jid></bind>")
            if self.server.args.push_bookmark:
                self.tasks.append(asyncio.get_running_loop().create_task(self.push_bookmark()))
        elif ns in ('urn:ietf:params:xml:ns:xmpp-session', 'urn:xmpp:ping', 'urn:xmpp:carbons:2'):
            self.result(iq)
        elif ns == 'jabber:iq:roster' and iq.get('type') == 'get':
            items = ''.join(f"<item jid='contact{i}@{DOMAIN}' name='Contact {i}' subscription='both'><group>Friends</group></item>"
                            for i in range(self.server.args.contacts))
            self.result(iq, f"<query xmlns='jabber:iq:roster'>{items}</query>")
        elif ns == 'jabber:iq:private':
            self.result(iq, "<query xmlns='jabber:iq:private'><storage xmlns='storage:bookmarks'/></query>")
        elif ns == 'vcard-temp':
            self.result(iq, "<vCard xmlns='vcard-temp'/>")
        elif ns == 'http://jabber.org/protocol/disco#info':
            self.disco_info(iq, to)
        elif ns == 'http://jabber.org/protocol/disco#items':
            room = self.server.rooms.get(to)
            items = ''.join(f"<item jid={q(f'{to}/nick{i}')} name={q(f'nick{i}')}/>" for i in range(room[1])) if room else ''
            self.result(iq, f"<query xmlns='{ns}'>{items}</query>")
        elif ns == NS_MAM and local == 'query':
            self.mam(iq, child, to)
        else:
            self.error(iq)

    def disco_info(self, iq, to):
        features = ['http://jabber.org/protocol/disco#info', NS_MAM, 'urn:xmpp:ping']
        identity = "<identity category='account' type='registered'/>"
        form = ''
        if to in self.server.rooms:
            occupants = self.server.rooms[to][1]
            identity = f"<identity category='conference' type='text' name={q(to.split('@')[0])}/>"
            features += ['http://jabber.org/protocol/muc', 'muc_public', 'muc_open', 'muc_semianonymous',
                         'muc_persistent', 'urn:xmpp:occupant-id:0', 'urn:xmpp:sid:0']
            form = ("<x xmlns='jabber:x:data' type='result'><field var='FORM_TYPE' type='hidden'>"
                    "<value>http://jabber.org/protocol/muc#roominfo</value></field>"
                    f"<field var='muc#roominfo_occupants'><value>{occupants}</value></field></x>")
        elif to in (None, DOMAIN):
            identity = "<identity category='server' type='im'/>"
            features += ['urn:xmpp:carbons:2']
        body = ''.join(f"<feature var={q(f)}/>" for f in features)
        self.result(iq, f"<query xmlns='http://jabber.org/protocol/disco#info'>{identity}{body}{form}</query>")

    def mam(self, iq, query, to):
        archive = self.server.rooms[to][0] if to in self.server.rooms else (
            self.server.personal if to in (None, f'{USER}@{DOMAIN}') else None)
        if archive is None:
            return self.error(iq, 'item-not-found')
        queryid = query.get('queryid') or ''
        fields = {f.get('var'): f.findtext('{jabber:x:data}value') for f in query.iter('{jabber:x:data}field')}
        rsm = query.find(f'{{{NS_RSM}}}set')
        wanted = self.server.args.max_page
        after = before = None
        if rsm is not None:
            if rsm.findtext(f'{{{NS_RSM}}}max'):
                wanted = int(rsm.findtext(f'{{{NS_RSM}}}max'))
            after = rsm.findtext(f'{{{NS_RSM}}}after')
            node = rsm.find(f'{{{NS_RSM}}}before')
            if node is not None:
                before = node.text or True
        limit = min(wanted, self.server.args.max_page)
        start = None
        if fields.get('start'):
            start = int(time.mktime(time.strptime(fields['start'], '%Y-%m-%dT%H:%M:%SZ')) - time.timezone)
        chosen, complete, count = archive.page(start, after, before, limit, fields.get('with'))
        self.server.stats['mam_queries'] += 1
        self.server.note(kind='mam', archive=archive.jid, max=wanted, after=after,
                         before=before if before is not True else 'last', results=len(chosen or []))
        if chosen is None:
            return self.error(iq, 'item-not-found')
        for entry in chosen:
            self.send(result_stanza(archive, queryid, entry, self.jid))
        self.server.stats['mam_results'] += len(chosen)
        rsm_out = (f"<set xmlns='{NS_RSM}'><first index='0'>{chosen[0][0]}</first><last>{chosen[-1][0]}</last>"
                   f"<count>{count}</count></set>" if chosen else f"<set xmlns='{NS_RSM}'><count>{count}</count></set>")
        self.result(iq, f"<fin xmlns='{NS_MAM}' complete={q('true' if complete else 'false')}>{rsm_out}</fin>")

    def presence(self, presence):
        to = presence.get('to')
        self.server.stats['client_presences'] = self.server.stats.get('client_presences', 0) + 1
        self.server.note(kind='client-presence', to=to, type=presence.get('type'))
        if to is None:
            if presence.get('type') is None and not self.joined:
                self.roster_presence()
            return
        room = to.split('/')[0]
        if room in self.server.rooms and presence.get('type') is None and room not in self.joined:
            self.join(room, to.split('/', 1)[1] if '/' in to else USER)
        elif room in self.joined and presence.get('type') == 'unavailable':
            task = self.joined.pop(room)
            task.cancel()
            self.send(f"<presence from={q(to)} to={q(self.jid)} type='unavailable'><x xmlns='http://jabber.org/protocol/muc#user'>"
                      "<item affiliation='none' role='none'/><status code='110'/></x></presence>")

    async def push_bookmark(self):
        """A PEP bookmarks notification naming --push-bookmark with autojoin,
        as a server pushes when another client adds a bookmark."""
        await asyncio.sleep(2)
        room = self.server.args.push_bookmark
        self.send(f"<message from='{USER}@{DOMAIN}' to={q(self.jid)} type='headline'>"
                  "<event xmlns='http://jabber.org/protocol/pubsub#event'><items node='urn:xmpp:bookmarks:1'>"
                  f"<item id={q(room)}><conference xmlns='urn:xmpp:bookmarks:1' autojoin='true' name='pushed'>"
                  f"<nick>{USER}</nick></conference></item></items></event></message>")
        self.server.note(kind='push-bookmark', room=room)
        await self.writer.drain()

    def roster_presence(self):
        rng = random.Random(f'{SEED}:roster')
        for i in range(self.server.args.contacts * 2 // 3):
            self.send(f"<presence from='contact{i}@{DOMAIN}/laptop' to={q(self.jid)}><show>{rng.choice(['away', 'chat', 'dnd', 'xa'])}</show>"
                      f"<status>{escape(sentence(rng, 1, 6))}</status><priority>0</priority>"
                      f"<c xmlns='http://jabber.org/protocol/caps' hash='sha-1' node='https://client.fixture.test' ver='fixturever{i % 4}='/>"
                      "</presence>")
            self.server.stats['presence_sent'] += 1

    def occupant(self, room, nick, rng, kind=None):
        role = rng.choice(['participant'] * 8 + ['moderator', 'visitor'])
        return (f"<presence from={q(f'{room}/{nick}')} to={q(self.jid)}{' type=' + q(kind) if kind else ''}>"
                f"<c xmlns='http://jabber.org/protocol/caps' hash='sha-1' node='https://client.fixture.test' ver='fixturever{rng.randrange(4)}='/>"
                f"<occupant-id xmlns='urn:xmpp:occupant-id:0' id={q('occ-' + nick)}/>"
                f"<x xmlns='http://jabber.org/protocol/muc#user'><item affiliation='none' role={q(role if not kind else 'none')}/></x>"
                "</presence>")

    def join(self, room, nick):
        archive, occupants = self.server.rooms[room]
        rng = random.Random(f'{SEED}:join:{room}')
        for i in range(occupants):
            self.send(self.occupant(room, f'nick{i}', rng))
        self.server.stats['presence_sent'] += occupants
        self.send(f"<presence from={q(f'{room}/{nick}')} to={q(self.jid)}><x xmlns='http://jabber.org/protocol/muc#user'>"
                  "<item affiliation='member' role='participant'/><status code='110'/></x></presence>")
        self.send(f"<message from={q(room)} to={q(self.jid)} type='groupchat'><subject>Fixture room {escape(room)}</subject></message>")
        self.joined[room] = asyncio.get_running_loop().create_task(self.live(room, archive, occupants))
        self.tasks.append(self.joined[room])

    async def live(self, room, archive, occupants):
        args = self.server.args
        if occupants < args.busy_occupants or (args.live_rate <= 0 and args.presence_rate <= 0):
            return
        rng = random.Random(f'{SEED}:live:{room}')
        n, gone = 0, set()
        rates = [r for r in (args.live_rate, args.presence_rate) if r > 0]
        tick = 1.0 / max(rates)
        next_message = next_presence = time.monotonic()
        while True:
            await asyncio.sleep(tick)
            now = time.monotonic()
            if args.live_rate > 0 and now >= next_message:
                next_message = now + 1.0 / args.live_rate
                n += 1
                nick = f'nick{rng.randrange(occupants)}'
                uid = f'live-{room.split("@")[0]}-{n:06d}'
                body = sentence(rng)
                archive.append((uid, int(time.time()), ('groupchat', f'{room}/{nick}', uid + '-o', body, '', room, nick)))
                self.send(f"<message from={q(f'{room}/{nick}')} to={q(self.jid)} type='groupchat' id={q(uid + '-o')}>"
                          f"<body>{escape(body)}</body><occupant-id xmlns='urn:xmpp:occupant-id:0' id={q('occ-' + nick)}/>"
                          f"<stanza-id xmlns='urn:xmpp:sid:0' by={q(room)} id={q(uid)}/></message>")
                self.server.stats['live_messages'] += 1
            if args.presence_rate > 0 and now >= next_presence:
                next_presence = now + 1.0 / args.presence_rate
                nick = f'nick{rng.randrange(occupants)}'
                kind = None if nick in gone else 'unavailable'
                (gone.discard if kind is None else gone.add)(nick)
                self.send(self.occupant(room, nick, rng, kind))
                self.server.stats['presence_sent'] += 1
            await self.writer.drain()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--port', type=int, required=True)
    parser.add_argument('--rooms', default='busy1:5000:800,busy2:5000:400,quiet1:200:20')
    parser.add_argument('--personal', type=int, default=1000, help='messages in the personal archive')
    parser.add_argument('--contacts', type=int, default=150)
    parser.add_argument('--max-page', type=int, default=50)
    parser.add_argument('--live-rate', type=float, default=0.5, help='messages a second in each busy room')
    parser.add_argument('--presence-rate', type=float, default=2.0, help='occupant presences a second in each busy room')
    parser.add_argument('--busy-occupants', type=int, default=100, help='rooms with this many occupants keep talking')
    parser.add_argument('--anchor', type=int, default=0)
    parser.add_argument('--push-bookmark', help='a room JID a PEP bookmarks event names with autojoin, 2 s after bind')
    parser.add_argument('--log')
    parser.add_argument('--stats')
    args = parser.parse_args()
    server = Server(args)
    loop = asyncio.new_event_loop()
    for sig in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(sig, loop.stop)
    try:
        loop.create_task(server.serve())
        loop.run_forever()
    finally:
        server.write_stats()


if __name__ == '__main__':
    sys.exit(main())
