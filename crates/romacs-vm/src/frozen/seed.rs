//! A seed (O7m §2.6): `romacs--seed-build`, one call that walks a seed's
//! roots and the definitions they mention into one code graph
//! (`code.rs`) under one work budget, refused work included, rolls a
//! refused entry back, and encodes the rest once.
//!
//! SEED is `(CODE-FORMAT ZYGOTE-INCARNATION ENTRIES)`, an ENTRY one of
//! `(variable SYM VALUE SPECIAL FILE)`, `(function SYM DEF FILE)`,
//! `(plist SYM PLIST)` and `(after-load KEY FUNCS)`. A symbol an entry
//! mentions, or a function or plist root names, whose definition does not
//! cross by name (`by_name`) is carried by value, a `function` and a
//! `plist` entry together. Its list structure
//! is the encoding's own (`Spine`), never consed in this VM.
use super::{Backing, HashMap, HashSet, Label, Object, Result, Value, Vm};
use super::code::{CodeWalk, Mark, Spine, Stop};

/// One entry: its kind (a symbol), its name, and the values it carries.
struct Entry { kind: Value, name: Value, values: Vec<Value> }

/// Why an entry was refused.
enum Reason { CannotFreeze(Value, Vec<Label>), TooLarge(usize), Unavailable(String), Unbound }

struct Refused { kind: Value, name: Value, reason: Reason }

/// A seed being built: the walk, the entries kept and refused, and the
/// symbols whose definitions were judged.
struct Seed<'a> {
    walk: CodeWalk<'a>,
    limit: usize,
    exhausted: bool,
    kept: Vec<Entry>,
    refused: Vec<Refused>,
    judged: HashSet<u64>,
    queue: std::collections::VecDeque<Vec<Entry>>,
    /// The kinds of the entries a mention makes.
    function: Value,
    plist: Value,
}

impl Seed<'_> {
    /// Walk the entries of UNIT, kept or refused together.
    fn unit(&mut self, unit: Vec<Entry>) {
        let Some(first) = unit.first() else { return };
        let (kind, name) = (first.kind, first.name);
        if self.exhausted {
            self.refused.push(Refused { kind, name, reason: Reason::TooLarge(self.walk.work) });
            return;
        }
        if unit.iter().any(|entry| entry.values.contains(&Value::UNBOUND)) {
            self.refused.push(Refused { kind, name, reason: Reason::Unbound });
            return;
        }
        let mark = self.walk.mark();
        for entry in &unit {
            for &value in [entry.kind, entry.name].iter().chain(&entry.values) {
                if let Err(stop) = self.walk.walk(value, self.limit) {
                    self.walk.rollback(mark);
                    let reason = match stop {
                        Stop::Refused(refusal) => Reason::CannotFreeze(refusal.object, refusal.path),
                        Stop::TooLarge => { self.exhausted = true; Reason::TooLarge(self.walk.work) }
                        Stop::Unavailable(why) => Reason::Unavailable(why),
                    };
                    self.refused.push(Refused { kind, name, reason });
                    return;
                }
            }
        }
        self.carry_mentions(mark);
        self.kept.extend(unit);
    }

    /// Queue the definitions the entries walked since MARK mention, each
    /// symbol judged once, that do not cross by name.
    fn carry_mentions(&mut self, mark: Mark) {
        let vm = self.walk.data.vm;
        let mentions: Vec<Value> = self.walk.mentions(mark).to_vec();
        for symbol in mentions {
            let Some(slot) = symbol.index_if(7).and_then(|i| i.checked_sub(1)) else { continue };
            if !self.judged.insert(symbol.0) || by_name(vm, slot) { continue; }
            self.queue.push_back(definition(vm, (self.function, self.plist), symbol, slot));
        }
    }
}

/// The definition of SYMBOL, at SLOT, carried by value: one unit of its
/// `function` entry, FILE nil, and its `plist` entry.
fn definition(vm: &Vm, (function, plist): (Value, Value), symbol: Value, slot: usize) -> Vec<Entry> {
    vec![
        Entry { kind: function, name: symbol, values: vec![vm.slots[slot].function, Value::NIL] },
        Entry { kind: plist, name: symbol, values: vec![vm.slots[slot].plist] },
    ]
}

