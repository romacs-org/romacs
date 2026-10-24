#!/usr/bin/env python3
"""Compile a disposable copy; shared source remains untouched.
Install the documented Leaf API, selector visibility and native variable seed;
exclude only the concurrently authored filesave module awaiting its own API.
"""
import os as _portable_os
from pathlib import Path as _PortablePath
import pathlib, shutil, subprocess, os, json, sys
MUTANT = '--mutant' in sys.argv
HERE=pathlib.Path(__file__).resolve().parent
ROOT=HERE.parents[3]
COPY=HERE/'verification-copy'
if COPY.exists(): shutil.rmtree(COPY)
COPY.mkdir()
for p in ROOT.iterdir():
    if p.name in ('.git','crates','target'): continue
    if p.name in ('Cargo.toml','Cargo.lock'): shutil.copy2(p,COPY/p.name)
    else: (COPY/p.name).symlink_to(p,target_is_directory=p.is_dir())
(COPY/'crates').mkdir()
for p in (ROOT/'crates').iterdir():
    if p.name != 'romacs-vm': (COPY/'crates'/p.name).symlink_to(p,target_is_directory=p.is_dir())
crate=COPY/'crates/romacs-vm'; crate.mkdir()
for name in ['Cargo.toml','build.rs']: shutil.copy2(ROOT/'crates/romacs-vm'/name,crate/name)
shutil.copytree(ROOT/'crates/romacs-vm/src',crate/'src')
# Exclude the concurrently authored sibling whose Leaf API is not integrated.
(crate/'src/prims/filesave.rs').unlink(missing_ok=True)
shutil.copy2(HERE/'leaf-api.rs',crate/'src/prim/leaf/filelock.rs')
p=crate/'src/prim.rs'; s=p.read_text(); s=s.replace('    mod fileio;', '    mod fileio;\n    mod filelock;'); p.write_text(s)
(crate/'tests').mkdir(); shutil.copy2(ROOT/'crates/romacs-vm/tests/filelock.rs',crate/'tests/filelock.rs')
p=crate/'src/prims/fileio.rs'; s=p.read_text(); s=s.replace('fn find_file_name_handler(cx:', 'pub(super) fn find_file_name_handler(cx:'); p.write_text(s)
p=crate/'src/forwarded.rs'; s=p.read_text(); s=s.replace('pub(crate) fn declare_fileio_specials(&mut self) {','pub(crate) fn declare_fileio_specials(&mut self) {\n        self.declare_special_raw("create-lockfiles", Value::TRUE);'); p.write_text(s)
if MUTANT:
    p=crate/'src/prims/filelock.rs'; s=p.read_text()
    old='Ok(Owner::Ours) => Ok(Step::Done(Value::TRUE))'
    assert s.count(old)==1
    p.write_text(s.replace(old, 'Ok(Owner::Ours) => Ok(Step::Done(Value::NIL))'))
env=os.environ.copy(); env['CARGO_TARGET_DIR']=str(_PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'lanes/save-filelock-check'); env['CARGO_BUILD_JOBS']='4'
r=subprocess.run(['cargo','test','-p','romacs-vm','--test','filelock','--','--nocapture'],cwd=COPY,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
label='mutant' if MUTANT else 'focused'
if (HERE/f'{label}.log').exists():
    index=1
    while (HERE/f'iteration-{index}.log').exists(): index+=1
    shutil.copy2(HERE/f'{label}.log',HERE/f'iteration-{index}.log')
(HERE/f'{label}.log').write_text(r.stdout)
(HERE/f'{label}-status.json').write_text(json.dumps({'status':r.returncode,'integration_shims':['fileio::find_file_name_handler pub(super)','create-lockfiles special seeded t','leaf-api.rs installed + registered'], 'excluded_sibling':'filesave.rs', 'mutant':MUTANT})+'\n')
print(r.stdout); print('status',r.returncode)
shutil.rmtree(COPY)
