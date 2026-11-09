//! Autoload loading: GNU `src/eval.c` `Fautoload_do_load` (eval.c:2494),
//! `load_with_autoload_queue` (eval.c:2465) and `un_autoload` (eval.c:2447).
//!
//! Calling, `funcall`ing or `apply`ing a symbol whose function cell holds
//! `(autoload FILE DOC INTERACTIVE TYPE)` loads FILE and retries the call
//! (eval.c:2760, 3176, 3530); compiling a call whose head is an autoload of a
//! macro loads FILE first and then expands (`Fmacroexpand`, eval.c:1326).
//! `require` shares the same load wrapper (fns.c `Frequire`).
//!
//! The autoload queue. While such a load runs, GNU's `Vautoload_queue` is `t`
//! and `defalias` over a non-nil definition (data.c:984-990) and `provide`
//! (fns.c:3751) push undo records on it. If the load signals, `un_autoload`
//! replays them newest first, so the function cells and `features` revert;
//! a function that had NO definition before the load stays defined, exactly as
//! in GNU. Loads nest: each level saves the outer queue and restores it on the
//! way out, and a nested load that succeeded is not undone by its caller's
//! failure (the inner level reset its queue to `t` before returning).
//!
//! VM2 has no `function-history` (DIVERGENCES.md VM2-AUTOLOAD1), so a symbol's
//! record is `(SYMBOL . OLD-DEFINITION)` captured at the `defalias`, where GNU
//! re-reads the previous definition from that symbol property when it undoes.
//!
//! Every Value retained across the load is collector-visible: the queue lives
//! in the hidden variable `#:vm2-autoload-queue`, the outer queue in its binding
//! record, FUNDEF and the saved match data in root frames, so a collection inside
//! the load traces and forwards them all.
use super::*;
use crate::brand::{Gc, Mx};
use crate::gc::RootFrame;

pub(super) const QUEUE: &str = "#:vm2-autoload-queue";
/// `Vm::autoload_args`'s frame: the arguments, rooted across the load, then the
/// file and at last the value.
const FUNDEF: usize = 0;
const FUNNAME: usize = 1;
const MACRO_ONLY: usize = 2;
const OUT: usize = 3;

