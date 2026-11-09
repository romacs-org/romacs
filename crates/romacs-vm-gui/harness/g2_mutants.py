#!/usr/bin/env python3
"""Assertion-kill native translation, dispatch and repaint mutants in an isolated tree."""
import json
import os
from pathlib import Path
import subprocess
import tarfile
import shutil
import tempfile

ROOT=Path(__file__).resolve().parents[3]

def main():
    tree=subprocess.check_output(['git','write-tree'],cwd=ROOT,text=True).strip()
    results=[]
    with tempfile.TemporaryDirectory(prefix='g2-mutants-',dir=tempfile.gettempdir()) as temp:
        clone=Path(temp)/'src';clone.mkdir()
        archive=subprocess.run(['git','archive',tree],cwd=ROOT,stdout=subprocess.PIPE,check=True).stdout
        import io
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:tar.extractall(clone,filter='data')
        env=dict(os.environ,CARGO_TARGET_DIR=str(ROOT/'target'))
        originals={p:(clone/p).read_text() for p in ['crates/romacs-vm-gui/src/native.rs','crates/romacs-vm-gui/src/main.rs']}
        for name,path,old,new,unit in [
            ('translation','crates/romacs-vm-gui/src/native.rs','    let (key, modifiers) = match event {','    return None;\n    let (key, modifiers) = match event {',True),
            ('drop-control-text','crates/romacs-vm-gui/src/native.rs','0..=0x7f | 0xa0..=0xff','0x20..=0x7e | 0xa0..=0xff',False),
            ('double-control','crates/romacs-vm-gui/src/native.rs','0..=31 | 127 => *c','0..=31 | 127 => *c | (1 << 26)',False),
            ('control-space','crates/romacs-vm-gui/src/native.rs','32 => 0','32 => 32 | (1 << 26)',False),
            ('dispatch','crates/romacs-vm-gui/src/main.rs','if let Some(event) = native::translate(event) {','if let Some(event) = { let _ = event; None::<romacs_vm::CommandEvent> } {',False),
            ('repaint','crates/romacs-vm-gui/src/main.rs','let next = vm.display_snapshot(400, 300)?;','let next = previous.clone();',False),
        ]:
            for p,s in originals.items():(clone/p).write_text(s)
            s=originals[path];assert old in s;(clone/path).write_text(s.replace(old,new,1))
            cmd=['cargo','test','-p','romacs-vm-gui','--lib','native::tests','--','--nocapture'] if unit else ['cargo','build','-p','romacs-vm-gui','--quiet']
            build=subprocess.run(cmd,cwd=clone,env=env,capture_output=True,text=True)
            if unit:
                output=build.stdout+build.stderr
                killed=build.returncode==101 and 'assertion `left == right` failed' in output
            else:
                assert build.returncode==0,build.stderr
                cmd=['python3',str(ROOT/'crates/romacs-vm-gui/harness/g2_live.py'),'--binary',str(ROOT/'target/debug/romacs-vm-gui'),'--oracle',str(ROOT/'target/debug/romacs-gui-oracle'),'--output',str(Path(temp)/('mutant-'+name)),'--red']
                run=subprocess.run(cmd,cwd=clone,env=env,capture_output=True,text=True)
                shutil.copytree(Path(temp)/("mutant-"+name), ROOT/"crates/romacs-vm-gui/evidence/g2a"/("mutant-"+name), dirs_exist_ok=True)
                output=run.stdout+run.stderr
                killed=run.returncode==1 and 'AssertionError:' in output
                # A stale native paint may be corrected by the later eval barrier;
                # the independent consecutive-presentation assertion catches that
                # extra corrective repaint rather than letting the barrier hide it.
            results.append({'name':name,'killed':killed,'command':cmd,'output':output})
            assert killed,results[-1]
    out=ROOT/'crates/romacs-vm-gui/evidence/g2a/gui-mutations.json'
    out.write_text(json.dumps({'tree':tree,'results':results},indent=2)+'\n')
    print(json.dumps({'killed':[r['name'] for r in results]}))
if __name__=='__main__':main()
