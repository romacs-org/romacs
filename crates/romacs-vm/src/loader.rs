//! UTF-8/Latin-1 source loading. Feature state lives in the Lisp `features`
//! variable, not a shadow Rust set. Unsupported coding/compiled files fail closed.
use super::*;
use crate::brand::{Gc, Mx};
use crate::gc::RootFrame;
use std::path::{Path, PathBuf};
/// What a source load sets for its file, as it was: `Vm::enter_source_regime`.
struct SourceRegime {
    lazy: bool,
    defer: bool,
    interpreting: bool,
    expanded_macros: Vec<(usize, u64)>,
    expanded_symbols: NameSet<String>,
}
romacs_snapshot::snapshot_struct!(SourceRegime { lazy, defer, interpreting, expanded_macros, expanded_symbols });
#[derive(Default)]
pub(super) struct Loader {
    pub(super) propagate: usize,
    active: Vec<PathBuf>,
    /// Every file a step was recorded in, once, in first-step order.
    pub(super) files: Vec<PathBuf>,
    steps: Vec<Step>,
    completed_files: usize,
    completed_forms: usize,
    /// The init's loads, while they are recorded (`world::layer`).
    pub(super) recording: Option<crate::world::layer::Recording>,
    /// Each compiled load's file (an index in `files`) and the artifact
    /// it ran (`compiled_lisp`), in load order.
    compiled: Vec<(usize, PathBuf)>,
    /// How many compiled loads are running their units.
    in_artifact: usize,
    /// What each source load in progress set for its file, saved.
    regimes: Vec<SourceRegime>,
    /// The outer file's `(defvar X)`s, saved by each load in progress.
    file_specials: Vec<NameSet<String>>,
}
romacs_snapshot::snapshot_struct!(Loader { propagate, active, files, steps, completed_files, completed_forms, recording, compiled, in_artifact, regimes, file_specials });
impl Loader {
    /// How many source loads are in progress.
    pub(super) fn active_depth(&self) -> usize {
        self.active.len()
    }
    /// PATH's index in `files`, added at the end when new.
    fn file_index(&mut self, path: &Path) -> usize {
        self.files.iter().rposition(|p| p == path).unwrap_or_else(|| {
            self.files.push(path.to_owned());
            self.files.len() - 1
        })
    }
    /// A form's step, entered: where `step_ok` or `step_failed` finds it.
    fn enter_step(&mut self, file: usize, ordinal: usize, byte: usize, line: usize) -> usize {
        self.steps.push(Step { file, ordinal, byte, line, status: StepStatus::Entered, error: None, source: None });
        self.steps.len() - 1
    }
    fn step_ok(&mut self, step: usize) {
        self.completed_forms += 1;
        self.steps[step].status = StepStatus::Ok;
    }
    fn step_failed(&mut self, step: usize, error: String, source: String) {
        let failed = &mut self.steps[step];
        failed.status = StepStatus::Error;
        failed.error = Some(error);
        failed.source = Some(source);
    }
}
/// One top-level form the loader read, as the load report tells it: the
/// file (`Vm::load_step_file`), the form's place and what became of it.
/// A form that signalled keeps its error and its text; a completed one
/// keeps neither, so a session's record stays small.
#[derive(Clone, Debug)]
pub struct Step {
    file: usize,
    /// 1-based, in its file.
    pub ordinal: usize,
    /// Byte offset of the form in its file.
    pub byte: usize,
    /// 1-based line of the form.
    pub line: usize,
    pub status: StepStatus,
    pub error: Option<String>,
    pub source: Option<String>,
}
romacs_snapshot::snapshot_struct!(Step { file, ordinal, byte, line, status, error, source });
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepStatus {
    /// Read and entered, not yet finished.
    Entered,
    Ok,
    Error,
}
romacs_snapshot::snapshot_enum!(StepStatus { StepStatus::Entered => (), StepStatus::Ok => (), StepStatus::Error => () });
impl StepStatus {
    fn name(self) -> &'static str {
        match self { StepStatus::Entered => "entered", StepStatus::Ok => "ok", StepStatus::Error => "error" }
    }
}
impl Vm {
    pub(super) fn source_string_if(&self, v: Value) -> Option<&str> {
        if v.tag() != 0 {
            return None;
        }
        if Self::image_string(v) {
            return self.string_value(v).ok();
        }
        match self.object_if(v) {
            Some(data::Object::String(s)) => Some(s.as_str()),
            Some(data::Object::Bytes(b)) if b.is_ascii() => std::str::from_utf8(b).ok(),
            _ => None,
        }
    }
    pub(super) fn source_string(&self, v: Value) -> Result<&str> {
        if Self::image_string(v) {
            return self.string_value(v);
        }
        match self.object(v)? {
            data::Object::String(s) => Ok(s.as_str()),
            data::Object::Bytes(b) if b.is_ascii() => std::str::from_utf8(b).map_err(|_| "not-unicode-string".into()),
            data::Object::RawString(_) | data::Object::Bytes(_) => Err("not-unicode-string".into()),
            _ => Err("string required".into()),
        }
    }
    pub(super) fn initialize_loader(&mut self) {
        for name in [
            "features",
            "load-path",
            "load-prefer-newer",
            "dump-mode",
            "command-line-args",
        ] {
            self.declare_special_raw(name, Value::NIL);
        }
    }
    pub fn set_load_path(&mut self, paths: &[PathBuf]) -> Result<()> {
        let values = paths
            .iter()
            .map(|p| self.literal(&Form::String(p.to_string_lossy().into_owned())))
            .collect::<Result<Vec<_>>>()?;
        let value = self.list_value(&values);
        let slot = self.slot("load-path");
        self.slots[slot].value = value;
        Ok(())
    }
    /// The loader's record as JSON: the counters and every step, with a
    /// failed step's `error` and `source`.
    pub fn load_report(&self) -> serde_json::Value {
        let steps: Vec<serde_json::Value> = self.loader.steps.iter().map(|step| {
            let mut json = serde_json::json!({"file": self.load_step_file(step), "ordinal": step.ordinal, "byte": step.byte,
                                              "line": step.line, "status": step.status.name()});
            if let Some(error) = &step.error { json["error"] = error.clone().into(); }
            if let Some(source) = &step.source { json["source"] = source.clone().into(); }
            json
        }).collect();
        let compiled: Vec<serde_json::Value> = self.loader.compiled.iter()
            .map(|(file, artifact)| serde_json::json!({"file": self.loader.files[*file], "artifact": artifact}))
            .collect();
        serde_json::json!({"completed_files":self.loader.completed_files,"completed_forms":self.loader.completed_forms,"steps":steps,
                           "compiled":compiled})
    }
    /// Every top-level form the loader has read this session, in order.
    pub fn load_steps(&self) -> &[Step] {
        &self.loader.steps
    }
    /// The file STEP's form is in.
    pub fn load_step_file(&self, step: &Step) -> &Path {
        &self.loader.files[step.file]
    }
    /// fns.c Fprovide up to its load hooks: record FEATURE (always noting it
    /// for the autoload queue) and SUBFEATURES.
    pub(crate) fn provide_feature(&mut self, feature: Value, subfeature: Value) -> std::result::Result<(), prim::Fault> {
        self.symbol_plist(feature).map_err(|_| prim::Fault::Wrong("symbolp", feature))?;
        if subfeature != Value::NIL && subfeature.tag() != 3 {
            return Err(prim::Fault::Wrong("listp", subfeature));
        }
        let present = self.feature_present(feature).map_err(prim::Fault::Error)?;
        self.autoload_note_provide();
        if !present {
            let slot = self.slot("features");
            let old = self.slots[slot].value;
            self.slots[slot].value = self.alloc_cons(feature, old);
        }
        if subfeature != Value::NIL {
            let key = self.intern("subfeatures");
            self.put_symbol_property(feature, key, subfeature).map_err(prim::Fault::Error)?;
        }
        let provide = self.intern("provide");
        let entry = self.alloc_cons(provide, feature);
        self.attach_load_history(entry).map_err(prim::Fault::Error)
    }
    fn feature_present(&self, feature: Value) -> Result<bool> {
        if !matches!(feature.tag(), 7) && feature != Value::NIL && feature != Value::TRUE {
            return Err("symbol required".into());
        }
        // Fmemq: stop at the first match, without copying the list; the
        // copy, bounds-checked per element, made each featurep linear in
        // `features' at ~80 instructions an element.
        let (limit, mut tail) = (self.list_limit(), self.slots[self.names["features"]].value);
        for _ in 0..=limit {
            if tail == Value::NIL {
                return Ok(false);
            }
            let (head, next) = self.cons_pair(tail)?;
            if head == feature {
                return Ok(true);
            }
            tail = next;
        }
        Err("circular list".into())
    }
    // Fmember's equal comparison and FOR_EACH_TAIL semantics. This read-only
    // helper is also used to recover a typed fault at the legacy call boundary.
    pub(super) fn feature_subfeature_matches(&self, feature: Value, query: Value)
        -> std::result::Result<bool, prim::Fault>
    {
        use prim::Fault;
        let Some(&slot) = self.names.get("subfeatures") else { return Ok(false); };
        let key = Value::reference(slot + 1, 7);
        let list = self.symbol_property(feature, key).map_err(|_| Fault::Wrong("symbolp", feature))?;
        let (mut tail, mut tortoise, mut steps) = (list, list, 0u64);
        while let Ok((item, rest)) = self.cons_pair(tail) {
            if self.equal(query, item).map_err(|e| match e {
                hash::HashFault::Wrong(p, v) => Fault::Wrong(p, v),
                hash::HashFault::Error("circular-list", Some(v)) => Fault::Circular(v),
                hash::HashFault::Error(m, _) => Fault::Error(m.into()),
                _ => Fault::Error("equal failed".into()),
            })? { return Ok(true); }
            tail = rest;
            if tail == tortoise { return Err(Fault::Circular(tail)); }
            steps += 1;
            if steps & 4095 == 0 && steps.is_power_of_two() { tortoise = tail; }
        }
        if tail != Value::NIL { return Err(Fault::Wrong("listp", list)); }
        Ok(false)
    }
    /// NAME's source file on `load-path`, and whether it was found by adding `.el`: a
    /// load GNU would satisfy with an `.elc` first when there is one
    /// (lread.c openp over `load-suffixes`).
    pub(super) fn source_path(&mut self, name: &str, nosuffix: bool) -> Result<Option<(PathBuf, bool)>> {
        // A compile process records each resolution, not its probes: a
        // load of its artifact resolves the name again on the `load-path'
        // then in force (`compiled_lisp').
        if crate::host::mode() != crate::host::Mode::Record {
            return self.resolve_source(name, nosuffix);
        }
        let found = crate::host::unrecorded(|| self.resolve_source(name, nosuffix))?;
        {
            crate::host::record("resolved", serde_json::json!({"name": name, "nosuffix": nosuffix,
                "path": found.as_ref().map(|(p, _)| p.to_string_lossy())}));
        }
        Ok(found)
    }
    fn resolve_source(&mut self, name: &str, nosuffix: bool) -> Result<Option<(PathBuf, bool)>> {
        // lread.c openp: a name under `~' is absolute, expanded through
        // the user's home, never looked up along `load-path'.
        let expanded = self.expand_home_name(name)?;
        let name = expanded.as_deref().unwrap_or(name);
        let path = Path::new(name);
        let roots = if path.is_absolute() {
            vec![PathBuf::new()]
        } else {
            let slot = self.names["load-path"];
            self.list_values(self.slots[slot].value)?
                .into_iter()
                .map(|v| {
                    if v == Value::NIL {
                        Ok(PathBuf::from("."))
                    } else {
                        Ok(PathBuf::from(self.source_string(v)?))
                    }
                })
                .collect::<Result<Vec<_>>>()?
        };
        for root in roots {
            let base = root.join(path);
            let candidates = if nosuffix {
                vec![base]
            } else {
                vec![PathBuf::from(format!("{}.el", base.display())), base]
            };
            for (i, p) in candidates.into_iter().enumerate() {
                if crate::host::probe(crate::host::Probe::IsFile, &p) {
                    return Ok(Some((p, !nosuffix && i == 0)));
                }
            }
        }
        Ok(None)
    }
    /// `featurep`, and the bootstrap's `#:vm2-provide`: data only, so no Lisp
    /// runs (the `provide` Lisp calls runs `after-load-alist` as a continuation,
    /// prims/provide.rs).
    pub(super) fn feature_primitive(&mut self, p: Primitive, args: &[Value]) -> Result<Value> {
        if args.is_empty() || args.len() > 2 {
            return Err("wrong primitive arity".into());
        }
        // Match GNU's CHECK_SYMBOL then CHECK_LIST before inspecting features.
        self.symbol_plist(args[0])?;
        let subfeature = args.get(1).copied().unwrap_or(Value::NIL);
        if p == Primitive::Provide && subfeature != Value::NIL && subfeature.tag() != 3 {
            return Err("provide subfeatures list required".into());
        }
        let present = self.feature_present(args[0])?;
        if p == Primitive::Featurep {
            let found = present && (subfeature == Value::NIL
                || self.feature_subfeature_matches(args[0], subfeature)
                    .map_err(|_| "featurep subfeature fault".to_owned())?);
            return Ok(if found { Value::TRUE } else { Value::NIL });
        }
        // fns.c:3751: recorded whether or not FEATURE is present.
        self.autoload_note_provide();
        if !present {
            let slot = self.slot("features");
            let old = self.slots[slot].value;
            self.slots[slot].value = self.alloc_cons(args[0], old);
        }
        if subfeature != Value::NIL {
            let key = self.intern("subfeatures");
            self.put_symbol_property(args[0], key, subfeature)?;
        }
        let provide = self.intern("provide");
        let entry = self.alloc_cons(provide, args[0]);
        self.attach_load_history(entry)?;
        Ok(args[0])
    }
    pub(super) fn loader_primitive(&mut self, p: Primitive, args: &[Value]) -> Result<Value> {
        match p {
            Primitive::Featurep | Primitive::Provide => self.feature_primitive(p, args),
            Primitive::Load | Primitive::Require | Primitive::AutoloadDoLoad => {
                let frame = self.root_frame(args);
                let out = self.root_frame(&[Value::NIL]);
                let outcome = threaded::toplevel(self, (p, &frame, &out), |mut m, gc, (p, args, out)| load_primitive(&mut m, gc, p, args, (out, 0)));
                let [value] = self.release_frame(out);
                self.drop_frame(frame);
                outcome.map(|()| value)
            }
            Primitive::MakeInterpretedClosure => {
                self.interpreted_closure_values(Primitive::MakeInterpretedClosure, args.to_vec())
            }
            Primitive::Eval => self.eval_primitive(args),
            Primitive::Macroexpand => self.macroexpand_primitive(args),
            Primitive::FormatModeLine => self.format_mode_line(args),
            _ => unreachable!(),
        }
    }
    /// `load`'s target: the file and whether a suffix was added to its name, or
    /// None when NOERROR found none.
    fn load_target(&mut self, args: &RootFrame) -> Result<Option<(PathBuf, bool)>> {
        let args = self.rooted_values(args).to_vec();
        if args.is_empty() || args.len() > 5 {
            return Err("wrong primitive arity".into());
        }
        let name = self.source_string(args[0])?.to_owned();
        let noerror = args.get(1).is_some_and(|v| *v != Value::NIL);
        let nosuffix = args.get(3).is_some_and(|v| *v != Value::NIL);
        let Some((path, suffixed)) = self.source_path(&name, nosuffix)? else {
            return if noerror { Ok(None) } else { Err(format!("file missing: {name}")) };
        };
        if args.get(4).is_some_and(|v| *v != Value::NIL) && path.extension().is_none() {
            return Err("load mustsuffix requires a source suffix".into());
        }
        Ok(Some((path, suffixed)))
    }
    /// `require` up to its load: the file, rooted, and NOERROR and MUST-SUFFIX
    /// (fns.c Frequire: `load_with_autoload_queue (FILE, NOERROR, Qt, Qnil,
    /// NILP (FILENAME))`, so a failed load is undone), or None with the feature
    /// already in OUT.
    fn require_target(&mut self, args: &RootFrame, out: Out<'_>) -> Result<Option<(RootFrame, bool, bool)>> {
        let args = self.rooted_values(args).to_vec();
        if args.is_empty() || args.len() > 3 {
            return Err("wrong primitive arity".into());
        }
        let feature = args[0];
        let noerror = args.get(2).is_some_and(|v| *v != Value::NIL);
        if self.loader.recording.is_some() && let Some(name) = self.symbol_name_of(feature) {
            let file = args.get(1).filter(|v| **v != Value::NIL).and_then(|v| self.source_string_if(*v)).map(str::to_owned);
            self.note_init_load(crate::world::layer::Kind::Require, name, file, noerror);
        }
        if self.symbol_plist(feature).is_ok() { self.attach_require(feature)?; }
        if self.feature_present(feature)? {
            self.set_rooted(out.0, out.1, feature);
            return Ok(None);
        }
        let (file, must_suffix) = match args.get(1) {
            Some(v) if *v != Value::NIL => (*v, false),
            _ => {
                let name = self.display(feature)?;
                (self.literal(&Form::String(name))?, true)
            }
        };
        Ok(Some((self.root_frame(&[file]), noerror, must_suffix)))
    }
    /// A top-level form of PATH at LINE has finished as its STEP. A session's
    /// own file's forms are its steps; a library's are inside the step that
    /// loaded it.
    fn form_done(&mut self, (step, path, line): (usize, &Path, usize)) {
        self.loader.step_ok(step);
        if self.loader.active_depth() == 1 {
            crate::flight::step(|| format!("form {}:{line}", path.display()), || self.flight_step_fingerprint());
        }
    }
    /// `require` after its load of PATH, the empty path when none was found.
    fn require_finished(&mut self, args: &RootFrame, out: Out<'_>, path: &Path) -> Result<()> {
        let feature = self.rooted(args, 0);
        if path.as_os_str().is_empty() {
            self.set_rooted(out.0, out.1, Value::NIL);
            return Ok(());
        }
        if !self.feature_present(feature)? {
            return Err(format!("feature was not provided: {} by {}", self.display(feature)?, path.display()));
        }
        self.set_rooted(out.0, out.1, feature);
        Ok(())
    }
    /// Enter macroexp.el internal-macroexpand-for-load's `condition-case` clause
    /// `(debug error)`, for what expands a top-level form: the debugger is
    /// considered as GNU's would, and no handler around the load sees an error
    /// the expansion signals, which `eager_expansion_failure` wraps. The mark
    /// for `Self::leave_load_expansion`.
    fn enter_load_expansion(&mut self) -> usize {
        let mark = self.signal_handlers.len();
        let (debug, error) = (self.intern("debug"), self.intern("error"));
        let clause = self.list_value(&[debug, error]);
        self.signal_handlers.push(threaded::SignalHandler::Condition(clause));
        mark
    }
    fn leave_load_expansion(&mut self, mark: usize) {
        self.signal_handlers.truncate(mark);
    }
    /// A `(defvar NAME)` a top-level form has just declared:
    /// `macroexpand--all-toplevel` records it in the file's `macroexp--dynvars`
    /// (macroexp.el), which macros such as `eval-and-compile` hand to `eval` as its
    /// lexical environment.
    /// The variable's slot and its new value, rooted, for the watchers' write;
    /// None when this file had declared it.
    fn declared_file_special(&mut self, name: &str) -> Result<Option<(usize, RootFrame)>> {
        if !self.file_specials.insert(name.to_owned()) {
            return Ok(None);
        }
        let symbol = self.intern(name);
        let dynvars = self.slot("macroexp--dynvars");
        let declared = self.variable_value(dynvars)?;
        let declared = self.keymap_cons(symbol, declared);
        Ok(Some((dynvars, self.root_frame(&[declared]))))
    }
    /// A form's EVALUATED outcome when a world's layer is building: a definition
    /// cut short by a refused effect defines nil here and is kept whole for
    /// restore.
    fn layer_form_finished(&mut self, evaluated: Result<()>, form: &Form, out: Out<'_>) -> Result<()> {
        if crate::host::mode() != crate::host::Mode::Build {
            return evaluated;
        }
        let mut evaluated = evaluated;
        if let (Err(_), Some(name)) = (&evaluated, crate::host::layer_form_cut()) {
            self.propagated_escape = None;
            let slot = self.slot(&name);
            if self.slots[slot].value == Value::UNBOUND {
                self.declare_special_raw(&name, Value::NIL);
            }
            self.set_rooted(out.0, out.1, Value::NIL);
            evaluated = Ok(());
        }
        if evaluated.is_ok() {
            self.world_form_done(form);
        }
        evaluated
    }
    /// macroexp.el internal-macroexpand-for-load: an error that expanding a
    /// top-level form signals reaches the loader's caller as (error "Eager
    /// macro-expansion failure: ERR"), ERR printed as `%S' prints it. A quit
    /// or another signal that is no error passes as it is, as does a throw
    /// to a catch outside the load.
    fn eager_expansion_failure(&mut self, error: String) -> String {
        let (condition, object) = match self.propagated_escape {
            Some(threaded::Escape::Signal(condition, data)) => {
                let cons = self.alloc_cons(condition, data);
                (condition, cons)
            }
            Some(threaded::Escape::Error { condition, object, .. }) => (condition, object),
            _ => return error,
        };
        let error_symbol = self.intern("error");
        if !self.condition_matches(condition, error_symbol) {
            return error;
        }
        let Ok(printed) = self.print_render(object, true, Value::NIL, false) else { return error };
        let message = format!("Eager macro-expansion failure: {printed}");
        let Ok(text) = self.literal(&Form::String(message.clone())) else { return error };
        let data = self.list_value(&[text]);
        self.propagated_escape = Some(threaded::Escape::Signal(error_symbol, data));
        message
    }
    pub(super) fn source_load_active(&self) -> bool {
        !self.loader.active.is_empty()
    }

