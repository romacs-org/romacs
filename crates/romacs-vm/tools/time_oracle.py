#!/usr/bin/env python3
"""Fresh-process GNU time observations; TZ/locale dependence is explicit."""
import os as _portable_os
from pathlib import Path as _PortablePath
import hashlib
import json
import os
from pathlib import Path
import subprocess
ROOT = Path(__file__).resolve().parents[1]
GNU = Path(_portable_os.environ.get('ROMACS_GNU_ORACLE', str(_PortablePath(_portable_os.environ.get('GNU_ORACLE_ROOT', _PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'gnu-emacs-pin')) / '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea/bin/emacs-32.0.50')))
rows = []
def add(p, key, form):
    rows.append(dict(primitive=p, id=p+'/'+key, form=form))
for fmt in ['%Y-%m-%d %H:%M:%S %Z %z', '%a %A %b %B %h %p %c %x %X %r', '%j %u %w %U %W %V %G %g', '%D %F %R %T %q %s', '%N %3N %6N %12N', '%-d %_d %03d %^B %#p %EY %Od', '%% %Q trailing%', 'hello λ\\0%Y']:
    add('format-time-string', str(len(rows)), '(format-time-string '+json.dumps(fmt)+' \'(0 0 123456 789012) t)')
for zone in ['t', '0', '3600', '-19800', "'(19800 \"IST\")", '"UTC0"', '"EST5"', '"America/New_York"', 'nil', "'wall"]:
    for timestamp in ['0', '1719792000', '-1']:
        add('format-time-string',zone+timestamp, f'(format-time-string "%F %T %Z %z %:z %::z %:::z" {timestamp} {zone})')
for p in ['format-time-string','current-time-string','current-time-zone','decode-time','float-time']:
    for bad in ["'bad", "'(0)", "'(0 . 0)", "'(0 0 bad)"]:
        args = '"%Y" '+bad+' t' if p == 'format-time-string' else bad+' t' if p != 'float-time' else bad
        add(p,'invalid-'+bad, '('+p+' '+args+')')
    add(p,'callable', f"(list (fboundp '{p}) (functionp #'{p}))")
    add(p,'arity-high',f"(apply #'{p} '(nil nil nil nil))")
for t in ['0','-1','1.25',"'(1 2 3 4)","'(-1 . 2)","'(0 0 bad)"]:
    for p in ['current-time-string','current-time-zone','decode-time','float-time']:
        add(p,t,f'({p} {t}'+(' t)' if p!='float-time' else ')'))
add('decode-time','precision',"(decode-time '(5 . 2) t t)")
add('decode-time','negative-precision',"(decode-time '(-1 . 3) t t)")
for p in ['current-time-string','current-time-zone','decode-time']:
    add(p,'named-zone',f'({p} 1719792000 "America/New_York")')
for p,args in [('format-time-string','"%Y %Z" 0 t'),('current-time-string','0 t'),('current-time-zone','0 t'),('decode-time','0 t'),('float-time','0')]:
    add(p,'funcall',f"(funcall #'{p} {args})")
    add(p,'apply',f"(apply #'{p} (list {args}))")
    add(p,'live-shape',{'format-time-string':'(stringp (format-time-string "%Y"))','current-time-string':'(stringp (current-time-string))','current-time-zone':'(= (length (current-time-zone)) 2)','decode-time':'(= (length (decode-time)) 9)','float-time':'(floatp (float-time))'}[p])
add('format-time-string','arity-low',"(apply #'format-time-string nil)")
add('format-time-string','wrong-format','(format-time-string 3 0 t)')
add('format-time-string','bad-time-before-format',"(format-time-string 3 'bad t)")
add('format-time-string','embedded-nul','(format-time-string (concat "%Y" (string 0) "%m") 0 t)')
for t in [-62167219200,-62135596800,951782400,1451606400,1483228800,1609459200,253402300800]:
    add('format-time-string','calendar-'+str(t),f'(format-time-string "%F %j %U %W %V %G %g" {t} t)')
    add('current-time-string','calendar-'+str(t),f'(current-time-string {t} t)')
for t in [1710053999,1710054000,1730613599,1730613600]:
    add('decode-time','dst-'+str(t),f'(decode-time {t} "America/New_York")')
