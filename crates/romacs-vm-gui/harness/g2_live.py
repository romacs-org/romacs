#!/usr/bin/env python3
"""Native virtual-keyboard -> command loop -> compositor pixels, versus GNU."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import live_oracle as live

ROOT = Path(__file__).resolve().parents[2]
specmod = importlib.util.spec_from_file_location('capture_scene', ROOT/'romacs-gui-oracle/harness/capture.py')
capture_scene = importlib.util.module_from_spec(specmod)
specmod.loader.exec_module(capture_scene)
PHASES = [('typed', ['aΔb'], 'aΔb'), ('delete', ['-k','BackSpace'], 'aΔ'),
          ('unbound', ['-k','F12'], 'aΔ'),
          ('ctrl-space', ['-M','ctrl','-k','space','-m','ctrl'], 'aΔ'),
          ('ctrl-home', ['\x01'], 'aΔ'),
          ('ctrl-forward', ['\x06'], 'aΔ'),
          ('ctrl-back', ['\x02'], 'aΔ'),
          ('ctrl-delete', ['\x04'], 'Δ'),
          ('restore-a', ['a'], 'aΔ'),
          ('ctrl-end', ['\x05'], 'aΔ'),
          ('signal', ['-M','ctrl','\x06','-m','ctrl'], 'aΔ'),
          ('recovery', ['x'], 'aΔx')]

def request(proc, reader, source):
    proc.stdin.write((json.dumps({'source':source})+'\n').encode());proc.stdin.flush()
    events=[]
    while True:
        record=reader.read()
        if 'result' in record or 'error' in record:
            assert 'error' not in record, record
            return events
        events.append(record)

def scene(args, metrics, text):
    s=live.scene(args.font, metrics, after=False)
    s['name']='g2-native'; s['frame']['height']=metrics['ascent']+metrics['descent']
    s['buffer']=[{'text':text}]; s['expect']['rows']=[{'text':text}]
    s['expect']['cursor']=None
    return s

def run(args, variant, flags, metrics, gnu=False):
    out=args.output/variant;out.mkdir()
    with tempfile.TemporaryDirectory(prefix='g2-',dir=args.runtime_root) as temp:
        runtime=Path(temp);runtime.chmod(0o700)
        config=runtime/'config';config.mkdir();(config/'rc.xml').write_text(live.RC_XML)
        home=runtime/'home';home.mkdir()
        env={'PATH':os.environ.get('PATH','/usr/bin:/bin'),'HOME':str(home),'LANG':'C.UTF-8',
             'XDG_RUNTIME_DIR':str(runtime),'XDG_CONFIG_HOME':str(home/'config'),
             'XDG_CACHE_HOME':str(home/'cache'),'XDG_DATA_HOME':str(home/'data'),
             'XDG_STATE_HOME':str(home/'state'),'TMPDIR':str(runtime),'WLR_BACKENDS':'headless',
             'WLR_HEADLESS_OUTPUTS':'1','WLR_RENDERER':'pixman','WLR_LIBINPUT_NO_DEVICES':'1',
             'LIBSEAT_BACKEND':'noop'}
        comp=keyboard=proc=None
        with (out/'compositor.log').open('w') as clog,(out/'binary.stderr').open('w') as blog:
            try:
                comp=subprocess.Popen([live.LABWC,'-C',str(config)],env=env,stdout=clog,stderr=clog)
                capture_scene.wait_for(lambda:any(p.is_socket() for p in runtime.glob('wayland-*')),args.timeout,'compositor',comp)
                socket=next(p for p in runtime.glob('wayland-*') if p.is_socket());env['WAYLAND_DISPLAY']=socket.name
                subprocess.run([live.RANDR,'--output','HEADLESS-1','--custom-mode','400x300'],env=env,check=True)
                keyboard=subprocess.Popen(['/usr/bin/wtype','-s','120000','-k','Shift_L'],env=env,stdout=blog,stderr=blog)
                time.sleep(.3)
                if gnu:
                    ready=runtime/'ready';source=runtime/'scene.el'
                    s=scene(args,metrics,'');s['frame']['height']=300
                    source.write_text(capture_scene.scene_el(s,ready) + '\n(setq cursor-type nil) (global-set-key (kbd "C-SPC") (quote beginning-of-line))\n')
                    cmd=['/usr/bin/emacs','-Q','--load',str(source)]
                    proc=subprocess.Popen(cmd,env=env,stdout=blog,stderr=blog)
                    capture_scene.wait_for(ready.exists,args.timeout,'GNU scene',proc)
                else:
                    cmd=[args.binary,'--wayland',str(socket),*flags]
                    proc=subprocess.Popen(cmd,env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=blog)
                    reader=live.ReplyReader(proc,args.timeout)
                    assert reader.read().get('ready') is True
                    request(proc,reader,'(progn (erase-buffer) (setq mode-line-format nil) (global-set-key (kbd "C-SPC") (quote beginning-of-line)))')
                live.save_json(out/'launch.json',{'command':cmd})
                outcomes=[]
                for label,keys,text in PHASES:
                    subprocess.run(['/usr/bin/wtype','-d','150',*keys],env=env,check=True,timeout=15)
                    time.sleep(.4)
                    events=[] if gnu else request(proc,reader,'nil')
                    outcomes.append({'phase':label,'events':events})
                    if not gnu and label.startswith('ctrl-'):
                        wanted={'ctrl-space':'beginning-of-line','ctrl-home':'beginning-of-line','ctrl-forward':'forward-char','ctrl-back':'backward-char','ctrl-delete':'delete-char','ctrl-end':'end-of-line'}[label]
                        assert len(events)==1 and events[0].get('native_command')==wanted, ('real renderer control event',label,events)
                    png=out/(label+'.png');live.capture(png,env,args,proc)
                    body=out/(label+'-body.png');h=metrics['ascent']+metrics['descent']
                    subprocess.run([args.magick,str(png),'-crop',f'400x{h}+0+0','+repage','PNG24:'+str(body)],check=True)
                    sp=out/(label+'-spec.json');live.save_json(sp,scene(args,metrics,text))
                    result=subprocess.run([args.oracle,'check',str(sp),str(body)],capture_output=True,text=True)
                    live.save_json(out/(label+'-check.json'),{'exit':result.returncode,'stdout':result.stdout,'stderr':result.stderr})
                    assert result.returncode==0, ('native key pixels',variant,label,result.stdout,result.stderr)
                    read=subprocess.run([args.oracle,'read',str(sp),str(body)],capture_output=True,text=True,check=True)
                    (out/(label+'-read.json')).write_text(read.stdout)
                live.save_json(out/'commands.json',outcomes)
                if not gnu:
                    byphase={p['phase']:p['events'] for p in outcomes}
                    unbound=byphase['unbound'];assert len(unbound)==1,unbound
                    assert unbound[0].get('bell') is True and unbound[0].get('message') and unbound[0].get('signal') is None,unbound
                    signal=byphase['signal'];assert len(signal)==1 and (signal[0].get('signal') or {}).get('condition')=='end-of-buffer',signal
                    assert byphase['recovery'][-1]['signal'] is None,byphase
                    assert proc.poll() is None,'signalling command killed loop'
                    all_events=[e for p in outcomes for e in p['events']]
                    assert len(all_events)==len(PHASES)+2,all_events
                    presentations=[e['presentation'] for e in all_events]
                    assert all(b==a+1 for a,b in zip(presentations,presentations[1:])),presentations
                return {'variant':variant,'pass':True,'pixel_phases':len(PHASES),'native_commands':None if gnu else len(all_events)}
            finally:
                live.stop(proc);live.stop(keyboard);live.stop(comp)

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',required=True);p.add_argument('--oracle',required=True);p.add_argument('--output',type=Path,required=True)
    p.add_argument('--font',type=Path,default=live.resolve_font())
    p.add_argument('--runtime-root',default=tempfile.gettempdir());p.add_argument('--timeout',type=float,default=30)
    p.add_argument('--settle',type=float,default=.2);p.add_argument('--grim',default='grim');p.add_argument('--magick',default='magick')
    p.add_argument('--red',action='store_true',help='only initial normal variant for pre-implementation evidence')
    args=p.parse_args();args.output.mkdir(parents=True)
    metrics=live.font_metrics(args.font,args.magick)
    results=[run(args,'normal',[],metrics)]
    if not args.red:
        results += [run(args,'low-gc',['--gc-floor','1'],metrics),run(args,'forced-gc',['--gc-floor','1','--gc-between-snapshot-and-paint'],metrics),run(args,'gnu',[],metrics,gnu=True)]
        for label,_,_ in PHASES:
            spec=args.output/'normal'/(label+'-spec.json')
            pair=subprocess.run([args.oracle,'pair',str(spec),str(args.output/'gnu'/(label+'-body.png')),str(args.output/'normal'/(label+'-body.png'))],capture_output=True,text=True)
            live.save_json(args.output/(label+'-pair.json'),{'exit':pair.returncode,'stdout':pair.stdout,'stderr':pair.stderr})
            assert pair.returncode==0,('GNU pixel pair',label,pair.stdout,pair.stderr)
    result={'pass':True,'runs':results,'repaint':'one full-frame 400x300 submission per native command (including unbound and signal); not a timing benchmark'}
    live.save_json(args.output/'summary.json',result);print(json.dumps(result))
if __name__=='__main__':main()
