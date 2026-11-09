#!/usr/bin/env python3
"""Re-record pre-pin GNU recordings at the pin, and carry their pins over only
when nothing but the oracle's identity changed.

    rerecord.py KEY... [--jobs N]      KEY a path under tools/primh, e.g. evidence/eval.json
    rerecord.py --stale [--jobs N]     every recording whose GNU header is not the pin

The oracle is $ROMACS_GNU_ORACLE, refused unless it reports the pin
(tools/lib/gnu_oracle.py). Each file is re-recorded IN PLACE by its own
recorder, unmodified:

  primh evidence    tools/primh/record.py primh SPEC   (spec bound by name AND spec_sha256)
  bulk corpus       tools/primh/record.py bulk GROUP
  evidence/widen    tools/primh/widen_oracle.py record --gnu ORACLE --replace
  varh evidence     tools/varh/varh.py record --gnu ORACLE

Recorders write no pins (`primh.py record` writes none; widen and varh write
an empty map): a pin is a human judgement about a known difference. So, per
file, the fresh recording must equal the committed one EXACTLY except the
oracle's identity (IDENTITY below) and the pin map; every case -- form,
expected answer, regime variant -- must be identical. Only then is the
committed pin map copied back verbatim, and the file serialized the way its
recorder writes (proved by re-serializing the committed bytes first). Any
other difference stops the run with the file left as recorded, unpinned, so
nothing unexamined is laundered through a carried-over pin.
"""
import argparse
import concurrent.futures as cf
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

PRIMH = Path(__file__).resolve().parent.parent
REPO = PRIMH.parents[3]
sys.path.insert(0, str(REPO / "scripts"))
import gnu_oracle  # noqa: E402

# The only fields a change of oracle may move.
IDENTITY = {("gnu", "revision"), ("gnu_binary_sha256",), ("gnu", "binary"), ("gnu", "binary_sha256"),
            ("gnu_binary",)}
PIN_KEYS = ("pinned", "pins")


def sha(data):
    return hashlib.sha256(data).hexdigest()


def serializers():
    """The formats the recorders write: primh/bulk/varh indent=1, widen indent=2."""
    return {
        "indent1": lambda v: json.dumps(v, ensure_ascii=False, indent=1) + "\n",
        "indent2": lambda v: json.dumps(v, ensure_ascii=False, indent=2) + "\n",
    }


def recordings():
    """Recordings tests/gnu_oracle_pin.rs guards whose header does not name the pin."""
    pin, out = gnu_oracle.pin(), []
    for directory, suffix in (("evidence", ".json"), ("bulk", ".corpus.json"), ("../varh/evidence", ".json")):
        for path in sorted((PRIMH / directory).glob(f"*{suffix}")):
            if json.loads(path.read_text()).get("gnu", {}).get("revision") != pin:
                out.append(f"{directory}/{path.name}")
    return out


def bind_spec(ev):
    name, want = ev["primitive"], ev["spec_sha256"]
    for s in sorted(PRIMH.glob("*specs*/*.json")):
        try:
            if json.loads(s.read_text()).get("name") == name and sha(s.read_bytes()) == want:
                return s
        except json.JSONDecodeError:
            continue
    raise SystemExit(f"no spec with name {name!r} and sha256 {want}")


def command(key, ev, oracle):
    py = sys.executable
    if key == "../varh/evidence/varh.json":
        return [py, str(PRIMH.parent / "varh" / "varh.py"), "record", "--gnu", oracle]
    if key == "evidence/widen.json":
        return [py, str(PRIMH / "widen_oracle.py"), "record", "--gnu", oracle, "--replace"]
    if key.startswith("bulk/") and key.endswith(".corpus.json"):
        return [py, str(PRIMH / "record.py"), "bulk", key[len("bulk/"):-len(".corpus.json")]]
    if key.startswith("evidence/") and ev.get("primitive"):
        return [py, str(PRIMH / "record.py"), "primh", str(bind_spec(ev))]
    raise SystemExit(f"{key}: no recorder known")


