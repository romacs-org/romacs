#!/usr/bin/env python3
"""Feed each GNU-read top-level form of FILE to one VM2 --eval-stream session.
Usage: drive.py VM2_BIN FILE...   Prints per-file accepted/total, first error, error histogram."""
import json, subprocess, sys, collections, os
vm2 = sys.argv[1]
HERE = os.path.dirname(os.path.abspath(__file__))
for path in sys.argv[2:]:
    spl = os.path.join(HERE, os.path.basename(path) + '.forms')
    subprocess.run(['emacs', '-Q', '--batch', '-l', os.path.join(HERE, 'split.el'), path, spl], check=True, capture_output=True)
    text = open(path, encoding='utf-8').read()
    forms = [json.loads(l) for l in open(spl)]
    p = subprocess.Popen([vm2, '--eval-stream'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=open(os.path.join(HERE, os.path.basename(path)+'.stderr'),'w'), text=True, bufsize=1)
    ok = 0; first = None; died = None; hist = collections.Counter(); heads_fail = collections.Counter()
    for i, f in enumerate(forms):
        src = text[f['s'] - 1:f['e'] - 1]
        p.stdin.write(json.dumps({'source': src}) + '\n'); p.stdin.flush()
        line = p.stdout.readline()
        if not line:
            died = (i, f['head'], text.count('\n', 0, f['s']) + 1)
            break
        r = json.loads(line)
        if 'error' in r:
            hist[r['error'][:60]] += 1; heads_fail[f['head']] += 1
            if first is None:
                first = (i, f['head'], r['error'][:120], text.count('\n', 0, f['s']) + 1)
        else:
            ok += 1
    p.stdin.close(); p.wait()
    print(f"== {os.path.basename(path)}: accepted {ok}/{len(forms)} top-level forms")
    print(f"   first failure: form#{first[0]} ({first[1]}) line {first[3]}: {first[2]}" if first else "   no failure")
    if died: print(f"   VM2 PROCESS DIED (panic) at form#{died[0]} ({died[1]}) line {died[2]}; forms after it not attempted")
    print("   errors:", dict(hist.most_common(6)))
    print("   failing heads:", dict(heads_fail.most_common(8)))
