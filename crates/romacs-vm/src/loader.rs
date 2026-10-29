//! UTF-8/Latin-1 source loading. Feature state lives in the Lisp `features`
//! variable, not a shadow Rust set. Unsupported coding/compiled files fail closed.
use super::*;
use std::path::{Path, PathBuf};
#[derive(Default)]
pub(super) struct Loader {
    pub(super) propagate: usize,
    active: Vec<PathBuf>,
    steps: Vec<serde_json::Value>,
    completed_files: usize,
    completed_forms: usize,
}
impl Vm {
    pub(super) fn source_string_if(&self, v: Value) -> Option<&str> {
        if v.tag() != 0 {
            return None;
        }
        if Self::image_value(v) {
            return self.string_value(v).ok();
        }
        match self.object_if(v) {
            Some(data::Object::String(s)) => Some(s.as_str()),
            Some(data::Object::Bytes(b)) if b.is_ascii() => std::str::from_utf8(b).ok(),
            _ => None,
        }
    }
    pub(super) fn source_string(&self, v: Value) -> Result<&str> {
        if Self::image_value(v) {
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
    pub fn load_report(&self) -> serde_json::Value {
        serde_json::json!({"completed_files":self.loader.completed_files,"completed_forms":self.loader.completed_forms,"steps":self.loader.steps})
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
        let slot = self.names["features"];
        Ok(self.list_values(self.slots[slot].value)?.contains(&feature))
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
    pub(super) fn source_path(&self, name: &str, nosuffix: bool) -> Result<Option<PathBuf>> {
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
            for p in candidates {
                if crate::host::probe(crate::host::Probe::IsFile, &p) {
                    return Ok(Some(p));
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
                let Some(path) = path else {
                    return if noerror {
                        Ok(Value::NIL)
                    } else {
                        Err(format!("file missing: {name}"))
                    };
                };
                if args.get(4).is_some_and(|v| *v != Value::NIL) && path.extension().is_none() {
                    return Err("load mustsuffix requires a source suffix".into());
                }
                self.load_file_raw(&path)?;
                Ok(Value::TRUE)
            }
            Primitive::Require => {
                if args.is_empty() || args.len() > 3 {
                    return Err("wrong primitive arity".into());
                }
                let feature = args[0];
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
        if let Some(expanded) = self.expand_macro(form)? {
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
            let compiled = if !lazy && let Some(source) = source {
                self.cached_source_form(source, form)
            } else {
                self.compile_request(form).map(|(function, _)| function)
            };
            self.seed_file_specials = false;
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
    pub(super) fn source_load_active(&self) -> bool {
        !self.loader.active.is_empty()
    }

    pub(crate) fn load_file_raw(&mut self, path: &Path) -> Result<Value> {
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
        let bytes =
            crate::host::read_file(&path).map_err(|e| format!("file error: {}: {e}", path.display()))?;
        let source = decode_source(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        // Each public variable binds as a `let' does, in pinned GNU's order:
        // Fload's lread--unescaped-character-literals, then
        // load-with-code-conversion's load-in-progress (held over the
        // after-load forms) and file names, then readevalloop's. GNU's
        // bindings of the reading buffer (standard-input, eval-buffer-list,
        // lexical-binding) have no counterpart: VM2 reads the file's bytes.
        let shorthands = self.file_shorthands(&source)?;
        // The source cache keys a form by its text; shorthands read that
        // text to other symbols, so such a file's forms are compiled afresh.
        let cacheable = shorthands == Value::NIL;
        let file = self.literal(&Form::String(named.clone()))?;
        let history_slot = self.slot("current-load-list");
        let outer = self.dynamic.len();
        let bound = self.bind_named(&[("lread--unescaped-character-literals", Value::NIL), ("load-in-progress", Value::TRUE)]);
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
        let result = (|| {
            let mut at = 0;
            let mut ordinal = 0;
            let mut result = Value::NIL;
            // The line of byte `counted`, kept as reading advances: counting
            // from the start for each form made a file's load quadratic.
            let (mut line, mut counted) = (1, 0);
            loop {
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
                self.loader.steps.push(serde_json::json!({"file":path,"ordinal":ordinal,"byte":at+begin,"line":line,"source":form_source,"status":"entered"}));
                let site = crate::host::enter_site(|| crate::host::Site {
                    file: path.to_string_lossy().into_owned(), line,
                    head: form_source.trim_start().lines().next().unwrap_or("").chars().take(80).collect(),
                });
                let evaluated = self.eval_load_toplevel(&form, exact, lazy, 0);
                drop(site);
                match evaluated {
                    Ok(v) => {
                        result = v;
                        self.loader.completed_forms += 1;
                        self.loader.steps[step]["status"] = "ok".into();
                    }
                    Err(e) => {
                        self.loader.steps[step]["status"] = "error".into();
                        self.loader.steps[step]["error"] = e.clone().into();
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
            Ok(result)
        })();
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
/// Source text as GNU internal multibyte bytes.
fn decode_source(bytes: &[u8]) -> Result<Vec<u8>> {
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
        "utf-8" => {
            String::from_utf8(bytes.to_vec()).map_err(|_| "source is not valid UTF-8")?.into_bytes()
        }
        "utf-8-emacs" => utf_8_emacs(bytes),
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
    Ok(source)
}

/// GNU's utf-8-emacs decoder: its own internal encoding, every byte that
/// starts no valid character becoming a raw-byte character. As in coding.c,
/// the internal 2-byte raw-byte forms (C0/C1 leads) and surrogate codes are
/// not characters in a file.
fn utf_8_emacs(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
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
                at += 1;
            }
        }
    }
    out
}