    /// The embedder's load of PATH, read as its name says: the last form's value.
    pub(crate) fn load_file_raw(&mut self, path: &Path) -> Result<Value> {
        let out = self.root_frame(&[Value::NIL]);
        let outcome = threaded::toplevel(self, (path, &out), |mut m, gc, (path, out)| load_file(&mut m, gc, path, false, false, (out, 0)));
        let [value] = self.release_frame(out);
        outcome.map(|()| value)
    }
    /// The name and resolved path of a load of PATH. GNU names the file as openp
    /// found it, expanded but not resolved through symlinks (`hist_file_name`,
    /// lread.c:1452): that name is `load-file-name`, its `load-history` entry and
    /// the after-load argument. The canonical path detects recursion and reads.
    fn admitted_load(&mut self, path: &Path) -> Result<(String, PathBuf)> {
        if path.extension().is_some_and(|s| s == "elc") {
            return Err("compiled .elc input is not supported".into());
        }
        let named = expanded_path(path)?;
        let path = crate::host::canonicalize(&named)
            .map_err(|e| format!("file missing: {}: {e}", path.display()))?;
        if self.loader.active.len() >= 128 {
            return Err(format!("recursive source load: {}", path.display()));
        }
        // lread.c Fload: a file may be in progress three times; the fourth
        // signals (error "Recursive load" FILE . LOADS-IN-PROGRESS).
        if self.loader.active.iter().filter(|active| **active == path).count() > 3 {
            let text = |vm: &mut Self, s: String| vm.literal(&Form::String(s));
            let mut data = vec![text(self, "Recursive load".into())?, text(self, path.to_string_lossy().into_owned())?];
            for active in self.loader.active.clone().iter().rev() {
                data.push(text(self, active.to_string_lossy().into_owned())?);
            }
            let condition = self.intern("error");
            let data = self.list_value(&data);
            self.propagated_escape = Some(threaded::Escape::Signal(condition, data));
            return Err(format!("Recursive load: {}", path.display()));
        }
        Ok((named, path))
    }
    /// The compiled form of PATH to load, recorded in a compile process.
    fn compiled_for_load(&mut self, path: &Path) -> Result<Option<crate::compiled_lisp::Compiled>> {
        let compiled = self.compiled_load(path)?;
        if let Some(compiled) = &compiled && crate::host::mode() == crate::host::Mode::Record {
            self.note_loaded(path);
            compiled.inherit_inputs();
        }
        Ok(compiled)
    }
    /// PATH's text as GNU internal multibyte bytes.
    fn source_for_load(&mut self, path: &Path) -> Result<Vec<u8>> {
        let bytes = crate::host::read_file(path).map_err(|e| format!("file error: {}: {e}", path.display()))?;
        if crate::host::mode() == crate::host::Mode::Record {
            self.note_loaded(path);
        }
        match decode_source(&bytes).map_err(|e| format!("{}: {e}", path.display()))? {
            Some(source) => Ok(source),
            None => self.detected_source(&bytes).map_err(|e| format!("{}: {e}", path.display())),
        }
    }
    /// The values a load binds, for `bind_named`: FILE, and what later fills in.
    fn load_values(&mut self, file: &str) -> Result<RootFrame> {
        let file = self.literal(&Form::String(file.to_owned()))?;
        Ok(self.root_frame(&[Value::NIL, Value::TRUE, file, Value::NIL, Value::NIL, Value::NIL, Value::NIL]))
    }
    /// `loadhist_initialize` (lread.c:1031): `load` binds `current-load-list` to
    /// (FILE), so `LOADHIST_ATTACH` conses this file's definitions before its name
    /// and `macroexp-file-name` finds the file being loaded. `macroexp--dynvars`
    /// binds to its own value.
    fn load_history_values(&mut self, values: &RootFrame) -> Result<()> {
        let dynvars = self.slot("macroexp--dynvars");
        let dynvars = self.variable_value(dynvars)?;
        self.set_rooted(values, LOAD_DYNVARS, dynvars);
        let file = self.rooted(values, LOAD_FILE);
        let history = self.list_value(&[file]);
        self.set_rooted(values, LOAD_HISTORY, history);
        Ok(())
    }
    /// Enter a source load of PATH: among the loads in progress, in its regime.
    /// What it replaces goes back with `Self::leave_source_regime`.
    fn enter_source_regime(&mut self, path: &Path, explicit: bool) {
        self.loader.active.push(path.to_owned());
        // GNU readevalloop (src/lread.c:2183-2188) decides ONCE per load:
        // forms are eagerly macroexpanded only if `internal-macroexpand-for-
        // load' (macroexp.el) is already fbound. Otherwise each form goes
        // through eval_sub, which expands a form only when it is reached and
        // leaves closure bodies unexpanded (R215). VM2 compiles eagerly in
        // the first regime and lazily in the second.
        let expander = self.slot("internal-macroexpand-for-load");
        let lazy = matches!(self.slots[expander].function, f if f == Value::UNBOUND || f == Value::NIL);
        let saved_lazy = std::mem::replace(&mut self.lazy_regime, lazy);
        let defer = std::mem::replace(&mut self.defer_expansion_errors, lazy);
        // GNU's readevalloop interprets a file it reads as source: its
        // `let's decide as they bind (VM2-LATE-SPECIAL1).
        let interpreting = self.gnu_interprets(path, explicit);
        let interpreting = std::mem::replace(&mut self.interpreting_source, interpreting);
        // readevalloop binds `internal-interpreter-environment' once for the
        // file, so a top-level `(defvar NAME)' reaches every LATER form of
        // this file and no other file (VM2-FILESPECIAL1 in DIVERGENCES.md).
        self.enter_file_specials();
        // A macro can load a file while an outer compilation is suspended.
        // Child compilations must not erase its nonconstant-macro guards.
        let expanded_macros = std::mem::take(&mut self.expanded_macros);
        let expanded_symbols = std::mem::take(&mut self.expanded_symbols);
        self.loader.regimes.push(SourceRegime { lazy: saved_lazy, defer, interpreting, expanded_macros, expanded_symbols });
    }
    /// A load's own set of `(defvar X)`s, empty; `Self::leave_file_specials` puts
    /// the outer file's back.
    fn enter_file_specials(&mut self) {
        let outer = std::mem::take(&mut self.file_specials);
        self.loader.file_specials.push(outer);
    }
    fn leave_file_specials(&mut self) {
        self.file_specials = self.loader.file_specials.pop().expect("a load's saved specials");
    }
    fn leave_source_regime(&mut self) {
        let saved = self.loader.regimes.pop().expect("a source load's regime");
        self.lazy_regime = saved.lazy;
        self.defer_expansion_errors = saved.defer;
        self.interpreting_source = saved.interpreting;
        self.leave_file_specials();
        self.expanded_macros = saved.expanded_macros;
        self.expanded_symbols = saved.expanded_symbols;
    }
    /// In a compile process, record that it loaded PATH, with its digest
    /// (`compiled_lisp`): a change to it changes what the compile saw.
    fn note_loaded(&self, path: &Path) {
        if let Some(store) = crate::compiled_lisp::store_dir()
            && let Ok(digest) = crate::compiled_lisp::store::source_digest(store, path)
        {
            crate::host::record("loaded", serde_json::json!({"path": path.to_string_lossy(),
                "sha256": crate::compiled_lisp::store::hex(&digest)}));
        }
    }
    /// lread.c build_load_history for an entire file: the file's entry,
    /// `current-load-list' in definition order, replaces any older entry
    /// for the same name at the front of `load-history'.
    /// LOADHIST_ATTACH (lisp.h): cons ENTRY onto `current-load-list', a C
    /// write that tells no watcher, so it runs no Lisp.
    pub(crate) fn attach_load_history(&mut self, entry: Value) -> Result<()> {
        let slot = self.slot("current-load-list");
        let old = self.variable_value(slot)?;
        let history = self.alloc_cons(entry, old);
        self.assign_variable(slot, history)
    }
    /// fns.c Frequire: the file's dependency is recorded even when FEATURE
    /// is present, once per file, and only while a file loads or reads.  A
    /// `require' run by a macro's expansion, as `eval-when-compile' runs it,
    /// is no dependency of the file: GNU's `.elc' does not make that call.
    fn attach_require(&mut self, feature: Value) -> Result<()> {
        if !self.expanding.is_empty() { return Ok(()); }
        let require = self.intern("require");
        let (progress, history) = (self.slot("load-in-progress"), self.slot("current-load-list"));
        let mut from_file = self.variable_value(progress)? != Value::NIL;
        let mut tail = self.variable_value(history)?;
        for _ in 0..self.list_limit() {
            let Ok((entry, rest)) = self.cons_pair(tail) else { break };
            if matches!(self.cons_pair(entry), Ok(pair) if pair == (require, feature)) { return Ok(()); }
            from_file |= rest == Value::NIL && self.source_string(entry).is_ok();
            tail = rest;
        }
        if from_file {
            let entry = self.alloc_cons(require, feature);
            self.attach_load_history(entry)?;
        }
        Ok(())
    }
    fn build_load_history(&mut self, history_slot: usize) -> Result<()> {
        let current = self.variable_value(history_slot)?;
        let mut entry = self.list_values(current)?;
        entry.reverse();
        let Some(&file) = entry.first() else { return Ok(()) };
        let name = self.source_string(file).ok().map(str::to_owned);
        let slot = self.slot("load-history");
        let mut kept = vec![];
        let history = self.variable_value(slot)?;
        for old in self.list_values(history)? {
            let same = matches!(self.cons_pair(old), Ok((head, _)) if name.is_some() && self.source_string(head).ok() == name.as_deref());
            if !same { kept.push(old); }
        }
        let entry = self.list_value(&entry);
        kept.insert(0, entry);
        let history = self.list_value(&kept);
        self.assign_variable(slot, history)?;
        // lread.c:2112 stores t without notifying; the load's unbinding
        // restores the outer list.
        let fwd = self.native_forwarding(history_slot).expect("current-load-list is forwarded");
        self.set_forwarded(history_slot, fwd, Value::TRUE, forwarded::SetFlag::Set).map_err(forwarded::fault_text)
    }
}
/// Where a value goes: a root frame's slot.
pub(crate) type Out<'f> = (&'f RootFrame, usize);
/// A request's value stored at OUT, or its failure.
fn stored(vm: &mut Vm, out: Out<'_>, value: Result<Value>) -> Result<()> {
    value.map(|value| vm.set_rooted(out.0, out.1, value))
}
// A load's values (`Vm::load_values`), each bound from its slot.
const LOAD_NIL: usize = 0;
const LOAD_T: usize = 1;
const LOAD_FILE: usize = 2;
const LOAD_SHORTHANDS: usize = 3;
const LOAD_DYNVARS: usize = 4;
const LOAD_HISTORY: usize = 5;
/// What the after-load call returns: its value goes nowhere.
const LOAD_SCRATCH: usize = 6;

