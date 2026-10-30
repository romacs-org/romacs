//! Versioned executable-world records in the existing authenticated page store.
//! Function bodies stay mapped until first call; only validated IR reaches unsafe
//! threaded lowering. Cold IR contains no process pointers or moving heap references.
use super::*;
use serde_json::{Value as Json, json};
use std::{collections::BTreeMap, path::Path};

const ABI: &str = "VM2CODE8";
fn array(v: &Json) -> Result<&[Json]> {
    v.as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| "image array required".into())
}
fn number(v: &Json) -> Result<usize> {
    usize::try_from(v.as_u64().ok_or("image unsigned integer required")?)
        .map_err(|_| "image integer overflow".into())
}
fn field<'a>(v: &'a Json, name: &str) -> Result<&'a Json> {
    v.get(name)
        .ok_or_else(|| format!("missing image field {name}"))
}
fn words(v: &Json) -> Result<Vec<usize>> {
    array(v)?.iter().map(number).collect()
}
fn value(v: &Json) -> Result<Value> {
    Ok(Value(v.as_u64().ok_or("image value required")?))
}

impl Vm {
    /// Save a quiescent compiled world. Mutable buffers, vectors, and captured
    /// cells are deliberately unsupported rather than silently flattened.
    /// `config_digest` is a digest of ordered configuration inputs, not a path.
    pub fn publish_code_image(
        &self,
        store: &Path,
        entry: usize,
        config_digest: [u8; 32],
    ) -> Result<String> {
        self.require_live_function(entry)?;
        if self.image.is_some()
            || !self.cells.is_empty()
            || !self.dynamic.is_empty()
            || !self.stack.is_empty()
        {
            return Err("code image requires a private quiescent world without cells".into());
        }
        // The trap set is runtime state an image does not carry yet.
        if !self.trapped.is_empty() {
            return Err("VM2 code image of a world with variable watchers is unsupported".into());
        }
        let mut builder = ImageBuilder::default();
        builder.string_raw(""); // directory is always string zero
        for _ in &self.functions {
            builder.string_raw("");
        }
        let mut objects = Vec::new();
        for o in &self.objects {
            if matches!(o, data::Object::CharTable(_) | data::Object::SubCharTable(_)) {
                return Err("code image cannot publish character tables; native graph serialization is not supported".into());
            }
            let data::Object::String(s) = o else {
                return Err("code image supports only immutable string/cons literals".into());
            };
            if !s.properties.is_empty() {
                return Err("code image cannot publish string text properties; property graph serialization is not supported".into());
            }
            objects.push(builder.string_raw(s.as_str()));
        }
        let relocate = |v: Value| -> Result<Value> {
            Ok(match v.tag() {
                3 => Value::reference(image::IMAGE | v.index(3)?, 3),
                0 if v != Value::NIL => *objects
                    .get(v.index(0)? - 1)
                    .ok_or("invalid literal object")?,
                5 => return Err("code image cannot retain captured cells".into()),
                _ => v,
            })
        };
        for &(a, b) in &self.conses {
            if [a, b].iter().any(|v| matches!(v.tag(), 4 | 5)) {
                return Err("immutable cons cannot reference private closure/cell".into());
            }
            builder.cons_raw(relocate(a)?, relocate(b)?);
        }
        let mut symbols = vec![Json::Null; self.slots.len()];
        for (i, name) in self.symbol_names.iter().enumerate() {
            let slot = &self.slots[i];
            let mut record = vec![
                json!(name),
                json!(relocate(slot.value)?.0),
                json!(relocate(slot.function)?.0),
                json!(slot.declared_special || (self.specials.contains(name) && !self.generated_slots.contains_key(&i))),
            ];
            if self.generated_slots.contains_key(&i) {
                record.push(json!(true));
            }
            symbols[i] = Json::Array(record);
        }
        // GNU `u.s.plist`: each symbol object's own property list, by slot.
        let mut plists = Vec::new();
        for (i, slot) in self.slots.iter().enumerate() {
            if slot.local != Value::NIL {
                return Err("code image cannot publish buffer-local bindings".into());
            }
            if slot.plist != Value::NIL {
                plists.push(json!([i, relocate(slot.plist)?.0]));
            }
        }
        let mut functions = Vec::new();
        // Generated, uninterned slots are existing GC roots. They keep literal
        // identity stable while cold bodies remain undecoded, even if the
        // original variable/plist is changed before a body's first call.
        let mut literal_slots = HashMap::default();
        for (id, f) in self.functions.iter().enumerate() {
            // Keep stable image ordinals without serializing retired request IR.
            if !self.request_code.live[id] {
                builder.strings[id + 1] = json!({"ops":[encode_op(Op::Const(0, Value::NIL)), encode_op(Op::Return(0))],"sites":[],"roots":{}}).to_string();
                functions.push(json!([1, 0, 0, 0, false]));
                continue;
            }
            let mut code = Vec::new();
            let mut roots = BTreeMap::new();
            for &op in &f.code {
                let mut op = op;
                let mut error = None;
                op.visit_values(&mut |v| match relocate(*v) {
                    Ok(new) => *v = new,
                    Err(e) => error = Some(e),
                });
                if let Some(e) = error {
                    return Err(e);
                }
                let mut private = false;
                op.visit_values(&mut |v| private |= matches!(v.tag(), 4 | 5));
                if private {
                    return Err("cold code cannot retain moving closure/cell values".into());
                }
                op.visit_values(&mut |v| {
                    if Self::image_value(*v) {
                        let slot = *literal_slots.entry(v.0).or_insert_with(|| {
                            let slot = symbols.len();
                            symbols.push(json!(["image-literal", v.0, Value::NIL.0, false, true]));
                            slot
                        });
                        roots.insert(v.0, slot);
                    }
                });
                code.push(encode_op(op));
            }
            builder.strings[id + 1] = json!({"ops":code,"sites":f.sites,"roots":roots}).to_string();
            functions.push(json!([f.regs, f.captures, f.arity, f.required, f.rest]));
        }
        let closures: Vec<_> = self
            .closures
            .iter()
            .map(|c| {
                let captures: Result<Vec<_>> = c
                    .captures
                    .iter()
                    .map(|&v| relocate(v).map(|v| v.0))
                    .collect();
                let metadata = c.metadata.iter().map(|&v| relocate(v).map(|v| v.0)).collect::<Result<Vec<_>>>()?;
                Ok(json!([c.function, captures?, metadata, relocate(c.document)?.0]))
            })
            .collect::<Result<_>>()?;
        let mut macros: Vec<_> = self.macros.iter().copied().collect();
        macros.sort_unstable();
        let mut properties = Vec::new();
        let mut entries: Vec<_> = self.conditions.entries.iter().collect();
        entries.sort_by_key(|(id, _)| **id);
        for (id, entry) in entries {
            for &(key, value) in &entry.properties {
                properties.push(json!([id.value().0, relocate(key)?.0, relocate(value)?.0]));
            }
        }
        builder.strings[0] = json!({"macros":macros,"properties":properties,"plists":plists,"abi":ABI,"compiler":env!("VM2_COMPILER_ID"),"config":config_digest.to_vec(),"entry":entry,"symbols":symbols,"functions":functions,"closures":closures,"caches":self.caches.len()}).to_string();
        // Each function starts on its own page. Padding belongs to the preceding
        // record and is not parsed or authenticated until that body is requested.
        // This prevents directory lookup from faulting in every cold body.
        let blob = 64 + (builder.conses.len() + builder.strings.len()) * 16;
        let mut at = blob;
        for s in &mut builder.strings[..self.functions.len() + 1] {
            at += s.len();
            let padding = (4096 - at % 4096) % 4096;
            s.extend(std::iter::repeat_n(' ', padding));
            at += padding;
        }
        builder.publish(store, config_digest)
    }

