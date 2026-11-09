//! Verify mode's reference marker (design note gc-smooth.md 4.7): a second,
//! plain account of what a collection must keep, sharing no tracing code with
//! the marker (gc.rs, gc_mark.rs). Its roots are read again from the VM's
//! fields, and its walk is one worklist over strong edges: an object's children
//! come from its own `visit_values', the visitor a moving collection rewrites
//! through, which moving stress proves apart from either marker.
//!
//! It does not follow a weak table's entries or those of `lambda_lists`
//! (ephemerons too), the finalizers a collection dooms, symbols only their
//! own obarray holds unless reached, nor the symbols code names: what it
//! reaches is a floor of what must live, and the mark must have reached all
//! of it. A miss is a reachable entry the sweep would free.
//!
//! What it cannot see: an omission in what it shares with the marker, each
//! kind's `visit_values' and the accessors of root sources
//! (`source_code_roots', `Charsets::trace', `Forwarded::visit_values',
//! `Op::visit_values'), whose own guard is moving stress, which rewrites
//! through them; and a field that holds Values but is in no root list at all,
//! this one, the marker's or a compaction's.
use super::*;

/// What the reference walk reached, by table, function, region and the
/// symbols of obarrays of their own.
struct Reached {
    tables: [Vec<bool>; 4],
    functions: Vec<bool>,
    regions: Vec<bool>,
    symbols: HashSet<usize>,
}

const TABLES: [&str; 4] = ["conses", "closures", "cells", "objects"];

impl Vm {
    /// Panic unless the mark reached everything the reference walk does from
    /// ROOTS: every entry is marked, and every function, region and obarray
    /// symbol is in CODE, REGIONS and SYMBOLS, the mark's own.
    pub(super) fn check_reachable(&mut self, roots: &[Value], code: &[bool], regions: &[bool], symbols: &HashSet<usize>) {
        let reached = self.reference_walk(roots);
        let marked = [self.conses.marks().collect::<Vec<_>>(), self.closures.marks().collect(), self.cells.marks().collect(), self.objects.marks().collect()];
        for (t, (reached, marked)) in reached.tables.iter().zip(&marked).enumerate() {
            if let Some(i) = reached.iter().zip(marked).position(|(&r, &m)| r && !m) {
                panic!("the reference marker reached {} entry {i}, which the mark did not", TABLES[t]);
            }
        }
        if let Some(id) = reached.functions.iter().enumerate().position(|(id, &r)| r && !code.get(id).copied().unwrap_or(true)) {
            panic!("the reference marker reached function {id}, which the mark did not");
        }
        if let Some(id) = reached.regions.iter().enumerate().position(|(id, &r)| r && !regions.get(id).copied().unwrap_or(true)) {
            panic!("the reference marker reached region {id}, which the mark did not");
        }
        if let Some(slot) = reached.symbols.iter().find(|slot| !symbols.contains(slot)) {
            panic!("the reference marker reached obarray symbol {slot}, which the mark did not");
        }
    }

    /// The reference walk, from ROOTS and the VM's own roots.
    fn reference_walk(&mut self, roots: &[Value]) -> Reached {
        let mut r = Reached {
            tables: [vec![false; self.conses.len()], vec![false; self.closures.len()], vec![false; self.cells.len()], vec![false; self.objects.len()]],
            functions: vec![false; self.functions.len()],
            regions: vec![false; self.regions.len()],
            symbols: HashSet::default(),
        };
        let (mut values, mut code) = (roots.to_vec(), Vec::new());
        values.extend_from_slice(&self.stack);
        values.extend(self.suspended_stacks.iter().flatten().copied());
        values.extend_from_slice(&self.external_roots);
        values.extend(self.doomed_finalizers.iter().copied());
        values.extend(self.permanent_symbol_names.values().copied());
        for binding in &self.terminal_bindings {
            values.push(binding.buffer);
            values.extend_from_slice(&binding.away);
            values.extend_from_slice(&binding.unremapped_maps);
        }
        if let Some(escape) = &mut self.propagated_escape { escape.visit_values(&mut |v| values.push(*v)); }
        for handler in &mut self.signal_handlers { handler.visit_values(&mut |v| values.push(*v)); }
        self.backtrace.visit_values(&mut |v| values.push(*v));
        self.profiler.visit_values(&mut |v| values.push(*v));
        self.forwarded.visit_values(&mut |v| values.push(*v));
        values.extend_from_slice(&self.standard_case_tables);
        values.push(self.command_point.1);
        values.extend(self.mode_line_forms.values().map(|&(known, _)| known));
        self.charsets.trace(&mut values);
        for &(slot, old, buffer, _) in &self.dynamic {
            values.extend([old, self.slots[slot].value, buffer, Value::reference(slot + 1, 7)]);
        }
        // Every symbol but those of an obarray of their own is a root, and a
        // buffer-local value is its buffer's.
        let local: HashSet<usize> = self.local_binding_slots.iter().copied().collect();
        for (slot, s) in self.slots.iter().enumerate() {
            if self.obarray_symbol_names.contains_key(&slot) { continue; }
            values.extend([s.function, s.plist, s.local]);
            if !local.contains(&slot) { values.push(s.value); }
            if let Some(Some(VariableRedirect::Alias(target))) = self.variable_redirects.get(slot) {
                values.push(Value::reference(target + 1, 7));
            }
        }
        for (id, entry) in &self.conditions.entries {
            let own = id.value().index_if(7).and_then(|n| n.checked_sub(1)).is_some_and(|slot| self.obarray_symbol_names.contains_key(&slot));
            if !own { for &(key, value) in &entry.properties { values.extend([key, value]); } }
        }
        let active = !self.stack.is_empty() || !self.suspended_stacks.is_empty();
        code.extend((0..self.request_code.live.len()).filter(|&id| self.request_code.live[id] && (active || self.request_code.permanent[id])));
        self.source_code_roots(&mut code);
        for engine in &mut self.suspended {
            engine.visit_values(&mut |v| values.push(*v));
        }
        let mut native = HashSet::default();
        loop {
            if let Some(id) = code.pop() {
                self.reference_function(id, &mut r, &mut values, &mut code);
            } else if let Some(v) = values.pop() {
                self.reference_value(v, &mut r, &mut values, &mut code, &mut native);
            } else {
                return r;
            }
        }
    }

