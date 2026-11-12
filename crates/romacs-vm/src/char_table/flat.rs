//! A char table's values as flat rows, for what reads one character after
//! another: case conversion and comparison, syntax scans, regexp syntax.
//!
//! A view holds what `Vm::char_table_get` gives for each character: the
//! table's own value, its default for nil, a parent's where it has none.
//! Its ASCII row is read when it is made, a page of 256 characters when one
//! of them is first read. The VM keeps a few (`VIEWS`), the tables read
//! last, and they hold while no char table is written (`writes`): a write
//! to any table drops them all, and so does a collection, since they hold
//! Values. A table's own count of writes would not do: a write to a parent,
//! or to a node another table shares (`set-char-table-range` with a
//! sub-char-table), changes what a table reads without counting there. A
//! view is made by the first read under its table, so a table set for one
//! call costs its row then, and nothing before.
use super::{CharTable, CharTableNode, CharTableNodeHandle, CharTableSlot, Result, Value, Vm, data, row_read, writes};
use std::cell::{Cell, OnceCell};

/// The characters a page holds.
const PAGE: usize = 256;
type Page = [Value; PAGE];
/// The pages of 65,536 characters, each read on first use.
type Plane = [OnceCell<Box<Page>>; 256];
/// The views a VM keeps, the one least recently made current replaced first.
const VIEWS: usize = 8;
/// The pages a view reads; a character of a page past them is read node
/// by node. With `VIEWS`, what the views hold stays near 2 MiB at most,
/// however many tables a session reads and however much text under them.
const PAGES: usize = 64;

pub(crate) struct FlatTable {
    /// The table read; the VM's views do not outlive a collection.
    table: Value,
    ascii: [Value; 128],
    planes: [OnceCell<Box<Plane>>; CharTable::LEN >> 16],
    /// The pages read so far.
    pages: Cell<usize>,
    /// When it was last made current (`FlatTables::clock`).
    used: u64,
}

impl FlatTable {
    /// `Vm::char_table_get` of CODE in VM's table this is the view of.
    #[inline]
    pub(crate) fn get(&self, vm: &Vm, code: u32) -> Result<Value> {
        match self.ascii.get(code as usize) {
            Some(&value) => Ok(value),
            None => self.paged(vm, code as usize),
        }
    }

    fn paged(&self, vm: &Vm, code: usize) -> Result<Value> {
        let plane = self.planes.get(code >> 16).ok_or("index out of range")?;
        if let Some(page) = plane.get().and_then(|plane| plane[(code >> 8) & 0xff].get()) {
            return Ok(page[code & (PAGE - 1)]);
        }
        if self.pages.get() == PAGES { return vm.char_table_get(self.table, code); }
        let mut page = Box::new([Value::NIL; PAGE]);
        vm.char_table_fill(self.table, code & !(PAGE - 1), &mut page[..])?;
        self.pages.set(self.pages.get() + 1);
        let plane = plane.get_or_init(|| Box::new([const { OnceCell::new() }; 256]));
        Ok(plane[(code >> 8) & 0xff].get_or_init(|| page)[code & (PAGE - 1)])
    }
}

/// The views a VM keeps, all read under one count of char-table writes:
/// at most `VIEWS`, each where it was made. Dropping them, at a write or
/// a collection, costs those few, whatever number of tables were read
/// before.
#[derive(Default)]
pub(crate) struct FlatTables {
    writes: u64,
    /// Counts the times a view was made current.
    clock: u64,
    views: Vec<FlatTable>,
}

impl FlatTables {
    /// Drop every view: a collection moves the Values they hold.
    pub(crate) fn forget(&mut self) {
        self.views.clear();
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.views.is_empty()
    }
    /// Where TABLE's view is, when it has one and no char table has been
    /// written since the views were read. Between collections a table
    /// keeps its Value and no other object takes it.
    fn place(&self, table: Value) -> Option<usize> {
        if self.writes != writes() { return None; }
        self.views.iter().position(|view| view.table == table)
    }
}