/// Whether `load_primitive` runs P.
pub(crate) fn loads(p: Primitive) -> bool {
    matches!(p, Primitive::Load | Primitive::Require | Primitive::AutoloadDoLoad)
}
/// `load`, `require` or `autoload-do-load` on ARGS, rooted; the value goes to OUT,
/// which holds nil.
/// A load's orchestration from here down holds the permission and no Value
/// (`crate::brand`): each request's value goes straight to a leaf.
pub(crate) fn load_primitive<'b>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>, p: Primitive, args: &RootFrame, out: Out<'_>) -> Result<()> {
    match p {
        Primitive::Load => load(m, gc, args, out),
        Primitive::Require => require(m, gc, args, out),
        Primitive::AutoloadDoLoad => autoload_do_load_primitive(m, gc, args, out),
        _ => unreachable!("a primitive that loads"),
    }
}
fn load<'b>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>, args: &RootFrame, out: Out<'_>) -> Result<()> {
    let Some((path, suffixed)) = m.leaf(args, Vm::load_target)? else { return Ok(()) };
    load_discarding(m, gc, &path, suffixed)?;
    m.leaf(out, |vm, out| vm.set_rooted(out.0, out.1, Value::TRUE));
    Ok(())
}
fn require<'b>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>, args: &RootFrame, out: Out<'_>) -> Result<()> {
    let Some((file, noerror, must_suffix)) = m.leaf((args, out), |vm, (args, out)| vm.require_target(args, out))? else { return Ok(()) };
    let path = crate::autoload::queued_load(m, gc, (&file, 0), noerror, must_suffix);
    m.leaf(file, Vm::drop_frame);
    let path = path?;
    m.leaf((args, out, path.as_path()), |vm, (args, out, path)| vm.require_finished(args, out, path))
}
fn autoload_do_load_primitive<'b>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>, args: &RootFrame, out: Out<'_>) -> Result<()> {
    let frame = m.leaf(args, |vm, args| {
        let args = vm.rooted_values(args).to_vec();
        if args.is_empty() || args.len() > 3 {
            return Err("wrong primitive arity".to_owned());
        }
        let arg = |i: usize| args.get(i).copied().unwrap_or(Value::NIL);
        Ok(vm.autoload_args(arg(0), arg(1), arg(2)))
    })?;
    let outcome = crate::autoload::do_load(m, gc, &frame);
    m.leaf((frame, outcome, out), |vm, (frame, outcome, out)| {
        let value = vm.autoload_result(frame, outcome);
        stored(vm, out, value)
    })
}
/// A load of PATH for its effects, `suffixed` when its name was found by adding
/// `.el`: the load is compiled when the file has a compiled form (`compiled_lisp`),
/// else the `.el` was named.
pub(crate) fn load_discarding<'b>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>, path: &Path, suffixed: bool) -> Result<()> {
    let out = m.leaf((), |vm, ()| vm.root_frame(&[Value::NIL]));
    let result = load_file(m, gc, path, suffixed, !suffixed, (&out, 0));
    m.leaf(out, Vm::drop_frame);
    result
}
/// Load PATH, the last form's value going to OUT. COMPILED when the file's
/// compiled form may run; EXPLICIT when a Lisp `load` named its `.el`
/// (`Vm::gnu_interprets`).
fn load_file<'b>(m: &mut Mx<'_, 'b, Vm>, mut gc: Gc<'_, 'b>, path: &Path, compiled: bool, explicit: bool, out: Out<'_>) -> Result<()> {
    let (named, path) = m.leaf(path, Vm::admitted_load)?;
    let artifact = if compiled { m.leaf(path.as_path(), Vm::compiled_for_load)? } else { None };
    let source = if artifact.is_some() { vec![] } else { m.leaf(path.as_path(), Vm::source_for_load)? };
    let values = m.leaf(named.as_str(), Vm::load_values)?;
    let result = match &artifact {
        Some(artifact) => load_compiled(m, gc.by_ref(), &named, &path, artifact, &values, out),
        None => load_source(m, gc.by_ref(), (&named, &path), (&source, explicit), &values, out),
    };
    m.leaf(values, Vm::drop_frame);
    result
}
/// Each public variable binds as a `let` does, in pinned GNU's order: Fload's
/// `lexical-binding` (lread.c:1323), bound t only in a source load inside a
/// compiled one, which binds it nil: VM2 reads every file lexically
/// (VM2-LEXBIND1), elsewhere under its default t; then its
/// `lread--unescaped-character-literals`, then `load-with-code-conversion`'s
/// `load-in-progress` (held over the after-load forms) and file names, then
/// readevalloop's. GNU's bindings of the reading buffer (`standard-input`,
/// `eval-buffer-list`) have no counterpart: VM2 reads the file's bytes.
fn load_source<'b>(m: &mut Mx<'_, 'b, Vm>, mut gc: Gc<'_, 'b>, (named, path): (&str, &Path), (source, explicit): (&[u8], bool), values: &RootFrame, out: Out<'_>) -> Result<()> {
    file_shorthands(m, gc.by_ref(), source, (values, LOAD_SHORTHANDS))?;
    let (outer, history_slot) = m.leaf((), |vm, ()| (vm.dynamic.len(), vm.slot("current-load-list")));
    let inner = match bind_source_load(m, gc.by_ref(), values) {
        Ok(inner) => inner,
        Err(error) => {
            crate::threaded::unwind_to(m, gc, outer);
            return Err(error);
        }
    };
    m.leaf((path, explicit), |vm, (path, explicit)| vm.enter_source_regime(path, explicit));
    let evaluated = eval_file_forms(m, gc.by_ref(), path, source, history_slot, out);
    m.leaf((), |vm, ()| vm.leave_source_regime());
    let result = leave_bindings(m, gc.by_ref(), inner, evaluated);
    // lread.c:1538: the eval-after-load forms for this file. The file stays among
    // the loads in progress until they finish, as GNU's unwind keeps it.
    let result = result.and_then(|()| after_load(m, gc.by_ref(), named, (values, LOAD_SCRATCH)));
    let result = leave_bindings(m, gc.by_ref(), outer, result);
    m.leaf((), |vm, ()| { vm.loader.active.pop(); });
    result
}
fn bind_source_load<'b>(m: &mut Mx<'_, 'b, Vm>, mut gc: Gc<'_, 'b>, values: &RootFrame) -> Result<usize> {
    if m.leaf((), |vm, ()| vm.loader.in_artifact > 0) {
        bind_named(m, gc.by_ref(), values, &[("lexical-binding", LOAD_T)])?;
    }
    bind_named(m, gc.by_ref(), values, &[("lread--unescaped-character-literals", LOAD_NIL), ("load-in-progress", LOAD_T)])?;
    let inner = m.leaf((), |vm, ()| vm.dynamic.len());
    m.leaf(values, Vm::load_history_values)?;
    bind_named(m, gc, values, &[("inhibit-file-name-operation", LOAD_NIL), ("load-file-name", LOAD_FILE), ("load-true-file-name", LOAD_FILE),
        ("read-symbol-shorthands", LOAD_SHORTHANDS), ("macroexp--dynvars", LOAD_DYNVARS), ("current-load-list", LOAD_HISTORY)])?;
    Ok(inner)
}
/// Read and evaluate SOURCE's forms, the last one's value going to OUT.
fn eval_file_forms<'b>(m: &mut Mx<'_, 'b, Vm>, mut gc: Gc<'_, 'b>, path: &Path, source: &[u8], history_slot: usize, out: Out<'_>) -> Result<()> {
    let step_file = m.leaf(path, |vm, path| vm.loader.file_index(path));
    let mut at = 0;
    let mut ordinal = 0;
    // The line of byte `counted`, kept as reading advances: counting from the
    // start for each form made a file's load quadratic.
    let (mut line, mut counted) = (1, 0);
    loop {
        // readevalloop's maybe_quit between forms: a form that reaches no
        // safepoint of its own still lets C-g end the load, the forms before it
        // kept.
        m.leaf((), |vm, ()| vm.poll_quit())?;
        let read = crate::threaded::read_next(m, gc.by_ref(), &source[at..], (), |vm, (), read| {
            read.map(|read| read.map(|(value, begin, used)| (vm.source_form(value, 0), begin, used)))
        });
        let Some((form, begin, used)) = read.map_err(|e| {
            format!(
                "{}: line {} form {}: reader: {e}",
                path.display(),
                source[..at].iter().filter(|b| **b == b'\n').count() + 1,
                ordinal + 1
            )
        })?
        else {
            break;
        };
        ordinal += 1;
        let form_source = String::from_utf8_lossy(&source[at + begin..at + used]);
        let form = form?;
        line += source[counted..at + begin].iter().filter(|b| **b == b'\n').count();
        counted = at + begin;
        let step = m.leaf((step_file, ordinal, at + begin, line), |vm, (file, ordinal, byte, line)| vm.loader.enter_step(file, ordinal, byte, line));
        let site = crate::host::enter_site(|| crate::host::Site {
            file: path.to_string_lossy().into_owned(), line,
            head: form_source.trim_start().lines().next().unwrap_or("").chars().take(80).collect(),
        });
        let evaluated = eval_toplevel(m, gc.by_ref(), &form, 0, out);
        let evaluated = m.leaf((evaluated, &form, out), |vm, (evaluated, form, out)| vm.layer_form_finished(evaluated, form, out));
        drop(site);
        match evaluated {
            Ok(()) => m.leaf((step, path, line), Vm::form_done),
            Err(e) => {
                m.leaf((step, e.clone(), form_source.to_string()), |vm, (step, error, source)| vm.loader.step_failed(step, error, source));
                return Err(format!(
                    "{}: line {} form {}: {}: {e}",
                    path.display(),
                    source[..at + begin].iter().filter(|b| **b == b'\n').count() + 1,
                    ordinal,
                    form_source.trim().chars().take(180).collect::<String>()
                ));
            }
        }
        at += used;
    }
    m.leaf(history_slot, |vm, history_slot| {
        vm.loader.completed_files += 1;
        vm.build_load_history(history_slot)
    })
}
/// GNU's load of a compiled file (lread.c Fload with `compiled`), running
/// COMPILED's units in order. NAMED is the file's name as openp found it, PATH
/// its resolved one. Its bindings are Fload's, in its order: the loads in
/// progress, `lexical-binding` nil (the variable; each unit keeps the dialect it
/// was compiled in), the unescaped-literal list, the file names,
/// `load-in-progress`, then readevalloop's `current-load-list` and
/// `macroexp--dynvars`. All of them unwind before the file's after-load forms run
/// (lread.c:1536), where a source load keeps them (`load-with-code-conversion`).
fn load_compiled<'b>(m: &mut Mx<'_, 'b, Vm>, mut gc: Gc<'_, 'b>, named: &str, path: &Path, compiled: &crate::compiled_lisp::Compiled, values: &RootFrame, out: Out<'_>) -> Result<()> {
    let (outer, history_slot) = m.leaf(path, |vm, path| {
        vm.loader.active.push(path.to_owned());
        (vm.dynamic.len(), vm.slot("current-load-list"))
    });
    if let Err(error) = bind_compiled_load(m, gc.by_ref(), values) {
        crate::threaded::unwind_to(m, gc, outer);
        m.leaf((), |vm, ()| { vm.loader.active.pop(); });
        return Err(error);
    }
    // An `.elc' carries no `(defvar X)' for readevalloop to see.
    m.leaf((path, compiled), |vm, (path, compiled)| {
        let step_file = vm.loader.file_index(path);
        vm.loader.compiled.push((step_file, compiled.artifact.clone()));
        vm.loader.in_artifact += 1;
        vm.enter_file_specials();
    });
    let evaluated = run_compiled_units(m, gc.by_ref(), path, compiled, history_slot, out);
    m.leaf((), |vm, ()| {
        vm.loader.in_artifact -= 1;
        vm.leave_file_specials();
    });
    let result = leave_bindings(m, gc.by_ref(), outer, evaluated);
    m.leaf((), |vm, ()| { vm.loader.active.pop(); });
    result?;
    after_load(m, gc, named, (values, LOAD_SCRATCH))
}
fn bind_compiled_load<'b>(m: &mut Mx<'_, 'b, Vm>, mut gc: Gc<'_, 'b>, values: &RootFrame) -> Result<()> {
    bind_named(m, gc.by_ref(), values, &[("lexical-binding", LOAD_NIL), ("lread--unescaped-character-literals", LOAD_NIL),
        ("load-file-name", LOAD_FILE), ("load-true-file-name", LOAD_FILE), ("inhibit-file-name-operation", LOAD_NIL),
        ("load-in-progress", LOAD_T)])?;
    m.leaf(values, Vm::load_history_values)?;
    bind_named(m, gc, values, &[("current-load-list", LOAD_HISTORY), ("macroexp--dynvars", LOAD_DYNVARS)])
}
fn run_compiled_units<'b>(m: &mut Mx<'_, 'b, Vm>, mut gc: Gc<'_, 'b>, path: &Path, compiled: &crate::compiled_lisp::Compiled, history_slot: usize, out: Out<'_>) -> Result<()> {
    let (step_file, slots) = m.leaf((path, compiled), |vm, (path, compiled)| (vm.loader.file_index(path), vm.compiled_slots(compiled)));
    let units: Vec<_> = compiled.units().collect();
    for (index, &(ordinal, byte, end, line)) in units.iter().enumerate() {
        m.leaf((), |vm, ()| vm.poll_quit())?;
        let step = m.leaf((step_file, ordinal, byte, line), |vm, (file, ordinal, byte, line)| vm.loader.enter_step(file, ordinal, byte, line));
        let entry = m.leaf((compiled, &slots, index), |vm, (compiled, slots, index)| {
            let entry = vm.install_compiled_unit(compiled, slots, index);
            vm.propagate_next |= entry.is_ok() && vm.loader.propagate != 0;
            entry
        });
        let evaluated = entry.and_then(|entry| crate::threaded::run_function(m, gc.by_ref(), entry, (entry, out), finished_form));
        match evaluated {
            Ok(()) => m.leaf((step, path, line), Vm::form_done),
            Err(e) => {
                // The form's text, for the report, from the source.
                let text = crate::host::read_file(path).ok()
                    .and_then(|bytes| decode_source(&bytes).ok().flatten())
                    .and_then(|source| source.get(byte..end).map(|b| String::from_utf8_lossy(b).into_owned()))
                    .unwrap_or_default();
                m.leaf((step, e.clone(), text.clone()), |vm, (step, error, source)| vm.loader.step_failed(step, error, source));
                return Err(format!("{}: line {line} form {ordinal}: {}: {e}", path.display(),
                                   text.trim().chars().take(180).collect::<String>()));
            }
        }
    }
    m.leaf(history_slot, |vm, history_slot| {
        vm.loader.completed_files += 1;
        vm.build_load_history(history_slot)
    })
}
/// A form's entry has a single owner, as `eval`'s does: its code goes as it
/// returns. The definitions and closures it made stay while reachable; only those
/// count toward a code collection.
fn finished_form(vm: &mut Vm, (entry, out): (usize, Out<'_>), outcome: Result<Value>) -> Result<()> {
    vm.release_request_entry(entry);
    stored(vm, out, outcome)
}
/// lread.c:1538: the eval-after-load forms for the file NAMED.
fn after_load<'b>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>, named: &str, scratch: Out<'_>) -> Result<()> {
    let hooked = m.leaf((), |vm, ()| {
        let hook_slot = vm.slot("do-after-load-evaluation");
        let hook = vm.slots[hook_slot].function;
        hook != Value::UNBOUND && hook != Value::NIL
    });
    if !hooked {
        return Ok(());
    }
    let call = Form::List(vec![Form::Sym("do-after-load-evaluation".into()), Form::String(named.to_owned())]);
    eval_toplevel(m, gc, &call, 0, scratch)
}
/// GNU lread.c `readevalloop_eager_expand_eval`: expand the head only, then recurse
/// into a resulting progn before expanding its next child. A prog2 macro can
/// produce a progn too; prog1 itself is not split. The last value goes to OUT.
fn eval_toplevel<'b>(m: &mut Mx<'_, 'b, Vm>, mut gc: Gc<'_, 'b>, form: &Form, depth: usize, out: Out<'_>) -> Result<()> {
    if depth >= 128 || !eval_depth::native_stack_room() {
        return Err("top-level macro expansion limit exceeded".into());
    }
    // These are per-compilation dependency collectors, not a lifetime expansion
    // budget across sibling top-level forms.
    let mark = m.leaf((), |vm, ()| {
        vm.expanded_macros.clear();
        vm.expanded_symbols.clear();
        vm.propagated_escape = None;
        vm.enter_load_expansion()
    });
    let expanded = crate::threaded::expand_form(m, gc.by_ref(), form);
    m.leaf(mark, Vm::leave_load_expansion);
    let expanded = match expanded {
        Ok(expanded) => expanded,
        Err(error) => return Err(m.leaf(error, Vm::eager_expansion_failure)),
    };
    if let Some(expanded) = expanded {
        return eval_toplevel(m, gc, &expanded, depth + 1, out);
    }
    // Before GNU subr.el has installed prog2, its bootstrap surface expansion
    // must expose the same top-level progn boundary.
    if matches!(form, Form::List(items) if matches!(items.first(), Some(Form::Sym(head)) if head == "prog2"))
        && let Some(expanded) = expand::expand(form)?
    {
        return eval_toplevel(m, gc, &expanded, depth + 1, out);
    }
    if let Form::List(items) = form
        && matches!(items.first(), Some(Form::Sym(head)) if head == "progn")
    {
        m.leaf(out, |vm, out| vm.set_rooted(out.0, out.1, Value::NIL));
        for child in &items[1..] {
            eval_toplevel(m, gc.by_ref(), child, depth + 1, out)?;
        }
        return Ok(());
    }
    eval_one_form(m, gc.by_ref(), form, out)?;
    if let Form::List(items) = form
        && let [Form::Sym(head), Form::Sym(name)] = items.as_slice()
        && head == "defvar"
        && !matches!(name.as_str(), "nil" | "t")
        && let Some((dynvars, declared)) = m.leaf(name.as_str(), Vm::declared_file_special)?
    {
        let written = crate::threaded::write_from(m, gc, dynvars, &declared, 0);
        m.leaf(declared, Vm::drop_frame);
        written?;
    }
    Ok(())
}
/// A top-level form that is no `progn`: evaluated directly when it can be, else
/// compiled and run.
fn eval_one_form<'b>(m: &mut Mx<'_, 'b, Vm>, mut gc: Gc<'_, 'b>, form: &Form, out: Out<'_>) -> Result<()> {
    if let Some(done) = m.leaf((form, out), |vm, (form, out)| vm.direct_form(form).map(|value| stored(vm, out, value))) {
        return done;
    }
    let mark = m.leaf((), |vm, ()| {
        vm.seed_file_specials = true;
        vm.file_level_request = true;
        vm.enter_load_expansion()
    });
    let compiled = crate::threaded::compile_form(m, gc.by_ref(), form);
    m.leaf(mark, |vm, mark| {
        vm.leave_load_expansion(mark);
        vm.seed_file_specials = false;
        vm.file_level_request = false;
    });
    let function = match compiled {
        Ok(function) => function,
        Err(error) => return Err(m.leaf(error, Vm::eager_expansion_failure)),
    };
    m.leaf((), |vm, ()| vm.propagate_next |= vm.loader.propagate != 0);
    crate::threaded::run_function(m, gc, function, (function, out), finished_form)
}
/// `load-with-code-conversion`'s `read-symbol-shorthands` for SOURCE (mule.el),
/// stored at OUT: what `hack-read-symbol-shorthands-function` finds with the
/// file's text in a temporary buffer, nil while that function is unset. A file
/// that never names the variable has none, so it is not copied into a buffer.
fn file_shorthands<'b>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>, source: &[u8], out: Out<'_>) -> Result<()> {
    let frame = m.leaf(source, |vm, source| {
        let slot = vm.slot("hack-read-symbol-shorthands-function");
        let function = vm.variable_value(slot).unwrap_or(Value::NIL);
        if matches!(function, Value::NIL | Value::UNBOUND) || !source.windows(22).any(|w| w == b"read-symbol-shorthands") {
            return None;
        }
        let text = vm.make_lisp_string(source.to_vec(), true);
        Some(vm.root_frame(&[Value::NIL, text, Value::NIL]))
    });
    let Some(frame) = frame else { return Ok(()) };
    let found = shorthands_in_text(m, gc, &frame, out);
    m.leaf(frame, Vm::drop_frame);
    found
}
/// FRAME holds a slot for the reader, the text, and a slot for the function.
fn shorthands_in_text<'b>(m: &mut Mx<'_, 'b, Vm>, mut gc: Gc<'_, 'b>, frame: &RootFrame, out: Out<'_>) -> Result<()> {
    crate::threaded::eval_text(m, gc.by_ref(), "(lambda (source function) (let ((buffer (get-buffer-create (generate-new-buffer-name \" *load-shorthands*\") t))) (unwind-protect (save-current-buffer (set-buffer buffer) (insert source) (funcall function)) (kill-buffer buffer))))",
        (frame, 0), stored)?;
    m.leaf(frame, |vm, frame| {
        let slot = vm.slot("hack-read-symbol-shorthands-function");
        vm.variable_value(slot).map(|function| vm.set_rooted(frame, 2, function))
    })?;
    crate::threaded::call_frame(m, gc, frame, out, stored)
}
/// Bind each (NAME, its value in VALUES) in order as `let` does. A watcher's
/// signal leaves the bindings made so far for the caller to unwind.
fn bind_named<'b>(m: &mut Mx<'_, 'b, Vm>, mut gc: Gc<'_, 'b>, values: &RootFrame, bindings: &[(&str, usize)]) -> Result<()> {
    for &(name, value) in bindings {
        let slot = m.leaf(name, Vm::slot);
        crate::threaded::bind_from(m, gc.by_ref(), slot, values, value)?;
    }
    Ok(())
}
/// Leave a region that finished with RESULT, popping its bindings down to MARK:
/// unbind on normal completion, unwind under an escape.
pub(crate) fn leave_bindings<'b, T>(m: &mut Mx<'_, 'b, Vm>, gc: Gc<'_, 'b>, mark: usize, result: Result<T>) -> Result<T> {
    match result {
        Ok(value) => crate::threaded::unbind_to(m, gc, mark).map(|()| value),
        Err(error) => {
            crate::threaded::unwind_to(m, gc, mark);
            Err(error)
        }
    }
}
/// An absolute file name with `.' and `..' resolved lexically, as
/// expand-file-name would leave openp's result.
fn expanded_path(path: &Path) -> Result<String> {
    let absolute = crate::host::absolute(path).map_err(|e| format!("file missing: {}: {e}", path.display()))?;
    let mut parts: Vec<std::path::Component> = vec![];
    for part in absolute.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => { if parts.len() > 1 { parts.pop(); } }
            other => parts.push(other),
        }
    }
    Ok(parts.iter().collect::<std::path::PathBuf>().to_string_lossy().into_owned())
}
/// Source text as GNU internal multibyte bytes, or None when it names no
/// coding system and is not UTF-8, so the coding priority must decide.
fn decode_source(bytes: &[u8]) -> Result<Option<Vec<u8>>> {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let header = String::from_utf8_lossy(
        bytes
            .split_inclusive(|b| *b == b'\n')
            .take(2)
            .flatten()
            .copied()
            .collect::<Vec<_>>()
            .as_slice(),
    )
    .into_owned();
    let coding = header.find("coding:").map(|at| {
        header[at + 7..]
            .trim_start()
            .split(|c: char| c.is_whitespace() || c == ';')
            .next()
            .unwrap_or("")
    });
    let name = coding.unwrap_or("utf-8");
    let base = name
        .strip_suffix("-unix")
        .or_else(|| name.strip_suffix("-dos"))
        .or_else(|| name.strip_suffix("-mac"))
        .unwrap_or(name);
    let mut source = match base {
        // GNU's utf-8 and utf-8-emacs share decode_coding_utf_8.
        "utf-8" | "utf-8-emacs" => {
            let (text, raw) = utf_8_decode(bytes);
            // Without a cookie GNU detects the coding system
            // (load-with-code-conversion); its UTF-8 detector accepts
            // exactly what this decodes without a raw byte.
            if raw && coding.is_none() { return Ok(None) }
            text
        }
        "latin-1" | "iso-latin-1" | "iso-8859-1" => bytes.iter().map(|b| char::from(*b)).collect::<String>().into_bytes(),
        "us-ascii" if bytes.is_ascii() => bytes.to_vec(),
        _ => return Err(format!("unsupported source coding system: {name}")),
    };
    // CR and LF are single bytes in every internal character sequence.
    if name.ends_with("-dos") {
        let mut out = Vec::with_capacity(source.len());
        for (i, &b) in source.iter().enumerate() {
            if !(b == b'\r' && source.get(i + 1) == Some(&b'\n')) { out.push(b); }
        }
        source = out;
    }
    if name.ends_with("-mac") {
        for b in &mut source { if *b == b'\r' { *b = b'\n'; } }
    }
    Ok(Some(source))
}

