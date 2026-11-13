//! Lisp annotation resolution. Raw per-key presence remains a storage concern.
use crate::{coordinates::*, data::Object, *};
use crate::indirect::Shared;

const YANK_EXCLUDED: &[&str] = &[
    "category",
    "field",
    "follow-link",
    "fontified",
    "font-lock-face",
    "help-echo",
    "intangible",
    "invisible",
    "keymap",
    "local-map",
    "mouse-face",
    "read-only",
    "yank-handler",
];

fn annotation_definition(name: &str) -> Option<serde_json::Value> {
    Some(match name {
        "kill-region" => serde_json::json!([
            ["beg", "end", "&optional", "region"],
            ["kill-new", ["buffer-substring", "beg", "end"]],
            ["delete-region", "beg", "end"],
            "nil"
        ]),
        "kill-ring-save" => serde_json::json!([
            ["beg", "end", "&optional", "region"],
            ["kill-new", ["buffer-substring", "beg", "end"]]
        ]),
        "yank" => serde_json::json!([
            ["&optional", "arg"],
            ["setq", "this-command", ["quote", "yank"]],
            [
                "insert-for-yank",
                ["current-kill", ["if", "arg", ["1-", "arg"], 0]]
            ]
        ]),
        "insert-for-yank" => serde_json::json!([
            ["string"],
            [
                "while",
                "string",
                [
                    "let",
                    [
                        ["part", ["#:vm2-yank-part", "string"]],
                        ["start", ["point"]]
                    ],
                    ["setq", "yank-undo-function", "t"],
                    [
                        "if",
                        ["aref", "part", 2],
                        ["funcall", ["aref", "part", 2], ["aref", "part", 1]],
                        ["insert", ["aref", "part", 1]]
                    ],
                    ["#:vm2-yank-finish", "start", ["point"], ["aref", "part", 0]],
                    [
                        "setq",
                        "string",
                        [
                            "if",
                            ["aref", "part", 3],
                            ["substring", "string", ["aref", "part", 3]],
                            "nil"
                        ]
                    ]
                ]
            ]
        ]),
        "#:vm2-yank-finish" => serde_json::json!([
            ["start", "end", "handler"],
            [
                "let",
                [[
                    "exclude",
                    ["if", ["car", ["cdr", ["cdr", "handler"]]], "nil", "t"]
                ]],
                [
                    "when",
                    "exclude",
                    [
                        "dolist",
                        ["entry", "yank-handled-properties"],
                        [
                            "let",
                            [["at", "start"]],
                            [
                                "while",
                                ["<", "at", "end"],
                                [
                                    "let",
                                    [[
                                        "run",
                                        ["#:vm2-yank-property-run", "at", "end", ["car", "entry"]]
                                    ]],
                                    [
                                        "funcall",
                                        ["cdr", "entry"],
                                        ["aref", "run", 0],
                                        "at",
                                        ["aref", "run", 1]
                                    ],
                                    ["setq", "at", ["aref", "run", 1]]
                                ]
                            ]
                        ]
                    ]
                ],
                ["#:vm2-yank-finalize", "start", "end", "handler", "exclude"]
            ]
        ]),
        _ => return None,
    })
}

