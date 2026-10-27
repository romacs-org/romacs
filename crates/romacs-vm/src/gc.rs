//! Strand-local stop-at-wait-point compacting collector. No process-global state.
use super::*;

#[derive(Clone, Copy, Debug, Default)]
pub struct HeapStats {
    pub collections: u64,
    pub allocations: u64,
    pub reclaimed: u64,
    pub live_objects: usize,
    pub live_bytes: usize,
    pub capacity_bytes: usize,
}

/// What a retention probe measured at each workload size.
#[derive(Clone, Debug, Default)]
pub struct RetentionReport {
    pub sizes: Vec<usize>,
    pub objects: Vec<usize>,
    pub bytes: Vec<usize>,
    pub collections: u64,
}

/// Run WORKLOAD at ascending SIZES and require retention to be flat in **both**
/// objects and bytes.
///
/// Three independent lanes have shipped the same defect through a green suite by
/// asserting only one of the two dimensions: a buffer-local table whose
/// `live_objects` stayed at 5 while `live_bytes` went 41,480 -> 82,672 ->
/// 165,056; a weak-key table that removed entries without ever compacting its
/// storage; and a window parent that dropped children with `retain`, which does
/// not reduce capacity. Objects alone cannot see a container that frees entries
/// but keeps its buffer, and bytes alone cannot see an object leak inside a
/// fixed allocation, so a retention test has to assert both or it is measuring
/// half the heap.
///
/// Flat, not merely sub-linear: a workload whose final state is identical at
/// every size must retain an identical amount, so any growth at all is the bug.
/// The panic names which dimension grew and prints both series, because "bytes
/// grew" and "objects grew" have completely different causes.
///
/// WORKLOAD must leave the heap in the *same* logical state for every size --
/// build `n` things and dispose of them -- otherwise flatness is not the
/// property you want and this helper is the wrong tool.
pub fn assert_no_retention_growth(
    vm: &mut Vm,
    sizes: &[usize],
    mut workload: impl FnMut(&mut Vm, usize),
) -> RetentionReport {
    assert!(
        sizes.len() >= 3,
        "a retention probe needs at least three sizes to distinguish flat from growing, got {:?}",
        sizes
    );
    let before = vm.heap_stats().collections;
    let mut report = RetentionReport::default();
    for &n in sizes {
        workload(vm, n);
        vm.collect(&mut []);
        let s = vm.heap_stats();
        report.sizes.push(n);
        report.objects.push(s.live_objects);
        report.bytes.push(s.live_bytes);
        eprintln!(
            "retention size={n} collections={} objects={} bytes={} capacity={}",
            s.collections, s.live_objects, s.live_bytes, s.capacity_bytes
        );
    }
    report.collections = vm.heap_stats().collections - before;
    assert_eq!(
        report.collections as usize,
        sizes.len(),
        "each size must be measured after exactly one counted collection: {report:?}"
    );
    let flat = |v: &[usize]| v.iter().all(|x| *x == v[0]);
    assert!(
        flat(&report.objects),
        "retained OBJECTS grew with workload size -- an object leak: sizes={:?} objects={:?} bytes={:?}",
        report.sizes,
        report.objects,
        report.bytes
    );
    assert!(
        flat(&report.bytes),
        "retained BYTES grew while objects stayed flat -- a container that drops \
         entries without reclaiming storage: sizes={:?} objects={:?} bytes={:?}",
        report.sizes,
        report.objects,
        report.bytes
    );
    report
}

impl Vm {
    /// Snapshot before constructing the result, just as GNU snapshots gcstat.
    /// Object arenas share spare storage: no typed free string/float/buffer pools.
    pub(crate) fn garbage_collect_statistics(&mut self) -> Value {
        let (mut strings, mut string_bytes, mut vectors, mut vector_slots, mut vector_free, mut floats, mut buffers) = (0, 0, 0, 0, 0, 0, 0);
        for object in &self.objects {
            match object {
                data::Object::String(s) | data::Object::RawString(s) => {
                    strings += 1;
                    string_bytes += s.bytes().len();
                }
                data::Object::Vector(v) | data::Object::Record(v) => {
                    vectors += 1;
                    vector_slots += v.len();
                    vector_free += v.capacity() - v.len();
                }
                data::Object::Float(_) => floats += 1,
                data::Object::Buffer(_) | data::Object::ByteBuffer(_) => buffers += 1,
                _ => {}
            }
        }
        let rows: [(&str, usize, usize, Option<usize>); 8] = [
            ("conses", size_of::<(Value, Value)>(), self.conses.len(), Some(self.conses.capacity() - self.conses.len())),
            ("symbols", size_of::<Slot>(), self.slots.len() - self.obarray_free_slots.len() - self.local_binding_slots.len(), Some(self.obarray_free_slots.len())),
            ("strings", size_of::<data::Object>() + size_of::<data::LispString>(), strings, Some(0)),
            ("string-bytes", 1, string_bytes, None),
            ("vectors", size_of::<data::Object>(), vectors, None),
            ("vector-slots", size_of::<Value>(), vector_slots, Some(vector_free)),
            ("floats", size_of::<data::Object>(), floats, Some(0)),
            ("buffers", size_of::<data::Object>(), buffers, None),
        ];
        let mut result = Value::NIL;
        for (name, size, used, free) in rows.into_iter().rev() {
            let mut row = Value::NIL;
            if let Some(free) = free { row = self.keymap_cons(Value::integer(free as i64).expect("heap count fits fixnum"), row); }
            row = self.keymap_cons(Value::integer(used as i64).expect("heap count fits fixnum"), row);
            row = self.keymap_cons(Value::integer(size as i64).expect("heap count fits fixnum"), row);
            let symbol = self.intern(name);
            row = self.keymap_cons(symbol, row);
            result = self.keymap_cons(row, result);
        }
        result
    }