/// A char table read for one borrow of the VM, which keeps it unwritten:
/// through its view when it has a current one, else node by node.
#[derive(Clone, Copy)]
pub(crate) struct TableReader<'a> {
    vm: &'a Vm,
    table: Value,
    flat: Option<&'a FlatTable>,
}

impl TableReader<'_> {
    /// `Vm::char_table_get` of CODE.
    #[inline]
    pub(crate) fn get(&self, code: u32) -> Result<Value> {
        match self.flat {
            Some(flat) => flat.get(self.vm, code),
            None => self.vm.char_table_get(self.table, code as usize),
        }
    }
}

impl CharTableNode {
    /// The values of this node of single characters from FROM into OUT.
    fn leaf_values(&self, from: usize, out: &mut [Value]) -> Option<()> {
        let slots = self.slots.get(from.checked_sub(self.min_char)?..)?.get(..out.len())?;
        for (value, slot) in out.iter_mut().zip(slots) {
            let CharTableSlot::Value(own) = slot else { return None };
            *value = *own;
        }
        Some(())
    }

    /// The raw values of the characters from FROM into OUT under HANDLE,
    /// which is to be the node at DEPTH whose characters start at `min_char`:
    /// `checked_span`'s values and refusals, a node's slots read together.
    fn read(handle: &CharTableNodeHandle, depth: usize, min_char: usize, from: usize, out: &mut [Value]) -> Result<()> {
        const GEOMETRY: &str = "invalid sub-char-table geometry";
        let width = *Self::WIDTHS.get(depth).ok_or(GEOMETRY)?;
        let whole = |node: &Self| node.depth == depth && node.min_char == min_char;
        if width == 1 {
            let node = handle.lock().unwrap();
            return whole(&node).then(|| node.leaf_values(from, out)).flatten().ok_or_else(|| GEOMETRY.into());
        }
        let end = from + out.len();
        let mut at = from;
        while at < end {
            let index = at.checked_sub(min_char).ok_or(GEOMETRY)? / width;
            let start = min_char + index * width;
            let stop = (start + width).min(end);
            // A slot may hold any node: this one's lock is not held over it.
            let slot = {
                let node = handle.lock().unwrap();
                if !whole(&node) { return Err(GEOMETRY.into()); }
                node.slots.get(index).cloned().ok_or(GEOMETRY)?
            };
            let part = &mut out[at - from..stop - from];
            match slot {
                CharTableSlot::Value(value) => part.fill(value),
                CharTableSlot::Node(child) => Self::read(&child, depth + 1, start, at, part)?,
            }
            at = stop;
        }
        Ok(())
    }
}

impl CharTable {
    /// This table's own values of the characters from FROM into OUT, its
    /// default for nil: `span`'s value of each, read a node at a time.
    fn read_own(&self, from: usize, out: &mut [Value]) -> Result<()> {
        row_read();
        if from + out.len() > Self::LEN { return Err("index out of range".into()); }
        // ASCII reads the table's own ASCII slot, which a literal may
        // detach from the root.
        let (low, high) = out.split_at_mut(128usize.saturating_sub(from).min(out.len()));
        match &self.ascii {
            _ if low.is_empty() => {}
            CharTableSlot::Value(value) => low.fill(*value),
            CharTableSlot::Node(node) => {
                let node = node.lock().unwrap();
                (node.depth == 3 && node.min_char == 0).then(|| node.leaf_values(from, low)).flatten()
                    .ok_or("invalid ASCII sub-char-table")?;
            }
        }
        CharTableNode::read(&self.root, 0, 0, from + low.len(), high)?;
        for value in out {
            if *value == Value::NIL { *value = self.default; }
        }
        Ok(())
    }
}