impl Vm {
    // Initialize allocation-bearing defaults only when first interned. VMs that
    // never use annotations need not retain an otherwise unused Lisp graph.
    pub(super) fn initialize_annotation_slot(&mut self, name: &str, slot: usize) {
        if name == "text-property-default-nonsticky" {
            let mut entries = vec![];
            for key in ["composition", "syntax-table", "display"] {
                let key = Value::reference(self.slot(key) + 1, 7);
                let value = self.alloc_cons(key, Value::TRUE);
                entries.push(value);
            }
            self.slots[slot].value = self.list_value(&entries);
        } else if name == "yank-excluded-properties" {
            let entries: Vec<_> = YANK_EXCLUDED
                .iter()
                .map(|key| Value::reference(self.slot(key) + 1, 7))
                .collect();
            self.slots[slot].value = self.list_value(&entries);
        }
        if name == "yank-handled-properties" {
            let mut entries = vec![];
            for (key, function) in [
                ("font-lock-face", "yank-handle-font-lock-face-property"),
                ("category", "yank-handle-category-property"),
            ] {
                let key = Value::reference(self.slot(key) + 1, 7);
                let function = Value::reference(self.slot(function) + 1, 7);
                let entry = self.alloc_cons(key, function);
                entries.push(entry);
            }
            self.slots[slot].value = self.list_value(&entries);
        }
        if let Some(definition) = annotation_definition(name) {
            let form = Form::from_json(definition).expect("valid built-in annotation definition");
            let items = form.list().expect("annotation definition list");
            // These definitions belong to the VM, not the user's macro world.
            // In particular, interning from the reader must never execute a
            // user macro (and thereby collect its partially constructed datum).
            let macros = std::mem::take(&mut self.macros);
            let compiled = self.function(
                items[0].list().unwrap(),
                &items[1..],
                &NameMap::default(),
                &NameSet::default(),
            );
            self.macros = macros;
            let (function, _) = compiled.expect("valid built-in annotation function");
            let value = self.alloc_closure(Closure {
                function,
                captures: vec![],
                metadata: vec![], document: Value::NIL,
            });
            self.define_function(slot, value, false)
                .expect("built-in annotation function publication");
        }
    }
    pub(super) fn annotation_string(&self, value: Value) -> Result<&str> {
        self.str_view(value)
            .ok_or("string required")?
            .as_str()
            .ok_or_else(|| "not-unicode-string".into())
    }
    fn annotation_set_variable(&mut self, name: &str, value: Value) {
        let slot = self.slot(name);
        self.slots[slot].value = value;
    }
    fn annotation_yank_primitive(&mut self, p: Primitive, start: usize, n: usize) -> Result<Value> {
        use Primitive::*;
        match p {
            KillNew => {
                if !(1..=2).contains(&n) {
                    return Err("wrong primitive arity".into());
                }
                let string = self.stack[start];
                self.str_view(string).ok_or("string required")?;
                let old = self.annotation_variable("kill-ring");
                let values = self.list_values(old)?;
                let ring = if n == 2 && self.stack[start + 1] != Value::NIL && old != Value::NIL {
                    let at = self.writable_cons(old)?;
                    self.conses.set_car(at, string);
                    old
                } else {
                    let max =
                        usize::try_from(self.annotation_variable("kill-ring-max").as_integer()?)
                            .map_err(|_| "positive kill-ring-max required")?;
                    if max == 0 {
                        return Err("positive kill-ring-max required".into());
                    }
                    let ring = self.alloc_cons(string, old);
                    if values.len() >= max {
                        let mut tail = ring;
                        for _ in 1..max {
                            tail = self.cons_pair(tail)?.1;
                        }
                        let at = self.writable_cons(tail)?;
                        self.conses.set_cdr(at, Value::NIL);
                    }
                    ring
                };
                self.annotation_set_variable("kill-ring", ring);
                self.annotation_set_variable("kill-ring-yank-pointer", ring);
                Ok(Value::NIL)
            }
            CurrentKill => {
                if !(1..=2).contains(&n) {
                    return Err("wrong primitive arity".into());
                }
                let ring = self.annotation_variable("kill-ring");
                let values = self.list_values(ring)?;
                if values.is_empty() {
                    return Err("kill ring is empty".into());
                }
                let mut tail = ring;
                let pointer = self.annotation_variable("kill-ring-yank-pointer");
                let mut offset = 0;
                for i in 0..values.len() {
                    if tail == pointer {
                        offset = i;
                        break;
                    }
                    tail = self.cons_pair(tail)?.1;
                }
                let at = (offset as i64 + self.stack[start].as_integer()?)
                    .rem_euclid(values.len() as i64) as usize;
                if n == 1 || self.stack[start + 1] == Value::NIL {
                    let mut tail = ring;
                    for _ in 0..at {
                        tail = self.cons_pair(tail)?.1;
                    }
                    self.annotation_set_variable("kill-ring-yank-pointer", tail);
                }
                Ok(values[at])
            }
            YankPart => {
                if n != 1 {
                    return Err("wrong primitive arity".into());
                }
                let source = self.stack[start];
                let text = self.str_view(source).ok_or("string required")?.to_owned();
                let properties = if let Some(Object::String(s) | Object::RawString(s)) =
                    self.object_if(source)
                {
                    s.properties.clone()
                } else {
                    properties::Properties::new()
                };
                let key = Value::reference(self.slot("yank-handler") + 1, 7);
                let mut at = 0;
                let handler = self.annotation_resolve(&self.annotation_raw(source, 0)?, key)?;
                while at < text.bytes().len() {
                    at = properties
                        .next_change(ByteOffset(at), ByteOffset(text.bytes().len()))
                        .0;
                    if at < text.bytes().len() {
                        let c = data::LispString::from_bytes(
                            text.bytes()[..at].to_vec(),
                            text.is_multibyte(),
                        )
                        .len();
                        if self.annotation_resolve(&self.annotation_raw(source, c)?, key)?
                            != handler
                        {
                            break;
                        }
                    }
                }
                let fields = self.list_values(handler)?;
                let function = fields.first().copied().unwrap_or(Value::NIL);
                let string = if at == text.bytes().len() {
                    source
                } else {
                    let mut segment = data::LispString::from_bytes(
                        text.bytes()[..at].to_vec(),
                        text.is_multibyte(),
                    );
                    segment.properties = properties.slice(ByteOffset(0)..ByteOffset(at));
                    let object = if segment.is_utf8() {
                        Object::String(Box::new(segment))
                    } else {
                        Object::RawString(Box::new(segment))
                    };
                    self.allocate_object(object)
                };
                let param = fields
                    .get(1)
                    .copied()
                    .filter(|v| *v != Value::NIL)
                    .unwrap_or(string);
                let next = if at < text.bytes().len() {
                    Value::integer(
                        data::LispString::from_bytes(
                            text.bytes()[..at].to_vec(),
                            text.is_multibyte(),
                        )
                        .len() as i64,
                    )?
                } else {
                    Value::NIL
                };
                Ok(self.allocate_object(Object::Vector(vec![handler, param, function, next])))
            }
            YankPropertyRun => {
                if n != 3 {
                    return Err("wrong primitive arity".into());
                }
                let index = self.buffer_index()?;
                let object = self.slots[self.buffer_slot].value;
                let a = usize::try_from(self.stack[start].as_integer()?)
                    .map_err(|_| "index out of range")?;
                let end = usize::try_from(self.stack[start + 1].as_integer()?)
                    .map_err(|_| "index out of range")?;
                let key = self.stack[start + 2];
                let value = self.annotation_resolve(&self.annotation_raw(object, a)?, key)?;
                let Object::Buffer(t) = &self.objects[index] else {
                    return Err("text buffer required".into());
                };
                let mut at =
                    t.byte_of_char(CharOffset(a.checked_sub(1).ok_or("index out of range")?))?;
                let limit =
                    t.byte_of_char(CharOffset(end.checked_sub(1).ok_or("index out of range")?))?;
                if at > limit || limit > t.accessible().end {
                    return Err("index out of range".into());
                }
                while at < limit {
                    at = t.properties.next_change(at, limit);
                    if at < limit
                        && self.annotation_resolve(
                            &self.annotation_raw(object, t.char_of_byte(at)?.0 + 1)?,
                            key,
                        )? != value
                    {
                        break;
                    }
                }
                let end = Value::integer((t.char_of_byte(at)?.0 + 1) as i64)?;
                Ok(self.allocate_object(Object::Vector(vec![value, end])))
            }
            YankFontLockFace | YankCategory => {
                if n != 3 {
                    return Err("wrong primitive arity".into());
                }
                let value = self.stack[start];
                if value == Value::NIL
                    || p == YankFontLockFace
                        && self.annotation_variable("font-lock-defaults") != Value::NIL
                {
                    return Ok(Value::NIL);
                }
                let category = if p == YankCategory {
                    let plist = self.list_values(self.annotation_symbol_plist(value)?)?;
                    if !plist.len().is_multiple_of(2) {
                        return Err("property list required".into());
                    }
                    plist
                        .chunks_exact(2)
                        .map(|pair| Ok((self.annotation_symbol_name(pair[0])?.to_owned(), pair[1])))
                        .collect::<Result<Vec<_>>>()?
                } else {
                    vec![]
                };
                let index = self.buffer_index()?;
                let Object::Buffer(t) = self.objects.entry_mut(index) else {
                    return Err("text buffer required".into());
                };
                let offset = |v: Value| -> Result<CharOffset> {
                    Ok(CharOffset(
                        usize::try_from(v.as_integer()?)
                            .ok()
                            .and_then(|v| v.checked_sub(1))
                            .ok_or("index out of range")?,
                    ))
                };
                let a = t.byte_of_char(offset(self.stack[start + 1])?)?;
                let b = t.byte_of_char(offset(self.stack[start + 2])?)?;
                let range = a.min(b)..a.max(b);
                if range.start < t.accessible().start || range.end > t.accessible().end {
                    return Err("index out of range".into());
                }
                if p == YankFontLockFace {
                    t.put_property(range, "face", value);
                } else {
                    let mut at = range.start;
                    while at < range.end {
                        let end = t.properties.next_change(at, range.end);
                        let raw: Vec<_> = t
                            .properties
                            .properties_at(at)
                            .into_iter()
                            .map(|(k, v)| (k.to_owned(), v))
                            .collect();
                        let mut merged = properties::Properties::new();
                        let span = ByteOffset(0)..ByteOffset(end.0 - at.0);
                        for (key, value) in category.iter().rev() {
                            merged.put(span.clone(), key, *value);
                        }
                        for (key, value) in &raw {
                            merged.put(span.clone(), key, *value);
                        }
                        let old: Vec<_> = t.properties.properties_at(at).into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
                        for key in t.properties.properties_keys() {
                            t.properties.remove(at..end, &key);
                        }
                        t.properties.copy_into(at, &merged);
                        if old != t.properties.properties_at(at).into_iter().map(|(k, v)| (k.to_owned(), v)).collect::<Vec<_>>() { t.state.record_change(false); }
                        at = end;
                    }
                }
                Ok(Value::NIL)
            }
            YankFinalize => {
                if n != 4 {
                    return Err("wrong primitive arity".into());
                }
                let fields = self.list_values(self.stack[start + 2])?;
                let noexclude = self.stack[start + 3] == Value::NIL;
                let excluded = if noexclude {
                    Some(vec![])
                } else if let Some(i) = self.names.get("yank-excluded-properties") {
                    let value = self.slots[*i].value;
                    if value == Value::TRUE {
                        None
                    } else {
                        Some(
                            self.list_values(value)?
                                .into_iter()
                                .map(|v| self.annotation_symbol_name(v).map(str::to_owned))
                                .collect::<Result<Vec<_>>>()?,
                        )
                    }
                } else {
                    Some(YANK_EXCLUDED.iter().map(|v| (*v).to_owned()).collect())
                };
                let index = self.buffer_index()?;
                let Object::Buffer(t) = self.objects.entry_mut(index) else {
                    return Err("text buffer required".into());
                };
                let offset = |v: Value| -> Result<CharOffset> {
                    Ok(CharOffset(
                        usize::try_from(v.as_integer()?)
                            .ok()
                            .and_then(|v| v.checked_sub(1))
                            .ok_or("index out of range")?,
                    ))
                };
                let a = t.byte_of_char(offset(self.stack[start])?)?;
                let b = t.byte_of_char(offset(self.stack[start + 1])?)?;
                let range = a.min(b)..a.max(b);
                if !noexclude {
                    if let Some(keys) = excluded {
                        for key in keys {
                            t.remove_property(range.clone(), &key);
                        }
                    } else {
                        let keys: Vec<_> = t.properties.slice(range.clone()).properties_keys();
                        for key in keys {
                            t.remove_property(range.clone(), &key);
                        }
                    }
                }
                if b > a {
                    let last = t.byte_of_char(CharOffset(t.char_of_byte(b)?.0 - 1))?;
                    if !t.properties.properties_at(last).is_empty() {
                        t.put_property(last..b, "rear-nonsticky", Value::TRUE);
                    }
                }
                if self.annotation_variable("yank-undo-function") == Value::TRUE {
                    self.annotation_set_variable(
                        "yank-undo-function",
                        fields.get(3).copied().unwrap_or(Value::NIL),
                    );
                }
                if let Some(command) = fields.get(4).filter(|v| **v != Value::NIL) {
                    self.annotation_set_variable("this-command", *command);
                }
                Ok(Value::NIL)
            }
            _ => unreachable!(),
        }
    }
    fn annotation_member(&self, key: &str, value: Value) -> Result<bool> {
        if value == Value::NIL {
            return Ok(false);
        }
        if value.tag() != 3 {
            return Ok(true);
        }
        for v in self.list_values(value)? {
            if self.annotation_symbol_name(v)? == key {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn annotation_nonsticky(&self, key: &str) -> Result<Option<bool>> {
        let Some(slot) = self.names.get("text-property-default-nonsticky") else {
            return Ok(["composition", "syntax-table", "display"]
                .contains(&key)
                .then_some(true));
        };
        for entry in self.list_values(self.slots[*slot].value)? {
            let (symbol, flag) = self.cons_pair(entry)?;
            if self.annotation_symbol_name(symbol)? == key {
                return Ok(Some(flag != Value::NIL));
            }
        }
        Ok(None)
    }
    pub(super) fn annotation_inherited(
        &mut self,
        left: Vec<(String, Value)>,
        right: Vec<(String, Value)>,
        same_run: bool,
    ) -> Result<Vec<(String, Value)>> {
        let front_key = Value::reference(self.slot("front-sticky") + 1, 7);
        let rear_key = Value::reference(self.slot("rear-nonsticky") + 1, 7);
        let lf = self.annotation_resolve(&left, front_key)?;
        let lr = self.annotation_resolve(&left, rear_key)?;
        let rf = self.annotation_resolve(&right, front_key)?;
        let rr = self.annotation_resolve(&right, rear_key)?;
        // Interior insertion can retain an unchanged sticky run plist.
        if same_run && left == right && !(lr != Value::NIL && lr.tag() != 3) {
            let mut all = true;
            if lf == Value::NIL || lf.tag() == 3 {
                for (key, _) in &left {
                    if !self.annotation_member(key, lf)?
                        && (self.annotation_member(key, lr)?
                            || self.annotation_nonsticky(key)?.is_some())
                    {
                        all = false;
                        break;
                    }
                }
            }
            if all {
                return Ok(left);
            }
        }
        let mut out = vec![];
        let mut front = vec![];
        let mut rear = vec![];
        let keys: Vec<_> = right
            .iter()
            .chain(
                left.iter()
                    .filter(|(k, _)| !right.iter().any(|(r, _)| r == k)),
            )
            .map(|(k, _)| k.clone())
            .collect();
        for key in keys {
            if key == "front-sticky" || key == "rear-nonsticky" {
                continue;
            }
            let l = left.iter().find(|(k, _)| k == &key).map(|(_, v)| *v);
            let r = right.iter().find(|(k, _)| k == &key).map(|(_, v)| *v);
            let default = self.annotation_nonsticky(&key)?;
            let mut use_left =
                l.is_some() && !self.annotation_member(&key, lr)? && default != Some(true);
            let mut use_right = self.annotation_member(&key, rf)? || default == Some(false);
            // Stickiness can name an absent right key; only a present value
            // competes with the left, whose explicit nil must otherwise survive.
            if use_left && use_right && r.is_some() {
                if l == Some(Value::NIL) {
                    use_left = false;
                } else if r == Some(Value::NIL) {
                    use_right = false;
                }
            }
            let (value, f, rn) = if use_left {
                (l.unwrap(), lf, lr)
            } else if use_right && r.is_some() {
                (r.unwrap(), rf, rr)
            } else {
                if use_right {
                    front.push(Value::reference(self.slot(&key) + 1, 7));
                    if self.annotation_member(&key, rr)? {
                        rear.push(Value::reference(self.slot(&key) + 1, 7));
                    }
                }
                continue;
            };
            if self.annotation_member(&key, f)? {
                front.push(Value::reference(self.slot(&key) + 1, 7));
            }
            if self.annotation_member(&key, rn)? {
                rear.push(Value::reference(self.slot(&key) + 1, 7));
            }
            out.push((key, value));
        }
        if !rear.is_empty() {
            let value = self.list_value(&rear);
            out.insert(0, ("rear-nonsticky".into(), value));
        }
        let category_front = if let Some((_, cat)) = out.iter().find(|(k, _)| k == "category") {
            self.annotation_plist(self.annotation_symbol_plist(*cat)?, front_key)?
                == Some(Value::TRUE)
        } else {
            false
        };
        if !front.is_empty() && !category_front {
            let value = self.list_value(&front);
            out.insert(0, ("front-sticky".into(), value));
        }
        Ok(out)
    }
    /// insert-and-inherit's storage: the arguments at point, inheriting the
    /// sticky properties around it, before the markers there with BEFORE.
    pub(crate) fn annotation_insert_inherit(&mut self, start: usize, n: usize, before: bool) -> Result<Value> {
        let index = self.buffer_index()?;
        if matches!(self.objects[index], Object::ByteBuffer(_)) {
            return self.unibyte_insert_inherit(index, start, n, before);
        }
        let Object::Buffer(t) = &self.objects[index] else {
            return Err("text buffer required".into());
        };
        // The neighbours are the whole text's, a narrowing's edges
        // included, as offset_intervals sees them.
        let at = t.point;
        let end = ByteOffset(t.len_bytes());
        let same_run = at.0 > 0 && at < end && t.properties.same_run_state(ByteOffset(at.0 - 1), at);
        let owned = |pairs: Vec<(&str, Value)>| pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect::<Vec<_>>();
        let left = if at.0 > 0 {
            let before = t.byte_of_char(CharOffset(t.char_of_byte(at)?.0 - 1))?;
            owned(t.properties.properties_at(before))
        } else {
            vec![]
        };
        let right = if at < end { owned(t.properties.properties_at(at)) } else { vec![] };
        let inherited = self.annotation_inherited(left, right, same_run)?;
        self.buffer_primitive(if before { Primitive::InsertBeforeMarkers } else { Primitive::Insert }, start, n)?;
        let Object::Buffer(t) = self.objects.entry_mut(index) else {
            unreachable!()
        };
        let end = t.point;
        t.properties.inherit_inserted(at..end, &inherited);
        Ok(Value::NIL)
    }
    /// annotation_insert_inherit in unibyte buffer INDEX, whose bytes are
    /// its characters: the sticky properties of the bytes around point
    /// win over the inserted strings' own.
    fn unibyte_insert_inherit(&mut self, index: usize, start: usize, n: usize, before: bool) -> Result<Value> {
        let Object::ByteBuffer(b) = &self.objects[index] else { unreachable!("unibyte buffer") };
        // The neighbours are the whole text's, a narrowing's edges included.
        let (at, end) = (b.point, b.bytes.len());
        let same_run = at.0 > 0 && at.0 < end && b.properties.get(&ByteOffset(at.0 - 1)) == b.properties.get(&at);
        let pairs = |p: usize| b.properties.get(&ByteOffset(p)).map(|m| m.iter().map(|(k, &v)| (k.clone(), v)).collect()).unwrap_or_default();
        let left: Vec<(String, Value)> = if at.0 > 0 { pairs(at.0 - 1) } else { vec![] };
        let right: Vec<(String, Value)> = if at.0 < end { pairs(at.0) } else { vec![] };
        let inherited = self.annotation_inherited(left, right, same_run)?;
        self.buffer_primitive(if before { Primitive::InsertBeforeMarkers } else { Primitive::Insert }, start, n)?;
        let Object::ByteBuffer(b) = self.objects.entry_mut(index) else { unreachable!("unibyte buffer") };
        if !inherited.is_empty() {
            for p in at.0..b.point.0 {
                let values = b.properties.entry(ByteOffset(p)).or_default();
                values.extend(inherited.iter().cloned());
            }
        }
        Ok(Value::NIL)
    }
    fn annotation_symbol_name(&self, value: Value) -> Result<&str> {
        if value == Value::NIL {
            return Ok("nil");
        }
        if value == Value::TRUE {
            return Ok("t");
        }
        let slot = value.index(7)?.checked_sub(1).ok_or("symbol required")?;
        #[cfg(test)]
        self.variable_read_probes.set(self.variable_read_probes.get() + 1);
        // Only an interned symbol names a property: its name maps back to it.
        let name = self.symbol_names.get(slot).ok_or("symbol required")?;
        if self.names.get(name) == Some(&slot) { Ok(name.as_str()) } else { Err("symbol required".into()) }
    }
    /// The value of the plain variable NAME in the current buffer: its
    /// buffer-local binding there, else its default; nil when void.
    pub(super) fn annotation_variable(&self, name: &str) -> Value {
        let Some(&slot) = self.names.get(name) else { return Value::NIL };
        let table = self.slots[slot].local;
        let local = (table != Value::NIL)
            .then(|| self.eq_hash_get(table, self.slots[self.buffer_slot].value))
            .flatten()
            .and_then(|binding| binding.index(7).ok());
        let value = self.slots[local.map_or(slot, |index| index - 1)].value;
        if value == Value::UNBOUND { Value::NIL } else { value }
    }
    pub(super) fn annotation_plist(&self, mut plist: Value, key: Value) -> Result<Option<Value>> {
        for _ in 0..=self.list_limit() {
            if plist == Value::NIL {
                return Ok(None);
            }
            let (k, tail) = self.cons_pair(plist)?;
            if tail == Value::NIL {
                return Ok(None);
            }
            let (v, tail) = self.cons_pair(tail)?;
            if k == key {
                return Ok(Some(v));
            }
            plist = tail;
        }
        Err("circular list".into())
    }
    /// The slot holding SYMBOL's own data. nil and t are immediates in VM2;
    /// their symbol data lives in the slots named "nil" and "t", as their
    /// function cells and alias targets already do (prim.rs). `None` means
    /// nil/t before anything was stored there, i.e. an empty plist.
    fn plist_owner(&self, symbol: Value) -> Result<Option<usize>> {
        if symbol == Value::NIL || symbol == Value::TRUE {
            let name = if symbol == Value::NIL { "nil" } else { "t" };
            return Ok(self.names.get(name).copied());
        }
        if symbol.tag() != 7 || symbol == Value::UNBOUND {
            return Err("symbol required".into());
        }
        let slot = symbol.index(7)?.checked_sub(1).ok_or("symbol required")?;
        if slot >= self.slots.len() {
            return Err("symbol required".into());
        }
        Ok(Some(slot))
    }
    fn plist_owner_mut(&mut self, symbol: Value) -> Result<usize> {
        if symbol == Value::NIL || symbol == Value::TRUE {
            return Ok(self.slot(if symbol == Value::NIL { "nil" } else { "t" }));
        }
        Ok(self.plist_owner(symbol)?.expect("non-nil symbols own a slot"))
    }
    /// GNU `Fsymbol_plist` (data.c:818) after CHECK_SYMBOL: the plist owned by
    /// the symbol OBJECT. Two symbols sharing a name never share it.
    pub(super) fn symbol_plist(&self, symbol: Value) -> Result<Value> {
        Ok(self.plist_owner(symbol)?.map_or(Value::NIL, |slot| self.slots[slot].plist))
    }
    /// GNU `Fsetplist` (data.c:1029) after CHECK_SYMBOL: install NEWPLIST
    /// unchecked, exactly as given.
    pub(super) fn set_symbol_plist(&mut self, symbol: Value, plist: Value) -> Result<Value> {
        if self.fence.is_some() {
            self.fence_property(symbol, None, crate::world::Access::Write, None);
        }
        let slot = self.plist_owner_mut(symbol)?;
        self.slots[slot].plist = plist;
        Ok(plist)
    }
    fn annotation_symbol_plist(&self, symbol: Value) -> Result<Value> {
        self.symbol_plist(symbol)
    }
    /// GNU `plist_get` (fns.c:2632): `eq` keys, FOR_EACH_TAIL_SAFE, so a
    /// malformed or circular plist answers nil instead of signalling.
    pub(super) fn plist_lookup(&self, plist: Value, key: Value) -> Value {
        let mut walk = TailWalk::new(plist);
        let mut tail = plist;
        // A tail that is no cons ends the walk; testing the tag first spares
        // the error `cons_pair' would format for it.
        while tail.index_if(3).is_some() && let Ok((k, rest)) = self.cons_pair(tail) {
            let Some((v, next)) = rest.index_if(3).and_then(|_| self.cons_pair(rest).ok()) else { break };
            if k == key {
                return v;
            }
            tail = next;
            if walk.cycle(tail) {
                break;
            }
        }
        Value::NIL
    }
    /// `get` of a property from the symbol's own plist (GNU `Fget`,
    /// fns.c:2653; `overriding-plist-environment` is not consulted, see
    /// DIVERGENCES.md VM2-PLIST1).
    pub(super) fn symbol_property(&self, symbol: Value, key: Value) -> Result<Value> {
        Ok(self.plist_lookup(self.symbol_plist(symbol)?, key))
    }
    /// `put` for VM-internal callers, faults flattened to text.
    pub(super) fn put_symbol_property(&mut self, symbol: Value, key: Value, value: Value) -> Result<Value> {
        self.put_symbol_property_checked(symbol, key, value).map_err(|fault| match fault {
            PlistFault::Symbol(_) => "symbol required".to_owned(),
            PlistFault::Plistp(_) => "plistp".to_owned(),
            PlistFault::Circular(_) => "circular list".to_owned(),
        })
    }
    /// GNU `Fput` (fns.c:2734) over `plist_put` (fns.c:2703): replace an
    /// existing value in place, else append (KEY VALUE) at the END. A tail
    /// that is not a proper plist signals `plistp` with the whole plist; a
    /// cycle signals `circular-list` with the tail FOR_EACH_TAIL stopped at.
    pub(super) fn put_symbol_property_checked(
        &mut self,
        symbol: Value,
        key: Value,
        value: Value,
    ) -> std::result::Result<Value, PlistFault> {
        if self.fence.is_some() {
            self.fence_property(symbol, Some(key), crate::world::Access::Write, Some(value));
        }
        let owner = self.plist_owner_mut(symbol).map_err(|_| PlistFault::Symbol(symbol))?;
        loop {
            let plist = self.slots[owner].plist;
            let mut walk = TailWalk::new(plist);
            let mut prev = None;
            let mut tail = plist;
            let mut immutable = false;
            while let Ok((k, rest)) = self.cons_pair(tail) {
                let Ok((_, next)) = self.cons_pair(rest) else { break };
                if k == key {
                    if Self::image_value(rest) {
                        immutable = true;
                        break;
                    }
                    self.conses.set_car(rest.index(3).expect("private cons"), value);
                    return Ok(value);
                }
                prev = Some(rest);
                tail = next;
                if walk.cycle(tail) {
                    return Err(PlistFault::Circular(tail));
                }
            }
            if !immutable {
                if tail != Value::NIL {
                    return Err(PlistFault::Plistp(plist));
                }
                if !prev.is_some_and(Self::image_value) {
                    // No collection can run inside a primitive (pacing is
                    // checked only at strand wait points), and nothing read
                    // before this allocation is used after it except `prev`,
                    // a cons index the allocation does not move.
                    let cell = self.list_value(&[key, value]);
                    match prev {
                        None => self.slots[owner].plist = cell,
                        Some(prev) => self.conses.set_cdr(prev.index(3).expect("private cons"), cell),
                    }
                    return Ok(value);
                }
            }
            // A mapped code image's plist is immutable: give the symbol a
            // private copy of the proper prefix, then repeat on the copy,
            // which has no image cells left to mutate.
            let copy = self.copy_plist_prefix(plist);
            self.slots[owner].plist = copy;
        }
    }
    /// Fresh conses for every cell of PLIST up to its first non-cons tail,
    /// which is kept. Bounded by the heap size, so a cycle cannot hang it.
    fn copy_plist_prefix(&mut self, plist: Value) -> Value {
        let mut cars = Vec::new();
        let mut tail = plist;
        for _ in 0..=self.list_limit() {
            let Ok((car, cdr)) = self.cons_pair(tail) else { break };
            cars.push(car);
            tail = cdr;
        }
        let mut out = tail;
        for car in cars.into_iter().rev() {
            let cell = self.alloc_cons(car, out);
            out = cell;
        }
        out
    }
    /// Fobject_intervals' layout: the text of OBJECT, a string or buffer,
    /// as 0-based character runs of eq property values over all of it, as
    /// the printer shows them, each with its properties, most recently set
    /// first; None without an interval tree (text that never had
    /// properties).
    pub(crate) fn object_intervals(&self, object: Value) -> Result<Option<Vec<(usize, usize, Vec<(String, Value)>)>>> {
        fn runs(properties: &crate::properties::Properties, len: usize, char_of: impl Fn(usize) -> Result<usize>)
            -> Result<Option<Vec<(usize, usize, Vec<(String, Value)>)>>> {
            if !properties.has_intervals() || len == 0 { return Ok(None); }
            properties.runs(ByteOffset(0)..ByteOffset(len)).into_iter()
                .map(|(run, values)| Ok((char_of(run.start.0)?, char_of(run.end.0)?, values)))
                .collect::<Result<Vec<_>>>().map(Some)
        }
        if Self::image_string(object) { return Ok(None); }
        match self.object(object)? {
            Object::String(s) | Object::RawString(s) => {
                let offsets: Vec<usize> = (0..=s.len()).map(|i| s.byte_offset(CharOffset(i)).unwrap()).collect();
                runs(&s.properties, s.bytes().len(), |b| Ok(offsets.partition_point(|&o| o < b)))
            }
            Object::Bytes(_) => Ok(None),
            Object::Buffer(t) => runs(&t.properties, t.len_bytes(), |b| Ok(t.char_of_byte(ByteOffset(b))?.0)),
            Object::ByteBuffer(t) => {
                if t.had_intervals && t.properties.is_empty() && !t.bytes.is_empty() {
                    return Ok(Some(vec![(0, t.bytes.len(), Vec::new())]));
                }
                runs(&crate::properties::Properties::from_map(&t.properties), t.bytes.len(), Ok)
            }
            _ => Err("buffer-or-string-p".into()),
        }
    }
    /// The symbol a text property's key names.
    pub(crate) fn property_key_symbol(&mut self, key: &str) -> Value {
        match key {
            "nil" => Value::NIL,
            "t" => Value::TRUE,
            _ => Value::reference(self.slot(key) + 1, 7),
        }
    }
    pub(super) fn annotation_raw(&self, object: Value, at: usize) -> Result<Vec<(String, Value)>> {
        if Self::image_string(object) {
            if at > self.string_value(object)?.chars().count() {
                return Err("index out of range".into());
            }
            return Ok(vec![]);
        }
        let pairs = match self.object(object)? {
            Object::String(s) | Object::RawString(s) => {
                let b = s.byte_offset(CharOffset(at)).ok_or("index out of range")?;
                s.properties.properties_at(ByteOffset(b))
            }
            Object::Bytes(bytes) => {
                if at > bytes.len() {
                    return Err("index out of range".into());
                }
                return Ok(vec![]);
            }
            Object::Buffer(t) => {
                let c = at.checked_sub(1).ok_or("index out of range")?;
                let b = t.byte_of_char(CharOffset(c))?;
                let r = t.accessible();
                if b < r.start || b > r.end {
                    return Err("index out of range".into());
                }
                t.properties.properties_at(b)
            }
            Object::ByteBuffer(t) => {
                let b = ByteOffset(at.checked_sub(1).ok_or("index out of range")?);
                if b < t.restriction.start || b > t.restriction.end {
                    return Err("index out of range".into());
                }
                return Ok(t
                    .properties
                    .get(&b)
                    .map(|m| m.iter().map(|(k, v)| (k.clone(), *v)).collect())
                    .unwrap_or_default());
            }
            _ => return Err("buffer or string required".into()),
        };
        Ok(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
    }
    fn annotation_base(&self, raw: &[(impl AsRef<str>, Value)], key: Value) -> Result<Option<Value>> {
        let name = self.annotation_symbol_name(key)?;
        if let Some((_, v)) = raw.iter().find(|(k, _)| k.as_ref() == name) {
            return Ok(Some(*v));
        }
        if let Some((_, category)) = raw.iter().find(|(k, _)| k.as_ref() == "category") {
            let plist = self.annotation_symbol_plist(*category)?;
            // GNU category nil, unlike directly stored nil, does not mask aliases/defaults.
            return Ok(self
                .annotation_plist(plist, key)?
                .filter(|v| *v != Value::NIL));
        }
        Ok(None)
    }
    /// KEY's aliases in `char-property-alias-alist`: `assq`, stopping at
    /// KEY's entry and copying nothing of the others.
    fn annotation_aliases(&self, key: Value) -> Result<Vec<Value>> {
        let mut tail = self.annotation_variable("char-property-alias-alist");
        for _ in 0..=self.list_limit() {
            if tail == Value::NIL {
                return Ok(Vec::new());
            }
            let (entry, rest) = self.cons_pair(tail)?;
            let (alias_key, aliases) = self.cons_pair(entry)?;
            if alias_key == key {
                return self.list_values(aliases);
            }
            tail = rest;
        }
        Err("circular list".into())
    }
    /// The names of KEY's aliases, which decide
    /// [`Self::annotation_resolve`] of KEY where neither KEY nor `category`
    /// gives it.
    pub(super) fn annotation_alias_names(&self, key: Value) -> Result<Vec<&str>> {
        self.annotation_aliases(key)?.into_iter().map(|alias| self.annotation_symbol_name(alias)).collect()
    }
    /// The name KEY's own property layer has.
    pub(super) fn annotation_key_name(&self, key: Value) -> Result<&str> {
        self.annotation_symbol_name(key)
    }
    pub(super) fn annotation_resolve(&self, raw: &[(impl AsRef<str>, Value)], key: Value) -> Result<Value> {
        if let Some(v) = self.annotation_base(raw, key)? {
            return Ok(v);
        }
        for alias in self.annotation_aliases(key)? {
            if let Some(v) = self.annotation_base(raw, alias)?.filter(|v| *v != Value::NIL) {
                return Ok(v);
            }
        }
        Ok(self
            .annotation_plist(self.annotation_variable("default-text-properties"), key)?
            .unwrap_or(Value::NIL))
    }
    pub(super) fn text_annotation_primitive(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
    ) -> Result<Value> {
        use Primitive::*;
        if matches!(
            p,
            KillNew
                | CurrentKill
                | YankPart
                | YankFinalize
                | YankPropertyRun
                | YankFontLockFace
                | YankCategory
        ) {
            return self.annotation_yank_primitive(p, start, n);
        }
        if p == InsertInherit {
            return self.annotation_insert_inherit(start, n, false);
        }
        if p == BufferSubstring {
            if n != 2 {
                return Err("wrong primitive arity".into());
            }
            let a = self.scalar_position(self.stack[start])?;
            let b = self.scalar_position(self.stack[start + 1])?;
            let index = self.buffer_index()?;
            if let Object::ByteBuffer(bb) = &self.objects[index] {
                // A unibyte buffer's substring is unibyte, its positions
                // bytes, its properties the text's (make_buffer_string_both).
                let at = |v: i64| usize::try_from(v).ok().and_then(|v| v.checked_sub(1)).ok_or("index out of range");
                let (a, b) = (at(a)?, at(b)?);
                let range = ByteOffset(a.min(b))..ByteOffset(a.max(b));
                if range.start < bb.restriction.start || range.end > bb.restriction.end {
                    return Err("index out of range".into());
                }
                let object = bb.text_object(range);
                return Ok(self.allocate_object(object));
            }
            let Object::Buffer(t) = &self.objects[index] else {
                return Err("text buffer required".into());
            };
            let a = usize::try_from(a)
                .ok()
                .and_then(|v| v.checked_sub(1))
                .ok_or("index out of range")?;
            let b = usize::try_from(b)
                .ok()
                .and_then(|v| v.checked_sub(1))
                .ok_or("index out of range")?;
            let a = t.byte_of_char(CharOffset(a))?;
            let b = t.byte_of_char(CharOffset(b))?;
            let range = a.min(b)..a.max(b);
            if range.start < t.accessible().start || range.end > t.accessible().end {
                return Err("index out of range".into());
            }
            return Ok(self.allocate_object(data::string_object(t.lisp_string(range)?)));
        }
        if matches!(p, Get | Put) {
            if n != if p == Get { 2 } else { 3 } {
                return Err("wrong primitive arity".into());
            }
            let symbol = self.stack[start];
            let key = self.stack[start + 1];
            if p == Get {
                return self.symbol_property(symbol, key);
            }
            let value = self.stack[start + 2];
            return self.put_symbol_property(symbol, key, value);
        }
        let raw_query = p == RawPropertiesAt;
        if !(if raw_query {
            (1..=2).contains(&n)
        } else {
            (2..=3).contains(&n)
        }) {
            return Err("wrong primitive arity".into());
        }
        let object = if n == if raw_query { 2 } else { 3 } {
            self.stack[start + n - 1]
        } else {
            Value::NIL
        };
        let object = if object == Value::NIL {
            self.buffer_index()?;
            self.slots[self.buffer_slot].value
        } else {
            object
        };
        // validate_interval_range: CHECK_FIXNUM_COERCE_MARKER.
        let at =
            usize::try_from(self.scalar_position(self.stack[start])?).map_err(|_| "index out of range")?;
        let raw = self.annotation_raw(object, at)?;
        if raw_query {
            let mut values = vec![];
            for (key, value) in raw {
                values.extend([self.property_key_symbol(&key), value]);
            }
            Ok(self.list_value(&values))
        } else {
            self.annotation_resolve(&raw, self.stack[start + 1])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(vm: &mut Vm, v: serde_json::Value) -> Value {
        let f = vm.compile(&Form::from_json(v).unwrap()).unwrap();
        vm.execute_raw(f).unwrap()
    }
    #[test]
    fn category_alias_default_order_and_explicit_nil_match_gnu() {
        let mut vm = Vm::new();
        let value = run(
            &mut vm,
            serde_json::json!(["let", [["default-text-properties",["quote",["p",99]]],["char-property-alias-alist",["quote",[["p","q","r"],["q","r"]]]]],
          ["put",["quote","cat"],["quote","p"],11],
          ["let",[["s",{"string":"abcd"}]],
            ["put-text-property",0,1,["quote","category"],["quote","cat"],"s"],
            ["put-text-property",1,2,["quote","q"],22,"s"],
            ["put-text-property",1,2,["quote","r"],33,"s"],
            ["put-text-property",2,3,["quote","p"],"nil","s"],
            ["vector",["get-text-property",0,["quote","p"],"s"],["get-char-property",1,["quote","p"],"s"],["get-text-property",2,["quote","p"],"s"],["get-text-property",3,["quote","p"],"s"],["get-text-property",4,["quote","p"],"s"]]]]),
        );
        assert_eq!(vm.display(value).unwrap(), "[11 22 nil 99 99]");
        let value = run(
            &mut vm,
            serde_json::json!(["let", [["char-property-alias-alist",["quote",[["p","q","r"]]]],["s",{"string":"x"}]], ["put-text-property",0,1,["quote","q"],"nil","s"],["put-text-property",0,1,["quote","r"],33,"s"],["get-text-property",0,["quote","p"],"s"]]),
        );
        assert_eq!(value, Value::integer(33).unwrap());
    }
    #[test]
    fn inserted_strings_graft_properties_including_nil_and_unicode() {
        let mut vm = Vm::new();
        let value = run(
            &mut vm,
            serde_json::json!(["let", [["s", {"string":"λ🙂"}]],
            ["put-text-property",0,1,["quote","p"],17,"s"],
            ["put-text-property",1,2,["quote","p"],"nil","s"],
            ["with-temp-buffer",["insert","s"],
              ["vector",["get-text-property",1,["quote","p"]],["text-properties-at",2],["buffer-size"]]]]),
        );
        assert_eq!(vm.display(value).unwrap(), "[17 (p nil) 2]");
    }
    #[test]
    fn category_plist_values_are_compacted_roots_and_keep_identity() {
        let mut vm = Vm::new();
        vm.set_gc_moving(true);
        vm.allocate_object(Object::Bytes(vec![0]));
        let before = run(
            &mut vm,
            serde_json::json!(["put", ["quote", "cat"], ["quote", "p"], ["vector", 42]]),
        );
        vm.collect(&mut []);
        let after = run(
            &mut vm,
            serde_json::json!(["get", ["quote", "cat"], ["quote", "p"]]),
        );
        assert_ne!(before, after, "must move heap indices");
        assert_eq!(vm.display(after).unwrap(), "[42]");
    }
}

use crate::{
    anchors::{Anchor, AnchorTree, Bias},
    coordinates::ByteOffset,
    text::Text,
};
use std::ops::{Deref, DerefMut, Range};
#[repr(C)]
#[derive(Clone)]
pub(crate) struct AnnotatedBuffer {
    text: Text,
    /// The markers and overlay ends of the text, shared with the buffer's
    /// indirect buffers as GNU's marker chain is (`BUF_MARKERS`).
    pub anchors: Shared<AnchorTree>,
    pub intervals: crate::intervals::IntervalIndex,
    /// Overlay starts whose overlay would turn over at an insertion, of
    /// every buffer sharing the text: GNU adjusts every member's overlays
    /// for an insertion in any (`adjust_overlays_for_insert`).
    pub reversed_pairs: Shared<std::collections::HashMap<Anchor, Anchor>>,
    pub evaporating: std::collections::HashMap<Anchor, Value>,
    pub local_keymap: Value,
    pub(crate) case_tables: [Value; 4],
    /// GNU's per-buffer variable slots (`struct buffer`, forwarded.rs).
    pub per_buffer: crate::forwarded::PerBuffer,
    /// The base buffer of an indirect buffer, else nil (`base_buffer`).
    pub(crate) base: Value,
    /// A base buffer's live indirect buffers (GNU counts them in
    /// `indirections` and finds them in the buffer list).
    pub(crate) indirect: Vec<Value>,
    /// Overlays inserted into this buffer's tree so far (Overlay::inserted).
    pub overlay_insertions: u64,
    /// What names the state of this buffer's overlays to the windows
    /// showing it: the display's overlay count at their last change
    /// (`Vm::overlay_changed`), 0 while none has shown anything. A value is
    /// given once, so it is another's neither after `buffer-swap-text` nor
    /// in the store `set-buffer-multibyte` makes.
    pub overlay_revision: u64,
}
romacs_snapshot::snapshot_struct!(AnnotatedBuffer { text, anchors, intervals, reversed_pairs, evaporating, local_keymap, case_tables, per_buffer, base, indirect, overlay_insertions, overlay_revision });
impl From<Text> for AnnotatedBuffer {
    fn from(text: Text) -> Self {
        let extent =
            u32::try_from(text.len_bytes()).expect("buffer exceeds anchor coordinate limit");
        Self {
            text,
            anchors: Shared::new(AnchorTree::new(extent)),
            intervals: Default::default(),
            reversed_pairs: Default::default(),
            evaporating: Default::default(),
            local_keymap: Value::NIL,
            case_tables: [Value::NIL; 4],
            per_buffer: Default::default(),
            base: Value::NIL,
            indirect: Vec::new(),
            overlay_insertions: 0,
            overlay_revision: 0,
        }
    }
}
impl Deref for AnnotatedBuffer {
    type Target = Text;
    fn deref(&self) -> &Text {
        &self.text
    }
}
impl DerefMut for AnnotatedBuffer {
    fn deref_mut(&mut self) -> &mut Text {
        &mut self.text
    }
}
impl AnnotatedBuffer {
    /// A new buffer sharing this one's text, markers and modification
    /// counts (Fmake_indirect_buffer), at its point and narrowing; all else
    /// is a new buffer's.
    pub(crate) fn share_text(&self) -> Self {
        Self {
            anchors: self.anchors.share(),
            reversed_pairs: self.reversed_pairs.share(),
            ..Self::from(self.text.share())
        }
    }
    pub(crate) fn put_property(&mut self, range: Range<ByteOffset>, key: &str, value: Value) {
        let mut at = range.start;
        let mut changed = false;
        while at < range.end {
            changed |= self.properties.get_property(at, key) != Some(value);
            at = self.properties.next_change(at, range.end);
        }
        self.properties.put(range, key, value);
        if changed { self.state.record_change(false); }
    }
    pub(crate) fn remove_property(&mut self, range: Range<ByteOffset>, key: &str) {
        let mut at = range.start;
        let mut changed = false;
        while at < range.end {
            changed |= self.properties.get_property(at, key).is_some();
            at = self.properties.next_change(at, range.end);
        }
        self.properties.remove(range, key);
        if changed { self.state.record_change(false); }
    }
    pub fn replace(&mut self, range: Range<ByteOffset>, bytes: &[u8]) -> Result<()> {
        self.replace_with_bias(range, bytes, false)
    }
    /// `Text::rewrite_unanchored' with the anchors: those at its start
    /// stay and those after it follow the new text.
    pub(crate) fn rewrite_character(&mut self, range: Range<ByteOffset>, replacement: &[u8]) -> Result<()> {
        let a = u32::try_from(range.start.0).map_err(|_| "buffer too large")?;
        let b = u32::try_from(range.end.0).map_err(|_| "buffer too large")?;
        let len = u32::try_from(replacement.len()).map_err(|_| "buffer too large")?;
        self.anchors.extent().checked_sub(b.checked_sub(a).ok_or("invalid range")?)
            .and_then(|n| n.checked_add(len)).ok_or("buffer too large")?;
        self.text.rewrite_unanchored(range, replacement)?;
        self.insert_anchors(b, len, true);
        assert!(self.anchors.delete(a, b));
        Ok(())
    }
    pub fn replace_with_bias(
        &mut self,
        range: Range<ByteOffset>,
        bytes: &[u8],
        before: bool,
    ) -> Result<()> {
        let a = u32::try_from(range.start.0).map_err(|_| "buffer too large")?;
        let b = u32::try_from(range.end.0).map_err(|_| "buffer too large")?;
        let len = u32::try_from(bytes.len()).map_err(|_| "buffer too large")?;
        self.anchors
            .extent()
            .checked_sub(b.checked_sub(a).ok_or("invalid range")?)
            .and_then(|n| n.checked_add(len))
            .ok_or("buffer too large")?;
        self.text.replace(range, bytes)?;
        assert!(self.anchors.delete(a, b));
        self.insert_anchors(a, len, before);

        Ok(())
    }
    fn insert_anchors(&mut self, pos: u32, len: u32, before: bool) {
        let adjust = !before && len != 0 && !self.reversed_pairs.is_empty();
        if adjust {
            while let Some(h) = self.anchors.find_at(pos, Bias::Right, |h, t| {
                self.reversed_pairs
                    .get(&h)
                    .is_some_and(|&end| t.resolve(end) == Some(pos))
            }) {
                assert!(self.anchors.set(h, pos, Bias::Left));
            }
        }
        assert!(self.anchors.insert(pos, len, before));
        if adjust {
            while let Some(h) = self
                .anchors
                .find_at(pos, Bias::Left, |h, _| self.reversed_pairs.contains_key(&h))
            {
                assert!(self.anchors.set(h, pos, Bias::Right));
                self.intervals.refresh(h, &self.anchors);
            }
        }
    }
    pub fn append_utf8(&mut self, s: &str) -> bool {
        if !self.state.live { return false; }
        let pos = self.anchors.extent();
        let Ok(len) = u32::try_from(s.len()) else {
            return false;
        };
        let Some(extent) = pos.checked_add(len) else {
            return false;
        };
        if !self.text.append_utf8(s) {
            return false;
        }
        if self.anchors.is_empty() {
            self.anchors.extend_empty(extent);
        } else {
            self.insert_anchors(self.anchors.extent(), len, false);
        }
        true
    }
    pub fn visit_values(&mut self, f: &mut impl FnMut(&mut Value)) {
        f(&mut self.local_keymap);
        for root in &mut self.case_tables { f(root); }
        self.per_buffer.visit_values(f);
        self.text.visit_view_values(f);
        // An indirect buffer holds its base alive; the text they share is
        // the base's to visit, once, as collection rewrites what it visits.
        f(&mut self.base);
        for buffer in &mut self.indirect { f(buffer); }
        if self.base == Value::NIL { self.text.visit_values(f); }
        self.intervals.visit_values(f);
        for v in self.evaporating.values_mut() {
            f(v);
        }
    }
    pub fn storage_bytes(&self, capacity: bool) -> usize {
        // The shared text is charged to its base buffer alone.
        let shared = if self.base != Value::NIL { 0 } else {
            self.text.storage_bytes(capacity)
                + self.anchors.storage_bytes(capacity)
                + (if capacity {
                    self.reversed_pairs.capacity() * 2
                } else {
                    self.reversed_pairs.len()
                }) * (size_of::<(Anchor, Anchor)>() + 1)
        };
        shared
            + self.text.state.storage_bytes(capacity)
            + self.indirect.capacity() * size_of::<Value>()
            + self.intervals.storage_bytes(capacity)
            + (if capacity {
                self.evaporating.capacity()
            } else {
                self.evaporating.len()
            }) * (size_of::<(Anchor, Value)>() + 1)
                * 2
    }
}
#[derive(Clone)]
pub(crate) struct Marker {
    pub buffer: Value,
    pub anchor: Option<Anchor>,
    pub bias: Bias,
    /// GNU's charpos where a marker left its buffer: what
    /// `marker-last-position' answers; 0 for one never placed.
    pub last_position: i64,
}
romacs_snapshot::snapshot_struct!(Marker { buffer, anchor, bias, last_position });
#[derive(Clone)]
pub(crate) struct Overlay {
    pub buffer: Value,
    pub start: Option<Anchor>,
    pub end: Option<Anchor>,
    pub front: Bias,
    pub rear: Bias,
    pub properties: Vec<(Value, Value)>,
    /// When it last entered its buffer's overlay tree, by that buffer's
    /// count: GNU's itree puts a node before those with an equal start.
    pub inserted: u64,
}
romacs_snapshot::snapshot_struct!(Overlay { buffer, start, end, front, rear, properties, inserted });
const _: () = assert!(std::mem::offset_of!(AnnotatedBuffer, text) == 0);
impl Marker {
    pub fn visit_values(&mut self, f: &mut impl FnMut(&mut Value)) {
        f(&mut self.buffer);
    }
}
impl Overlay {
    pub fn visit_values(&mut self, f: &mut impl FnMut(&mut Value)) {
        f(&mut self.buffer);
        for (k, v) in &mut self.properties {
            f(k);
            f(v);
        }
    }
}

/// Why a Lisp `put` failed, in GNU's condition vocabulary.
#[derive(Debug)]
pub(super) enum PlistFault {
    /// `(wrong-type-argument symbolp OBJ)`.
    Symbol(Value),
    /// `(wrong-type-argument plistp PLIST)`, fns.c:2726.
    Plistp(Value),
    /// `(circular-list TAIL)`, FOR_EACH_TAIL.
    Circular(Value),
}

/// GNU's FOR_EACH_TAIL cycle detector (lisp.h:5871-5904), step for step: the
/// tortoise starts at the head and only jumps to TAIL at iteration counts that
/// are powers of two and multiples of FOR_EACH_TAIL_THRESHOLD. Values are
/// compared, never retained across an allocation.
pub(super) struct TailWalk {
    tortoise: Value,
    l: usize,
}
impl TailWalk {
    const THRESHOLD: usize = 4096;
    pub(super) fn new(head: Value) -> Self {
        Self { tortoise: head, l: 0 }
    }
    /// After TAIL has advanced one pair: true when it is back at the tortoise.
    pub(super) fn cycle(&mut self, tail: Value) -> bool {
        if tail == self.tortoise {
            return true;
        }
        self.l += 1;
        if self.l & (Self::THRESHOLD - 1) == 0 && self.l.is_power_of_two() {
            self.tortoise = tail;
        }
        false
    }
}