    /// Whether a Value is retained by the current mark: immediates, symbols,
    /// primitives and mapped image objects always survive.
    fn survives(&self, marks: &[Vec<bool>; 4], symbols: &HashSet<usize>, v: Value) -> bool {
        if Self::image_value(v) {
            return true;
        }
        let i = v.payload() as usize;
        match v.tag() {
            0 if v != Value::NIL => marks[3][i - 1],
            3 => marks[0][i],
            4 => marks[1][i],
            5 => marks[2][i],
            7 if v != Value::UNBOUND && self.obarray_symbol_names.contains_key(&(i - 1)) => symbols.contains(&(i - 1)),
            _ => true,
        }
    }
    /// GNU `sweep_weak_table (h, false)`, run whenever the worklist drains:
    /// an entry whose keep rule already holds makes its other half live.
    /// Marking only grows, so the rounds reach a fixpoint; weak tables reached
    /// by the new marks join the next round. True when anything was pushed.
    fn weak_round(
        &self,
        marks: &[Vec<bool>; 4],
        weak: &[usize],
        symbols: &HashSet<usize>,
        pending: &mut Vec<Value>,
        local_live: &mut HashSet<usize>,
    ) -> bool {
        for &t in weak {
            let data::Object::HashTable(table) = &self.objects[t] else {
                unreachable!("weak table index");
            };
            for (_, key, value) in table.entries() {
                let (k, v) = (self.survives(marks, symbols, key), self.survives(marks, symbols, value));
                // The buffer-keyed ephemeron owns a local value indirectly
                // through a stable symbol-slot ID. The slot is NOT a global
                // root: trace its moving Value only once its owner is live.
                if k && table.weakness == hash::Weakness::Key {
                    if let Some(slot) = value.index_if(7).and_then(|i| i.checked_sub(1)) {
                        if self.local_binding_slots.contains(&slot) && local_live.insert(slot) {
                            pending.push(self.slots[slot].value);
                        }
                    }
                }
                if table.weakness.keep(k, v) {
                    if !k {
                        pending.push(key);
                    }
                    if !v {
                        pending.push(value);
                    }
                }
            }
        }
        !pending.is_empty()
    }
    /// Allocated heap accounting; unreachable objects are included until collection.
    /// After collection these are retained/live counts, excluding compiler and image.
    pub fn heap_stats(&self) -> HeapStats {
        let mut s = self.gc_stats;
        s.allocations += (self.heap_objects() - self.gc_retained) as u64;
        s.live_objects = self.heap_objects();
        s.live_bytes = self.conses.len() * std::mem::size_of::<(Value, Value)>()
            + self.cells.len() * std::mem::size_of::<Value>()
            + self.closures.len() * std::mem::size_of::<Closure>()
            + self
                .closures
                .iter()
                .map(|c| (c.captures.len() + c.metadata.len()) * 8)
                .sum::<usize>();
        // Native nodes may be retained by both tables and suspended cursors.
        // Keep this accounting separate from the moving arena object counts.
        let mut native_nodes = HashSet::default();
        s.live_bytes += self.objects.len() * std::mem::size_of::<data::Object>()
            + self
                .objects
                .iter()
                .map(|o| match o {
                    data::Object::Float(_) => 0,
                    data::Object::Bignum(n) => size_of::<num_bigint::BigInt>()
                        + n.bits().div_ceil(64) as usize * size_of::<u64>(),
                    data::Object::Window(w) => w.storage_bytes(),
                    data::Object::Frame(f) => f.storage_bytes(),
                    data::Object::Terminal(t) => t.storage_bytes(),
                    data::Object::WindowConfiguration(c) => c.storage_bytes(),
                    data::Object::Sqlite(h) => h.storage_bytes(),
                    data::Object::Process(p) => p.storage_bytes(),
                    data::Object::String(v) | data::Object::RawString(v) => v.storage_bytes(false),
                    data::Object::Bytes(v) => v.len(),
                    data::Object::BoolVector(v) => size_of::<data::BoolVector>() + v.bytes.len(),
                    data::Object::ByteBuffer(b) => {
                        size_of::<bytes::ByteBuffer>() + b.storage_bytes(false)
                    }
                    data::Object::Marker(_) => size_of::<annotations::Marker>(),
                    data::Object::Overlay(o) => {
                        size_of::<annotations::Overlay>()
                            + o.properties.len() * size_of::<(Value, Value)>()
                    }
                    data::Object::Vector(v) | data::Object::Record(v) => v.len() * std::mem::size_of::<Value>(),
                    data::Object::CharTable(t) => t.storage_bytes_seen(false, &mut native_nodes),
                    data::Object::SubCharTable(node) => char_table::CharTableNode::storage_bytes(node, false, &mut native_nodes),
                    data::Object::MapKeymapCursor(c) => c.storage_bytes(false, &mut native_nodes),
                    data::Object::Buffer(t) => {
                        size_of::<annotations::AnnotatedBuffer>() + t.storage_bytes(false)
                    }
                    data::Object::SyntaxTable(t) => {
                        size_of_val(t)
                            + t.len() * size_of::<(u32, romacs_regex_engine::SyntaxClass)>()
                    }
                    data::Object::HashTable(t) => t.storage_bytes(false),
                    data::Object::Obarray(t) => t.storage_bytes(false),
                })
                .sum::<usize>();
        s.capacity_bytes = self.conses.capacity() * 16
            + self.cells.capacity() * 8
            + self.closures.capacity() * std::mem::size_of::<Closure>()
            + self
                .closures
                .iter()
                .map(|c| (c.captures.capacity() + c.metadata.capacity()) * 8)
                .sum::<usize>();
        native_nodes.clear();
        s.capacity_bytes += self.objects.capacity() * std::mem::size_of::<data::Object>()
            + self
                .objects
                .iter()
                .map(|o| match o {
                    data::Object::Float(_) => 0,
                    data::Object::Bignum(n) => size_of::<num_bigint::BigInt>()
                        + n.bits().div_ceil(64) as usize * size_of::<u64>(),
                    data::Object::Window(w) => w.storage_bytes(),
                    data::Object::Frame(f) => f.storage_bytes(),
                    data::Object::Terminal(t) => t.storage_bytes(),
                    data::Object::WindowConfiguration(c) => c.storage_bytes(),
                    data::Object::Sqlite(h) => h.storage_bytes(),
                    data::Object::Process(p) => p.storage_bytes(),
                    data::Object::String(v) | data::Object::RawString(v) => v.storage_bytes(true),
                    data::Object::Bytes(v) => v.capacity(),
                    data::Object::BoolVector(v) => size_of::<data::BoolVector>() + v.bytes.capacity(),
                    data::Object::ByteBuffer(b) => {
                        size_of::<bytes::ByteBuffer>() + b.storage_bytes(true)
                    }
                    data::Object::Marker(_) => size_of::<annotations::Marker>(),
                    data::Object::Overlay(o) => {
                        size_of::<annotations::Overlay>()
                            + o.properties.capacity() * size_of::<(Value, Value)>()
                    }
                    data::Object::Vector(v) | data::Object::Record(v) => v.capacity() * std::mem::size_of::<Value>(),
                    data::Object::CharTable(t) => t.storage_bytes_seen(true, &mut native_nodes),
                    data::Object::SubCharTable(node) => char_table::CharTableNode::storage_bytes(node, true, &mut native_nodes),
                    data::Object::MapKeymapCursor(c) => c.storage_bytes(true, &mut native_nodes),
                    data::Object::Buffer(t) => {
                        size_of::<annotations::AnnotatedBuffer>() + t.storage_bytes(true)
                    }
                    data::Object::SyntaxTable(t) => {
                        size_of_val(t)
                            + t.len() * size_of::<(u32, romacs_regex_engine::SyntaxClass)>()
                    }
                    data::Object::HashTable(t) => t.storage_bytes(true),
                    data::Object::Obarray(t) => t.storage_bytes(true),
                })
                .sum::<usize>();
        s
    }
    /// Collect at a strand wait point. All externally retained Values must be in
    /// `roots`; compaction rewrites them. Never share private Values across VMs.
    /// Runtime register, global, binding and inline-cache roots are added here.
    pub(super) fn collect_marked(&mut self, roots: &mut [Value]) {
        let before = self.heap_objects();
        self.gc_stats.allocations += (before - self.gc_retained) as u64;
        let mut marks = [
            vec![false; self.conses.len()],
            vec![false; self.closures.len()],
            vec![false; self.cells.len()],
            vec![false; self.objects.len()],
        ];
        let mut pending = roots.to_vec();
        pending.extend_from_slice(&self.stack);
        pending.extend_from_slice(&self.external_roots);
        pending.extend(self.permanent_symbol_names.values().copied());
        pending.extend(self.terminal_bindings.iter().map(|binding| binding.buffer));
        if let Some(escape) = &mut self.propagated_escape {
            escape.visit_values(&mut |v| pending.push(*v));
        }
        for handler in &mut self.signal_handlers {
            handler.visit_values(&mut |v| pending.push(*v));
        }
        self.backtrace.visit_values(&mut |v| pending.push(*v));
        let mut local_live = HashSet::default();
        let mut obarray_symbols_live = HashSet::default();
        for (slot, s) in self.slots.iter().enumerate() {
            if self.obarray_symbol_names.contains_key(&slot) { continue; }
            if let Some(Some(VariableRedirect::Alias(target))) = self.variable_redirects.get(slot) {
                pending.push(Value::reference(target + 1, 7));
            }
            pending.push(s.function);
            // GNU alloc.c:6673 marks `u.s.plist` with the symbol. VM2 symbols
            // are never reclaimed (DIVERGENCES VM2-BOOT4), so a plist and a
            // buffer-local table are roots for every slot, interned or not.
            pending.push(s.plist);
            pending.push(s.local);
            if !self.local_binding_slots.contains(&slot) {
                pending.push(s.value);
            }
        }
        // GNU's C globals and `buffer_defaults`: roots outside every value
        // cell (forwarded.rs). Per-buffer values are traced with their buffer.
        self.forwarded.visit_values(&mut |v| pending.push(*v));
        pending.extend_from_slice(&self.standard_case_tables);
        pending.push(self.command_point.1);
        self.charsets.trace(&mut pending);
        for &(slot, old, buffer, _) in &self.dynamic {
            // Both saved and active values stay roots even if Lisp switches
            // away from their buffer during the dynamic extent.
            pending.extend([old, self.slots[slot].value, buffer, Value::reference(slot + 1, 7)]);
            if self.local_binding_slots.contains(&slot) {
                local_live.insert(slot);
            }
        }
        let mut code_marks = vec![false; self.functions.len()];
        let active = !self.stack.is_empty();
        let mut code_pending: Vec<_> = self
            .request_code
            .live
            .iter()
            .enumerate()
            .filter_map(|(id, &live)| {
                (live && (active || self.request_code.permanent[id])).then_some(id)
            })
            .collect();
        self.source_code_roots(&mut code_pending);
        for (id, entry) in &self.conditions.entries {
            let custom = id.value().index_if(7).and_then(|n| n.checked_sub(1))
                .is_some_and(|slot| self.obarray_symbol_names.contains_key(&slot));
            if !custom {
                for &(key, value) in &entry.properties { pending.extend([key, value]); }
            }
        }
        let mut marked = 0;
        let mut native_nodes = HashSet::default();
        // Weak hash tables reached by marking; their entries are not traced
        // until `weak_round` (the ephemeron fixpoint) decides which survive.
        let mut weak_tables = vec![];
        while !pending.is_empty()
            || !code_pending.is_empty()
            || self.weak_round(&marks, &weak_tables, &obarray_symbols_live, &mut pending, &mut local_live)
        {
            while let Some(id) = code_pending.pop() {
                // Some low-level arena tests construct capture-only closures.
                if id >= code_marks.len() || std::mem::replace(&mut code_marks[id], true) {
                    continue;
                }
                // R215: a lazily held body is reachable exactly when its
                // function is, which may be through a closure alone.
                if let Some(lazy) = &self.functions[id].lazy {
                    pending.push(lazy.body);
                    for name in &lazy.names {
                        if let Name::Generated(id) = name {
                            if let Some(&slot) = self.generated_symbols.get(id) { pending.push(Value::reference(slot + 1, 7)); }
                        }
                    }
                    let mut forms: Vec<_> = lazy.args.iter().collect();
                    while let Some(form) = forms.pop() {
                        match form {
                            Form::Generated(id) => if let Some(&slot) = self.generated_symbols.get(id) { pending.push(Value::reference(slot + 1, 7)); },
                            Form::List(items) | Form::Vector(items) => forms.extend(items),
                            Form::Dotted(items, tail) => { forms.extend(items); forms.push(tail); }
                            _ => {}
                        }
                    }
                }
                for op in &self.functions[id].code {
                    let mut op = *op;
                    op.visit_values(&mut |v| pending.push(*v));
                    match op {
                        Op::Global(_, slot) | Op::SetGlobal(slot, _) | Op::Bind(slot, _) | Op::DeclareSpecial(slot)
                        | Op::Define(slot, _) | Op::DefineMacro(slot, _) | Op::ResolveHead(_, slot) => pending.push(Value::reference(slot + 1, 7)),
                        Op::Closure(_, child, _) => code_pending.push(child),
                        Op::Direct(_, slot, cache, _, _) | Op::Prepared(_, _, slot, cache, _, _) => {
                            pending.extend([Value::reference(slot + 1, 7), self.caches[cache].target]);
                        }
                        _ => {}
                    }
                }
                for program in std::iter::once(&self.execution).chain(&self.spare_programs) {
                    program.trace_function(id, &mut pending, &mut code_pending);
                }
            }
            let Some(mut v) = pending.pop() else { continue };
            // Walk the cdr spine directly: a list is the common editor graph,
            // not two worklist pushes and a generic type dispatch per cons.
            while v.tag() == 3 && !Self::image_value(v) {
                let i = v.payload() as usize;
                if std::mem::replace(&mut marks[0][i], true) {
                    break;
                }
                marked += 1;
                let (a, b) = self.conses[i];
                if (3..=5).contains(&(a.tag())) || a.tag() == 7 || a.tag() == 0 && a != Value::NIL {
                    pending.push(a);
                }
                v = b;
            }
            let tag = v.tag();
            if tag == 7 && v != Value::UNBOUND {
                let slot = v.payload() as usize - 1;
                if let Some(&name) = self.obarray_symbol_names.get(&slot) {
                    if obarray_symbols_live.insert(slot) {
                        let s = &self.slots[slot];
                        pending.extend([name, s.value, s.function, s.plist, s.local]);
                        if let Some(entry) = self.conditions.entries.get(&conditions::SymbolId::new(v).unwrap()) {
                            for &(key, value) in &entry.properties { pending.extend([key, value]); }
                        }
                        if let Some(Some(VariableRedirect::Alias(target))) = self.variable_redirects.get(slot) {
                            pending.push(Value::reference(target + 1, 7));
                        }
                    }
                }
                continue;
            }
            let arena = match tag {
                // Mapped objects are untraced by contract: the collector must
                // never read an image page (PROT_NONE proofs assert it). Tag-3
                // image conses already fall through to `continue`; tag-0 image
                // STRINGS did not, and indexed marks[3] with the image bit as a
                // heap index. Neither lane could see this alone - mapped strings
                // did not exist when this walk was written.
                0 if v != Value::NIL && !Self::image_value(v) => 3,
                4 | 5 => (tag - 3) as usize,
                _ => continue,
            };
            let i = v.payload() as usize - usize::from(tag == 0);
            let mark = &mut marks[arena][i];
            if std::mem::replace(mark, true) {
                continue;
            }
            marked += 1;
            if tag == 0 {
                match &mut self.objects[i] {
                    data::Object::Window(w) => w.visit_values(&mut |v| pending.push(*v)),
                    data::Object::Obarray(t) => t.visit_values(&mut |v| pending.push(*v)),
                    data::Object::Frame(f) => f.visit_values(&mut |v| pending.push(*v)),
                    data::Object::Terminal(t) => t.visit_values(&mut |v| pending.push(*v)),
                    data::Object::WindowConfiguration(c) => c.visit_values(&mut |v| pending.push(*v)),
                    data::Object::Sqlite(_) => {}
                    data::Object::Process(p) => p.visit_values(&mut |v| pending.push(*v)),
                    data::Object::Vector(values) | data::Object::Record(values) => pending.extend_from_slice(values),
                    data::Object::CharTable(t) => t.visit_values_seen(&mut native_nodes, &mut |v| pending.push(*v)),
                    data::Object::SubCharTable(node) => char_table::visit_char_table_node_values(node, &mut native_nodes, &mut |v| pending.push(*v)),
                    data::Object::MapKeymapCursor(c) => c.visit_values(&mut native_nodes, &mut |v| pending.push(*v)),
                    data::Object::Buffer(t) => t.visit_values(&mut |v| pending.push(*v)),
                    data::Object::ByteBuffer(t) => t.visit_values(&mut |v| pending.push(*v)),
                    data::Object::Marker(m) => m.visit_values(&mut |v| pending.push(*v)),
                    data::Object::Overlay(o) => o.visit_values(&mut |v| pending.push(*v)),
                    data::Object::String(s) | data::Object::RawString(s) => {
                        s.properties.visit_values(&mut |v| pending.push(*v))
                    }
                    data::Object::HashTable(t) if t.weakness == hash::Weakness::None => {
                        t.visit_values(&mut |v| pending.push(*v))
                    }
                    data::Object::HashTable(_) => weak_tables.push(i),
                    data::Object::Float(_)
                    | data::Object::Bignum(_)
                    | data::Object::BoolVector(_)
                    | data::Object::Bytes(..)
                    | data::Object::SyntaxTable(_) => {}
                }
            } else if tag == 4 {
                pending.extend_from_slice(&self.closures[i].captures);
                pending.extend_from_slice(&self.closures[i].metadata);
                pending.push(self.closures[i].document);
                code_pending.push(self.closures[i].function);
            } else {
                if let Some(&pair) = self.cell_pairs.get(&i) {
                    self.cells[i] = self.cons_pair(pair).expect("live lexical pair").1;
                    pending.push(pair);
                } else {
                    pending.push(self.cells[i]);
                }
            }
        }
        // GNU `sweep_weak_table (h, true)`: drop entries the rule rejects
        // before forwarding, which asserts that every rewritten Value is live.
        for &t in &weak_tables {
            let dead: Vec<_> = match &self.objects[t] {
                data::Object::HashTable(table) => table
                    .entries()
                    .filter(|&(_, key, value)| {
                        !(self.survives(&marks, &obarray_symbols_live, key) && self.survives(&marks, &obarray_symbols_live, value))
                    })
                    .map(|(slot, _, _)| slot)
                    .collect(),
                _ => unreachable!("weak table index"),
            };
            if let data::Object::HashTable(table) = &mut self.objects[t] {
                for slot in dead {
                    table.sweep(slot);
                }
                table.compact_local_bindings();
            }
        }
        for (i, &live) in marks[3].iter().enumerate() {
            if live {
                continue;
            }
            let data::Object::Marker(m) = &self.objects[i] else {
                continue;
            };
            let (buffer, anchor) = (m.buffer, m.anchor);
            let Some(anchor) = anchor else { continue };
            let Ok(index) = buffer.index(0) else { continue };
            match self.objects.get_mut(index.saturating_sub(1)) {
                Some(data::Object::Buffer(b)) => { b.anchors.remove(anchor); }
                Some(data::Object::ByteBuffer(b)) => {
                    if let Some(held) = &mut b.held { held.anchors.remove(anchor); }
                }
                _ => {}
            }
        }
        // Dead-owner entries have been swept. Discard only their unowned
        // slot values before forwarding; every live-owner and active-binding
        // value was marked above and is rewritten by the ordinary slot pass.
        self.free_local_binding_slots.clear();
        for &slot in &self.local_binding_slots {
            if !local_live.contains(&slot) {
                self.slots[slot].value = Value::UNBOUND;
                self.slots[slot].plist = Value::NIL;
                self.slots[slot].local = Value::NIL;
                self.free_local_binding_slots.push(slot);
            }
        }
        self.retire_code(&code_marks);
        // A completely live arena needs no forwarding table or rewrite pass.
        if marked == before && obarray_symbols_live.len() == self.obarray_symbol_names.len() {
            self.gc_retained = before;
            self.gc_stats.collections += 1;
            self.gc_threshold = before.saturating_mul(2).max(self.gc_floor);
            return;
        }
        let maps: Vec<Vec<usize>> = marks
            .iter()
            .map(|m| {
                let mut next = 0;
                m.iter()
                    .map(|&live| {
                        if live {
                            let n = next;
                            next += 1;
                            n
                        } else {
                            usize::MAX
                        }
                    })
                    .collect()
            })
            .collect();
        let rewrite = |v: &mut Value| {
            let tag = v.tag();
            let arena = match tag {
                // Same omission as the mark phase, same cause: tag 3..=5 was
                // guarded against image values but tag 0 was not. A mapped
                // string never moves, so rewriting its reference is wrong as
                // well as out of bounds.
                0 if *v != Value::NIL && !Self::image_value(*v) => 3,
                3..=5 if !Self::image_value(*v) => (tag - 3) as usize,
                _ => return,
            };
            let offset = usize::from(tag == 0);
            let i = maps[arena][v.payload() as usize - offset];
            assert_ne!(i, usize::MAX, "rewriting an unmarked root");
            *v = Value::reference(i + offset, tag);
        };
        let mut i = 0;
        self.conses.retain_mut(|p| {
            let live = marks[0][i];
            i += 1;
            if live {
                rewrite(&mut p.0);
                rewrite(&mut p.1);
            }
            live
        });
        let mut i = 0;
        self.closures.retain_mut(|c| {
            let live = marks[1][i];
            i += 1;
            if live {
                for v in c.captures.iter_mut().chain(c.metadata.iter_mut()) {
                    rewrite(v);
                }
                rewrite(&mut c.document);
            }
            live
        });
        self.cell_pairs = std::mem::take(&mut self.cell_pairs).into_iter().filter_map(|(i, mut pair)| {
            if !marks[2][i] { return None; }
            let mut cell = Value::reference(i, 5);
            rewrite(&mut cell);
            rewrite(&mut pair);
            Some((cell.payload() as usize, pair))
        }).collect();
        let mut i = 0;
        self.cells.retain_mut(|v| {
            let live = marks[2][i];
            i += 1;
            if live {
                rewrite(v);
            }
            live
        });
        for v in roots
            .iter_mut()
            .chain(self.stack.iter_mut())
            .chain(self.external_roots.iter_mut())
            .chain(self.terminal_bindings.iter_mut().map(|binding| &mut binding.buffer))
        {
            rewrite(v);
        }
        if let Some(escape) = &mut self.propagated_escape {
            escape.visit_values(&mut |v| rewrite(v));
        }
        for handler in &mut self.signal_handlers {
            handler.visit_values(&mut |v| rewrite(v));
        }
        self.backtrace.visit_values(&mut |v| rewrite(v));
        for (&slot, name) in &mut self.obarray_symbol_names {
            if obarray_symbols_live.contains(&slot) { rewrite(name); }
            else {
                let version = self.slots[slot].version.checked_add(1).expect("symbol version exhausted");
                self.slots[slot] = Slot::new();
                self.slots[slot].version = version;
                self.symbol_names[slot].clear();
                self.symbol_names[slot].shrink_to_fit();
                if let Some(redirect) = self.variable_redirects.get_mut(slot) { *redirect = None; }
                self.macros.remove(&slot);
                self.conditions.entries.remove(&conditions::SymbolId::new(Value::reference(slot + 1, 7)).unwrap());
                if let Some(id) = self.generated_slots.remove(&slot) { self.generated_symbols.remove(&id); }
                self.obarray_free_slots.push(slot);
            }
        }
        for name in self.permanent_symbol_names.values_mut() { rewrite(name); }
        self.obarray_symbol_names.retain(|slot, _| obarray_symbols_live.contains(slot));
        for s in &mut self.slots {
            rewrite(&mut s.value);
            rewrite(&mut s.function);
            rewrite(&mut s.plist);
            rewrite(&mut s.local);
        }
        self.forwarded.visit_values(&mut |v| rewrite(v));
        for v in &mut self.standard_case_tables { rewrite(v); }
        rewrite(&mut self.command_point.1);
        self.charsets.rewrite(&rewrite);
        for (_, v, buffer, _) in &mut self.dynamic {
            rewrite(v);
            rewrite(buffer);
        }
        for c in &mut self.caches {
            rewrite(&mut c.target);
        }
        let mut i = 0;
        // Forward each physical native slot once, even when several live
        // cursors retain the same subtree (possibly detached from its table).
        native_nodes.clear();
        self.objects.retain_mut(|object| {
            let live = marks[3][i];
            i += 1;
            if live {
                match object {
                    data::Object::Vector(values) | data::Object::Record(values) => {
                        for v in values {
                            rewrite(v);
                        }
                    }
                    data::Object::Window(w) => w.visit_values(&mut |v| rewrite(v)),
                    data::Object::Frame(f) => f.visit_values(&mut |v| rewrite(v)),
                    data::Object::Terminal(t) => t.visit_values(&mut |v| rewrite(v)),
                    data::Object::WindowConfiguration(c) => c.visit_values(&mut |v| rewrite(v)),
                    data::Object::Sqlite(_) => {}
                    data::Object::Process(p) => p.visit_values(&mut |v| rewrite(v)),
                    data::Object::Buffer(t) => t.visit_values(&mut |v| rewrite(v)),
                    data::Object::ByteBuffer(t) => t.visit_values(&mut |v| rewrite(v)),
                    data::Object::Marker(m) => m.visit_values(&mut |v| rewrite(v)),
                    data::Object::Overlay(o) => o.visit_values(&mut |v| rewrite(v)),
                    data::Object::String(s) | data::Object::RawString(s) => {
                        s.properties.visit_values(&mut |v| rewrite(v))
                    }
                    data::Object::HashTable(t) => t.forward(&mut |v| rewrite(v)),
                    data::Object::Obarray(t) => {
                        t.reclaim_removed(&obarray_symbols_live);
                        t.visit_values(&mut |v| rewrite(v));
                    },
                    data::Object::CharTable(t) => t.visit_values_seen(&mut native_nodes, &mut |v| rewrite(v)),
                    data::Object::SubCharTable(node) => char_table::visit_char_table_node_values(node, &mut native_nodes, &mut |v| rewrite(v)),
                    data::Object::MapKeymapCursor(c) => c.visit_values(&mut native_nodes, &mut |v| rewrite(v)),
                    data::Object::Float(_)
                    | data::Object::Bignum(_)
                    | data::Object::BoolVector(_)
                    | data::Object::Bytes(..)
                    | data::Object::SyntaxTable(_) => {}
                }
            }
            live
        });
        for f in &mut self.functions {
            // Function Arcs are private to this VM; no executable shares IR.
            let f = Arc::get_mut(f).expect("source IR must be exclusively owned at a wait point");
            for op in &mut f.code {
                op.visit_values(&mut |v| rewrite(v));
            }
            // Retired functions dropped their bodies before forwarding, so
            // every body still present was marked through its function.
            if let Some(lazy) = &mut f.lazy {
                rewrite(&mut Arc::get_mut(lazy).expect("lazy body is private at a wait point").body);
            }
        }
        for program in std::iter::once(&mut self.execution).chain(self.spare_programs.iter_mut()) {
            program.visit_values(&mut |v| rewrite(v));
        }
        self.conditions.visit_values(&mut |v| rewrite(v));
        self.objects
            .shrink_to(self.objects.len().saturating_mul(2).max(1024));
        // Release grossly oversized arenas, but retain modest slack for allocation.
        self.conses
            .shrink_to(self.conses.len().saturating_mul(2).max(16384));
        self.cells
            .shrink_to(self.cells.len().saturating_mul(2).max(1024));
        self.closures
            .shrink_to(self.closures.len().saturating_mul(2).max(1024));
        self.gc_retained = self.heap_objects();
        self.gc_stats.reclaimed += (before - self.gc_retained) as u64;
        self.gc_stats.collections += 1;
        self.gc_threshold = self.gc_retained.saturating_mul(2).max(self.gc_floor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_native_nodes_count_once_and_forward_once() {
        let mut vm = Vm::new();
        vm.conses.extend([(Value::NIL, Value::NIL); 2]);
        let payload = Value::reference(vm.conses.len(), 3);
        vm.conses.push((Value::TRUE, Value::NIL));
        let mut first = char_table::CharTable::new(Value::NIL);
        first.set(65, payload).unwrap();
        let mut second = char_table::CharTable::new(Value::NIL);
        second.root = Arc::clone(&first.root);
        second.ascii = first.ascii.clone();
        let native_bytes = char_table::CharTableNode::storage_bytes(
            &first.root, false, &mut HashSet::default(),
        );
        vm.objects.push(data::Object::CharTable(Box::new(first)));
        vm.objects.push(data::Object::CharTable(Box::new(second)));
        let mut roots = [Value::reference(1, 0), Value::reference(2, 0)];
        let stats = vm.heap_stats();
        assert_eq!(stats.live_objects, 5, "native nodes are not arena objects");
        assert_eq!(stats.live_bytes, 3 * size_of::<(Value, Value)>()
            + 2 * size_of::<data::Object>() + 2 * size_of::<char_table::CharTable>()
            + native_bytes);
        vm.collect(&mut roots);
        for root in roots {
            let data::Object::CharTable(table) = vm.object(root).unwrap() else { panic!() };
            assert_eq!(table.get(65).unwrap(), Value::reference(0, 3));
        }
        assert_eq!(vm.heap_stats().live_objects, 3);
        assert_eq!(vm.heap_stats().reclaimed, 2);
        // A completely live arena must still take the original no-move path.
        vm.collect(&mut roots);
        assert_eq!(vm.heap_stats().live_objects, 3);
        assert_eq!(vm.heap_stats().reclaimed, 2);
    }

    #[test]
    fn request_code_dies_after_cache_clear() {
        let mut vm = Vm::new();
        for n in 0..128 {
            assert_eq!(
                vm.eval_source_raw(&format!("(let ((x {n})) (+ x 1))"))
                    .unwrap()
                    .as_integer()
                    .unwrap(),
                n + 1
            );
        }
        vm.clear_source_cache();
        vm.collect(&mut []);
        assert!(
            vm.functions.iter().filter(|f| !f.code.is_empty()).count() <= 1,
            "unreachable request functions must be reclaimed"
        );
    }
    #[test]
    fn strings_and_vectors_survive_relocation_and_reclaim_exactly() {
        use data::Object;
        for vector in [false, true] {
            let mut vm = Vm::new();
            // Dead prefix plus two live strings or vectors; vector edges include
            // a cons-car object, a cell and a closure, and a self cycle.
            vm.objects
                .push(Object::String(Box::new(data::LispString::new("x"))));
            vm.objects
                .push(Object::String(Box::new(data::LispString::new("λa"))));
            let string = Value::reference(2, 0);
            let mut roots = if vector {
                vm.conses.push((string, Value::NIL));
                vm.cells.push(string);
                vm.closures.push(Closure {
                    function: 0,
                    captures: vec![string],
                    metadata: vec![], document: Value::NIL,
                });
                vm.objects.push(Object::Vector(vec![
                    string,
                    Value::reference(0, 3),
                    Value::reference(0, 5),
                    Value::reference(0, 4),
                    Value::reference(3, 0),
                ]));
                vec![Value::reference(3, 0)]
            } else {
                vec![string]
            };
            let allocated = if vector { 6 } else { 2 };
            let retained = allocated - 1;
            assert_eq!(vm.heap_stats().allocations, allocated as u64);
            vm.collect(&mut roots);
            assert_eq!(vm.heap_objects(), retained);
            assert_eq!(vm.heap_stats().reclaimed, 1);
            assert!(matches!(&vm.objects[0], Object::String(s) if s.as_str() == "λa"));
            if vector {
                assert!(
                    matches!(&vm.objects[1], Object::Vector(v) if v[0] == Value::reference(1, 0) && v[4] == roots[0])
                );
                assert_eq!(vm.conses[0].0, Value::reference(1, 0));
                assert_eq!(vm.cells[0], Value::reference(1, 0));
                assert_eq!(vm.closures[0].captures[0], Value::reference(1, 0));
            } else {
                assert_eq!(roots[0], Value::reference(1, 0));
            }
            vm.collect(&mut []);
            assert_eq!(vm.heap_objects(), 0);
            assert_eq!(vm.heap_stats().allocations, allocated as u64);
            assert_eq!(vm.heap_stats().reclaimed, allocated as u64);
        }
    }
    #[test]
    fn statistics_measure_real_storage_and_exclude_private_slots() {
        fn rows(vm: &mut Vm) -> HashMap<String, Vec<i64>> {
            let mut result = HashMap::default();
            let mut list = vm.garbage_collect_statistics();
            while list != Value::NIL {
                let (mut row, next) = vm.cons_pair(list).unwrap();
                let (name, rest) = vm.cons_pair(row).unwrap();
                row = rest;
                let mut numbers = Vec::new();
                while row != Value::NIL {
                    let (number, next) = vm.cons_pair(row).unwrap();
                    numbers.push(number.integer_value().unwrap());
                    row = next;
                }
                result.insert(vm.symbol_names[name.payload() as usize - 1].clone(), numbers);
                list = next;
            }
            result
        }
        let mut vm = Vm::new();
        let symbol_count = vm.slots.len();
        let private = vm.slots.len();
        vm.slots.push(Slot::new());
        vm.symbol_names.push("private".into());
        vm.local_binding_slots.insert(private);
        vm.objects.push(data::Object::String(Box::new(data::LispString::new("abc"))));
        vm.objects.push(data::Object::Vector(vec![Value::NIL; 7]));
        vm.objects.push(data::Object::Float(1.0));
        let actual = rows(&mut vm);
        assert_eq!(actual["symbols"][1], symbol_count as i64, "GC_PUBLIC_SYMBOLS_ONLY");
        assert_eq!(actual["strings"][1], 1, "GC_REAL_STRINGS");
        assert_eq!(actual["string-bytes"], [1, 3], "GC_REAL_STRING_BYTES");
        assert_eq!(actual["vectors"][1], 1, "GC_REAL_VECTORS");
        assert_eq!(actual["vector-slots"][1], 7, "GC_REAL_VECTOR_SLOTS");
        assert_eq!(actual["floats"][1], 1, "GC_REAL_FLOATS");
        assert!(!actual.contains_key("intervals"), "GC_NO_FABRICATED_INTERVAL_POOL");
    }

    #[test]
    fn private_cycles_survive_compaction_and_are_reclaimed_when_unrooted() {
        let mut vm = Vm::new();
        vm.conses.push((Value::integer(99).unwrap(), Value::NIL));
        vm.cells.push(Value::reference(0, 4));
        vm.closures.push(Closure {
            function: 0,
            captures: vec![Value::reference(0, 5)],
            metadata: vec![], document: Value::NIL,
        });
        vm.conses
            .push((Value::reference(0, 5), Value::reference(1, 3)));
        let mut roots = [Value::reference(1, 3)];
        vm.collect(&mut roots);
        assert_eq!(vm.heap_objects(), 3);
        assert_eq!(roots[0], Value::reference(0, 3));
        assert_eq!(vm.conses[0].1, roots[0]);
        assert_eq!(vm.cells[0], Value::reference(0, 4));
        assert_eq!(vm.closures[0].captures[0], Value::reference(0, 5));
        vm.collect(&mut []);
        assert_eq!(vm.heap_stats().allocations, 4);
        assert_eq!(vm.heap_stats().reclaimed, 4);
        assert_eq!(vm.heap_objects(), 0);
    }
}
