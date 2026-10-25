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
            "load-file-name",
            "load-in-progress",
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
                if p.is_file() {
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
        {
            self.file_specials.insert(name.clone());
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
        let path = std::fs::canonicalize(path)
            .map_err(|e| format!("file missing: {}: {e}", path.display()))?;
        if self.loader.active.len() >= 128 || self.loader.active.contains(&path) {
            return Err(format!("recursive source load: {}", path.display()));
        }
        let bytes =
            std::fs::read(&path).map_err(|e| format!("file error: {}: {e}", path.display()))?;
        let source = decode_source(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        let file_slot = self.slot("load-file-name");
        let progress_slot = self.slot("load-in-progress");
        let old_file = self.slots[file_slot].value;
        let old_progress = self.slots[progress_slot].value;
        self.slots[file_slot].value =
            self.literal(&Form::String(path.to_string_lossy().into_owned()))?;
        self.slots[progress_slot].value = Value::TRUE;
        // Root both old scoped values across nested executor collections.
        let mark = self.dynamic.len();
        self.backtrace.bind(self.dynamic.len(), Value::reference(file_slot + 1, 7));
        self.dynamic.push((file_slot, old_file, self.slots[self.buffer_slot].value, false));
        self.backtrace.bind(self.dynamic.len(), Value::reference(progress_slot + 1, 7));
        self.dynamic.push((progress_slot, old_progress, self.slots[self.buffer_slot].value, false));
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
            loop {
                let read = self.read_source_span(&source[at..]);
                let Some((value, begin, used)) = read.map_err(|e| {
                    format!(
                        "{}: line {} form {}: reader: {e}",
                        path.display(),
                        source[..at].bytes().filter(|b| *b == b'\n').count() + 1,
                        ordinal + 1
                    )
                })?
                else {
                    break;
                };
                ordinal += 1;
                let form_source = &source[at + begin..at + used];
                let form = self.source_form(value, 0)?;
                let step = self.loader.steps.len();
                self.loader.steps.push(serde_json::json!({"file":path,"ordinal":ordinal,"byte":at+begin,"line":source[..at+begin].bytes().filter(|b|*b==b'\n').count()+1,"source":form_source,"status":"entered"}));
                let evaluated = self.eval_load_toplevel(&form, Some(form_source), lazy, 0);
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
                            source[..at + begin].bytes().filter(|b| *b == b'\n').count() + 1,
                            ordinal,
                            form_source.trim().chars().take(180).collect::<String>()
                        ));
                    }
                }
                at += used;
            }
            self.loader.completed_files += 1;
            Ok(result)
        })();
        self.lazy_regime = saved_regime;
        self.defer_expansion_errors = saved_defer;
        self.file_specials = saved_file_specials;
        self.expanded_macros = saved_expanded_macros;
        self.expanded_symbols = saved_expanded_symbols;
        self.loader.active.pop();
        self.unbind(self.dynamic.len() - mark);
        result
    }
}
fn decode_source(bytes: &[u8]) -> Result<String> {
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
        "utf-8" | "utf-8-emacs" => {
            String::from_utf8(bytes.to_vec()).map_err(|_| "source is not valid UTF-8")?
        }
        "latin-1" | "iso-latin-1" | "iso-8859-1" => bytes.iter().map(|b| char::from(*b)).collect(),
        "us-ascii" if bytes.is_ascii() => String::from_utf8(bytes.to_vec()).unwrap(),
        _ => return Err(format!("unsupported source coding system: {name}")),
    };
    if name.ends_with("-dos") {
        source = source.replace("\r\n", "\n");
    }
    if name.ends_with("-mac") {
        source = source.replace('\r', "\n");
    }
    Ok(source)
}
