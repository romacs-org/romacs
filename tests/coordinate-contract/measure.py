#!/usr/bin/env python3
"""Exact-scheduling, image-asserted primitive workload acquisition.

Existing syscall attribute owner: scripts/romacs_accept_counter.py.
No affinity, count scaling, elapsed-time claims or guard relaxation.
"""
import argparse, ctypes, fcntl, hashlib, json, os, platform, signal, statistics, struct, subprocess, sys, time
from pathlib import Path
if not __debug__:
    raise RuntimeError('Assertions are part of the fail-closed acquisition guard; do not use python -O')
ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from romacs_accept_counter import attributes
p = argparse.ArgumentParser()
p.add_argument('--binary', required=True)
p.add_argument('--binary-commit', required=True, help='source revision recorded by the shipping build')
p.add_argument('--image', required=True)
p.add_argument('--runtime', required=True)
p.add_argument('--output', required=True)
p.add_argument('--gnu', default='/usr/bin/emacs')
a = p.parse_args()
D = Path(a.output).resolve(); D.mkdir(parents=True, exist_ok=True)
ROM = str(Path(a.binary).resolve()); GNU = a.gnu
(D / 'raw').mkdir(exist_ok=True)
if (D / 'runtime').exists():
    assert (D / 'runtime').resolve() == Path(a.runtime).resolve(), 'runtime changed on resume'
else:
    (D / 'runtime').symlink_to(Path(a.runtime).resolve())
def runtime_fingerprint(root):
    digest = hashlib.sha256()
    for directory in ['lisp', 'lisp-bundled', 'etc', 'admin/unidata']:
        for file in sorted((root / directory).rglob('*')):
            if file.is_file():
                digest.update(str(file.relative_to(root)).encode() + b'\0')
                digest.update(hashlib.sha256(file.read_bytes()).digest())
    return digest.hexdigest()

manifest = {
    'image': str(Path(a.image).resolve()), 'binary': ROM,
    'runtime': str(Path(a.runtime).resolve()),
    'runtime_sha256': runtime_fingerprint(Path(a.runtime).resolve()),
    'binary_sha256': hashlib.sha256(Path(ROM).read_bytes()).hexdigest(),
    'image_sha256': hashlib.sha256(Path(a.image).read_bytes()).hexdigest(),
    'gnu_sha256': hashlib.sha256(Path(GNU).read_bytes()).hexdigest(),
    'workloads_sha256': hashlib.sha256((Path(__file__).parent / 'workloads.json').read_bytes()).hexdigest(),
    'binary_commit': a.binary_commit,
    'checkout_commit': subprocess.check_output(['git', '-C', str(ROOT), 'rev-parse', 'HEAD'], text=True).strip(),
    'start_load': Path('/proc/loadavg').read_text().strip(),
}
if (D / 'manifest.json').exists():
    old = json.loads((D / 'manifest.json').read_text())
    assert all(old[k] == v for k, v in manifest.items() if k not in ['start_load', 'checkout_commit']), 'measurement identity changed on resume'
else:
    (D / 'manifest.json').write_text(json.dumps(manifest, indent=2))
class Unscheduled(ValueError): pass