/// Whether the definition of the symbol at SLOT crosses by name, the
/// receiver's own being the same, or by value (O7m §2.6, D3): the one
/// place that decides it, which will read whether the cell was written
/// since the world was restored (`romacs--written-p`). Until then, only
/// what no load can make differently crosses by name: a void cell, and a
/// subr.
fn by_name(vm: &Vm, slot: usize) -> bool {
    let function = vm.slots[slot].function;
    function == Value::UNBOUND || function == Value::NIL || function.tag() == 6
}

impl Vm {
    /// `romacs--seed-build`: ROOTS walked with what they mention under one
    /// budget of LIMIT nodes, TABLE the zygote's (its `:incarnation` is
    /// the seed's). `(BACKING . REFUSALS)`.
    pub(crate) fn seed_build(&mut self, roots: Value, table: Value, limit: usize) -> Result<Value> {
        let (function, plist) = (self.intern("function"), self.intern("plist"));
        // The walk reads each closure's interactive spec by lookup, which a
        // stale table would answer by scanning it whole.
        if let Some(&slot) = self.names.get(super::prim::INTERACTIVE_TABLE) {
            self.freshen_table(self.slots[slot].value);
        }
        let units = self.seed_roots(roots, (function, plist), limit)?;
        let incarnation = self.plist_value(table, ":incarnation", limit)?;
        // A root's own definition is a unit already: its name mentions
        // nothing more.
        let judged = units.iter().flatten().filter(|e| e.kind == function || e.kind == plist).map(|e| e.name.0).collect();
        let (bytes, refused) = {
            let mut seed = Seed { walk: CodeWalk::new(self), limit, exhausted: false, kept: vec![], refused: vec![],
                                  judged, queue: units.into(), function, plist };
            while let Some(unit) = seed.queue.pop_front() {
                seed.unit(unit);
            }
            let bytes = seed.encode(incarnation)?;
            (bytes, seed.refused)
        };
        // The encoded bytes count toward the next collection, as any
        // allocation's do: the object's own funnel charges it empty.
        self.allocation.allocated += bytes.len() as u64;
        let backing = Backing::new(bytes).map_err(|e| e.to_string())?;
        let backing = self.allocate_object(Object::Backing(backing));
        let refusals = refused.into_iter().map(|r| self.refusal_value(r)).collect::<Vec<_>>();
        let refusals = self.list_value(&refusals);
        Ok(self.alloc_cons(backing, refusals))
    }

    /// ROOTS, each `(variable SYM [FILE])`, `(function SYM [FILE])`,
    /// `(plist SYM)` or `(after-load KEY FUNCS)`, as the units the seed
    /// walks. A variable is read at the end of its alias chain. A function
    /// or plist root whose definition crosses by value is that definition,
    /// both entries in one unit as a mention carries it, one unit for both
    /// roots of a symbol; one crossing by name is its entry alone. KINDS
    /// are the `function` and `plist` symbols. No more than LIMIT roots
    /// are read.
    fn seed_roots(&mut self, roots: Value, kinds: (Value, Value), limit: usize) -> Result<Vec<Vec<Entry>>> {
        let mut units = vec![];
        let mut definitions: HashMap<u64, usize> = HashMap::default();
        for root in self.list_items(roots, limit)? {
            let items = self.list_items(root, 3)?;
            let (&kind, &name) = (items.first().ok_or("empty seed root")?, items.get(1).ok_or("seed root has no name")?);
            let file = items.get(2).copied().unwrap_or(Value::NIL);
            let kind_name = self.seed_symbol_name(kind);
            if kind_name == "after-load" {
                units.push(vec![Entry { kind, name, values: vec![file] }]);
                continue;
            }
            let slot = name.index_if(7).and_then(|i| i.checked_sub(1)).ok_or("seed root names no symbol")?;
            if matches!(kind_name.as_str(), "function" | "plist") && !by_name(self, slot) {
                let unit = *definitions.entry(name.0).or_insert_with(|| {
                    units.push(definition(self, kinds, name, slot));
                    units.len() - 1
                });
                if kind_name == "function" { units[unit][0].values[1] = file; }
                continue;
            }
            let values = match kind_name.as_str() {
                "variable" => {
                    let base = self.indirect_variable(slot);
                    let special = if self.slot_is_special(base) { Value::TRUE } else { Value::NIL };
                    vec![self.default_value(base)?, special, file]
                }
                "function" => {
                    let def = self.slots[slot].function;
                    vec![if def == Value::UNBOUND { self.intern("void") } else { def }, file]
                }
                "plist" => vec![self.slots[slot].plist],
                other => return Err(format!("unknown seed root kind {other}")),
            };
            units.push(vec![Entry { kind, name, values }]);
        }
        Ok(units)
    }