    /// Load metadata and stable symbol slots only. No source reader/compiler,
    /// body decoding, or threaded lowering runs here. Config mismatch fails closed.
    pub fn from_code_image(
        image: Arc<ImageRegion>,
        config_digest: [u8; 32],
    ) -> Result<(Self, usize)> {
        let directory: Json = serde_json::from_str(image.string(0)?).map_err(|e| e.to_string())?;
        if field(&directory, "abi")?.as_str() != Some(ABI)
            || field(&directory, "compiler")?.as_str() != Some(env!("VM2_COMPILER_ID"))
        {
            return Err("executable image compiler/ABI mismatch".into());
        }
        if words(field(&directory, "config")?)?
            != config_digest
                .iter()
                .map(|&b| b as usize)
                .collect::<Vec<_>>()
        {
            return Err("executable image configuration mismatch".into());
        }
        let mut vm = Self::new();
        let mut names = vec![String::new(); vm.names.len()];
        for (name, &i) in &vm.names {
            names[i] = name.clone();
        }
        vm.names.clear();
        vm.initial_obarray.clear();
        vm.symbol_names.clear();
        vm.slots.clear();
        // Keep intrinsic specials, including lazy annotation variables that may
        // not have been interned in the saved world yet.
        for (i, symbol) in array(field(&directory, "symbols")?)?.iter().enumerate() {
            let s = array(symbol)?;
            if !(4..=5).contains(&s.len()) {
                return Err("invalid symbol record".into());
            }
            let generated = if s.len() == 5 {
                s[4].as_bool().ok_or("invalid generated symbol flag")?
            } else {
                false
            };
            let name = s[0].as_str().ok_or("invalid symbol name")?;
            if generated {
                if i < names.len() {
                    return Err("invalid generated symbol metadata".into());
                }
                let id = expand::generated_id();
                vm.generated_symbols.insert(id, i);
                vm.generated_slots.insert(i, id);
            } else if vm.names.insert(name.into(), i).is_some() {
                return Err("duplicate image symbol".into());
            } else {
                vm.set_initial_obarray(i, true);
            }
            vm.symbol_names.push(name.into());
            let (v, f) = (value(&s[1])?, value(&s[2])?);
            if i < names.len() && name != names[i] {
                return Err("primitive symbol ABI mismatch".into());
            }
            vm.slots.push(Slot { value: v, function: f, ..Slot::new() });
            if s[3].as_bool().ok_or("invalid special flag")? {
                vm.slots[i].declared_special = true;
                if !generated { vm.specials.insert(name.into()); }
            }
        }
        if vm.slots.len() < names.len() {
            return Err("missing primitive symbols".into());
        }
        for slot in words(field(&directory, "macros")?)? {
            if slot >= vm.slots.len() {
                return Err("invalid image macro slot".into());
            }
            vm.macros.insert(slot);
        }
        for (id, meta) in array(field(&directory, "functions")?)?.iter().enumerate() {
            let m = array(meta)?;
            if m.len() != 5 {
                return Err("invalid function metadata".into());
            }
            let f = Function {
                capture_mutations: Vec::new(),
                mapped_code: Some(id + 1),
                code: vec![],
                sites: vec![],
                regs: number(&m[0])?,
                captures: number(&m[1])?,
                arity: number(&m[2])?,
                required: number(&m[3])?,
                rest: m[4].as_bool().ok_or("invalid rest flag")?,
                lazy: None,
                dynamic: None,
            };
            if f.regs == 0
                || f.required > f.arity
                || f.captures
                    .checked_add(f.arity)
                    .and_then(|n| n.checked_add(usize::from(f.rest)))
                    .is_none_or(|n| n > f.regs)
                || f.regs > 1_000_000
            {
                return Err("invalid register/argument metadata".into());
            }
            vm.install_function(f);
        }
        for closure in array(field(&directory, "closures")?)? {
            let c = array(closure)?;
            if c.len() != 4 {
                return Err("invalid closure record".into());
            }
            let function = number(&c[0])?;
            let captures = array(&c[1])?
                .iter()
                .map(value)
                .collect::<Result<Vec<_>>>()?;
            if vm
                .functions
                .get(function)
                .is_none_or(|f| f.captures != captures.len())
            {
                return Err("invalid closure metadata".into());
            }
            let metadata = array(&c[2])?.iter().map(value).collect::<Result<Vec<_>>>()?;
            vm.closures.push(Closure { function, captures, metadata, document: value(&c[3])? });
        }
        vm.request_code.image_caches = number(field(&directory, "caches")?)?;
        vm.caches = (0..vm.request_code.image_caches)
            .map(|_| Cache {
                version: 0,
                target: Value::UNBOUND,
            })
            .collect();
        image
            .symbol_slots
            .store(vm.slots.len(), std::sync::atomic::Ordering::Release);
        vm.image = Some(image);
        for record in array(field(&directory, "properties")?)? {
            let record = array(record)?;
            if record.len() != 3 {
                return Err("invalid image property record".into());
            }
            let symbol = value(&record[0])?;
            vm.validate_code_value(symbol)?;
            let id = conditions::SymbolId::new(symbol).ok_or("invalid property symbol")?;
            let key = value(&record[1])?;
            let value = value(&record[2])?;
            vm.validate_code_value(key)?;
            vm.validate_code_value(value)?;
            vm.conditions.put(id, key, value);
        }
        for record in array(field(&directory, "plists")?)? {
            let record = array(record)?;
            if record.len() != 2 {
                return Err("invalid image plist record".into());
            }
            let slot = number(&record[0])?;
            if slot >= vm.slots.len() {
                return Err("invalid image plist slot".into());
            }
            vm.slots[slot].plist = value(&record[1])?;
        }
        for s in &vm.slots {
            vm.validate_code_value(s.value)?;
            vm.validate_code_value(s.function)?;
            vm.validate_code_value(s.plist)?;
        }
        for c in &vm.closures {
            for &v in c.captures.iter().chain(c.metadata.iter()).chain(std::iter::once(&c.document)) {
                vm.validate_code_value(v)?;
            }
        }
        let entry = number(field(&directory, "entry")?)?;
        if vm
            .functions
            .get(entry)
            .is_none_or(|f| f.arity != 0 || f.captures != 0)
        {
            return Err("invalid image entry".into());
        }
        vm.restore_private_image_roots()?;
        Ok((vm, entry))
    }
    // Every reachable Lisp object is strand-owned; only encoded bytes stay
    // shared. One memo covers data roots and the generated cold-literal roots.
    fn restore_private_image_roots(&mut self) -> Result<()> {
        let mut copies = HashMap::default();
        let mut pending = Vec::new();
        for slot in 0..self.slots.len() {
            let value = self.private_image_root(self.slots[slot].value, &mut copies, &mut pending)?;
            let function = self.private_image_root(self.slots[slot].function, &mut copies, &mut pending)?;
            self.slots[slot].value = value;
            self.slots[slot].function = function;
            self.slots[slot].plist = self.private_image_root(self.slots[slot].plist, &mut copies, &mut pending)?;
        }
        for index in 0..self.closures.len() {
            self.closures[index].document = self.private_image_root(self.closures[index].document, &mut copies, &mut pending)?;
            for slot in 0..self.closures[index].metadata.len() {
                self.closures[index].metadata[slot] = self.private_image_root(self.closures[index].metadata[slot], &mut copies, &mut pending)?;
            }
            for capture in 0..self.closures[index].captures.len() {
                self.closures[index].captures[capture] = self.private_image_root(self.closures[index].captures[capture], &mut copies, &mut pending)?;
            }
        }
        let mut conditions = std::mem::take(&mut self.conditions);
        let mut error = None;
        conditions.visit_values(&mut |v| match self.private_image_root(*v, &mut copies, &mut pending) {
            Ok(private) => *v = private,
            Err(e) => error = Some(e),
        });
        self.conditions = conditions;
        if let Some(error) = error { return Err(error); }
        while let Some((source, destination)) = pending.pop() {
            let (car, cdr) = self.cons_pair(source)?;
            let car = self.private_image_root(car, &mut copies, &mut pending)?;
            let cdr = self.private_image_root(cdr, &mut copies, &mut pending)?;
            self.conses[destination] = (car, cdr);
        }
        Ok(())
    }
    fn private_image_root(
        &mut self,
        value: Value,
        copies: &mut HashMap<u64, Value>,
        pending: &mut Vec<(Value, usize)>,
    ) -> Result<Value> {
        if !Self::image_value(value) {
            return Ok(value);
        }
        if let Some(&private) = copies.get(&value.0) {
            return Ok(private);
        }
        self.validate_code_value(value)?;
        let private = if value.tag() == 3 {
            let index = self.conses.len();
            self.conses.push((Value::NIL, Value::NIL));
            pending.push((value, index));
            Value::reference(index, 3)
        } else {
            let bytes = self.string_value(value)?.as_bytes().to_vec();
            self.make_lisp_string(bytes, true)
        };
        copies.insert(value.0, private);
        Ok(private)
    }
    fn validate_code_value(&self, v: Value) -> Result<()> {
        let index = v.payload() as usize;
        let valid = match v.tag() {
            1 => true,
            0 => {
                v == Value::NIL
                    || Self::image_value(v)
                        && self
                            .image
                            .as_ref()
                            .unwrap()
                            .string_reference_raw(index & !image::IMAGE)
                            .is_ok()
            }
            2 => v == Value::TRUE,
            3 => {
                Self::image_value(v)
                    && self
                        .image
                        .as_ref()
                        .unwrap()
                        .reference_raw(index & !image::IMAGE)
                        .is_ok()
            }
            4 => index < self.closures.len(),
            6 => prim::descriptor(index).is_some(),
            7 => index <= self.slots.len(),
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("invalid executable image value".into())
        }
    }
    pub(super) fn decode_image_function(&self, id: usize) -> Result<Function> {
        let f = self.functions.get(id).ok_or("invalid image function")?;
        let index = f.mapped_code.ok_or("function is not mapped")?;
        let body: Json = serde_json::from_str(
            self.image
                .as_ref()
                .ok_or("missing code image")?
                .string(index)?,
        )
        .map_err(|e| e.to_string())?;
        let code = array(field(&body, "ops")?)?
            .iter()
            .map(decode_op)
            .collect::<Result<Vec<_>>>()?;
        let sites = array(field(&body, "sites")?)?
            .iter()
            .map(words)
            .collect::<Result<Vec<_>>>()?;
        let f = Function {
            capture_mutations: Vec::new(),
            mapped_code: None,
            code,
            sites,
            regs: f.regs,
            captures: f.captures,
            arity: f.arity,
            required: f.required,
            rest: f.rest,
            lazy: None,
            dynamic: f.dynamic,
        };
        self.validate_image_function(&f)?;
        let roots = field(&body, "roots")?.as_object().ok_or("invalid image literal roots")?;
        let mut f = f;
        let mut error = None;
        for op in &mut f.code {
            op.visit_values(&mut |v| {
                if Self::image_value(*v) {
                    let private = roots.get(&v.0.to_string()).ok_or("missing image literal root")
                        .and_then(|slot| number(slot).map_err(|_| "invalid image literal root"))
                        .and_then(|slot| self.slots.get(slot).map(|s| s.value).ok_or("invalid image literal slot"));
                    match private {
                        Ok(private) => *v = private,
                        Err(e) => error = Some(e),
                    }
                }
            });
        }
        if let Some(error) = error { return Err(error.into()); }
        Ok(f)
    }
    pub(super) fn materialize_image_function(&mut self, id: usize) -> Result<()> {
        if self
            .functions
            .get(id)
            .ok_or("invalid function")?
            .mapped_code
            .is_some()
        {
            self.functions[id] = Arc::new(self.decode_image_function(id)?);
        }
        Ok(())
    }
    fn validate_image_function(&self, f: &Function) -> Result<()> {
        let reg = |r| r < f.regs;
        let slot = |s| s < self.slots.len();
        let target = |pc| pc < f.code.len();
        let args = |a: usize, n: usize| a.checked_add(n).is_some_and(|end| end <= f.regs);
        if !matches!(f.code.last(), Some(Op::Return(_) | Op::Jump(_))) {
            return Err("image function can fall off end".into());
        }
        for site in &f.sites {
            if !site.iter().all(|&r| reg(r)) {
                return Err("invalid image capture register".into());
            }
        }
        for &op in &f.code {
            let mut copy = op;
            let mut private = false;
            copy.visit_values(&mut |v| private |= matches!(v.tag(), 4 | 5));
            if private {
                return Err("cold code cannot retain moving values".into());
            }
            let valid = match op {
                Op::Const(d, v) => {
                    self.validate_code_value(v)?;
                    reg(d)
                }
                Op::Move(d, s) | Op::ReadCell(d, s) | Op::WriteCell(d, s) => reg(d) && reg(s),
                Op::Cell(r) | Op::Return(r) => reg(r),
                Op::Global(d, s) | Op::ResolveHead(d, s) => reg(d) && slot(s),
                Op::Prepared(d, t, s, c, a, n) => reg(d) && reg(t) && slot(s) && c < self.caches.len() && args(a, n),
                Op::Sealed(d, s, a, n) => reg(d) && slot(s) && args(a, n),
                Op::SetGlobal(s, r) | Op::Bind(s, r) | Op::Define(s, r) | Op::DefineMacro(s, r) => {
                    slot(s) && reg(r)
                }
                Op::Unbind(_) | Op::PopHandlers(_) | Op::ResumeExit => true,
                Op::DeclareSpecial(s) => slot(s),
                Op::LeaveProtect(r) => reg(r),
                Op::Protect(d, cleanup, end) => reg(d) && target(cleanup) && target(end),
                Op::Jump(t) => target(t),
                Op::Unless(r, t) => reg(r) && target(t),
                Op::Closure(d, id, site) => {
                    reg(d)
                        && self.functions.get(id).is_some_and(|other| {
                            f.sites.get(site).is_some_and(|s| s.len() == other.captures)
                        })
                }
                Op::Call(d, t, a, n) => reg(d) && reg(t) && args(a, n),
                Op::Direct(d, s, c, a, n) => {
                    reg(d) && slot(s) && c < self.caches.len() && args(a, n)
                }
                Op::Catch(d, r, t) => reg(d) && reg(r) && target(t),
                Op::Condition(d, v, t) => {
                    self.validate_code_value(v)?;
                    reg(d) && target(t)
                }
            };
            if !valid {
                return Err("invalid executable image operand".into());
            }
        }
        Ok(())
    }
    pub fn image_functions_materialized(&self) -> usize {
        self.functions
            .iter()
            .filter(|f| f.mapped_code.is_none())
            .count()
    }
}

