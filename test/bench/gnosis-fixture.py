#!/usr/bin/env python3
"""Write a deterministic synthetic Gnosis database for core-bench.

test/bench/gnosis-fixture.py OUT.db [N]

Schema: the Gnosis tables read by the themata dashboard (themata,
thema_tag, scheduler_baseline, scheduler_state, scheduler_config) with the
live column order, constraints and indexes.  Values use Gnosis's emacsql
encoding (strings and lists stored as `prin1' text, numbers as numbers).

Shape follows aggregate statistics of a real collection (no content is
copied): ~97% cloze, 2% MCQ, 1% basic; keimenon median ~100 chars with 15%
embedded newlines and occasional [[id:...][...]] links; hypothesis `(nil)'
for 68%; ~27 tags per thema (snake_case, ~17 chars) from an ~11K
vocabulary; 0.7% unsuspended.  Text mixes English with Greek (~5%) and
Cyrillic (~6%) scripts.
"""
import random
import sqlite3
import sys
from pathlib import Path

SCHEMA = [
    """CREATE TABLE themata (
       id INTEGER PRIMARY KEY,
       type TEXT NOT NULL,
       keimenon TEXT NOT NULL,
       hypothesis TEXT NOT NULL,
       answer TEXT NOT NULL,
       source_guid TEXT, archived_at_us INTEGER
        CHECK (archived_at_us IS NULL
               OR (typeof(archived_at_us) = 'integer' AND archived_at_us > 0)),
       accepted_aliases TEXT, rubric TEXT)""",
    """CREATE TABLE scheduler_config (id INTEGER PRIMARY KEY NOT NULL, algorithm TEXT NOT NULL,
       model TEXT NOT NULL, implementation TEXT NOT NULL, desired_retention REAL NOT NULL,
       parameters TEXT NOT NULL)""",
    """CREATE TABLE scheduler_baseline (thema_id INTEGER PRIMARY KEY NOT NULL, due_day INTEGER NOT NULL,
       reps INTEGER NOT NULL, lapses INTEGER NOT NULL,
       FOREIGN KEY (thema_id) REFERENCES themata (id) ON DELETE CASCADE)""",
    """CREATE TABLE scheduler_state (thema_id INTEGER PRIMARY KEY NOT NULL, config_id INTEGER NOT NULL,
       stability REAL, difficulty REAL, last_reviewed_at_us INTEGER, last_review_day INTEGER,
       due_day INTEGER NOT NULL, reps INTEGER NOT NULL, lapses INTEGER NOT NULL, suspended INTEGER NOT NULL,
       FOREIGN KEY (thema_id) REFERENCES scheduler_baseline (thema_id) ON DELETE CASCADE,
       FOREIGN KEY (config_id) REFERENCES scheduler_config (id))""",
    """CREATE TABLE thema_tag (
       thema_id INTEGER NOT NULL,
       tag TEXT NOT NULL,
       PRIMARY KEY (thema_id, tag),
       FOREIGN KEY (thema_id) REFERENCES themata (id) ON DELETE CASCADE)""",
    "CREATE INDEX idx_thema_tag_thema_id ON thema_tag(thema_id)",
    "CREATE INDEX idx_thema_tag_tag ON thema_tag(tag)",
    "CREATE INDEX idx_themata_active ON themata(id) WHERE archived_at_us IS NULL",
    "CREATE INDEX idx_scheduler_state_due ON scheduler_state(suspended, due_day, reps)",
]

EN = ("cardiac tamponade pulsus paradoxus systolic pressure inspiration emphysema enlargement "
      "air spaces necrosis gangrenous mummified tissue hyperthyroidism propranolol methadone "
      "receptor agonist opioid withdrawal eukaryotes polymerase inhibited amanitin kidney "
      "glomerulus nephritic syndrome hematuria proteinuria vasculitis granulomatosis anemia "
      "hemolysis spherocytosis membrane deficiency enzyme pathway glycolysis insulin secretion "
      "pancreatic islet beta cells thyroid hormone cortisol adrenal medulla catecholamines "
      "pheochromocytoma episodic hypertension headache sweating tachycardia associated with "
      "characterized by caused by treated with first line most common presentation of the a an "
      "is which type may be in patients during increased decreased").split()
GREEK = ("α-adrenergic β-blocker μ-receptor κ-opioid γ-globulin δ-wave Δ ενζυμο αιμοσφαιρίνη "
         "καρδιά νεφρός πνεύμονας ήπαρ σπλήνας αρτηρία φλέβα οξύ χρόνιο σύνδρομο").split()
