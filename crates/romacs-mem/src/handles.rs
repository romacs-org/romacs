//! Heap-owned registered storage. Handles only name entries; no borrowed value
//! can survive a mutable heap boundary. S4 will scan these same tables.
use crate::{Heap, Value};
use std::{
    cell::{Cell, RefCell},
    marker::PhantomData,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};
const CHUNK_WORDS: usize = 64 * 1024;

pub(crate) struct Tables {
    pub alive: Cell<bool>,
    pub id: u64,
    pub weak_refs: RefCell<std::collections::BTreeMap<u64, u64>>,
    pub next_weak: Cell<u64>,
    pub roots: RefCell<Vec<Option<RootEntry>>>,
    pub stacks: RefCell<Vec<StackEntry>>,
}
pub(crate) struct RootEntry {
    pub word: u64,
    pub owner: &'static str,
}
pub(crate) struct StackEntry {
    generation: u32,
    pub storage: Option<Rc<RefCell<Storage>>>,
}
pub(crate) struct Storage {
    chunks: Vec<Box<[AtomicU64]>>,
    #[cfg(feature = "verify")]
    births: Vec<Box<[u64]>>,
    len: u32,
    incarnation: u64,
}
impl Tables {
    pub fn new() -> Self {
        // Only identities are global, never heaps or Lisp objects. Never reuse after teardown.
        static NEXT_HEAP: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_HEAP
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("heap identity exhausted");
        Self {
            alive: Cell::new(true),
            id,
            weak_refs: RefCell::new(std::collections::BTreeMap::new()),
            next_weak: Cell::new(0),
            roots: RefCell::new(Vec::new()),
            stacks: RefCell::new(Vec::new()),
        }
    }
    fn check(&self, heap: &Heap) {
        assert!(
            self.alive.get() && std::ptr::eq(self, Rc::as_ptr(&heap.tables)),
            "dead or foreign heap handle"
        );
    }
}
impl Storage {
    pub(crate) fn trace(&self, work: &mut Vec<crate::raw::RawValue>) {
        for i in 0..self.len {
            work.push(crate::raw::RawValue::from_value(Value::word(
                self.word(i).load(Ordering::Relaxed),
            )));
        }
    }
    fn new() -> Self {
        Self {
            chunks: Vec::new(),
            #[cfg(feature = "verify")]
            births: Vec::new(),
            len: 0,
            incarnation: 0,
        }
    }
    fn word(&self, i: u32) -> &AtomicU64 {
        assert!(i < self.len, "stack index outside traced range");
        &self.chunks[i as usize / CHUNK_WORDS][i as usize % CHUNK_WORDS]
    }
    fn push(&mut self, word: u64) {
        let next = self.len.checked_add(1).expect("stack exhausted");
        if self.len as usize == self.chunks.len() * CHUNK_WORDS {
            self.chunks
                .push((0..CHUNK_WORDS).map(|_| AtomicU64::new(0)).collect());
            #[cfg(feature = "verify")]
            self.births.push(vec![0; CHUNK_WORDS].into_boxed_slice());
        }
        #[cfg(feature = "verify")]
        {
            let i = self.len as usize;
            self.births[i / CHUNK_WORDS][i % CHUNK_WORDS] = self.incarnation;
        }
        self.len = next;
        self.word(next - 1).store(word, Ordering::Relaxed);
    }
}

/// Registered, owned root. Not Clone and not Send. Forgetting it leaks a root,
/// never leaves a dangling value. The table is owned jointly with the heap.
pub struct Root {
    tables: Rc<Tables>,
    index: usize,
}
impl Root {
    pub fn new(heap: &Heap, value: Value<'_>, owner: &'static str) -> Self {
        let mut roots = heap.tables.roots.borrow_mut();
        let index = roots
            .iter()
            .position(Option::is_none)
            .unwrap_or(roots.len());
        let entry = Some(RootEntry {
            word: value.0,
            owner,
        });
        if index == roots.len() {
            roots.push(entry);
        } else {
            roots[index] = entry;
        }
        Self {
            tables: heap.tables.clone(),
            index,
        }
    }
    pub fn get<'h>(&self, heap: &'h Heap) -> Value<'h> {
        self.tables.check(heap);
        Value::word(
            self.tables.roots.borrow()[self.index]
                .as_ref()
                .expect("retired root")
                .word,
        )
    }
    pub fn set(&mut self, heap: &Heap, value: Value<'_>) {
        self.tables.check(heap);
        self.tables.roots.borrow_mut()[self.index]
            .as_mut()
            .expect("retired root")
            .word = value.0;
    }
    pub fn owner(&self) -> &'static str {
        self.tables.roots.borrow()[self.index]
            .as_ref()
            .expect("retired root")
            .owner
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        self.tables.roots.borrow_mut()[self.index] = None;
    }
}