pub(crate) fn encode_op(op: Op) -> Json {
    use Op::*;
    match op {
        Const(a, v) => json!([0, a, v.0]),
        Move(a, b) => json!([1, a, b]),
        Cell(a) => json!([2, a]),
        ReadCell(a, b) => json!([3, a, b]),
        WriteCell(a, b) => json!([4, a, b]),
        Global(a, b) => json!([5, a, b]),
        SetGlobal(a, b) => json!([6, a, b]),
        Bind(a, b) => json!([7, a, b]),
        Unbind(a) => json!([8, a]),
        Jump(a) => json!([9, a]),
        Unless(a, b) => json!([10, a, b]),
        Closure(a, b, c) => json!([11, a, b, c]),
        Define(a, b) => json!([12, a, b]),
        Call(a, b, c, d) => json!([13, a, b, c, d]),
        Direct(a, b, c, d, e) => json!([14, a, b, c, d, e]),
        Return(a) => json!([15, a]),
        Catch(a, b, c) => json!([16, a, b, c]),
        Condition(a, v, c) => json!([17, a, v.0, c]),
        PopHandlers(a) => json!([18, a]),
        DeclareSpecial(a) => json!([20, a]),
        DefineMacro(a, b) => json!([19, a, b]),
        Protect(a, b, c) => json!([21, a, b, c]),
        LeaveProtect(a) => json!([22, a]),
        ResumeExit => json!([23]),
        ResolveHead(a, b) => json!([24, a, b]),
        Prepared(a, b, c, d, e, f) => json!([25, a, b, c, d, e, f]),
        Sealed(a, b, c, d) => json!([26, a, b, c, d]),
    }
}
pub(crate) fn decode_op(v: &Json) -> Result<Op> {
    use Op::*;
    let w = words(v)?;
    Ok(match w.as_slice() {
        [0, a, v] => Const(*a, Value(*v as u64)),
        [1, a, b] => Move(*a, *b),
        [2, a] => Cell(*a),
        [3, a, b] => ReadCell(*a, *b),
        [4, a, b] => WriteCell(*a, *b),
        [5, a, b] => Global(*a, *b),
        [6, a, b] => SetGlobal(*a, *b),
        [7, a, b] => Bind(*a, *b),
        [8, a] => Unbind(*a),
        [9, a] => Jump(*a),
        [10, a, b] => Unless(*a, *b),
        [11, a, b, c] => Closure(*a, *b, *c),
        [12, a, b] => Define(*a, *b),
        [13, a, b, c, d] => Call(*a, *b, *c, *d),
        [14, a, b, c, d, e] => Direct(*a, *b, *c, *d, *e),
        [15, a] => Return(*a),
        [16, a, b, c] => Catch(*a, *b, *c),
        [17, a, v, c] => Condition(*a, Value(*v as u64), *c),
        [18, a] => PopHandlers(*a),
        [20, a] => DeclareSpecial(*a),
        [19, a, b] => DefineMacro(*a, *b),
        [21, a, b, c] => Protect(*a, *b, *c),
        [22, a] => LeaveProtect(*a),
        [23] => ResumeExit,
        [24, a, b] => ResolveHead(*a, *b),
        [25, a, b, c, d, e, f] => Prepared(*a, *b, *c, *d, *e, *f),
        [26, a, b, c, d] => Sealed(*a, *b, *c, *d),
        _ => return Err("invalid image opcode".into()),
    })
}

