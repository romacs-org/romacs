#!/usr/bin/env python3
"""G1 whole-compositor captures, fixed spec-derived body/band regions, independent OCR."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import live_oracle as live

BG = (48, 48, 48)  # g1.el's VM2 defface mode-line, never observer-derived
ERROR = '(setq mode-line-format \'("" "a" (:eval (error "boom")) "b"))'


def band_spec(font, metrics, text):
    spec = live.scene(font, metrics, after=False)
    spec['name'] = 'g1-band-' + text
    spec['frame'].update(height=metrics['ascent'] + metrics['descent'], bg='#303030')
    spec['expect']['rows'] = [{'text': text}]
    return spec


def crop(args, capture, target, height, y):
    # Fixed geometry declared before launch, not inferred from image/observer.
    subprocess.run([args.magick, str(capture), '-crop', f'400x{height}+0+{y}',
                    '+repage', 'PNG24:' + str(target)], check=True)


def check_band(args, capture, target, text, metrics):
    h = metrics['ascent'] + metrics['descent']
    cropped = target / (capture.stem + '-band.png')
    crop(args, capture, cropped, h, 300-h)
    spec = target / (capture.stem + '-band-spec.json')
    live.save_json(spec, band_spec(args.font, metrics, text))
    live.judge(args.oracle, spec, cropped, target / (capture.stem + '-band-check.json'), reject=False)
    pixels = live.rgb(capture, args.magick)
    rows = []
    for y in range(300-h, 300):
        count = sum(tuple(pixels[(y*400+x)*3:(y*400+x)*3+3]) == BG for x in range(400))
        rows.append(count / 400)
    assert min(rows) >= .90, ('band background coverage', rows)
    # Band paint must not escape its rectangle; ignore grayscale body ink.
    assert not any(tuple(pixels[i:i+3]) == BG for i in range(0, (300-h)*400*3, 3)), 'band bleeds into body'
    return {'background_row_fractions': rows}


def check_body(args, capture, target, metrics):
    h = metrics['ascent'] + metrics['descent']
    body_height = ((300-h)//h)*h
    cropped = target / 'body.png'
    crop(args, capture, cropped, body_height, 0)
    spec = live.scene(args.font, metrics, after=False)
    spec['frame']['height'] = body_height
    path = target / 'body-spec.json'
    live.save_json(path, spec)
    live.judge(args.oracle, path, cropped, target / 'body-check.json', reject=False)
    pixels = live.rgb(capture, args.magick)
    cw = metrics['cell_width']
    red = lambda r,g,b: r > 120 and g < 60 and b < 60 and abs(g-b) < 25
    hello = greek = ink = 0
    for y in range(h):
        for x in range(11*cw):
            r,g,b = pixels[(y*400+x)*3:(y*400+x)*3+3]
            if 6*cw <= x < 11*cw:
                hello += red(r,g,b)
            elif x < 5*cw:
                greek += red(r,g,b)
                ink += r > 120 and g > 120 and b > 120
    assert hello >= 30 and greek == 0 and ink > 0, ('spec-derived face pixels',hello,greek,ink)
    return {'hello_red': hello, 'greek_red': greek, 'greek_default_ink': ink}


def check_observer(records, presentation, body, band, vscroll):
    matches = [r for r in records if r.get('diagnostic') == 'submitted-scene' and r.get('presentation') == presentation]
    assert len(matches) == 1, 'one observer sequence per submitted frame'
    record = matches[0]
    glyphs = record['glyphs']
    assert ''.join(g['char'] for g in glyphs if g['role']=='Text') == body, 'observer body runs'
    assert ''.join(g['char'] for g in glyphs if g['role']=='ModeLine') == band, 'observer band runs'
    assert record['vscroll_px'] == vscroll, 'snapshot vscroll'
    for g in glyphs:
        if g['role'] == 'Text':
            assert g['foreground'] == ('#ff0000' if g['char'] in 'Hello' else '#ffffff'), 'observer declared colour'
        else:
            assert g['background'] == '#303030', 'observer band colour'


def records(path):
    result=[]
    for line in path.read_text().splitlines():
        try: result.append(json.loads(line))
        except ValueError: pass
    return result


def run(args, name, flags, metrics):
    out = args.output / name
    out.mkdir()
    with tempfile.TemporaryDirectory(prefix='g1-',dir=args.runtime_root) as temp:
        runtime=Path(temp); runtime.chmod(0o700)
        config=runtime/'config'; config.mkdir(); (config/'rc.xml').write_text(live.RC_XML)
        home=runtime/'home'; home.mkdir()
        env={'PATH':os.environ.get('PATH','/usr/bin:/bin'),'HOME':str(home),'LANG':'C.UTF-8',
             'XDG_RUNTIME_DIR':str(runtime),'XDG_CONFIG_HOME':str(home/'config'),
             'XDG_CACHE_HOME':str(home/'cache'),'XDG_DATA_HOME':str(home/'data'),
             'XDG_STATE_HOME':str(home/'state'),'TMPDIR':str(runtime),
             'WLR_BACKENDS':'headless','WLR_HEADLESS_OUTPUTS':'1','WLR_RENDERER':'pixman',
             'WLR_LIBINPUT_NO_DEVICES':'1','LIBSEAT_BACKEND':'noop'}
        proc=comp=None
        with (out/'compositor.log').open('w') as clog,(out/'binary.stderr').open('w') as blog:
            try:
                comp=subprocess.Popen([args.labwc,'-C',str(config)],env=env,stdout=clog,stderr=clog)
                deadline=time.monotonic()+args.timeout
                while not (sockets:=sorted(p for p in runtime.glob('wayland-*') if p.is_socket())):
                    assert comp.poll() is None and time.monotonic()<deadline,'private compositor startup'
                    time.sleep(.05)
                env['WAYLAND_DISPLAY']=sockets[0].name
                subprocess.run([args.wlr_randr,'--output','HEADLESS-1','--custom-mode','400x300'],env=env,check=True)
                command=[args.binary,'--wayland',str(sockets[0]),'--g1',*flags]
                live.save_json(out/'launch.json',{'command':command})
                proc=subprocess.Popen(command,env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=blog)
                reader=live.ReplyReader(proc,args.timeout)
                ready=reader.read(); assert ready.get('ready') is True,ready
                live.save_json(out/'ready.json',ready)
                first=out/'first.png'; live.capture(first,env,args,proc)
                body=check_body(args,first,out,metrics)
                band=check_band(args,first,out,' g1 v=0',metrics)
                for request,label,text in [({'apply_journal':True},'next',' g1 v=7'),
                                           ({'source':ERROR},'error','ab')]:
                    assert proc.stdin is not None
                    proc.stdin.write((json.dumps(request)+'\n').encode()); proc.stdin.flush()
                    reply=reader.read(); assert 'error' not in reply and 'result' in reply,reply
                    live.save_json(out/(label+'-reply.json'),reply)
                    capture=out/(label+'.png');live.capture(capture,env,args,proc)
                    check_band(args,capture,out,text,metrics)
                assert proc.poll() is None,'error killed process'
                log=records(out/'binary.stderr')
                check_observer(log,1,'Ωμέγα Hello',' g1 v=0',0)
                check_observer(log,2,'Ωμέγα Hello',' g1 v=7',7)
                check_observer(log,3,'Ωμέγα Hello','ab',7)
                faults=[r['message'] for r in log if r.get('diagnostic')=='mode-line']
                assert sum(':eval' in s and 'set-window-vscroll' in s for s in faults)==1,faults
                assert sum('boom' in s for s in faults)==1,faults
                paints=[r for r in log if r.get('diagnostic')=='paint']
                assert len(paints)==3 and all(r['unresolved_face_text_total']==0 for r in paints),paints
                return {'pass':True,'variant':name,'body_pixels':body,**band}
            finally:
                live.stop(proc);live.stop(comp)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',required=True);parser.add_argument('--oracle',required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--font',type=Path,default=Path('/usr/share/fonts/TTF/LiberationMono-Regular.ttf'))
    parser.add_argument('--labwc',default=live.LABWC);parser.add_argument('--wlr-randr',default=live.RANDR)
    parser.add_argument('--grim',default='grim');parser.add_argument('--magick',default='magick')
    parser.add_argument('--runtime-root',default=tempfile.gettempdir())
    parser.add_argument('--timeout',type=float,default=45);parser.add_argument('--settle',type=float,default=.2)
    args=parser.parse_args();args.output.mkdir()
    if not args.font.exists():
        # Same deterministic font discovery as the existing G1a harness.
        args.font=live.resolve_font()
    try:
        metrics=live.font_metrics(args.font,args.magick)
        results=[run(args,name,flags,metrics) for name,flags in
                 [('normal',[]),('low-gc',['--gc-floor','1']),
                  ('forced-gc',['--gc-floor','1','--gc-between-snapshot-and-paint'])]]
        for variant in ('low-gc','forced-gc'):
            for phase in ('first','next','error'):
                assert live.rgb(args.output/'normal'/(phase+'.png'),args.magick) == live.rgb(args.output/variant/(phase+'.png'),args.magick), ('GC changed compositor pixels',variant,phase)
        result={'pass':True,'runs':results,'metrics':metrics,
                'observer_scope':'Exact submitted scene; pixel oracle independent of observer.'}
    except Exception as exc:
        result={'pass':False,'error':str(exc)}
    live.save_json(args.output/'summary.json',result);print(json.dumps(result))
    return 0 if result['pass'] else 1

if __name__=='__main__':raise SystemExit(main())