    /// V's children onto VALUES and CODE, the first time the walk meets it.
    fn reference_value(&mut self, v: Value, r: &mut Reached, values: &mut Vec<Value>, code: &mut Vec<usize>, native: &mut HashSet<usize>) {
        if Self::image_value(v) {
            let id = crate::frozen::region_of(v.payload() as usize);
            if id != 0 && !std::mem::replace(&mut r.regions[id], true) {
                values.extend_from_slice(self.region_symbols(id));
            }
            return;
        }
        let first = |seen: &mut Vec<bool>, i: usize| i < seen.len() && !std::mem::replace(&mut seen[i], true);
        match v.tag() {
            3 => {
                let i = v.payload() as usize;
                if first(&mut r.tables[0], i) { values.extend([self.conses[i].0, self.conses[i].1]); }
            }
            4 => {
                let i = v.payload() as usize;
                if first(&mut r.tables[1], i) {
                    let closure = &self.closures[i];
                    values.extend_from_slice(&closure.captures);
                    values.extend_from_slice(&closure.metadata);
                    values.push(closure.document);
                    code.push(closure.function);
                }
            }
            5 => {
                let i = v.payload() as usize;
                if first(&mut r.tables[2], i) { values.push(self.cell_pairs.get(&i).copied().unwrap_or(self.cells[i])); }
            }
            7 if v.payload() >= 1 => {
                let slot = v.payload() as usize - 1;
                if let Some(&name) = self.obarray_symbol_names.get(&slot) && r.symbols.insert(slot) {
                    let s = &self.slots[slot];
                    values.extend([name, s.value, s.function, s.plist, s.local]);
                    if let Some(entry) = self.conditions.entries.get(&conditions::SymbolId::new(v).unwrap()) {
                        for &(key, value) in &entry.properties { values.extend([key, value]); }
                    }
                    if let Some(Some(VariableRedirect::Alias(target))) = self.variable_redirects.get(slot) {
                        values.push(Value::reference(target + 1, 7));
                    }
                }
            }
            0 if v != Value::NIL => {
                let i = v.payload() as usize - 1;
                if !first(&mut r.tables[3], i) { return; }
                let mut push = |v: &mut Value| values.push(*v);
                use data::Object::*;
                match &mut self.objects.collector_entries_mut()[i] {
                    Vector(slots) | Record(slots) => slots.iter_mut().for_each(&mut push),
                    Finalizer(function) => push(function),
                    String(s) | RawString(s) => s.properties.visit_values(&mut push),
                    HashTable(t) if t.weakness == hash::Weakness::None => t.visit_values(&mut push),
                    HashTable(_) => {}
                    CharTable(t) => t.collector_visit_values(native, &mut push),
                    SubCharTable(node) => char_table::collector_visit_node(node, native, &mut push),
                    MapKeymapCursor(c) => c.collector_visit_values(native, &mut push),
                    Window(w) => w.visit_values(&mut push),
                    Frame(f) => f.visit_values(&mut push),
                    Terminal(t) => t.visit_values(&mut push),
                    WindowConfiguration(c) => c.visit_values(&mut push),
                    Treesit(t) => t.visit_values(&mut push),
                    Process(p) => p.visit_values(&mut push),
                    Buffer(b) => b.visit_values(&mut push),
                    ByteBuffer(b) => b.visit_values(&mut push),
                    Marker(m) => m.visit_values(&mut push),
                    Overlay(o) => o.visit_values(&mut push),
                    Obarray(t) => t.visit_values(&mut push),
                    Float(_) | Bignum(_) | BoolVector(_) | Bytes(..) | SyntaxTable(_) | Sqlite(_) | Thread | Free => {}
                }
            }
            _ => {}
        }
    }

    /// Function ID's values and the functions it makes, the first time.
    fn reference_function(&self, id: usize, r: &mut Reached, values: &mut Vec<Value>, code: &mut Vec<usize>) {
        if id >= r.functions.len() || std::mem::replace(&mut r.functions[id], true) { return; }
        let function = &self.functions[id];
        for op in &function.code {
            let mut op = *op;
            op.visit_values(&mut |v| values.push(*v));
            match op {
                Op::Closure(_, child, _) => code.push(child),
                Op::Direct(_, _, cache, _, _) => values.push(self.caches[cache].target),
                _ => {}
            }
        }
        if let Some(lazy) = &function.lazy { values.push(lazy.body); }
        if let Some(body) = &function.pending {
            values.extend_from_slice(&body.values);
            code.extend_from_slice(&body.children);
        }
        for program in std::iter::once(&self.execution).chain(&self.spare_programs) {
            program.lowered_values(id, &mut |v| values.push(v), &mut |f| code.push(f));
        }
    }
}
