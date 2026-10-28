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
//! in the hidden variable `#:vm2-autoload-queue`, the outer queue, FUNDEF and
//! the saved match data are held as dynamic-binding records of hidden
//! variables (the same rooting `load_file` uses for `load-file-name`), so a
//! collection inside the load traces and forwards them all.
use super::*;

pub(super) const QUEUE: &str = "#:vm2-autoload-queue";
const PENDING: &str = "#:vm2-autoload-fundef";
const MATCH: &str = "#:vm2-autoload-match-data";

impl Vm {
    pub(super) fn initialize_autoload(&mut self) {
        for name in [QUEUE, PENDING, MATCH] {
            self.declare_special_raw(name, Value::NIL);
        }
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
    fn signal_error(&mut self, condition: &str, data: &[Value], text: String) -> String {
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
                self.conses.push((tag, f));
                return Ok(Value::reference(self.conses.len() - 1, 3));
            }
            v = f;
            steps += 1;
            if steps > self.slots.len() {
                return Ok(Value::NIL);
            }
        }
        Ok(v)
    }
    /// GNU `Fautoload_do_load (FUNDEF, FUNNAME, MACRO-ONLY)`.
    pub(super) fn autoload_do_load(&mut self, fundef: Value, funname: Value, macro_only: Value) -> Result<Value> {
        if !self.autoload_object(fundef) {
            return Ok(fundef);
        }
        let kind = match self.autoload_type(fundef) {
            Ok(kind) => kind,
            Err(e) => {
                let listp = self.intern("listp");
                return Err(self.signal_error("wrong-type-argument", &[listp, fundef], e));
            }
        };
        let is_macro = kind == Value::TRUE || self.symbol_named(kind, "macro");
        if self.symbol_named(macro_only, "macro") && !is_macro {
            return Ok(fundef);
        }
        // `will_dump_p ()' is never true for a VM2 session: no dump exists.
        if !(funname == Value::NIL || funname == Value::TRUE || (funname.tag() == 7 && funname != Value::UNBOUND)) {
            let symbolp = self.intern("symbolp");
            return Err(self.signal_error("wrong-type-argument", &[symbolp, funname], "wrong type argument: symbolp".into()));
        }
        // A best-effort load (MACRO-ONLY on a non-macro) ignores load errors.
        let ignore_errors = if is_macro { Value::NIL } else { macro_only };
        let file = match self.autoload_cdr(fundef).and_then(|d| self.autoload_car(d)) {
            Ok(file) => file,
            Err(e) => {
                let listp = self.intern("listp");
                return Err(self.signal_error("wrong-type-argument", &[listp, fundef], e));
            }
        };
        // Root FUNDEF for the duration of the load; it is compared afterwards.
        let pending = self.slot(PENDING);
        let mark = self.dynamic.len();
        let buffer = self.slots[self.buffer_slot].value;
        self.dynamic.push((pending, self.slots[pending].value, buffer, false));
        self.slots[pending].value = fundef;
        let loaded = self.load_with_autoload_queue(file, ignore_errors != Value::NIL, true);
        let fundef = self.slots[pending].value;
        self.unbind(self.dynamic.len() - mark);
        let loaded = loaded?;
        if funname == Value::NIL || ignore_errors != Value::NIL {
            return Ok(Value::NIL);
        }
        let fun = self.autoload_indirect(funname)?;
        if self.equal(fun, fundef).map_err(|_| "equal failed comparing an autoload")? {
            let name = self.display(funname)?;
            let message = format!("Autoloading file {} failed to define function {name}", loaded.display());
            let text = self.literal(&Form::String(message.clone()))?;
            return Err(self.signal_error("error", &[text], message));
        }
        Ok(fun)
    }
    /// GNU `load_with_autoload_queue (FILE, NOERROR, Qt, Qnil, MUST-SUFFIX)`
    /// over `save_match_data_load`. Returns the file that was loaded, or the
    /// empty path when NOERROR found no file (GNU's `load' returned nil).
    pub(super) fn load_with_autoload_queue(&mut self, file: Value, noerror: bool, must_suffix: bool) -> Result<std::path::PathBuf> {
        let queue = self.slot(QUEUE);
        let saved_match = self.slot(MATCH);
        let mark = self.dynamic.len();
        let buffer = self.slots[self.buffer_slot].value;
        // record_unwind_protect (un_autoload, Vautoload_queue); Vautoload_queue = Qt;
        self.dynamic.push((queue, self.slots[queue].value, buffer, false));
        self.slots[queue].value = Value::TRUE;
        // record_unwind_save_match_data (): Fmatch_data (Qnil, Qnil, Qnil).
        let match_data = self.save_match_data_for_load()?;
        self.dynamic.push((saved_match, self.slots[saved_match].value, buffer, false));
        self.slots[saved_match].value = match_data;
        let result = self.load_source_named(file, noerror, must_suffix);
        // Restore match data (Fset_match_data (saved, Qt)), then the queue.
        let saved = self.slots[saved_match].value;
        let restored = self.regexp_set_match_data(saved, Value::TRUE);
        if result.is_err() {
            self.un_autoload(queue);
        }
        self.unbind(self.dynamic.len() - mark);
        let path = result?;
        restored.map_err(|_| "set-match-data failed restoring an autoload's match data")?;
        Ok(path)
    }
    fn save_match_data_for_load(&mut self) -> Result<Value> {
        self.regexp_match_data_saved().ok_or_else(|| "match-data failed saving an autoload's match data".into())
    }
    /// `Fload (FILE, NOERROR, Qt, Qnil, MUST-SUFFIX)`, VM2's source loader.
    fn load_source_named(&mut self, file: Value, noerror: bool, must_suffix: bool) -> Result<std::path::PathBuf> {
        let name = self.source_string(file)?.to_owned();
        let Some(path) = self.source_path(&name, false)? else {
            if noerror {
                return Ok(std::path::PathBuf::new());
            }
            let text = self.literal(&Form::String("Cannot open load file".into()))?;
            let reason = self.literal(&Form::String("No such file or directory".into()))?;
            return Err(self.signal_error("file-missing", &[text, reason, file], format!("file missing: {name}")));
        };
        if must_suffix && path.extension().is_none() {
            return Err("load mustsuffix requires a source suffix".into());
        }
        self.load_file_raw(&path)?;
        Ok(crate::host::canonicalize(&path).unwrap_or(path))
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
            self.conses.push((tag, old));
            Value::reference(self.conses.len() - 1, 3)
        } else {
            old
        };
        self.conses.push((Value::reference(slot + 1, 7), old));
        let record = Value::reference(self.conses.len() - 1, 3);
        let tail = self.slots[queue].value;
        let tail = if tail == Value::TRUE { Value::NIL } else { tail };
        self.conses.push((record, tail));
        self.slots[queue].value = Value::reference(self.conses.len() - 1, 3);
    }
    /// fns.c:3751: `provide` while a queue is active records `(0 . features)`.
    pub(super) fn autoload_note_provide(&mut self) {
        let Some(&queue) = self.names.get(QUEUE) else { return };
        if self.slots[queue].value == Value::NIL {
            return;
        }
        let features = self.slot("features");
        self.conses.push((Value::integer(0).unwrap(), self.slots[features].value));
        let record = Value::reference(self.conses.len() - 1, 3);
        let tail = self.slots[queue].value;
        let tail = if tail == Value::TRUE { Value::NIL } else { tail };
        self.conses.push((record, tail));
        self.slots[queue].value = Value::reference(self.conses.len() - 1, 3);
    }
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
