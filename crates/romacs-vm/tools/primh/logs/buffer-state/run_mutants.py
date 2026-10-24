#!/usr/bin/env python3
"""Single-site mutants: assert edit applied, compile, require target assertion, restore."""
import json, os, subprocess
from pathlib import Path
crate=Path(__file__).resolve().parents[4]
os.chdir(crate)
log=Path('tools/primh/logs/buffer-state')
p='src/prims/buffer_state.rs'
mutants=[
('overlayp-false',p,'cx.kind(object) == Kind::Overlay','false','primh_overlayp','overlayp/0032'),
('overlayp-true',p,'cx.kind(object) == Kind::Overlay','true','primh_overlayp','overlayp/0000'),
('overlayp-marker',p,'cx.kind(object) == Kind::Overlay','matches!(cx.kind(object), Kind::Overlay | Kind::Marker)','primh_overlayp','overlayp/0030'),
('live-ignore-death',p,'is_some_and(|b| b.live)','is_some_and(|_| true)','primh_buffer_live_p','buffer-live-p/0032'),
('live-false',p,'is_some_and(|b| b.live)','is_some_and(|_| false)','primh_buffer_live_p','buffer-live-p/0029'),
('live-accept-any',p,'cx.buffer_state(object).is_some_and(|b| b.live)','true','primh_buffer_live_p','buffer-live-p/0000'),
('modified-false',p,'cx.buffer_state(buffer).unwrap().modified()','false','primh_buffer_modified_p','buffer-modified-p/0032'),
('modified-true',p,'cx.buffer_state(buffer).unwrap().modified()','true','primh_buffer_modified_p','buffer-modified-p/0028'),
('modified-inverted',p,'cx.buffer_state(buffer).unwrap().modified()','!cx.buffer_state(buffer).unwrap().modified()','primh_buffer_modified_p','buffer-modified-p/0028'),
('name-none',p,'let name = cx.buffer_state(buffer).unwrap().name.clone();','let name: Option<String> = None;','primh_buffer_name','buffer-name/0028'),
('name-wrong',p,'Some(name) => cx.string(&name)','Some(_) => cx.string("wrong")','primh_buffer_name','buffer-name/0028'),
('name-dead','src/bytes.rs','self.name = None;','self.name = Some("dead".into());','primh_buffer_name','buffer-name/0035'),
('temp-cleanup','src/eval_buffer.rs','self.kill_buffer_model(self.stack[start])?;','let _ = self.stack[start];','buffer::model_tests::named_temporary_buffer_dies_on_normal_error_and_throw_exits','(with-temp-buffer (setq escaped-temp'),
('modified-property','src/annotations.rs','if changed { self.state.record_change(false); }','if false { self.state.record_change(false); }','buffer::model_tests::buffer_identity_and_ticks_survive_representation_changes_and_collection','assertion `left == right` failed'),
]
results=[]
for name,file,old,new,test,case in mutants:
 path=Path(file); original=path.read_text(); assert old in original,(name,'not applicable')
 # Property helper has two identical sites: explicitly mutate the first only.
 mutated=original.replace(old,new,1); assert mutated!=original
 path.write_text(mutated)
 try:
  cmd=['cargo','test','-p','romacs-vm','--jobs','8']
  cmd+=['--lib'] if name in ('modified-property', 'temp-cleanup') else ['--test','primh_buffer_state']
  run=subprocess.run(cmd+[test,'--','--exact','--nocapture'],capture_output=True,text=True)
  output=run.stdout+run.stderr; (log/f'mutant-{name}.log').write_text(output)
  assert run.returncode!=0 and 'panicked at' in output and case in output and 'could not compile' not in output,(name,output[-4000:])
  results.append({'mutant':name,'file':file,'old':old,'new':new,'changed':True,'killed_by':test,'assertion':case,'exit':run.returncode})
 finally: path.write_text(original)
 (log/'mutants.json').write_text(json.dumps(results,indent=2)+'\n')
 print(name,'KILLED',case,flush=True)
