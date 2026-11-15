"""GNU's Lisp commits as the upstream digest reads them (tools/lisp-upstream).

Pure functions over `git log --full-diff --no-renames --name-status -z` text: the commits that
touch lisp/, each with all its files, so a commit that also changes src/ or test/ shows it.
"""
import fnmatch

# GNU's files under lisp/ that are not Lisp we would port: generated, or build and history files.
SKIP = ('lisp/ldefs-boot.el', 'lisp/ChangeLog.*', 'lisp/*/ChangeLog.*', 'lisp/Makefile.in',
        'lisp/README', 'lisp/*/README', 'lisp/COPYING')
# Outside lisp/, the changes that mean a Lisp-only port may be incomplete.
ALSO = ('src/', 'test/', 'etc/NEWS')


def parse_log(text):
    """Commits from `git log -z --name-status --format=commit %H%x1f%cs%x1f%s`: a header
    record, then (STATUS, PATH) records; --no-renames leaves no two-path record."""
    commits, current = [], None
    fields = text.split('\0')
    i = 0
    while i < len(fields):
        field = fields[i].lstrip('\n')
        if field.startswith('commit '):
            hash_, date, subject = field[len('commit '):].split('\x1f', 2)
            current = dict(commit=hash_, date=date, subject=subject, files=[], other=[])
            commits.append(current)
            i += 1
        elif field and current is not None and i + 1 < len(fields):
            status, path = field[0], fields[i + 1]
            (current['files'] if path.startswith('lisp/') else current['other']).append(dict(status=status, path=path))
            i += 2
        else:
            i += 1
    return commits


def classify(commits, exists):
    """Each Lisp file's meaning to us (`ours`, `absent`, or `new`, GNU added it), the skipped
    files dropped, and each commit's `also`: the ALSO areas it changes outside lisp/."""
    out = []
    for c in commits:
        files = [dict(f, ours='new' if f['status'] == 'A' else 'ours' if exists(f['path']) else 'absent')
                 for f in c['files'] if not any(fnmatch.fnmatch(f['path'], p) for p in SKIP)]
        also = sorted({a.rstrip('/') for f in c['other'] for a in ALSO if f['path'].startswith(a)})
        out.append(dict(commit=c['commit'], date=c['date'], subject=c['subject'], files=files, also=also))
    return out


def select(commits, path=None, ours_only=False):
    """The commits with a file left after the PATH glob and, with OURS_ONLY, only files we have
    or GNU added (dropping commits that change only files we do not keep)."""
    out = []
    for c in commits:
        files = [f for f in c['files'] if (not path or fnmatch.fnmatch(f['path'], path))
                 and (not ours_only or f['ours'] != 'absent')]
        if files:
            out.append(dict(c, files=files))
    return out