for z in ["'bad", "'(0)", "'(0 bad)"]:
    add('format-time-string','zone-'+z,f'(format-time-string "%Z" 0 {z})')
for key, timestamp in [
    ('negative-picoseconds', "'(-2 65534 -3 -4)"),
    ('positive-halfway-even', "'(9007199254740993 . 9007199254740992)"),
    ('positive-halfway-odd', "'(9007199254740995 . 9007199254740992)"),
    ('negative-halfway-even', "'(-9007199254740993 . 9007199254740992)"),
    ('negative-halfway-odd', "'(-9007199254740995 . 9007199254740992)"),
    ('small-picoseconds', "'(0 0 0 1)"),
]:
    add('float-time', key, f'(float-time {timestamp})')
add('format-time-string', 'unicode-result-bytes', '(let ((s (format-time-string "λ%Y" 0 t))) (list (aref s 0) (aref s 1) (aref s 2) (aref s 3) (aref s 4) (aref s 5)))')
add('format-time-string', 'unicode-result-unibyte', '(multibyte-string-p (format-time-string "λ%Y" 0 t))')
for value in ['0.0', '-0.0', '1.0', '3.5', '0.000000000001', '-0.000000000001', '5e-324', '-5e-324']:
    add('decode-time', 'float-precision-'+value, f'(decode-time {value} t t)')
for timestamp in [0, -62198755200, -65322892800, 253402300800]:
    for fmt in ['%_5V|%+6Y', '%Y|%y|%C|%G|%g', '%_6Y|%06Y|%-Y|%+Y|%+6G|%_5g|%+4C']:
        add('format-time-string', 'year-flags-'+str(timestamp)+'-'+fmt, f'(format-time-string {json.dumps(fmt)} {timestamp} t)')
for value in ['-5e-324', '-1e-20', '5e-324', '1e-20']:
    add('format-time-string', 'tiny-float-'+value, f'(format-time-string "%F %T.%N" {value} t)')
    add('decode-time', 'tiny-float-'+value, f'(decode-time {value} t)')
    add('current-time-string', 'tiny-float-'+value, f'(current-time-string {value} t)')
for timestamp in [0, -62198755200, -65322892800, 253402300800]:
    for width in [1, 2, 3, 4, 6]:
        fmt = f'%+{width}y|%+{width}g'
        add('format-time-string', 'short-year-sign-'+str(timestamp)+'-'+str(width), f'(format-time-string {json.dumps(fmt)} {timestamp} t)')
for row in rows:
    row['source'] = "(prin1-to-string (condition-case e (list 'ok "+row['form']+") (error (list 'sig e))))"
    result = subprocess.run([str(GNU),'--batch','-Q','--eval','(princ '+row['source']+')'], env={**os.environ,'TZ':'UTC0','LC_ALL':'C'}, capture_output=True,text=True,check=True,timeout=15)
    row['expected'] = result.stdout
    assert not result.stderr, result.stderr
out = dict(pin='45b0f7699df36b3849edc3bdef37d4cb9e75b9ea', binary=str(GNU), sha256=hashlib.sha256(GNU.read_bytes()).hexdigest(), source_sha256=hashlib.sha256(Path(str(_PortablePath(_portable_os.environ.get('ROMACS_GNU_SOURCE', _PortablePath(_portable_os.environ.get('GNU_ORACLE_ROOT', _PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'gnu-emacs-pin')) / 'src')) / 'src/timefns.c')).read_bytes()).hexdigest(), environment={'TZ':'UTC0','LC_ALL':'C'}, regime='GNU -Q interpreted, fresh process per row', rows=rows)
path=ROOT/'evidence/time/oracle.json'
if path.exists():
    prior=json.loads(path.read_text())
    current={r['id']:r['expected'] for r in rows}
    assert all(current.get(r['id'])==r['expected'] for r in prior['rows']), 'Prior GNU observation changed or was removed'
out['environment_files']={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in [Path('/etc/localtime'),Path('/usr/share/zoneinfo/America/New_York')]}
out['generator_sha256']=hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps(out,ensure_ascii=True,indent=2)+'\n')
print(json.dumps({'path':str(path),'rows':len(rows),'primitives':len({r['primitive'] for r in rows})}))