impl Vm {
    /// Whether TABLE's chain of char-table parents loops: Floyd's walk,
    /// which allocates nothing.
    pub(crate) fn char_table_parents_cycle(&self, table: Value) -> bool {
        let parent = |t: Value| match self.object_if(t) {
            Some(data::Object::CharTable(c)) => Some(c.parent()),
            _ => None,
        };
        let (mut slow, mut fast) = (table, table);
        loop {
            let Some(next) = parent(fast).and_then(parent) else { return false };
            fast = next;
            let Some(trail) = parent(slow) else { return false };
            slow = trail;
            if slow == fast {
                return true;
            }
        }
    }

    /// Whether TABLE reads flat: a char table whose parents do not loop,
    /// none of them a `char-code-property-table`, whose blocks may be the
    /// compressed strings `char_table_own` decodes.
    fn char_table_flattens(&self, table: Value) -> bool {
        if !matches!(self.object_if(table), Some(data::Object::CharTable(_))) || self.char_table_parents_cycle(table) {
            return false;
        }
        let mut at = table;
        while let Some(data::Object::CharTable(t)) = self.object_if(at) {
            if self.is_uniprop_table(t) { return false; }
            at = t.parent();
        }
        true
    }

    /// `char_table_get` of each character from FROM into OUT, at most a
    /// page, for a TABLE that `char_table_flattens`.
    fn char_table_fill(&self, table: Value, from: usize, out: &mut [Value]) -> Result<()> {
        let data::Object::CharTable(t) = self.object(table)? else { return Err("char-table required".into()) };
        t.read_own(from, out)?;
        let mut parent = t.parent();
        let mut inherited = [Value::NIL; PAGE];
        let inherited = &mut inherited[..out.len()];
        while out.contains(&Value::NIL) {
            let Some(data::Object::CharTable(t)) = self.object_if(parent) else { break };
            t.read_own(from, inherited)?;
            for (value, &theirs) in out.iter_mut().zip(inherited.iter()) {
                if *value == Value::NIL { *value = theirs; }
            }
            parent = t.parent();
        }
        Ok(())
    }

    /// TABLE's view, when it has a current one.
    #[inline]
    pub(crate) fn flat_table(&self, table: Value) -> Option<&FlatTable> {
        let flat = &self.flat_tables;
        flat.place(table).map(|place| &flat.views[place])
    }

    /// Make TABLE's view current: read its ASCII row unless it has one
    /// read since the last write to a char table. Whether it had one.
    /// No view is made of what does not read flat
    /// (`char_table_flattens`), which is read node by node.
    pub(crate) fn flatten_table(&mut self, table: Value) -> bool {
        self.flat_tables.clock += 1;
        let used = self.flat_tables.clock;
        if let Some(place) = self.flat_tables.place(table) {
            self.flat_tables.views[place].used = used;
            return true;
        }
        let now = writes();
        if self.flat_tables.writes != now {
            self.flat_tables.forget();
            self.flat_tables.writes = now;
        }
        let mut ascii = [Value::NIL; 128];
        if !self.char_table_flattens(table) || self.char_table_fill(table, 0, &mut ascii).is_err() { return false; }
        let planes = [const { OnceCell::new() }; CharTable::LEN >> 16];
        let view = FlatTable { table, ascii, planes, pages: Cell::new(0), used };
        let views = &mut self.flat_tables.views;
        match views.iter().enumerate().min_by_key(|(_, view)| view.used) {
            Some((oldest, _)) if views.len() == VIEWS => views[oldest] = view,
            _ => views.push(view),
        }
        false
    }

