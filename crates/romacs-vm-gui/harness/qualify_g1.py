#!/usr/bin/env python3
"""Qualify G1 assertions with preserved real captures and controlled mutants."""
import argparse
import copy
from pathlib import Path
import types
import subprocess
import g1_live as g
import live_oracle as live


def rejected(label, fn):
    try:
        fn()
    except AssertionError as error:
        return {'name': label, 'rejected': True, 'assertion': str(error)}
    except RuntimeError as error:
        # Oracle execution/setup errors (exit 2) are never behavioral red evidence.
        assert 'oracle exit 1, expected 0' in str(error), error
        return {'name': label, 'rejected': True, 'assertion': str(error)}
    raise AssertionError(label + ': mutant survived')


def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--live',type=Path,required=True)
    ap.add_argument('--baseline',type=Path,required=True)
    ap.add_argument('--output',type=Path,required=True)
    ap.add_argument('--oracle',required=True)
    args=ap.parse_args();args.output.mkdir()
    font=Path(subprocess.check_output(['fc-match','-f','%{file}','Liberation Mono']).decode())
    config=types.SimpleNamespace(font=font,oracle=args.oracle,magick='magick')
    metrics=live.font_metrics(font,'magick');h=metrics['ascent']+metrics['descent']
    evidence=[]
    for name,text in [('baseline-no-band',' g1 v=0'),('baseline-no-ab','ab')]:
        out=args.output/name;out.mkdir()
        evidence.append(rejected(name,lambda:g.check_band(config,args.baseline,out,text,metrics)))
    evidence.append(rejected('baseline-no-observer',lambda:g.check_observer([],1,'Ωμέγα Hello',' g1 v=0',0)))
    first=args.live/'first.png'
    original=live.rgb(first,'magick')
    for name in ['background','body-bleed','missing-b']:
        out=args.output/name;out.mkdir()
        pixels=bytearray(live.rgb(args.live/'error.png','magick') if name=='missing-b' else original)
        if name=='background':
            for i in range((300-h)*400*3,len(pixels),3):
                if tuple(pixels[i:i+3])==g.BG:pixels[i:i+3]=bytes((80,80,80))
        elif name=='body-bleed':
            i=((300-h-1)*400+200)*3;pixels[i:i+3]=bytes(g.BG)
        else:
            for y in range(300-h,300):
                for x in range(metrics['cell_width'],2*metrics['cell_width']):
                    i=(y*400+x)*3;pixels[i:i+3]=bytes(g.BG)
        path=out/'mutant.png';live.write_rgb(path,pixels,'magick')
        text='ab' if name=='missing-b' else ' g1 v=0'
        evidence.append(rejected(name,lambda:g.check_band(config,path,out,text,metrics)))
    records=g.records(args.live/'binary.stderr')
    for name in ['observer-colour','observer-band','snapshot-vscroll']:
        mutant=copy.deepcopy(records)
        presentation=2 if name=='snapshot-vscroll' else 1
        record=next(r for r in mutant if r.get('diagnostic')=='submitted-scene' and r['presentation']==presentation)
        if name=='observer-colour':
            next(x for x in record['glyphs'] if x['char']=='H')['foreground']='#00ff00'
        elif name=='observer-band':
            record['glyphs']=[x for x in record['glyphs'] if x['role']!='ModeLine']
        else:record['vscroll_px']=0
        band=' g1 v=7' if presentation==2 else ' g1 v=0'
        evidence.append(rejected(name,lambda:g.check_observer(mutant,presentation,'Ωμέγα Hello',band,7 if presentation==2 else 0)))
    live.save_json(args.output/'qualification.json',{'pass':True,'controls':evidence})
    print({'pass':True,'rejected_controls':len(evidence)})

if __name__=='__main__':main()