CYR = ("седмица година месец ден сърце бъбрек бял дроб черен дроб кръв болест лечение "
       "симптом лекар болница здраве Здравей благодаря учебник").split()
TYPES = ['"cloze"'] * 970 + ['"mcq"'] * 20 + ['"basic"'] * 9 + ['"image-occlusion"']


def q(s):
    """prin1 a string with literal Unicode and escaped quotes/backslashes."""
    out = ['"']
    for ch in s:
        if ch in '"\\':
            out.append('\\' + ch)
        else:
            out.append(ch)
    out.append('"')
    return ''.join(out)


def words(r, n, script):
    pool = EN
    picks = [r.choice(pool) for _ in range(n)]
    if script == 'el':
        for _ in range(max(1, n // 6)):
            picks[r.randrange(n)] = r.choice(GREEK)
    elif script == 'ru':
        picks = [r.choice(CYR) for _ in range(n)]
    return picks


def keimenon(r, script):
    n = max(3, int(r.gauss(16, 6)))
    w = words(r, n, script)
    for _ in range(r.randint(0, 3)):
        i = r.randrange(n)
        w[i] = r.choice(['*%s*', '_%s_', '/%s/']) % w[i]
    if r.random() < 0.0015:
        w[r.randrange(n)] = '[[id:%08x-%04x][%s]]' % (r.getrandbits(32), r.getrandbits(16), r.choice(EN))
    text = ' '.join(w)
    if r.random() < 0.15:
        text += '\n' + ' '.join(words(r, r.randint(1, 4), script))
    return text


def main():
    out = Path(sys.argv[1])
    n = int(sys.argv[2]) if len(sys.argv) > 2 else 90000
    r = random.Random(0x6e6f7369)
    vocab = sorted({'_'.join(r.choice(EN) for _ in range(r.randint(1, 4))) for _ in range(14000)})
    hot = vocab[:40]
    if out.exists():
        out.unlink()
    db = sqlite3.connect(out)
    for stmt in SCHEMA:
        db.execute(stmt)
    db.execute("INSERT INTO scheduler_config VALUES (1, '\"fsrs\"', '\"gnosis-fsrs6-v1\"', "
               "'\"fsrs-rs-6.6.1\"', 0.9, '[0.212 1.2931 2.3065 8.2956]')")
    ids = set()
    while len(ids) < n:
        ids.add(r.randrange(1, 2 ** 60))
    ids = sorted(ids)
    them, tags, base, state = [], [], [], []
    for i in ids:
        script = 'ru' if r.random() < 0.06 else 'el' if r.random() < 0.05 else 'en'
        typ = r.choice(TYPES)
        k = keimenon(r, script)
        if typ == '"mcq"':
            opts = [' '.join(words(r, r.randint(1, 3), script)) for _ in range(4)]
            hyp = '(' + ' '.join(q(o) for o in opts) + ')'
            ans = '(' + q(opts[0]) + ')'
        else:
            hyp = '(nil)' if r.random() < 0.7 else '(' + q(' / '.join(words(r, 2, script))) + ')'
            ans = '(' + ' '.join(q(' '.join(words(r, r.randint(1, 3), script)))
                                for _ in range(1 if r.random() < 0.9 else 2)) + ')'
        them.append((i, typ, q(k), hyp, ans))
        tset = set(r.sample(hot, r.randint(3, 8)))
        tset.update(r.sample(vocab, max(1, min(80, int(r.gauss(21, 8))))))
        tags.extend((i, q(t)) for t in sorted(tset))
        due = 20261015 + r.randint(-30, 400)
        reps = r.randint(0, 30)
        base.append((i, due, reps, r.randint(0, 3)))
        state.append((i, 1, r.random() * 100, r.random() * 10, None, None, due, reps,
                      r.randint(0, 3), 0 if r.random() < 0.993 else 1))
    db.executemany("INSERT INTO themata (id, type, keimenon, hypothesis, answer) VALUES (?,?,?,?,?)", them)
    db.executemany("INSERT INTO thema_tag VALUES (?,?)", tags)
    db.executemany("INSERT INTO scheduler_baseline VALUES (?,?,?,?)", base)
    db.executemany("INSERT INTO scheduler_state VALUES (?,?,?,?,?,?,?,?,?,?)", state)
    db.commit()
    db.execute("ANALYZE")
    db.commit()
    db.execute("VACUUM")
    db.close()
    print(f'{out}: {n} themata, {len(tags)} tags')


if __name__ == '__main__':
    main()
