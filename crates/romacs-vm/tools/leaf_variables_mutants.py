#!/usr/bin/env python3
import json,subprocess
from pathlib import Path
from leaf_sequences_run import LOGS
root=Path(__file__).resolve().parents[3]
runner=root/'crates/romacs-vm/tools/leaf_sequences_run.py'
mutants=[('non-special',root/'crates/romacs-vm/src/prims/leaf_variables.rs','cx.make_variable_non_special(symbol)?;','let _ = symbol;', 'leaf_non_special','LEAF_NON_SPECIAL_CLEAR'),('debug-default',root/'crates/romacs-vm/src/forwarded.rs','name: "debug-on-error", gnu: "eval.c:4485", kind: Kind::Lisp(Init::Nil)','name: "debug-on-error", gnu: "eval.c:4485", kind: Kind::Lisp(Init::True)','leaf_debug','LEAF_DEBUG_DEFAULT')]
rows=[]
for name,path,old,new,test,assertion in mutants:
    original=path.read_text(); assert original.count(old)==1
    try:
        path.write_text(original.replace(old,new))
        r=subprocess.run(['python3',str(runner),'vars-mutant-'+name,'cargo','test','-p','romacs-vm','--test','leaf_variables',test,'--','--nocapture'],cwd=root)
        log=(LOGS / f'vars-mutant-{name}.log').read_text()
        rows.append(dict(mutant=name,exit=r.returncode,assertion=assertion,killed=r.returncode!=0 and 'failed: '+assertion in log))
    finally: path.write_text(original)
(LOGS / 'vars-mutants.json').write_text(json.dumps(rows,indent=2)+'\n')
assert all(r['killed'] for r in rows),rows