#[cfg(test)]
mod image_alias_tests {
    use super::*;
    #[test]
    fn root_to_cold_constant_and_plist_identity_survives_restore() {
        let path = std::env::temp_dir().join(format!("image-alias-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let mut source = Vm::new();
        source.eval_source("(setq freeze-shared '(1 2)) (setplist 'freeze-owner freeze-shared)").unwrap();
        let root_slot = source.names["freeze-shared"];
        let plist_slot = source.names["freeze-owner"];
        let root = source.slots[root_slot].value;
        let owner = conditions::SymbolId::new(Value::reference(plist_slot + 1, 7)).unwrap();
        source.conditions.put(owner, Value::TRUE, root);
        let entry = source.compile(&Form::List(vec![Form::Sym("quote".into()), Form::Object(handle::FormObject::new(&source, root))])).unwrap();
        assert_eq!(source.execute_raw(entry).unwrap(), root, "PRE_IMAGE_COLD_ALIAS_IDENTITY");
        assert_eq!(source.slots[plist_slot].plist, root, "PRE_IMAGE_PLIST_ALIAS_IDENTITY");
        let key = source.publish_code_image(&path, entry, [63; 32]).unwrap();
        let image = unsafe { ImageRegion::open(&path, &key) }.unwrap();
        let (mut loaded, entry) = Vm::from_code_image(image, [63; 32]).unwrap();
        loaded.collect_garbage();
        let cold = loaded.execute_raw(entry).unwrap();
        let root = loaded.slots[root_slot].value;
        let plist = loaded.slots[plist_slot].plist;
        assert_eq!(root, cold, "ROOT_TO_COLD_ALIAS_IDENTITY");
        assert_eq!(root, plist, "ROOT_TO_PLIST_ALIAS_IDENTITY");
        assert_eq!(cold, plist, "COLD_TO_PLIST_ALIAS_PRESERVED");
        assert_eq!(root, loaded.conditions.entries[&owner].properties[0].1, "ROOT_TO_CONDITION_PROPERTY_IDENTITY");
        loaded.eval_source("(put 'freeze-owner 1 9)").unwrap();
        loaded.collect_garbage();
        assert_eq!(loaded.slots[root_slot].value, loaded.slots[plist_slot].plist, "PLIST_PUT_PRESERVES_ALIAS");
        assert_eq!(loaded.display(loaded.slots[root_slot].value).unwrap(), "(1 9)", "PLIST_PUT_MUTATES_SHARED_ROOT");
        std::fs::remove_dir_all(path).unwrap();
    }
}