/// coding.c decode_coding_utf_8, which utf-8 and utf-8-emacs share: the
/// internal encoding (characters up to MAX_CHAR, beyond Unicode included),
/// every byte that starts no valid character becoming a raw-byte
/// character. As in coding.c, the internal 2-byte raw-byte forms (C0/C1
/// leads) and surrogate codes are not characters in a file. Also says
/// whether any raw byte was made.
fn utf_8_decode(bytes: &[u8]) -> (Vec<u8>, bool) {
    let mut out = Vec::with_capacity(bytes.len());
    let mut raw_seen = false;
    let mut at = 0;
    while at < bytes.len() {
        let decoded = crate::lstring::decode_char(&bytes[at..])
            .filter(|_| bytes[at] < 0x80 || bytes[at] > 0xc1)
            .filter(|&(code, _)| !(0xd800..=0xdfff).contains(&code));
        match decoded {
            Some((_, width)) => { out.extend_from_slice(&bytes[at..at + width]); at += width; }
            None => {
                let mut raw = [0; 5];
                let n = crate::lstring::char_string(crate::lstring::BYTE8_OFFSET + u32::from(bytes[at]), &mut raw);
                out.extend_from_slice(&raw[..n]);
                raw_seen = true;
                at += 1;
            }
        }
    }
    (out, raw_seen)
}