/// Owned handle to registered, fixed-chunk heap storage. Growth never moves a
/// word, and truncation immediately removes words from the traced range.
pub struct ValueStack {
    tables: Rc<Tables>,
    storage: Rc<RefCell<Storage>>,
    index: u32,
    generation: u32,
}
impl ValueStack {
    pub fn new(heap: &Heap) -> Self {
        let storage = Rc::new(RefCell::new(Storage::new()));
        let mut entries = heap.tables.stacks.borrow_mut();
        // Exhausted generations are retired rather than wrapping and reviving frames.
        let index = entries
            .iter()
            .position(|e| e.storage.is_none() && e.generation < u32::MAX)
            .unwrap_or(entries.len());
        let generation = if index == entries.len() {
            assert!(index < u32::MAX as usize, "stack table exhausted");
            entries.push(StackEntry {
                generation: 1,
                storage: Some(storage.clone()),
            });
            1
        } else {
            let e = &mut entries[index];
            e.generation += 1;
            e.storage = Some(storage.clone());
            e.generation
        };
        Self {
            tables: heap.tables.clone(),
            storage,
            index: index as u32,
            generation,
        }
    }
    pub fn len(&self) -> u32 {
        self.storage.borrow().len
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn push(&mut self, heap: &Heap, value: Value<'_>) {
        self.tables.check(heap);
        self.storage.borrow_mut().push(value.0);
    }
    pub fn truncate(&mut self, len: u32) {
        assert!(self.tables.alive.get(), "dead stack");
        let mut s = self.storage.borrow_mut();
        assert!(len <= s.len, "truncate grows stack");
        if len != s.len {
            s.incarnation = s
                .incarnation
                .checked_add(1)
                .expect("frame incarnation exhausted");
            s.len = len;
        }
    }
    pub fn frame(&self, base: u32, len: u32) -> Frame {
        assert!(self.tables.alive.get(), "dead stack");
        let s = self.storage.borrow();
        assert!(
            base.checked_add(len).is_some_and(|end| end <= s.len),
            "frame outside stack"
        );
        Frame {
            stack: self.index,
            generation: self.generation,
            base,
            len,
            #[cfg(feature = "verify")]
            incarnation: s.incarnation,
            local: PhantomData,
        }
    }
}
impl Drop for ValueStack {
    fn drop(&mut self) {
        self.tables.stacks.borrow_mut()[self.index as usize].storage = None;
    }
}

/// Copyable frame identity, not a borrow. Fields are private so ranges and
/// generations can only originate from a registered stack. Deliberately !Send.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    stack: u32,
    generation: u32,
    base: u32,
    len: u32,
    #[cfg(feature = "verify")]
    incarnation: u64,
    local: PhantomData<*const ()>,
}
impl Frame {
    pub fn len(self) -> u32 {
        self.len
    }
    pub fn is_empty(self) -> bool {
        self.len == 0
    }
    fn index(self, s: &Storage, i: u32) -> u32 {
        assert!(i < self.len, "frame index out of bounds");
        let index = self.base.checked_add(i).expect("frame index overflow");
        assert!(index < s.len, "popped frame");
        #[cfg(feature = "verify")]
        assert!(
            s.births[index as usize / CHUNK_WORDS][index as usize % CHUNK_WORDS]
                <= self.incarnation,
            "stale frame incarnation"
        );
        index
    }
    pub fn get<'h>(self, heap: &'h Heap, i: u32) -> Value<'h> {
        assert!(heap.tables.alive.get(), "dead heap");
        let entries = heap.tables.stacks.borrow();
        let entry = entries
            .get(self.stack as usize)
            .expect("unknown frame stack");
        assert_eq!(entry.generation, self.generation, "stale frame generation");
        let s = entry
            .storage
            .as_ref()
            .expect("retired frame stack")
            .borrow();
        Value::word(s.word(self.index(&s, i)).load(Ordering::Relaxed))
    }
    pub fn set(self, stack: &mut ValueStack, i: u32, value: Value<'_>) {
        assert!(stack.tables.alive.get(), "dead stack");
        assert!(
            self.stack == stack.index && self.generation == stack.generation,
            "wrong frame stack"
        );
        let s = stack.storage.borrow();
        s.word(self.index(&s, i)).store(value.0, Ordering::Relaxed);
    }
}

/// Pure subr view; only the heap is borrowed, so the evaluator can store the
/// result through its disjoint ValueStack handle without a mutable heap borrow.
#[derive(Clone, Copy)]
pub struct Args<'h> {
    heap: &'h Heap,
    frame: Frame,
}
impl<'h> Args<'h> {
    pub fn new(heap: &'h Heap, frame: Frame) -> Self {
        if !frame.is_empty() {
            frame.get(heap, frame.len() - 1);
        }
        Self { heap, frame }
    }
    pub fn heap(self) -> &'h Heap {
        self.heap
    }
    pub fn len(self) -> u32 {
        self.frame.len()
    }
    pub fn is_empty(self) -> bool {
        self.frame.is_empty()
    }
    pub fn get(self, i: u32) -> Value<'h> {
        self.frame.get(self.heap, i)
    }
}