    fn seed_symbol_name(&self, symbol: Value) -> String {
        symbol.index_if(7).and_then(|i| self.symbol_names.get(i.checked_sub(1)?)).cloned().unwrap_or_default()
    }

    /// The items of LIST, no more than MAX of them: a longer or circular
    /// list is refused.
    fn list_items(&self, mut list: Value, max: usize) -> Result<Vec<Value>> {
        let mut items = vec![];
        while list != Value::NIL {
            if items.len() == max {
                return Err(format!("a seed list is circular or longer than its limit {max}"));
            }
            let (car, cdr) = self.cons_pair(list)?;
            items.push(car);
            list = cdr;
        }
        Ok(items)
    }

    /// The value after KEY in the plist PLIST, or nil; no more than MAX
    /// items are read.
    fn plist_value(&self, plist: Value, key: &str, max: usize) -> Result<Value> {
        let items = self.list_items(plist, max)?;
        Ok(items.chunks(2).find(|pair| self.seed_symbol_name(pair[0]) == key).and_then(|pair| pair.get(1).copied()).unwrap_or(Value::NIL))
    }

    /// REFUSED as `(KIND NAME REASON . DETAIL)`.
    fn refusal_value(&mut self, refused: Refused) -> Value {
        let (kind, reason) = (refused.kind, refused.reason);
        let (why, detail) = match reason {
            Reason::CannotFreeze(object, path) => {
                let kind = crate::prim::type_name(self, object);
                let kind = self.intern(kind);
                let path = self.refusal_path(&path);
                ("cannot-freeze", vec![kind, path])
            }
            Reason::TooLarge(work) => ("too-large", vec![i64::try_from(work).ok().and_then(|n| Value::integer(n).ok()).unwrap_or(Value::NIL)]),
            Reason::Unavailable(why) => ("body-unavailable", vec![self.make_lisp_string(why.into_bytes(), true)]),
            Reason::Unbound => ("unbound", vec![]),
        };
        let why = self.intern(why);
        let mut items = vec![kind, refused.name, why];
        items.extend(detail);
        self.list_value(&items)
    }
}

impl Seed<'_> {
    /// The seed's encoding: `(CODE-FORMAT ZYGOTE-INCARNATION ENTRIES)`.
    fn encode(&mut self, incarnation: Value) -> Result<Vec<u8>> {
        self.walk.walk(incarnation, usize::MAX).map_err(|_| "the zygote's incarnation cannot be frozen")?;
        let mut entries = Spine::Value(Value::NIL);
        for entry in self.kept.iter().rev() {
            let mut list = Spine::Value(Value::NIL);
            for &value in entry.values.iter().rev().chain([entry.name, entry.kind].iter()) {
                list = self.walk.cons(Spine::Value(value), list);
            }
            entries = self.walk.cons(list, entries);
        }
        let format = Value::integer(i64::try_from(super::code::code_format() >> 4).map_err(|_| "code format out of range")?)?;
        let tail = self.walk.cons(entries, Spine::Value(Value::NIL));
        let tail = self.walk.cons(Spine::Value(incarnation), tail);
        let root = self.walk.cons(Spine::Value(format), tail);
        self.walk.encode(root)
    }
}