impl Vm {
    pub(super) fn initialize_autoload(&mut self) {
        self.declare_special_raw(QUEUE, Value::NIL);
    }
    fn symbol_named(&self, v: Value, name: &str) -> bool {
        v.tag() == 7
            && v != Value::UNBOUND
            && self.symbol_names.get(v.payload() as usize - 1).is_some_and(|n| n == name)
    }
    /// `AUTOLOADP`: a cons whose car is the symbol `autoload`.
    pub(super) fn autoload_object(&self, v: Value) -> bool {
        v.tag() == 3 && self.cons_pair(v).is_ok_and(|(head, _)| self.symbol_named(head, "autoload"))
    }
    /// `(nth 4 FUNDEF)`, GNU's `Fnth (make_fixnum (4), fundef)`.
    fn autoload_type(&self, fundef: Value) -> Result<Value> {
        let mut tail = fundef;
        for _ in 0..4 {
            if tail == Value::NIL {
                return Ok(Value::NIL);
            }
            tail = self.autoload_cdr(tail)?;
        }
        if tail == Value::NIL { Ok(Value::NIL) } else { Ok(self.autoload_car(tail)?) }
    }
    fn autoload_car(&self, v: Value) -> Result<Value> {
        if v == Value::NIL { return Ok(v) }
        self.cons_pair(v).map(|(a, _)| a).map_err(|_| "wrong type argument: listp".into())
    }
    fn autoload_cdr(&self, v: Value) -> Result<Value> {
        if v == Value::NIL { return Ok(v) }
        self.cons_pair(v).map(|(_, d)| d).map_err(|_| "wrong type argument: listp".into())
    }
    /// True when FUNDEF is an autoload whose TYPE is `t` or `macro`.
    pub(super) fn autoload_macro(&self, fundef: Value) -> bool {
        self.autoload_object(fundef)
            && self.autoload_type(fundef).is_ok_and(|kind| kind == Value::TRUE || self.symbol_named(kind, "macro"))
    }
    pub(crate) fn signal_error(&mut self, condition: &str, data: &[Value], text: String) -> String {
        let condition = self.intern(condition);
        let data = self.list_value(data);
        self.propagated_escape = Some(threaded::Escape::Signal(condition, data));
        text
    }
    /// GNU `Findirect_function (FUNNAME, Qnil)`: the end of the alias chain,
    /// a macro reconstructed as `(macro . F)` from VM2's slot flag.
    fn autoload_indirect(&mut self, funname: Value) -> Result<Value> {
        let mut v = funname;
        let mut steps = 0;
        while v.tag() == 7 && v != Value::UNBOUND && v != Value::NIL {
            let slot = v.payload() as usize - 1;
            let f = self.slots.get(slot).ok_or("invalid symbol")?.function;
            if f == Value::UNBOUND || f == Value::NIL {
                return Ok(Value::NIL);
            }
            if self.macros.contains(&slot) {
                let tag = self.intern("macro");
                return Ok(self.alloc_cons(tag, f));
            }
            v = f;
            steps += 1;
            if steps > self.slots.len() {
                return Ok(Value::NIL);
            }
        }
        Ok(v)
    }
    /// `autoload_do_load`'s frame.
    pub(crate) fn autoload_args(&mut self, fundef: Value, funname: Value, macro_only: Value) -> RootFrame {
        self.root_frame(&[fundef, funname, macro_only, Value::NIL])
    }
    /// `do_load`'s OUTCOME, and FRAME released: the value it left there.
    pub(crate) fn autoload_result(&mut self, frame: RootFrame, outcome: Result<()>) -> Result<Value> {
        let [_, _, _, value] = self.release_frame(frame);
        outcome.map(|()| value)
    }
    /// GNU `Fautoload_do_load (FUNDEF, FUNNAME, MACRO-ONLY)`, for a caller not
    /// yet converted, which keeps its own Values rooted across it.
    pub(super) fn autoload_do_load(&mut self, fundef: Value, funname: Value, macro_only: Value) -> Result<Value> {
        let args = self.autoload_args(fundef, funname, macro_only);
        let outcome = threaded::toplevel(self, &args, |mut m, gc, args| do_load(&mut m, gc, args));
        self.autoload_result(args, outcome)
    }
    /// `Fautoload_do_load` up to its load: whether a load's errors are ignored,
    /// with the file to load in the frame, or None and the value there.
    fn autoload_prepared(&mut self, args: &RootFrame) -> Result<Option<bool>> {
        let (fundef, funname, macro_only) = (self.rooted(args, FUNDEF), self.rooted(args, FUNNAME), self.rooted(args, MACRO_ONLY));
        if !self.autoload_object(fundef) {
            self.set_rooted(args, OUT, fundef);
            return Ok(None);
        }
        let kind = match self.autoload_type(fundef) {
            Ok(kind) => kind,
            Err(error) => {
                let listp = self.intern("listp");
                return Err(self.signal_error("wrong-type-argument", &[listp, fundef], error));
            }
        };
        let is_macro = kind == Value::TRUE || self.symbol_named(kind, "macro");
        if self.symbol_named(macro_only, "macro") && !is_macro {
            self.set_rooted(args, OUT, fundef);
            return Ok(None);
        }
        // `will_dump_p ()' is never true for a VM2 session: no dump exists.
        if !(funname == Value::NIL || funname == Value::TRUE || (funname.tag() == 7 && funname != Value::UNBOUND)) {
            let symbolp = self.intern("symbolp");
            return Err(self.signal_error("wrong-type-argument", &[symbolp, funname], "wrong type argument: symbolp".into()));
        }
        // A best-effort load (MACRO-ONLY on a non-macro) ignores load errors.
        let ignore_errors = !is_macro && macro_only != Value::NIL;
        let file = match self.autoload_cdr(fundef).and_then(|d| self.autoload_car(d)) {
            Ok(file) => file,
            Err(error) => {
                let listp = self.intern("listp");
                return Err(self.signal_error("wrong-type-argument", &[listp, fundef], error));
            }
        };
        if self.loader.recording.is_some() && let Some(name) = self.source_string_if(file).map(str::to_owned) {
            self.note_init_load(crate::world::layer::Kind::Autoload, name, None, ignore_errors);
        }
        self.set_rooted(args, OUT, file);
        Ok(Some(ignore_errors))
    }
    /// `Fautoload_do_load` after its load of the file LOADED: the definition it
    /// must have replaced, or nil when none was asked for.
    fn autoload_finished(&mut self, args: &RootFrame, ignore_errors: bool, loaded: &std::path::Path) -> Result<()> {
        let (fundef, funname) = (self.rooted(args, FUNDEF), self.rooted(args, FUNNAME));
        if funname == Value::NIL || ignore_errors {
            self.set_rooted(args, OUT, Value::NIL);
            return Ok(());
        }
        let fun = self.autoload_indirect(funname)?;
        if self.equal(fun, fundef).map_err(|_| "equal failed comparing an autoload")? {
            let name = self.display(funname)?;
            let message = format!("Autoloading file {} failed to define function {name}", loaded.display());
            let text = self.literal(&Form::String(message.clone()))?;
            return Err(self.signal_error("error", &[text], message));
        }
        self.set_rooted(args, OUT, fun);
        Ok(())
    }
    /// Bind the queue as GNU's `load_with_autoload_queue` does, the match data
    /// saved: the binding mark and the data's frame.
    fn enter_autoload_queue(&mut self) -> Result<(usize, RootFrame)> {
        // record_unwind_save_match_data (): Fmatch_data (Qnil, Qnil, Qnil).
        let match_data = self.save_match_data_for_load()?;
        let saved = self.root_frame(&[match_data]);
        let queue = self.slot(QUEUE);
        let mark = self.dynamic.len();
        let buffer = self.slots[self.buffer_slot].value;
        // record_unwind_protect (un_autoload, Vautoload_queue); Vautoload_queue = Qt;
        self.dynamic.push((queue, self.slots[queue].value, buffer, false));
        self.slots[queue].value = Value::TRUE;
        Ok((mark, saved))
    }
    /// Restore the match data (`Fset_match_data (saved, Qt)`) and undo what a
    /// FAILED load recorded: whether the match data went back.
    fn leave_autoload_queue(&mut self, saved: RootFrame, failed: bool) -> bool {
        let [match_data] = self.release_frame(saved);
        let restored = self.regexp_set_match_data(match_data, Value::TRUE).is_ok();
        if failed {
            let queue = self.slot(QUEUE);
            self.un_autoload(queue);
        }
        restored
    }
    fn save_match_data_for_load(&mut self) -> Result<Value> {
        self.regexp_match_data_saved().ok_or_else(|| "match-data failed saving an autoload's match data".into())
    }
    /// The file `Fload` finds for FILE, and whether a suffix was added to its
    /// name; None when NOERROR found none.
    fn queued_source(&mut self, file: Value, noerror: bool, must_suffix: bool) -> Result<Option<(std::path::PathBuf, bool)>> {
        let name = self.source_string(file)?.to_owned();
        let Some((path, suffixed)) = self.source_path(&name, false)? else {
            if noerror {
                return Ok(None);
            }
            let text = self.literal(&Form::String("Cannot open load file".into()))?;
            let reason = self.literal(&Form::String("No such file or directory".into()))?;
            return Err(self.signal_error("file-missing", &[text, reason, file], format!("file missing: {name}")));
        };
        if must_suffix && path.extension().is_none() {
            return Err("load mustsuffix requires a source suffix".into());
        }
        Ok(Some((path, suffixed)))
    }
    /// GNU `un_autoload`: undo the current level's records, newest first.
    fn un_autoload(&mut self, queue: usize) {
        let mut tail = std::mem::replace(&mut self.slots[queue].value, Value::TRUE);
        let features = self.slot("features");
        let mut steps = 0;
        while let Ok((record, next)) = self.cons_pair(tail) {
            if let Ok((head, old)) = self.cons_pair(record) {
                if head == Value::integer(0).unwrap() {
                    self.slots[features].value = old;
                } else if head.tag() == 7 && head != Value::UNBOUND {
                    let slot = head.payload() as usize - 1;
                    let (definition, is_macro) = match self.cons_pair(old) {
                        Ok((tag, body)) if self.symbol_named(tag, "macro") => (body, true),
                        _ => (old, false),
                    };
                    self.define_function(slot, definition, is_macro).expect("function version exhausted");
                }
            }
            tail = next;
            steps += 1;
            if steps > self.list_limit() {
                break;
            }
        }
    }
    /// data.c:984-990: `defalias` over a non-nil definition while a queue is
    /// active records the old definition. Call before the cell is written.
    pub(super) fn autoload_note_defalias(&mut self, slot: usize) {
        let Some(&queue) = self.names.get(QUEUE) else { return };
        if self.slots[queue].value == Value::NIL {
            return;
        }
        let old = self.slots[slot].function;
        if old == Value::UNBOUND || old == Value::NIL {
            return;
        }
        let old = if self.macros.contains(&slot) {
            let tag = self.intern("macro");
            self.alloc_cons(tag, old)
        } else {
            old
        };
        let record = self.alloc_cons(Value::reference(slot + 1, 7), old);
        let tail = self.slots[queue].value;
        let tail = if tail == Value::TRUE { Value::NIL } else { tail };
        self.slots[queue].value = self.alloc_cons(record, tail);
    }
    /// fns.c:3751: `provide` while a queue is active records `(0 . features)`.
    pub(super) fn autoload_note_provide(&mut self) {
        let Some(&queue) = self.names.get(QUEUE) else { return };
        if self.slots[queue].value == Value::NIL {
            return;
        }
        let features = self.slot("features");
        let record = self.alloc_cons(Value::integer(0).unwrap(), self.slots[features].value);
        let tail = self.slots[queue].value;
        let tail = if tail == Value::TRUE { Value::NIL } else { tail };
        self.slots[queue].value = self.alloc_cons(record, tail);
    }
}

