#!/usr/bin/env python3
"""G3a: real split producer and native input, independently read per-window pixels."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import g2_live as g2
import live_oracle as live

VM_SETUP = '''(progn
 (setq mode-line-format nil a (selected-window))
 (set-buffer (get-buffer-create "g3-top")) (erase-buffer)
 (set-window-buffer a (current-buffer))
 (setq lower (/ (window-pixel-height a) 2))
 (set-window-new-pixel a (- (window-pixel-height a) lower))
 (setq b (split-window-internal a lower nil 0.5))
 (set-buffer (get-buffer-create "g3-bottom")) (erase-buffer)
 (insert "bottom Ω") (set-window-buffer b (current-buffer))
 (select-window a))'''

def run(args, variant, flags, metrics, gnu=False):
    out = args.output / variant
    out.mkdir()
    line = metrics['ascent'] + metrics['descent']
    top_height = 300 - line - (300 - line) // 2
    with tempfile.TemporaryDirectory(prefix='g3-', dir=args.runtime_root) as temp:
        runtime = Path(temp); runtime.chmod(0o700)
        config = runtime/'config'; config.mkdir(); (config/'rc.xml').write_text(live.RC_XML)
        home = runtime/'home'; home.mkdir()
        env = {'PATH': os.environ.get('PATH','/usr/bin:/bin'), 'HOME':str(home), 'LANG':'C.UTF-8',
               'XDG_RUNTIME_DIR':str(runtime), 'XDG_CONFIG_HOME':str(home/'config'),
               'XDG_CACHE_HOME':str(home/'cache'), 'XDG_DATA_HOME':str(home/'data'),
               'XDG_STATE_HOME':str(home/'state'), 'TMPDIR':str(runtime), 'WLR_BACKENDS':'headless',
               'WLR_HEADLESS_OUTPUTS':'1', 'WLR_RENDERER':'pixman', 'WLR_LIBINPUT_NO_DEVICES':'1',
               'LIBSEAT_BACKEND':'noop'}
        comp = keyboard = proc = None
        with (out/'compositor.log').open('w') as clog, (out/'binary.stderr').open('w') as blog:
            try:
                comp = subprocess.Popen([live.LABWC,'-C',str(config)], env=env, stdout=clog, stderr=clog)
                g2.capture_scene.wait_for(lambda:any(p.is_socket() for p in runtime.glob('wayland-*')),args.timeout,'compositor',comp)
                socket = next(p for p in runtime.glob('wayland-*') if p.is_socket()); env['WAYLAND_DISPLAY']=socket.name
                subprocess.run([live.RANDR,'--output','HEADLESS-1','--custom-mode','400x300'],env=env,check=True)
                keyboard = subprocess.Popen(['/usr/bin/wtype','-s','120000','-k','Shift_L'],env=env,stdout=blog,stderr=blog)
                time.sleep(.3)
                if gnu:
                    ready=runtime/'ready'; source=runtime/'scene.el'
                    scene=g2.scene(args,metrics,''); scene['frame']['height']=300
                    # Drive GNU through the SAME prepared-size transaction the VM2
                    # scene uses (VM_SETUP above), not high-level `split-window`.
                    # High-level split applies a Lisp rounding policy before the C
                    # primitive ever sees the request: window.el:2735-2741 rounds a
                    # pixelwise size to a whole character row whenever ROUND-MAYBE is
                    # t and `window-resize-pixelwise' is nil, and window.el:5758-5760
                    # passes ROUND-MAYBE t. At this frame's 23px line height a
                    # requested 139px upper window became 138px, so the two editors
                    # were being asked to perform DIFFERENT operations and the oracle
                    # was measuring GNU's rounding policy rather than either
                    # implementation's geometry. `split-window-internal' is the C
                    # primitive (src/window.c:5530-5534) and requires exact prepared
                    # sizes, so both sides now compute lower/upper identically.
                    extra='''
(setq cursor-type nil mode-line-format nil)
(let* ((a (selected-window))
       (lower (/ (window-pixel-height a) 2)))
  (set-window-new-pixel a (- (window-pixel-height a) lower))
  (let ((b (split-window-internal a lower nil 0.5)))
    (set-window-buffer b (get-buffer-create "g3-bottom"))
    (with-current-buffer "g3-bottom" (erase-buffer) (insert "bottom Ω"))
    (select-window a)))
(redisplay t)
'''
                    base=g2.capture_scene.scene_el(scene,ready)
                    # Wait for the post-split ready marker, not initial unsplit redisplay.
                    base=base.replace(str(ready),str(runtime/'initial-ready'))
                    extra += f'(with-temp-file "{(out / "geometry.el").resolve()}" (prin1 (list (frame-pixel-width) (frame-pixel-height) (window-pixel-height) (window-pixel-edges (selected-window)) (window-pixel-edges (next-window)) (window-pixel-edges (minibuffer-window)) (frame-char-height)) (current-buffer)))\n'
                    # Let the inherited harness's 1.5s configure/redisplay barrier
                    # complete before calculating a pixel split; pre-map geometry
                    # is not the geometry of the maximized compositor surface.
                    source.write_text(base+'\n(run-with-timer 2.0 nil (lambda ()\n'+extra+f'(with-temp-file "{ready}" (insert "ready"))))\n')
                    cmd=['/usr/bin/emacs-32.0.50','-Q','--load',str(source)]
                    proc=subprocess.Popen(cmd,env=env,stdout=blog,stderr=blog)
                    g2.capture_scene.wait_for(ready.exists,args.timeout,'GNU scene',proc)
                else:
                    cmd=[args.binary,'--wayland',str(socket),*flags]
                    proc=subprocess.Popen(cmd,env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=blog)
                    reader=live.ReplyReader(proc,args.timeout)
                    assert reader.read().get('ready') is True
                    g2.request(proc,reader,VM_SETUP)
                live.save_json(out/'launch.json',{'command':cmd,'source':None if gnu else VM_SETUP})
                subprocess.run(['/usr/bin/wtype','-d','150','TOP Δ'],env=env,check=True,timeout=15)
                time.sleep(.4)
                events=[] if gnu else g2.request(proc,reader,'nil')
                live.save_json(out/'commands.json',events)
                png=out/'split.png';live.capture(png,env,args,proc)
                for label,y,text in [('top',0,'TOP Δ'),('bottom',top_height,'bottom Ω')]:
                    body=out/(label+'-body.png')
                    subprocess.run([args.magick,str(png),'-crop',f'400x{line}+0+{y}','+repage','PNG24:'+str(body)],check=True)
                    spec=g2.scene(args,metrics,text); spec['name']='g3-'+label
                    sp=out/(label+'-spec.json');live.save_json(sp,spec)
                    result=subprocess.run([args.oracle,'check',str(sp),str(body)],capture_output=True,text=True)
                    live.save_json(out/(label+'-check.json'),{'exit':result.returncode,'stdout':result.stdout,'stderr':result.stderr})
                    assert result.returncode==0,('window pixels',variant,label,result.stdout,result.stderr)
                    read=subprocess.run([args.oracle,'read',str(sp),str(body)],capture_output=True,text=True,check=True)
                    (out/(label+'-read.json')).write_text(read.stdout)
                return {'variant':variant,'pass':True,'regions':{'top':[0,0,400,line],'bottom':[0,top_height,400,line]},'text':['TOP Δ','bottom Ω']}
            finally:
                live.stop(proc); live.stop(keyboard); live.stop(comp)

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',required=True);p.add_argument('--oracle',required=True)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--font',type=Path,default=live.resolve_font())
    p.add_argument('--runtime-root',default=tempfile.gettempdir())
    p.add_argument('--timeout',type=float,default=30);p.add_argument('--settle',type=float,default=.2)
    p.add_argument('--grim',default='grim');p.add_argument('--magick',default='magick');p.add_argument('--red',action='store_true')
    args=p.parse_args();args.output.mkdir(parents=True)
    metrics=live.font_metrics(args.font,args.magick)
    results=[run(args,'normal',[],metrics)]
    if not args.red:
        results += [run(args,'low-gc',['--gc-floor','1'],metrics),run(args,'forced-gc',['--gc-floor','1','--gc-between-snapshot-and-paint'],metrics),run(args,'gnu',[],metrics,True)]
        for label in ['top','bottom']:
            pair=subprocess.run([args.oracle,'pair',str(args.output/'normal'/(label+'-spec.json')),str(args.output/'gnu'/(label+'-body.png')),str(args.output/'normal'/(label+'-body.png'))],capture_output=True,text=True)
            live.save_json(args.output/(label+'-pair.json'),{'exit':pair.returncode,'stdout':pair.stdout,'stderr':pair.stderr})
            assert pair.returncode==0,('GNU pixel pair',label,pair.stdout,pair.stderr)
    live.save_json(args.output/'summary.json',{'pass':True,'runs':results})
    print(json.dumps({'pass':True,'runs':results}))
if __name__=='__main__':main()
