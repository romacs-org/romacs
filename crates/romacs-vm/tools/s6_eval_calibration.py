#!/usr/bin/env python3
"""Guarded S6 calibration. VM2-warm retains source IR; GNU/SBCL eval read and
interpret/compile anew. This agent-only warm-state asymmetry is intentional.
Cold VM2 clears its source lookup before every request (same persistent VM).
Prepared handles bypass the source frontend after registration. Startup and
registration costs remain in raw counts, but amortize out of adjacent slopes.
"""
import argparse,json,statistics,subprocess,hashlib
from pathlib import Path
from bench import guarded,load,COUNTER,SOURCE
import sbcl_eval_calibration as old

def main():
    p=argparse.ArgumentParser();p.add_argument('--probe',required=True);p.add_argument('--output',required=True);args=p.parse_args()
    cases={name:body.replace('N','20') for name,_,body in load(SOURCE,'s6_cases').CASES if name!='buf-insert'}
    cases['tiny-add']='(+ 1 2)';old.CL['tiny-add']='(+ 1 2)'
    attrs=load(COUNTER,'s6_counter').attributes
    report={'probe_sha256':hashlib.sha256(Path(args.probe).read_bytes()).hexdigest(),'scope':__doc__,'load_before':Path('/proc/loadavg').read_text(),'resolution':36,'floor':72,'reps':3,'samples':[],'cases':[]}
    def save():Path(args.output).write_text(json.dumps(report,indent=2)+'\n')
    try:
        for name,source in cases.items():
            expected=int(subprocess.check_output(['/usr/bin/emacs','-Q','--batch','--eval',f'(progn (setq lexical-binding t) (princ {source}))'],text=True))
            row={'name':name,'source':source,'expected':expected,'medians':{},'slopes':{}}
            for engine in ['vm2-cold','vm2-warm','vm2-handle','gnu-eval','sbcl-compile','sbcl-interpret','gnu-compiled','sbcl-compiled']:
                medians={}
                for count in [1,256,512,1024]:
                    counts=[]
                    for rep in range(3):
                        if engine.startswith('vm2-'):
                            mode={'vm2-cold':'cold','vm2-warm':'eval','vm2-handle':'handle'}[engine]
                            command=[args.probe,mode,str(count),str(expected),source]
                        else:command=old.command(engine,old.CL[name] if engine.startswith('sbcl') else source,expected,count,args.probe)
                        sample=guarded(command,b'',attrs)
                        assert sample['exit']==0 and sample['stdout'].strip()==str(expected),(name,engine,sample)
                        assert sample['count']>0 and sample['enabled']>0 and sample['enabled']==sample['running']
                        sample.update(name=name,engine=engine,requests=count,rep=rep);report['samples'].append(sample);counts.append(sample['count']);save()
                    medians[str(count)]=statistics.median(counts)
                slopes=[]
                for lo,hi in [(256,512),(512,1024)]:
                    delta=medians[str(hi)]-medians[str(lo)];assert delta>72,(name,engine,delta);slopes.append(delta/(hi-lo))
                row['medians'][engine]=medians;row['slopes'][engine]=slopes
            report['cases'].append(row);save();print(name,json.dumps(row['slopes']),flush=True)
        assert len(report['samples'])==576 and len(report['cases'])==6
        report['complete']=True
    finally:report['load_after']=Path('/proc/loadavg').read_text();save()
if __name__=='__main__':main()
