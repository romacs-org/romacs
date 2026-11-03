//! UTF-8/Latin-1 source loading. Feature state lives in the Lisp `features`
//! variable, not a shadow Rust set. Unsupported coding/compiled files fail closed.
use super::*;
use std::path::{Path, PathBuf};
#[derive(Default)]
pub(super) struct Loader {
    pub(super) propagate: usize,
    active: Vec<PathBuf>,
    /// Every file a step was recorded in, once, in first-step order.
    files: Vec<PathBuf>,
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
}
romacs_snapshot::snapshot_struct!(Loader { propagate, active, files, steps, completed_files, completed_forms, recording, compiled, in_artifact });
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
            data::Object::Bytes(b) if b.is_ascii() => Ok(unsafe { std::str::from_utf8_unchecked(b) }),
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
            self.conses.push((feature, old));
            self.slots[slot].value = Value::reference(self.conses.len() - 1, 3);
        }
        if subfeature != Value::NIL {
            let key = self.intern("subfeatures");
            self.put_symbol_property(feature, key, subfeature).map_err(prim::Fault::Error)?;
        }
        Ok(())
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
    pub(super) fn loader_primitive(&mut self, p: Primitive, args: &[Value]) -> Result<Value> {
        match p {
            Primitive::Featurep | Primitive::Provide => {
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
                    self.conses.push((args[0], old));
                    self.slots[slot].value = Value::reference(self.conses.len() - 1, 3);
                }
                if subfeature != Value::NIL {
                    let key = self.intern("subfeatures");
                    self.put_symbol_property(args[0], key, subfeature)?;
                }
                Ok(args[0])
            }
            Primitive::Load => {
                if args.is_empty() || args.len() > 5 {
                    return Err("wrong primitive arity".into());
                }
                let name = self.source_string(args[0])?.to_owned();
                let noerror = args.get(1).is_some_and(|v| *v != Value::NIL);
                let nosuffix = args.get(3).is_some_and(|v| *v != Value::NIL);
                let path = self.source_path(&name, nosuffix)?;
                let Some((path, suffixed)) = path else {
                    return if noerror {
                        Ok(Value::NIL)
                    } else {
                        Err(format!("file missing: {name}"))
                    };
                };
                if args.get(4).is_some_and(|v| *v != Value::NIL) && path.extension().is_none() {
                    return Err("load mustsuffix requires a source suffix".into());
                }
                self.load_file_kind(&path, suffixed)?;
                Ok(Value::TRUE)
            }
            Primitive::Require => {
                if args.is_empty() || args.len() > 3 {
                    return Err("wrong primitive arity".into());
                }
                let feature = args[0];
                if self.loader.recording.is_some() && let Some(name) = self.symbol_name_of(feature) {
                    let file = args.get(1).filter(|v| **v != Value::NIL).and_then(|v| self.source_string_if(*v)).map(str::to_owned);
                    let noerror = args.get(2).is_some_and(|v| *v != Value::NIL);
                    self.note_init_load(crate::world::layer::Kind::Require, name, file, noerror);
                }
                if self.feature_present(feature)? {
                    return Ok(feature);
                }
                // fns.c Frequire: load_with_autoload_queue (FILE, NOERROR, Qt,
                // Qnil, NILP (FILENAME)), so a failed load is undone.
                let (file, must_suffix) = match args.get(1) {
                    Some(v) if *v != Value::NIL => (*v, false),
                    _ => {
                        let name = self.display(feature)?;
                        (self.literal(&Form::String(name))?, true)
                    }
                };
                let noerror = args.get(2).is_some_and(|v| *v != Value::NIL);
                let path = self.load_with_autoload_queue(file, noerror, must_suffix)?;
                if path.as_os_str().is_empty() {
                    return Ok(Value::NIL);
                }
                if !self.feature_present(feature)? {
                    return Err(format!(
                        "feature was not provided: {} by {}",
                        self.display(feature)?,
                        path.display()
                    ));
                }
                Ok(feature)
            }
            Primitive::MakeInterpretedClosure => {
                let start = self.stack.len();
                self.stack.extend_from_slice(args);
                let result = self.interpreted_closure_primitive(Primitive::MakeInterpretedClosure, start, args.len());
                self.stack.truncate(start);
                result
            }
            Primitive::Eval => self.eval_primitive(args),
            Primitive::Macroexpand => self.macroexpand_primitive(args),
            Primitive::AutoloadDoLoad => {
                if args.is_empty() || args.len() > 3 {
                    return Err("wrong primitive arity".into());
                }
                let arg = |i: usize| args.get(i).copied().unwrap_or(Value::NIL);
                self.autoload_do_load(arg(0), arg(1), arg(2))
            }
            Primitive::ReadHostEvent => self.read_host_event(args),
            Primitive::FormatModeLine => self.format_mode_line(args),
            _ => unreachable!(),
        }
    }
    /// GNU lread.c readevalloop_eager_expand_eval: expand the head only,
    /// then recurse into a resulting progn before expanding its next child.
    /// A prog2 macro can produce a progn too; prog1 itself is not split.
    fn eval_load_toplevel(&mut self, form: &Form, source: Option<&str>, lazy: bool, depth: usize) -> Result<Value> {
        if depth >= 128 || !eval_depth::native_stack_room() {
            return Err("top-level macro expansion limit exceeded".into());
        }
        // These are per-compilation dependency collectors, not a lifetime
        // expansion budget across sibling top-level forms.
        self.expanded_macros.clear();
        self.expanded_symbols.clear();
        self.propagated_escape = None;
        let expanded = match self.expanding_for_load(|vm| vm.expand_macro(form)) {
            Ok(expanded) => expanded,
            Err(error) => return Err(self.eager_expansion_failure(error)),
        };
        if let Some(expanded) = expanded {
            return self.eval_load_toplevel(&expanded, None, lazy, depth + 1);
        }
        // Before GNU subr.el has installed prog2, its bootstrap surface
        // expansion must expose the same top-level progn boundary.
        if matches!(form, Form::List(items) if matches!(items.first(), Some(Form::Sym(head)) if head == "prog2"))
            && let Some(expanded) = expand::expand(form)?
        {
            return self.eval_load_toplevel(&expanded, None, lazy, depth + 1);
        }
        if let Form::List(items) = form
            && matches!(items.first(), Some(Form::Sym(head)) if head == "progn")
        {
            let mut result = Value::NIL;
            for child in &items[1..] {
                result = self.eval_load_toplevel(child, None, lazy, depth + 1)?;
            }
            return Ok(result);
        }
        let result = if let Some(value) = self.direct_form(form) {
            value
        } else {
            self.seed_file_specials = true;
            let compiled = self.expanding_for_load(|vm| if !lazy && let Some(source) = source {
                vm.cached_source_form(source, form)
            } else {
                vm.compile_request(form).map(|(function, _)| function)
            });
            self.seed_file_specials = false;
            let compiled = match compiled {
                Ok(function) => Ok(function),
                Err(error) => Err(self.eager_expansion_failure(error)),
            };
            compiled.and_then(|function| {
                self.propagate_next |= self.loader.propagate != 0;
                self.execute_raw(function)
            })
        };
        if result.is_ok()
            && let Form::List(items) = form
            && let [Form::Sym(head), Form::Sym(name)] = items.as_slice()
            && head == "defvar"
            && !matches!(name.as_str(), "nil" | "t")
            && self.file_specials.insert(name.clone())
        {
            // macroexpand--all-toplevel records the declaration in the
            // file's `macroexp--dynvars' (macroexp.el), which macros such as
            // eval-and-compile hand to `eval' as its lexical environment.
            let symbol = self.intern(name);
            let dynvars = self.slot("macroexp--dynvars");
            let declared = self.variable_value(dynvars)?;
            let declared = self.keymap_cons(symbol, declared);
            self.set_variable_value(dynvars, declared)?;
        }
        result
    }
    /// RUN, which expands a top-level form, under macroexp.el
    /// internal-macroexpand-for-load's `condition-case' clause `(debug
    /// error)': the debugger is considered as GNU's would, and no
    /// handler around the load sees an error the expansion signals, which
    /// `eager_expansion_failure' wraps.
    fn expanding_for_load<T>(&mut self, run: impl FnOnce(&mut Self) -> T) -> T {
        let mark = self.signal_handlers.len();
        let (debug, error) = (self.intern("debug"), self.intern("error"));
        let clause = self.list_value(&[debug, error]);
        self.signal_handlers.push(threaded::SignalHandler::Condition(clause));
        let result = run(self);
        self.signal_handlers.truncate(mark);
        result
    }
    /// macroexp.el internal-macroexpand-for-load: an error that expanding a
    /// top-level form signals reaches the loader's caller as (error "Eager
    /// macro-expansion failure: ERR"), ERR printed as `%S' prints it. A quit
    /// or another signal that is no error passes as it is, as does a throw
    /// to a catch outside the load.
    fn eager_expansion_failure(&mut self, error: String) -> String {
        let (condition, object) = match self.propagated_escape {
            Some(threaded::Escape::Signal(condition, data)) => {
                self.conses.push((condition, data));
                (condition, Value::reference(self.conses.len() - 1, 3))
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

    pub(crate) fn load_file_raw(&mut self, path: &Path) -> Result<Value> {
        self.load_file_kind(path, false)
    }
    /// Load PATH; COMPILED when its name was found by adding `.el`, so the
    /// load is compiled when the file has a compiled form
    /// (`compiled_lisp`).
    pub(crate) fn load_file_kind(&mut self, path: &Path, compiled: bool) -> Result<Value> {
        if path.extension().is_some_and(|s| s == "elc") {
            return Err("compiled .elc input is not supported".into());
        }
        // GNU names the file as openp found it, expanded but not resolved
        // through symlinks (hist_file_name, lread.c:1452): that name is
        // `load-file-name', its `load-history' entry and the after-load
        // argument. The canonical path detects recursion and reads.
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
        if compiled && let Some(compiled) = self.compiled_load(&path)? {
            if crate::host::mode() == crate::host::Mode::Record {
                self.note_loaded(&path);
                compiled.inherit_inputs();
            }
            return self.load_artifact(named, path, compiled);
        }
        let bytes =
            crate::host::read_file(&path).map_err(|e| format!("file error: {}: {e}", path.display()))?;
        if crate::host::mode() == crate::host::Mode::Record {
            self.note_loaded(&path);
        }
        let source = match decode_source(&bytes).map_err(|e| format!("{}: {e}", path.display()))? {
            Some(source) => source,
            None => self.detected_source(&bytes).map_err(|e| format!("{}: {e}", path.display()))?,
        };
        // Each public variable binds as a `let' does, in pinned GNU's order:
        // Fload's `lexical-binding' (lread.c:1323), bound t only in a source
        // load inside a compiled one, which binds it nil: VM2 reads every
        // file lexically (VM2-LEXBIND1), elsewhere under its default t; then
        // its lread--unescaped-character-literals, then
        // load-with-code-conversion's load-in-progress (held over the
        // after-load forms) and file names, then readevalloop's. GNU's
        // bindings of the reading buffer (standard-input, eval-buffer-list)
        // have no counterpart: VM2 reads the file's bytes.
        let shorthands = self.file_shorthands(&source)?;
        // The source cache keys a form by its text; shorthands read that
        // text to other symbols, so such a file's forms are compiled afresh.
        let cacheable = shorthands == Value::NIL;
        let file = self.literal(&Form::String(named.clone()))?;
        let history_slot = self.slot("current-load-list");
        let outer = self.dynamic.len();
        let lexical: &[(&str, Value)] = if self.loader.in_artifact > 0 { &[("lexical-binding", Value::TRUE)] } else { &[] };
        let bound = self.bind_named(lexical)
            .and_then(|()| self.bind_named(&[("lread--unescaped-character-literals", Value::NIL), ("load-in-progress", Value::TRUE)]));
        let inner = self.dynamic.len();
        // loadhist_initialize (lread.c:1031): `load' binds current-load-list
        // to (FILE), so LOADHIST_ATTACH conses this file's definitions before
        // its name and macroexp-file-name finds the file being loaded.
        let bound = bound.and_then(|()| {
            let dynvars = self.slot("macroexp--dynvars");
            let dynvars = self.variable_value(dynvars)?;
            let history = self.list_value(&[file]);
            self.bind_named(&[("inhibit-file-name-operation", Value::NIL), ("load-file-name", file), ("load-true-file-name", file),
                ("read-symbol-shorthands", shorthands), ("macroexp--dynvars", dynvars), ("current-load-list", history)])
        });
        if let Err(error) = bound {
            self.unwind(self.dynamic.len() - outer);
            return Err(error);
        }
        self.loader.active.push(path.clone());
        // GNU readevalloop (src/lread.c:2183-2188) decides ONCE per load:
        // forms are eagerly macroexpanded only if `internal-macroexpand-for-
        // load' (macroexp.el) is already fbound. Otherwise each form goes
        // through eval_sub, which expands a form only when it is reached and
        // leaves closure bodies unexpanded (R215). VM2 compiles eagerly in
        // the first regime and lazily in the second.
        let expander = self.slot("internal-macroexpand-for-load");
        let lazy = matches!(self.slots[expander].function, f if f == Value::UNBOUND || f == Value::NIL);
        let saved_regime = std::mem::replace(&mut self.lazy_regime, lazy);
        let saved_defer = std::mem::replace(&mut self.defer_expansion_errors, lazy);
        // readevalloop binds `internal-interpreter-environment' once for the
        // file, so a top-level `(defvar NAME)' reaches every LATER form of
        // this file and no other file (VM2-FILESPECIAL1 in DIVERGENCES.md).
        let saved_file_specials = std::mem::take(&mut self.file_specials);
        // A macro can load a file while an outer compilation is suspended.
        // Child compilations must not erase its nonconstant-macro guards.
        let saved_expanded_macros = std::mem::take(&mut self.expanded_macros);
        let saved_expanded_symbols = std::mem::take(&mut self.expanded_symbols);
        // The last form's value is rooted across reading the next one, which
        // can call Lisp (a `\N{NAME}' character).
        let last = self.root_frame(&[Value::NIL]);
        let step_file = self.loader.file_index(&path);
        let result = (|| {
            let mut at = 0;
            let mut ordinal = 0;
            // The line of byte `counted`, kept as reading advances: counting
            // from the start for each form made a file's load quadratic.
            let (mut line, mut counted) = (1, 0);
            loop {
                // readevalloop's maybe_quit between forms: a form that
                // reaches no safepoint of its own still lets C-g end the load,
                // the forms before it kept.
                self.poll_quit()?;
                let read = self.read_internal_span(&source[at..]);
                let Some((value, begin, used)) = read.map_err(|e| {
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
                // A form holding characters outside Unicode has no exact
                // &str text: it is reported lossily and not compile-cached.
                let exact = std::str::from_utf8(&source[at + begin..at + used]).ok().filter(|_| cacheable);
                let form_source = String::from_utf8_lossy(&source[at + begin..at + used]);
                let form = self.source_form(value, 0)?;
                line += source[counted..at + begin].iter().filter(|b| **b == b'\n').count();
                counted = at + begin;
                let step = self.loader.steps.len();
                self.loader.steps.push(Step { file: step_file, ordinal, byte: at + begin, line, status: StepStatus::Entered, error: None, source: None });
                let site = crate::host::enter_site(|| crate::host::Site {
                    file: path.to_string_lossy().into_owned(), line,
                    head: form_source.trim_start().lines().next().unwrap_or("").chars().take(80).collect(),
                });
                let mut evaluated = self.eval_load_toplevel(&form, exact, lazy, 0);
                if crate::host::mode() == crate::host::Mode::Build {
                    // A layer's definition cut short by a refused effect
                    // defines nil here and is kept whole for restore.
                    if let (Err(_), Some(name)) = (&evaluated, crate::host::layer_form_cut()) {
                        self.propagated_escape = None;
                        let slot = self.slot(&name);
                        if self.slots[slot].value == Value::UNBOUND {
                            self.declare_special_raw(&name, Value::NIL);
                        }
                        evaluated = Ok(Value::NIL);
                    }
                    if evaluated.is_ok() {
                        self.world_form_done(&form);
                    }
                }
                drop(site);
                match evaluated {
                    Ok(v) => {
                        self.set_rooted(&last, 0, v);
                        self.loader.completed_forms += 1;
                        self.loader.steps[step].status = StepStatus::Ok;
                    }
                    Err(e) => {
                        let failed = &mut self.loader.steps[step];
                        failed.status = StepStatus::Error;
                        failed.error = Some(e.clone());
                        failed.source = Some(form_source.to_string());
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
            self.loader.completed_files += 1;
            self.build_load_history(history_slot)?;
            Ok(())
        })();
        let [value] = self.release_frame(last);
        let result = result.map(|()| value);
        self.lazy_regime = saved_regime;
        self.defer_expansion_errors = saved_defer;
        self.file_specials = saved_file_specials;
        self.expanded_macros = saved_expanded_macros;
        self.expanded_symbols = saved_expanded_symbols;
        let result = self.unbind_after(self.dynamic.len() - inner, result);
        // lread.c:1538: run the eval-after-load forms for this file, with
        // the last form's value rooted across them. The file stays among the
        // loads in progress until they finish, as GNU's unwind keeps it.
        let result = result.and_then(|result| {
            let hook_slot = self.slot("do-after-load-evaluation");
            let hook = self.slots[hook_slot].function;
            if hook == Value::UNBOUND || hook == Value::NIL {
                return Ok(result);
            }
            let call = Form::List(vec![Form::Sym("do-after-load-evaluation".into()), Form::String(named)]);
            let rooted = handle::FormObject::new(self, result);
            self.eval_load_toplevel(&call, None, false, 0)?;
            rooted.value(self)
        });
        let result = self.unbind_after(self.dynamic.len() - outer, result);
        self.loader.active.pop();
        result
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
    /// GNU's load of a compiled file (lread.c Fload with `compiled'),
    /// running COMPILED's units in order. NAMED is the file's name as
    /// openp found it, PATH its resolved one. Its bindings are Fload's,
    /// in its order: the loads in progress, `lexical-binding' nil (the
    /// variable; each unit keeps the dialect it was compiled in), the
    /// unescaped-literal list, the file names, `load-in-progress', then
    /// readevalloop's `current-load-list' and `macroexp--dynvars'. All of
    /// them unwind before the file's after-load forms run (lread.c:1536),
    /// where a source load keeps them (load-with-code-conversion).
    fn load_artifact(&mut self, named: String, path: PathBuf, compiled: crate::compiled_lisp::Compiled) -> Result<Value> {
        let file = self.literal(&Form::String(named.clone()))?;
        let history_slot = self.slot("current-load-list");
        let outer = self.dynamic.len();
        self.loader.active.push(path.clone());
        let bound = self.bind_named(&[("lexical-binding", Value::NIL), ("lread--unescaped-character-literals", Value::NIL),
            ("load-file-name", file), ("load-true-file-name", file), ("inhibit-file-name-operation", Value::NIL),
            ("load-in-progress", Value::TRUE)]).and_then(|()| {
            let dynvars = self.slot("macroexp--dynvars");
            let dynvars = self.variable_value(dynvars)?;
            let history = self.list_value(&[file]);
            self.bind_named(&[("current-load-list", history), ("macroexp--dynvars", dynvars)])
        });
        if let Err(error) = bound {
            self.unwind(self.dynamic.len() - outer);
            self.loader.active.pop();
            return Err(error);
        }
        // An `.elc' carries no `(defvar X)' for readevalloop to see.
        let saved_file_specials = std::mem::take(&mut self.file_specials);
        let last = self.root_frame(&[Value::NIL]);
        let step_file = self.loader.file_index(&path);
        self.loader.compiled.push((step_file, compiled.artifact.clone()));
        let slots = self.compiled_slots(&compiled);
        let units: Vec<_> = compiled.units().collect();
        self.loader.in_artifact += 1;
        let result = (|| {
            for (index, &(ordinal, byte, end, line)) in units.iter().enumerate() {
                self.poll_quit()?;
                let step = self.loader.steps.len();
                self.loader.steps.push(Step { file: step_file, ordinal, byte, line, status: StepStatus::Entered, error: None, source: None });
                let evaluated = self.install_compiled_unit(&compiled, &slots, index).and_then(|entry| {
                    self.propagate_next |= self.loader.propagate != 0;
                    let outcome = self.execute_raw(entry);
                    self.release_request_entry(entry);
                    outcome
                });
                match evaluated {
                    Ok(v) => {
                        self.set_rooted(&last, 0, v);
                        self.loader.completed_forms += 1;
                        self.loader.steps[step].status = StepStatus::Ok;
                    }
                    Err(e) => {
                        // The form's text, for the report, from the source.
                        let text = crate::host::read_file(&path).ok()
                            .and_then(|bytes| decode_source(&bytes).ok().flatten())
                            .and_then(|source| source.get(byte..end).map(|b| String::from_utf8_lossy(b).into_owned()))
                            .unwrap_or_default();
                        let failed = &mut self.loader.steps[step];
                        failed.status = StepStatus::Error;
                        failed.error = Some(e.clone());
                        failed.source = Some(text.clone());
                        return Err(format!("{}: line {line} form {ordinal}: {}: {e}", path.display(),
                                           text.trim().chars().take(180).collect::<String>()));
                    }
                }
            }
            self.loader.completed_files += 1;
            self.build_load_history(history_slot)
        })();
        self.loader.in_artifact -= 1;
        let [value] = self.release_frame(last);
        self.file_specials = saved_file_specials;
        let result = self.unbind_after(self.dynamic.len() - outer, result.map(|()| value));
        self.loader.active.pop();
        result.and_then(|result| {
            let hook_slot = self.slot("do-after-load-evaluation");
            let hook = self.slots[hook_slot].function;
            if hook == Value::UNBOUND || hook == Value::NIL {
                return Ok(result);
            }
            let call = Form::List(vec![Form::Sym("do-after-load-evaluation".into()), Form::String(named)]);
            let rooted = handle::FormObject::new(self, result);
            self.eval_load_toplevel(&call, None, false, 0)?;
            rooted.value(self)
        })
    }
    /// load-with-code-conversion's `read-symbol-shorthands' for SOURCE (mule.el):
    /// what `hack-read-symbol-shorthands-function' finds with the file's text
    /// in a temporary buffer, nil while that function is unset. A file that
    /// never names the variable has none, so it is not copied into a buffer.
    fn file_shorthands(&mut self, source: &[u8]) -> Result<Value> {
        let slot = self.slot("hack-read-symbol-shorthands-function");
        let function = self.variable_value(slot).unwrap_or(Value::NIL);
        if matches!(function, Value::NIL | Value::UNBOUND) || !source.windows(22).any(|w| w == b"read-symbol-shorthands") {
            return Ok(Value::NIL);
        }
        let text = self.make_lisp_string(source.to_vec(), true);
        let text = handle::FormObject::new(self, text);
        let reader = self.eval_source_raw("(lambda (source function) (let ((buffer (get-buffer-create (generate-new-buffer-name \" *load-shorthands*\") t))) (unwind-protect (save-current-buffer (set-buffer buffer) (insert source) (funcall function)) (kill-buffer buffer))))")?;
        // Compiling may move heap values: read the function and text after it.
        let function = self.variable_value(slot)?;
        let text = text.value(self)?;
        self.reader_call(reader, &[text, function])
    }
    /// Bind each (NAME VALUE) in order as `let' does. A watcher's signal
    /// leaves the bindings made so far for the caller to unwind.
    fn bind_named(&mut self, bindings: &[(&str, Value)]) -> Result<()> {
        for &(name, value) in bindings {
            let slot = self.slot(name);
            self.bind_variable(slot, value)?;
        }
        Ok(())
    }
    /// lread.c build_load_history for an entire file: the file's entry,
    /// `current-load-list' in definition order, replaces any older entry
    /// for the same name at the front of `load-history'.
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
        self.set_variable_value(slot, history)?;
        // lread.c:2112 stores t without notifying; the load's unbinding
        // restores the outer list.
        let fwd = self.native_forwarding(history_slot).expect("current-load-list is forwarded");
        self.set_forwarded(history_slot, fwd, Value::TRUE, forwarded::SetFlag::Set).map_err(forwarded::fault_text)
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