/// GNU `Fautoload_do_load` on the arguments in ARGS (`Vm::autoload_args`), the
/// value left there. It holds the permission and no Value (`crate::brand`).
pub(crate) fn do_load<'b>(m: &mut Mx<'_, 'b, Vm>, mut gc: Gc<'_, 'b>, args: &RootFrame) -> Result<()> {
    let Some(ignore_errors) = m.leaf(args, Vm::autoload_prepared)? else { return Ok(()) };
    let loaded = queued_load(m, gc.by_ref(), (args, OUT), ignore_errors, true)?;
    m.leaf((args, ignore_errors, loaded.as_path()), |vm, (args, ignore_errors, loaded)| vm.autoload_finished(args, ignore_errors, loaded))
}
/// GNU `load_with_autoload_queue (FILE, NOERROR, Qt, Qnil, MUST-SUFFIX)` over
/// `save_match_data_load`, FILE rooted. Returns the file that was loaded, or the
/// empty path when NOERROR found no file (GNU's `load` returned nil).
pub(crate) fn queued_load<'b>(m: &mut Mx<'_, 'b, Vm>, mut gc: Gc<'_, 'b>, file: crate::loader::Out<'_>, noerror: bool, must_suffix: bool) -> Result<std::path::PathBuf> {
    let (mark, saved) = m.leaf((), |vm, ()| vm.enter_autoload_queue())?;
    let result = load_queued_source(m, gc.by_ref(), file, noerror, must_suffix);
    let restored = m.leaf((saved, result.is_err()), |vm, (saved, failed)| vm.leave_autoload_queue(saved, failed));
    let path = crate::loader::leave_bindings(m, gc, mark, result)?;
    if !restored {
        return Err("set-match-data failed restoring an autoload's match data".into());
    }
    Ok(path)
}
/// `Fload (FILE, NOERROR, Qt, Qnil, MUST-SUFFIX)`, VM2's source loader.
fn load_queued_source<'b>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>, file: crate::loader::Out<'_>, noerror: bool, must_suffix: bool) -> Result<std::path::PathBuf> {
    let found = m.leaf((file, noerror, must_suffix), |vm, ((frame, i), noerror, must_suffix)| {
        let file = vm.rooted(frame, i);
        vm.queued_source(file, noerror, must_suffix)
    })?;
    let Some((path, suffixed)) = found else { return Ok(std::path::PathBuf::new()) };
    crate::loader::load_discarding(m, gc, &path, suffixed)?;
    Ok(crate::host::canonicalize(&path).unwrap_or(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let p = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(format!(
            ".hermes/cache/scratch/romacs/vm2/autoload-unit-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }
    const GARBAGE: &str = "(let ((i 0)) (while (< i 200000) (cons i i) (setq i (1+ i))))";

    /// FUNDEF is compared with the new definition AFTER the load. Garbage
    /// allocated before it and 200000 conses inside the load make the moving
    /// collector run (asserted) and relocate FUNDEF's conses; only a traced
    /// root still names them, so "failed to define" is reported correctly.
    #[test]
    fn fundef_survives_moving_collections_inside_the_load() {
        let d = dir("fundef");
        std::fs::write(d.join("al-gc-nodef.el"), format!("{GARBAGE}\n(defun al-gc-other () 1)\n")).unwrap();
        let mut vm = Vm::new();
        vm.set_gc_moving(true);
        vm.set_load_path(std::slice::from_ref(&d)).unwrap();
        // No collection while the garbage and then FUNDEF are allocated, so
        // all 200000 dead conses precede FUNDEF in the arena and the first
        // collection inside the load must move it.
        vm.set_collection_floor(usize::MAX / 4);
        vm.eval_source_raw(&format!("{GARBAGE} (autoload 'al-gc \"al-gc-nodef\")")).unwrap();
        vm.set_collection_floor(1);
        let symbol = vm.intern("al-gc");
        let slot = symbol.payload() as usize - 1;
        let fundef = vm.slots[slot].function;
        let before = vm.heap_stats().collections;
        let result = vm.autoload_do_load(fundef, symbol, Value::NIL);
        assert!(vm.heap_stats().collections > before, "the load must collect for this proof to mean anything");
        assert_ne!(vm.slots[slot].function, fundef, "FUNDEF's conses must have moved for this proof to mean anything");
        let error = result.expect_err("al-gc-nodef.el defines no al-gc");
        assert!(error.starts_with("Autoloading file ") && error.ends_with("failed to define function al-gc"), "{error}");
        let cell = vm.slots[slot].function;
        assert_eq!(vm.display(cell).unwrap(), "(autoload \"al-gc-nodef\" nil nil nil)");
    }

    /// A load's bindings are made with what they bind rooted, so a watcher on one
    /// may collect, and the load goes on with what moved. A watcher's signal ends
    /// the load, and every binding it made unwinds.
    #[test]
    fn a_watcher_on_a_load_binding_collects_and_can_end_the_load() {
        let d = dir("watched");
        std::fs::write(d.join("al-watched.el"), "(setq al-name load-file-name)\n(setq al-history (nth (- (length current-load-list) 1) current-load-list))\n").unwrap();
        let mut vm = Vm::new();
        vm.set_gc_moving(true);
        vm.set_load_path(std::slice::from_ref(&d)).unwrap();
        vm.set_collection_floor(usize::MAX / 4);
        vm.eval_source_raw(&format!("{GARBAGE} (defvar al-name nil) (defvar al-history nil) (defvar al-fail nil) \
            (add-variable-watcher 'load-file-name (lambda (_symbol _new _operation _where) (garbage-collect) (when al-fail (error \"al-watcher\"))))")).unwrap();
        let before = vm.heap_stats().collections;
        vm.eval_source_raw("(load \"al-watched\")").unwrap();
        assert!(vm.heap_stats().collections > before, "the watcher must collect for this proof to mean anything");
        let expected = d.join("al-watched.el").to_string_lossy().into_owned();
        let shown = vm.eval_source_raw("(list al-name al-history (car (car load-history)) load-file-name)").unwrap();
        assert_eq!(vm.display(shown).unwrap(), format!("({expected:?} {expected:?} {expected:?} nil)"));
        let shown = vm.eval_source_raw("(setq al-fail t) (setq al-name nil) (list (condition-case err (load \"al-watched\") (error (car (cdr err)))) al-name load-in-progress load-file-name)").unwrap();
        assert_eq!(vm.display(shown).unwrap(), "(\"al-watcher\" nil nil nil)");
        assert!(!vm.source_load_active(), "the failed load has left the loads in progress");
    }
    /// The undo records live in the queue variable. Collections inside the
    /// failing load move the old definition; the undo must reinstall it.
    #[test]
    fn undo_records_survive_moving_collections_inside_the_load() {
        let d = dir("undo");
        std::fs::write(
            d.join("al-gc-bad.el"),
            format!("(defun al-gc-existing () 'redefined)\n{GARBAGE}\n(provide 'al-gc-bad)\n(error \"al-gc-bad fails\")\n"),
        )
        .unwrap();
        let mut vm = Vm::new();
        vm.set_load_path(std::slice::from_ref(&d)).unwrap();
        vm.set_collection_floor(usize::MAX / 4);
        vm.eval_source_raw(&format!("{GARBAGE} (defun al-gc-existing () (list 'original 1)) (autoload 'al-gc-bad \"al-gc-bad\")")).unwrap();
        vm.set_collection_floor(1);
        let symbol = vm.intern("al-gc-bad");
        let fundef = vm.slots[symbol.payload() as usize - 1].function;
        let before = vm.heap_stats().collections;
        assert!(vm.autoload_do_load(fundef, symbol, Value::NIL).is_err());
        assert!(vm.heap_stats().collections > before);
        let shown = vm.eval_source_raw("(list (al-gc-existing) (featurep 'al-gc-bad) (car (symbol-function 'al-gc-bad)))").unwrap();
        assert_eq!(vm.display(shown).unwrap(), "((original 1) nil autoload)");
    }
}