def instructions(binary,file,tag):
    env=os.environ.copy(); env['ROMACS_RUNTIME_ROOT']=str(D/'runtime')
    for k in list(env):
        if k.startswith('NEOVM_') or k.startswith('EMACS'): env.pop(k,None)
    env['HOME']=str(D/'home'/('romacs' if binary==ROM else 'gnu'))
    env['XDG_CACHE_HOME']=env['HOME']+'/.cache'
    command=[binary,'-Q','--batch']
    if binary==ROM:
        image=json.loads((D/'manifest.json').read_text())['image']
        env['SWEEP_EXPECT_IMAGE']=image
        command+=['--dump-file',image]
    else: env.pop('SWEEP_EXPECT_IMAGE',None)
    command+=['-l',str(file)]
    Path(env['HOME']).mkdir(parents=True,exist_ok=True)
    out=D/'raw'/f'{tag}.out'; err=D/'raw'/f'{tag}.err'
    child=os.fork()
    if child==0:
        try:
            os.kill(os.getpid(),signal.SIGSTOP)
            a=os.open(out,os.O_WRONLY|os.O_CREAT|os.O_TRUNC,0o600); b=os.open(err,os.O_WRONLY|os.O_CREAT|os.O_TRUNC,0o600)
            os.dup2(a,1);os.dup2(b,2)
            os.execve(binary,command,env)
        except BaseException: os._exit(127)
    event=None; reaped=False
    start=time.monotonic()
    try:
        os.waitpid(child,os.WUNTRACED)
        attr=(ctypes.c_ubyte*128).from_buffer(attributes()); libc=ctypes.CDLL(None,use_errno=True)
        event=libc.syscall({'x86_64':298,'aarch64':241}[platform.machine()],ctypes.byref(attr),child,-1,-1,8)
        if event<0: raise OSError(ctypes.get_errno(),'perf_event_open')
        fcntl.ioctl(event,0x2403,0);fcntl.ioctl(event,0x2400,0)
        os.kill(child,signal.SIGCONT)
        while True:
            pid,status=os.waitpid(child,os.WNOHANG)
            if pid: reaped=True;break
            if time.monotonic()-start>90: raise TimeoutError(tag)
            time.sleep(.005)
        fcntl.ioctl(event,0x2401,0)
        data=os.read(event,24)
        if len(data)!=24: raise ValueError('short read')
        count,enabled,running=struct.unpack('QQQ',data)
        record=dict(tag=tag,binary=binary,count=count,enabled=enabled,running=running,rc=os.waitstatus_to_exitcode(status),wall=time.monotonic()-start,load=Path('/proc/loadavg').read_text().strip(),output=out.read_text(),stderr=str(err))
        with (D/'raw.jsonl').open('a') as f: f.write(json.dumps(record)+'\n')
        if not count or not enabled or running != enabled: raise Unscheduled('PMU scheduling rejected '+str(record))
        if record['rc']: raise ValueError('editor exit '+str(record['rc'])+' '+err.read_text()[-1500:])
        answers=[s for s in record['output'].splitlines() if s.startswith('SWEEP=')]
        if len(answers)!=1: raise ValueError('missing/duplicate answer '+record['output'][:1000])
        record['answer']=answers[0]
        return record
    finally:
        if event is not None and event>=0: os.close(event)
        if not reaped: os.kill(child,signal.SIGKILL);os.waitpid(child,0)


rows = json.loads((Path(__file__).parent / 'workloads.json').read_text())['workloads']
done = set()
if (D / 'results.jsonl').exists():
    done = {(r['name'], r['fixture']) for r in map(json.loads, (D / 'results.jsonl').read_text().splitlines())}
for fixture in ['mixed', 'ascii']:
    for case in rows:
        name = case['name']
        if (name, fixture) in done: continue
        n = case['iterations']; forms = {}
        for mult in [1, 2]:
            form = case['form'].replace('ITERATIONS_TOKEN', str(n * mult))
            if fixture == 'ascii': form = form.replace('λ', 'l')
            forms[mult] = D / f'{fixture}-{name}-{mult}.el'
            forms[mult].write_text(form)
        slopes = {'romacs': [], 'gnu': []}; tags = []
        for rep in range(3):
            pair = {}
            for mult, engine in [(1, 'romacs'), (1, 'gnu'), (2, 'gnu'), (2, 'romacs')]:
                for attempt in range(10):
                    try:
                        r = instructions(ROM if engine == 'romacs' else GNU, forms[mult], f'{fixture}-{name}-{rep}-{engine}-{mult}-try{attempt}')
                        assert r['count'] > 0 and r['running'] == r['enabled'] > 0
                        break
                    except Unscheduled:
                        if attempt == 9: raise
                        time.sleep(1)
                pair[engine, mult] = r; tags.append(r['tag'])
            for mult in [1, 2]:
                assert pair['romacs', mult]['answer'] == pair['gnu', mult]['answer']
            for engine in slopes:
                delta = pair[engine, 2]['count'] - pair[engine, 1]['count']
                assert delta > 36, 'slope unresolved at measured counter resolution'
                slopes[engine].append(delta / n)
        rom = statistics.median(slopes['romacs']); gnu = statistics.median(slopes['gnu'])
        result = dict(name=name, fixture=fixture, iterations=n, romacs=rom, gnu=gnu, ratio=rom/gnu, slopes=slopes, accepted_tags=tags)
        with (D / 'results.jsonl').open('a') as f: f.write(json.dumps(result) + '\n')
        print(json.dumps(result), flush=True)
assert runtime_fingerprint(Path(a.runtime).resolve()) == manifest['runtime_sha256'], 'runtime changed during acquisition'
assert hashlib.sha256(Path(ROM).read_bytes()).hexdigest() == manifest['binary_sha256'], 'binary changed during acquisition'
assert hashlib.sha256(Path(a.image).read_bytes()).hexdigest() == manifest['image_sha256'], 'image changed during acquisition'
(D / 'verification.json').write_text(json.dumps({
    'runtime_unchanged': True,
    'raw_sha256': hashlib.sha256((D / 'raw.jsonl').read_bytes()).hexdigest(),
    'results_sha256': hashlib.sha256((D / 'results.jsonl').read_bytes()).hexdigest(),
    'end_load': Path('/proc/loadavg').read_text().strip(),
}, indent=2))
