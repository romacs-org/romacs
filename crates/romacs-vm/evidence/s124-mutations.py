#!/usr/bin/env python3
"""Compiling, assertion-killed mutants; always restore exact original bytes."""
import os as _portable_os
from pathlib import Path as _PortablePath
import hashlib, json, os, pathlib, subprocess
root=pathlib.Path(__file__).resolve().parents[3]
evidence=root/'crates/romacs-vm/evidence'
src=root/'crates/romacs-vm/src'
cases=[]
def case(suite,test,file,old,new): cases.append((suite,test,file,old,new))
case('eval_data','equal_is_deep_and_type_sensitive','eval_data.rs','let mut pending = vec![(a, b)];','if a != b { return Ok(false); } let mut pending = vec![(a, b)];')
case('eval_data','equal_handles_cycles_and_ignores_string_properties','eval_data.rs','if a == b || !seen.insert((a.0, b.0)) {','if a != b && seen.contains(&(a.0, b.0)) { return Ok(false); } if a == b || !seen.insert((a.0, b.0)) {')
case('eval_data','min_preserves_winner_type_and_exact_integer_order','eval_data.rs','Some(std::cmp::Ordering::Less)','Some(std::cmp::Ordering::Greater)')
case('eval_data','append_copies_prefix_and_shares_final_tail','eval_data.rs','let mut prefix = Vec::new();','return Ok(self.stack[start+n-1]); let mut prefix = Vec::new();')
case('eval_data','append_accepts_vectors_unicode_and_unibyte_strings','eval_data.rs','Some(data::Object::Vector(values)) => prefix.extend_from_slice(values),','Some(data::Object::Vector(_values)) => {},')
case('eval_data','prin1_quotes_nested_strings_and_honors_noescape','eval_data.rs','let quoted = n == 1 || self.stack[start + 1] == Value::NIL;','let quoted = false;')
case('eval_data','prin1_honors_escape_newlines_and_rejects_unsupported_circle_mode','eval_data.rs','let escape_newlines = self.eval_data_print_option("print-escape-newlines")?;','let escape_newlines = false;')
case('eval_data','data_results_survive_counted_moving_collection','eval_data.rs','let mut prefix = Vec::new();','return Ok(Value::NIL); let mut prefix = Vec::new();')
case('eval_buffer','get_buffer_lookup_identity_and_absence','eval_buffer.rs','GetBuffer | SwitchToBuffer => {','GetBuffer | SwitchToBuffer => { if p == GetBuffer { return Ok(Value::TRUE); }')
case('eval_buffer','switch_to_buffer_creates_and_preserves_named_buffers','eval_buffer.rs','self.slots[self.buffer_slot].value = value;','self.slots[self.buffer_slot].value = Value::NIL;')
case('eval_buffer','erase_buffer_widens_and_moves_markers','eval_buffer.rs','b.replace(ByteOffset(0)..ByteOffset(end), b"")?;','b.replace(ByteOffset(0)..ByteOffset(0), b"")?;')
case('eval_buffer','forward_line_unicode_boundaries_and_shortages','eval_buffer.rs','Value::integer(shortage)','Value::integer(shortage + 1)')
case('eval_buffer','modes_and_update_data_model','eval_buffer.rs','let mode = self.intern("emacs-lisp-mode");','let mode = self.intern("wrong-mode");')
case('eval_buffer','daemon_truth_and_initializer_refusal','eval_buffer.rs','Daemonp => Ok(Value::NIL),','Daemonp => Ok(Value::TRUE),')
case('eval_buffer','locate_library_searches_without_loading','eval_buffer.rs','if candidate.is_file() {','if false && candidate.is_file() {')
case('eval_buffer','named_buffer_roots_survive_counted_collection','eval_buffer.rs','self.slots[self.buffer_slot].value = value;','self.slots[self.buffer_slot].value = Value::NIL;')
case('eval_variables','fboundp_checks_function_cells_without_calling_them','variable_runtime.rs','Fboundp => Ok(truth(','Fboundp => Ok(truth(false && ')
case('eval_variables','set_uses_symbol_value_not_lexical_binding','variable_runtime.rs','self.set_variable_value(slot, value)?;','self.set_variable_value(slot, Value::NIL)?;')
case('eval_variables','makunbound_voids_dynamic_binding_then_unwinds','variable_runtime.rs','else { Value::UNBOUND };','else { Value::TRUE };')
case('eval_variables','explicit_local_only_affects_current_buffer_and_keeps_void_local','variable_runtime.rs','if !create || explicit || shadowed {','if !create || shadowed {')
case('eval_variables','local_values_survive_counted_collection','variable_runtime.rs','self.set_variable_value(slot, value)?;','self.set_variable_value(slot, Value::NIL)?;')
case('eval_variables','ordinary_startup_variables_are_bound_without_fake_functions','lib.rs','vm.declare_special("menu-bar-lines", Value::integer(0).unwrap());','vm.declare_special("menu-bar-lines", Value::integer(1).unwrap());')
case('eval_buffer_names','generated_buffers_are_named_and_unique_and_scratch_is_lazy','buffer.rs','self.slots[slot].value = buffer;','self.slots[slot].value = Value::NIL;')
case('eval_truncate','truncate_lines_is_buffer_local_when_set','variable_runtime.rs','self.symbol_names[slot] == "truncate-lines"','self.symbol_names[slot] == "not-truncate-lines"')
case('eval_review_regressions','temporary_local_values_follow_owner_lifetime_not_global_slots','gc.rs','if !self.local_binding_slots.contains(&slot) { pending.push(s.value); }','pending.push(s.value);')
case('eval_review_regressions','temporary_local_values_follow_owner_lifetime_not_global_slots','gc.rs','if k && table.weakness == hash::Weakness::Key {','if false && k && table.weakness == hash::Weakness::Key {')
case('eval_review_regressions','protected_constants_reject_all_variable_mutators','variable_runtime.rs','slot == self.multibyte_slot || self.fixnum_slots.contains(&slot)','slot == self.multibyte_slot')
case('eval_review_regressions','protected_constants_reject_all_variable_mutators','data.rs','error == "setting constant" && matches!','error == "not setting constant" && matches!')
case('eval_review_regressions','mode_establishes_local_binding_after_explicit_localization_elsewhere','eval_buffer.rs','self.make_local_variable(slot)?;','/* omitted localization */')
case('eval_review_regressions','local_if_set_detects_existing_local_binding_but_not_other_buffers','variable_runtime.rs','Ok(truth(automatic || local))','Ok(truth(automatic))')
case('eval_review_regressions','local_if_set_detects_existing_local_binding_but_not_other_buffers','variable_runtime.rs','let local = self.variable_binding_slot(slot, false)? != slot;\n                let automatic = self.local_table(slot)?.is_some() && self.symbol_property(symbol, explicit)? == Value::NIL;','let automatic = self.local_table(slot)?.is_some() && self.symbol_property(symbol, explicit)? == Value::NIL;\n                let local = self.variable_binding_slot(slot, false)? != slot;')
case('eval_review2_regressions','menu_mode_noninteractive_arguments_enable_or_explicitly_toggle_truth','eval_buffer.rs','let enabled = if n > 0','let enabled = false; let _enabled = if n > 0')
case('eval_review2_regressions','menu_and_library_search_use_current_buffer_variable_values','eval_buffer.rs','self.set_variable_value(slot, value)?;','self.slots[slot].value = value;')
case('eval_review2_regressions','menu_and_library_search_use_current_buffer_variable_values','eval_buffer.rs','self.variable_value(path_slot)?','self.slots[path_slot].value')
case('eval_review2_regressions','let_uses_existing_local_else_default_without_creating_local','threaded.rs','let slot = e.vm.variable_binding_slot(slot, false)?;','let slot = e.vm.variable_binding_slot(slot, true)?;')
case('eval_review2_regressions','let_uses_existing_local_else_default_without_creating_local','variable_runtime.rs','if !create || explicit || shadowed {','if !create || explicit {')
case('eval_review2_regressions','let_uses_existing_local_else_default_without_creating_local','gc.rs','rewrite(buffer);','let _ = buffer;')
case('eval_review2_regressions','ascii_unibyte_buffer_names_share_identity_with_text_names','eval_buffer.rs','Ok(Some(String::from_utf8(bytes.clone()).unwrap()))','Ok(None)')
case('eval_binding_kind','localizing_inside_plain_let_does_not_convert_its_binding_kind','variable_runtime.rs','*localized && *bound == slot','*bound == slot')
case('eval_float_equal','equal_signed_zero_uses_bits_in_lists_without_changing_numeric_equal','eval_data.rs','if x.to_bits() == y.to_bits() => {}','if x == y => {}')
case('eval_float_equal','equal_nan_bits_include_sign_in_scalars_and_vectors','eval_data.rs','if x.to_bits() == y.to_bits() => {}','if x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan()) => {}')
case('eval_mode_process','mode_initialization_does_not_reallocate_active_register_storage','eval_buffer.rs','self.make_local_variable(slot)?;','let symbol = Value::reference(slot+1,7); let argument = self.stack.len(); self.stack.push(symbol); let localized = self.variable_primitive(MakeLocalVariable, argument, 1); self.stack.pop(); localized?;')
results=[]
env=dict(os.environ,CARGO_TARGET_DIR=str(_PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'lanes/s124-target'))
for case_id,(suite,test,file,old,new) in enumerate(cases):
 p=src/file; original=p.read_bytes(); text=original.decode(); assert old in text,(test,old)
 try:
  p.write_text(text.replace(old,new,1))
  run=subprocess.run(['cargo','test','-p','romacs-vm','--test',suite,test,'--','--exact'],cwd=root,env=env,text=True,capture_output=True)
  log=run.stdout+run.stderr
  logfile=evidence/f's124-mutant-{case_id}-{test}.log'
  logfile.write_text(log)
  killed=run.returncode==101 and 'panicked at' in log and ('assertion' in log or 'called `Result::unwrap' in log or 'temporary local graphs grow with iterations' in log or 'accepted protected mutation' in log or 'fresh mode process failed' in log) and 'could not compile' not in log
  results.append(dict(test=test,file=file,exit=run.returncode,assertion_killed=killed,log=str(logfile),mutation=[old,new],sha256=hashlib.sha256(original).hexdigest()))
  print(test,run.returncode,'KILLED' if killed else 'NOT PROVEN',flush=True)
 finally: p.write_bytes(original)
(evidence/'s124-mutations.json').write_text(json.dumps(results,indent=2)+'\n')
assert all(r['assertion_killed'] for r in results)