    /// TABLE to read for this borrow of the VM (`TableReader`).
    #[inline]
    pub(crate) fn char_table_reader(&self, table: Value) -> TableReader<'_> {
        TableReader { vm: self, table, flat: self.flat_table(table) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::char_table::{reads, writes_here};

    /// CHECK's value from a run during which no other thread wrote a char
    /// table: the count of writes is the process's, and another test's VM
    /// on another thread drops this one's views with each of its own.
    fn undisturbed<T>(vm: &mut Vm, mut check: impl FnMut(&mut Vm) -> T) -> T {
        loop {
            let before = (writes(), writes_here());
            let value = check(vm);
            if writes() - before.0 == u64::try_from(writes_here() - before.1).unwrap() { return value; }
            std::thread::yield_now();
        }
    }

    /// Whether each of TABLES has a view once each was to be given one.
    fn flattened(vm: &mut Vm, tables: &[Value]) -> Vec<bool> {
        undisturbed(vm, |vm| {
            for &table in tables { vm.flatten_table(table); }
            tables.iter().map(|&table| vm.flat_table(table).is_some()).collect()
        })
    }

    /// A view gives `char_table_get` of every character read through it:
    /// own values, defaults, parents, whole-slot and single writes, at
    /// each level of the subdivision.
    #[test]
    fn a_view_gives_what_a_node_by_node_read_gives() {
        let mut vm = Vm::new();
        let int = |n: usize| Value::integer(i64::try_from(n).unwrap()).unwrap();
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move |bound: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            usize::try_from(seed % u64::try_from(bound).unwrap()).unwrap()
        };
        for round in 0..40 {
            let mut table = Value::NIL;
            for level in 0..=round % 3 {
                let mut child = CharTable::new(Value::NIL);
                if next(3) == 0 { child.set_default(int(900 + level)); }
                for _ in 0..next(12) {
                    let from = match next(4) { 0 => next(128), 1 => next(0x3000), 2 => next(0x30000), _ => next(CharTable::LEN) };
                    let to = (from + match next(4) { 0 => 0, 1 => next(40), 2 => next(5000), _ => next(200_000) }).min(CharTable::LEN - 1);
                    let value = if next(5) == 0 { Value::NIL } else { int(next(1000)) };
                    child.set_range(from, to, value).unwrap();
                }
                child.set_parent(table);
                table = vm.allocate_object(data::Object::CharTable(Box::new(child)));
            }
            let mut codes: Vec<usize> = (0..300).collect();
            codes.extend([0x3ff, 0x400, 0xffff, 0x10000, 0x10001, 0x3f_ff7f, 0x3f_ff80, CharTable::LEN - 1]);
            codes.extend((0..400).map(|_| next(CharTable::LEN)));
            codes.extend((0..200).map(|_| next(0x3000)));
            let flat = undisturbed(&mut vm, |vm| {
                vm.flatten_table(table);
                let reader = vm.char_table_reader(table);
                for &code in &codes {
                    assert_eq!(reader.get(u32::try_from(code).unwrap()), vm.char_table_get(table, code), "round {round}, character {code}");
                }
                assert!(reader.get(u32::try_from(CharTable::LEN).unwrap()).is_err());
                reader.flat.is_some()
            });
            assert!(flat, "round {round}: a plain table reads flat");
        }
    }

    /// A literal's ASCII slot may differ from its root's: ASCII reads it.
    #[test]
    fn a_view_reads_a_detached_ascii_slot() {
        let mut vm = Vm::new();
        let mut fields = vec![Value::NIL; 68];
        fields[3] = Value::integer(7).unwrap();
        fields[4] = Value::integer(8).unwrap();
        let table = vm.char_table_from_literal(&fields).unwrap();
        assert_eq!(flattened(&mut vm, &[table]), [true]);
        let read = undisturbed(&mut vm, |vm| {
            vm.flatten_table(table);
            let reader = vm.char_table_reader(table);
            [65, 128, 255, 65536].map(|code| (reader.get(code), vm.char_table_get(table, code as usize)))
        });
        for (flat, plain) in read { assert_eq!(flat, plain); }
    }