def diff(old, new, path=()):
    """Paths at which OLD and NEW differ (recursing through dicts only)."""
    if isinstance(old, dict) and isinstance(new, dict):
        out = []
        for k in list(old) + [k for k in new if k not in old]:
            if k not in old or k not in new:
                out.append(path + (k,))
            else:
                out += diff(old[k], new[k], path + (k,))
        return out
    return [] if old == new else [path]


def run(key, oracle, logdir):
    target = PRIMH / key
    old_bytes = target.read_bytes()
    old = json.loads(old_bytes)
    fmt = next((n for n, f in serializers().items() if f(old) == old_bytes.decode()), None)
    if fmt is None:
        return key, f"REFUSED: no recorder format reproduces the committed bytes of {key}"
    cmd = command(key, old, oracle)
    env = dict(os.environ, ROMACS_GNU_ORACLE=oracle)
    log = logdir / (key.replace("/", "_").replace("..", "up") + ".log")
    with open(log, "w") as stream, open(os.devnull) as null:
        r = subprocess.run(cmd, stdin=null, stdout=stream, stderr=subprocess.STDOUT, env=env, cwd=REPO)
    if r.returncode != 0:
        target.write_bytes(old_bytes)
        return key, f"RECORDER FAILED exit {r.returncode} (see {log}); committed bytes restored"
    new = json.loads(target.read_bytes())
    if new.get("gnu", {}).get("revision") != gnu_oracle.pin():
        return key, f"REFUSED: fresh header revision {new.get('gnu', {}).get('revision')!r} is not the pin"
    changed = [p for p in diff(old, new) if p not in IDENTITY and p[0] not in PIN_KEYS]
    if changed:
        return key, ("CHANGED beyond the oracle identity; left recorded and UNPINNED: "
                     + ", ".join("/".join(map(str, p)) for p in changed[:20]))
    pin_key = next((k for k in PIN_KEYS if k in old), None)
    if pin_key and new.get(pin_key) not in (None, {}):
        return key, f"REFUSED: the recorder wrote a non-empty {pin_key}"
    restored = {k: (old[k] if k == pin_key else new[k]) for k in old}
    if set(restored) != set(old) or diff(old, restored) != [p for p in diff(old, new) if p[0] not in PIN_KEYS]:
        return key, "REFUSED: restore would not equal the committed file outside the identity"
    target.write_text(serializers()[fmt](restored))
    moved = sorted("/".join(map(str, p)) for p in diff(old, restored))
    pins = len(old.get(pin_key) or {}) if pin_key else 0
    return key, f"OK cases identical; {pins} pin(s) restored; moved: {', '.join(moved)}"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("keys", nargs="*")
    ap.add_argument("--stale", action="store_true")
    ap.add_argument("--jobs", type=int, default=6)
    ap.add_argument("--logs", default=str(Path(os.environ.get("TMPDIR", "/tmp")) / "rerecord-logs"))
    a = ap.parse_args()
    keys = list(a.keys)
    if a.stale:
        keys += [k for k in recordings() if k not in keys]
    if not keys and a.stale:
        print("every recording names the pin; nothing to re-record")
        return
    if not keys:
        ap.error("name KEYs or --stale")
    ident = gnu_oracle.require()
    print(f"oracle {ident['path']} revision {ident['revision']} sha256 {ident['sha256'][:16]}", flush=True)
    logdir = Path(a.logs)
    logdir.mkdir(parents=True, exist_ok=True)
    with cf.ThreadPoolExecutor(a.jobs) as pool:
        results = dict(pool.map(lambda k: run(k, ident["path"], logdir), keys))
    bad = 0
    for key in keys:
        print(f"{key}: {results[key]}")
        bad += not results[key].startswith("OK")
    print(f"{len(keys) - bad} of {len(keys)} re-recorded at the pin with pins restored; {bad} refused")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