    /// A page is read once for all its characters, a table's parents
    /// with it; what does not read flat is read node by node.
    #[test]
    fn a_page_is_read_once() {
        let mut vm = Vm::new();
        let table = vm.eval_source_raw("(let ((p (make-char-table nil)) (c (make-char-table nil)))
              (set-char-table-range p '(900 . 1200) 'parent) (aset c 945 'own) (set-char-table-parent c p) c)").unwrap();
        let (read, descents) = undisturbed(&mut vm, |vm| {
            vm.flat_tables.forget();
            vm.flatten_table(table);
            let before = reads();
            let reader = vm.char_table_reader(table);
            let read = [945, 946, 1000, 899].map(|code| vm.display(reader.get(code).unwrap()).unwrap());
            for code in 768..1024 { reader.get(code).unwrap(); }
            let after = reads();
            (read, [after[0] - before[0], after[1] - before[1]])
        });
        assert_eq!(read, ["own", "parent", "parent", "nil"]);
        assert_eq!(descents, [0, 2], "no character alone; one page, of the table and of its parent");
        let uniprop = vm.eval_source_raw("(progn (put 'char-code-property-table 'char-table-extra-slots 5)
              (make-char-table 'char-code-property-table))").unwrap();
        assert_eq!(flattened(&mut vm, &[uniprop, Value::NIL]), [false, false]);
    }

    /// The VM keeps the views of the tables read last, however many were
    /// read, and a view reads at most `PAGES` pages, the rest node by node.
    #[test]
    fn the_views_and_their_pages_are_bounded() {
        let mut vm = Vm::new();
        let tables: Vec<Value> = (0..3 * VIEWS).map(|i| {
            let mut table = CharTable::new(Value::NIL);
            table.set_range(0, CharTable::LEN - 1, Value::integer(i64::try_from(i).unwrap()).unwrap()).unwrap();
            vm.allocate_object(data::Object::CharTable(Box::new(table)))
        }).collect();
        let (kept, oldest) = undisturbed(&mut vm, |vm| {
            for &table in &tables { vm.flatten_table(table); }
            (vm.flat_tables.views.len(), vm.flat_table(tables[0]).is_some())
        });
        assert_eq!((kept, oldest), (VIEWS, false));
        let last = *tables.last().unwrap();
        let (pages, read) = undisturbed(&mut vm, |vm| {
            vm.flatten_table(last);
            let reader = vm.char_table_reader(last);
            let read: Vec<Value> = (0..2 * PAGES).map(|page| reader.get(u32::try_from(1000 + page * PAGE).unwrap()).unwrap()).collect();
            (vm.flat_table(last).unwrap().pages.get(), read)
        });
        assert_eq!(pages, PAGES);
        assert!(read.iter().all(|&value| value == Value::integer(i64::try_from(3 * VIEWS - 1).unwrap()).unwrap()));
    }

    /// A write to any char table, and a collection, leave no view to read.
    #[test]
    fn a_write_or_a_collection_drops_the_views() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(setq flat-a (make-char-table nil 1) flat-b (make-char-table nil 2))").unwrap();
        let table = |vm: &Vm, name: &str| vm.slots[vm.names[name]].value;
        let (a, b) = (table(&vm, "flat-a"), table(&vm, "flat-b"));
        assert_eq!(flattened(&mut vm, &[a, b]), [true, true]);
        vm.eval_source_raw("(aset flat-b 65 3)").unwrap();
        assert!(vm.flat_table(a).is_none() && vm.flat_table(b).is_none(), "a write to one table drops every view");
        assert_eq!(flattened(&mut vm, &[a]), [true]);
        assert!(vm.flat_table(b).is_none(), "a view is made by a read, not by a write");
        vm.collect(&mut []);
        let a = table(&vm, "flat-a");
        assert!(vm.flat_table(a).is_none(), "a collection drops the views");
        assert_eq!(vm.char_table_reader(a).get(65).unwrap().integer_value(), Some(1));
    }
}
