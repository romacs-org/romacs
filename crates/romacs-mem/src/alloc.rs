//! S2 storage, not object semantics or tracing. The allocator owns mappings,
//! the header owns allocation/mark bits, and the mutator alone changes either.
use crate::{AllocError, Kind, os::Mapping};
use std::{
    ptr::NonNull,
    sync::atomic::{AtomicU64, Ordering},
};
pub const PAGE_BYTES: usize = 64 * 1024;
pub const HEADER_BYTES: usize = 4096;
pub const CHUNK_BYTES: usize = 4 * 1024 * 1024;
pub const SYMBOLS_PER_PAGE: usize = (PAGE_BYTES - HEADER_BYTES) / 48;
pub const SYMBOL_LIMIT: usize = 1 << 30;

pub const VECTOR_CLASSES: [usize; 25] = [
    16, 32, 48, 64, 80, 96, 128, 160, 192, 256, 320, 384, 512, 640, 768, 1024, 1280, 1536, 2048,
    2560, 3072, 4096, 5120, 6144, 8192,
];
const BIT_WORDS: usize = (PAGE_BYTES - HEADER_BYTES) / 8 / 64;
const NONE: u32 = u32::MAX;
const CLASSES: usize = 3 + VECTOR_CLASSES.len();

/// GNU memory-use-counts order. Counters are cumulative allocator events;
/// live bytes and allocation debt are separate, never reconstructed by scanning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub conses: u64,
    pub floats: u64,
    pub vector_cells: u64,
    pub symbols: u64,
    pub string_chars: u64,
    pub intervals: u64,
    pub strings: u64,
}
impl Counters {
    pub fn memory_use_counts(self) -> [u64; 7] {
        [
            self.conses,
            self.floats,
            self.vector_cells,
            self.symbols,
            self.string_chars,
            self.intervals,
            self.strings,
        ]
    }
    fn allocated(&mut self, kind: Kind, units: usize) {
        match kind {
            Kind::Cons => self.conses += 1,
            Kind::Float => self.floats += 1,
            Kind::String => {
                self.strings += 1;
                self.string_chars += units as u64;
            }
            Kind::Symbol => self.symbols += 1,
            Kind::Vectorlike => self.vector_cells += units as u64,
            Kind::Fixnum => unreachable!("immediates do not allocate"),
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct AllocationStats {
    pub counts: Counters,
    pub bytes_since_cycle: usize,
    pub live_bytes: usize,
    pub mapped_bytes: usize,
    /// Initialized allocator pages (headers only after release), large spans,
    /// and committed symbol pages. Excludes untouched chunk tails and external
    /// Rust allocations; this is a storage budget, not resident RSS.
    pub committed_bytes: usize,
    pub committed_symbol_bytes: usize,
    pub released_pages: usize,
    pub pages: usize,
    pub large_objects: usize,
}
/// Fixed class cursors: direct indices into the page table, no future JIT policy.
#[repr(C)]
struct MutatorHot {
    pages: [usize; CLASSES],
}
impl Default for MutatorHot {
    fn default() -> Self {
        Self {
            pages: [usize::MAX; CLASSES],
        }
    }
}

#[repr(C)]
struct Header {
    class: usize,
    size: usize,
    capacity: u32,
    bump: u32,
    free: u32,
    live: u32,
    empty_cycles: u8,
    released: bool,
    allocated: [u64; BIT_WORDS],
    mark: [AtomicU64; BIT_WORDS],
    #[cfg(feature = "verify")]
    verify: [u64; BIT_WORDS],
}
const _: () = assert!(std::mem::size_of::<Header>() <= HEADER_BYTES);
struct Page {
    base: NonNull<u8>,
    chunk: usize,
    kind: Kind,
}
impl Page {
    /// `base` is an exclusively owned committed page from a chunk or symbol arena.
    unsafe fn new(base: NonNull<u8>, chunk: usize, kind: Kind, class: usize, size: usize) -> Self {
        let header = Header {
            class,
            size,
            capacity: ((PAGE_BYTES - HEADER_BYTES) / size) as u32,
            bump: 0,
            free: NONE,
            live: 0,
            empty_cycles: 0,
            released: false,
            allocated: [0; BIT_WORDS],
            mark: std::array::from_fn(|_| AtomicU64::new(0)),
            #[cfg(feature = "verify")]
            verify: [0; BIT_WORDS],
        };
        // SAFETY: caller supplies aligned writable page; header fits first 4 KiB.
        unsafe {
            base.cast::<Header>().as_ptr().write(header);
        }
        Self { base, chunk, kind }
    }
    fn h(&self) -> &Header {
        // SAFETY: initialized header lives until this page's owning mapping is freed.
        unsafe { self.base.cast::<Header>().as_ref() }
    }
    fn h_mut(&mut self) -> &mut Header {
        // SAFETY: page metadata has a single mutator owner; no header borrow escapes.
        unsafe { self.base.cast::<Header>().as_mut() }
    }
    fn capacity(&self) -> u32 {
        self.h().capacity
    }
    fn available(&self) -> bool {
        self.h().free != NONE || self.h().bump < self.capacity() || self.h().released
    }
    fn cell(&self, index: u32) -> NonNull<u8> {
        assert!(index < self.capacity());
        // SAFETY: index lies in page payload, slot size is a supported class.
        unsafe {
            NonNull::new_unchecked(
                self.base
                    .as_ptr()
                    .add(HEADER_BYTES + index as usize * self.h().size),
            )
        }
    }
    fn allocated(&self, index: u32) -> bool {
        self.h().allocated[index as usize / 64] & (1 << (index % 64)) != 0
    }
    #[inline]
    fn alloc(&mut self) -> NonNull<u8> {
        if self.h().released {
            let h = self.h_mut();
            h.released = false;
            h.bump = 0;
            h.free = NONE;
        }
        let index = if self.h().free != NONE {
            let i = self.h().free;
            // SAFETY: free-list links occupy first u32 of freed, suitably aligned cells.
            let next = unsafe { self.cell(i).cast::<u32>().as_ptr().read() };
            self.h_mut().free = next;
            i
        } else {
            assert!(self.h().bump < self.capacity());
            let i = self.h().bump;
            self.h_mut().bump += 1;
            i
        };
        let ptr = self.cell(index);
        let size = self.h().size;
        // SAFETY: this cell is free and belongs exclusively to this page. S3 will
        // initialize typed layouts before publishing; S2 guarantees zero storage.
        unsafe {
            ptr.as_ptr().write_bytes(0, size);
        }
        let h = self.h_mut();
        h.allocated[index as usize / 64] |= 1 << (index % 64);
        h.live += 1;
        h.empty_cycles = 0;
        ptr
    }
    fn index(&self, address: usize) -> Option<u32> {
        let offset = address.checked_sub(self.base.as_ptr() as usize + HEADER_BYTES)?;
        if offset % self.h().size != 0 {
            return None;
        }
        let index = offset / self.h().size;
        (index < (self.h().bump as usize) && self.allocated(index as u32)).then_some(index as u32)
    }
    fn mark(&self, index: u32) {
        assert!(self.allocated(index));
        self.h().mark[index as usize / 64].fetch_or(1 << (index % 64), Ordering::Relaxed);
    }
    /// Sweeps allocation bits. S3/S4 add typed drops before this cell-free step.
    fn sweep(&mut self, permanent: u32) -> usize {
        let mut freed = 0;
        for i in permanent..self.h().bump {
            if self.allocated(i)
                && self.h().mark[i as usize / 64].load(Ordering::Relaxed) & (1 << (i % 64)) == 0
            {
                let next = self.h().free;
                // SAFETY: unmarked allocated cell is now exclusively retired. No
                // typed object exists yet; S4 must drop payloads before reaching here.
                unsafe {
                    self.cell(i).cast::<u32>().as_ptr().write(next);
                }
                let h = self.h_mut();
                h.free = i;
                h.allocated[i as usize / 64] &= !(1 << (i % 64));
                h.live -= 1;
                freed += h.size;
            }
        }
        for bits in &self.h().mark {
            bits.store(0, Ordering::Relaxed);
        }
        if self.h().live == 0 {
            self.h_mut().empty_cycles = self.h().empty_cycles.saturating_add(1);
        }
        freed
    }
}
struct Chunk {
    mapping: Mapping,
    next: usize,
}
struct Large {
    tag: u64,
    mapping: Mapping,
    bytes: usize,
    marked: bool,
    #[cfg(feature = "verify")]
    verified: bool,
}
struct SymbolArena {
    mapping: Mapping,
    limit: usize,
    pages: Vec<Page>,
    hot: usize,
}
impl SymbolArena {
    fn new(limit: usize) -> Result<Self, AllocError> {
        if !(crate::sym::BUILTIN_COUNT as usize..=SYMBOL_LIMIT).contains(&limit) {
            return Err(AllocError::InvalidSize);
        }
        let reservation = limit.div_ceil(SYMBOLS_PER_PAGE) * PAGE_BYTES;
        Ok(Self {
            mapping: Mapping::new(reservation, PAGE_BYTES, true)?,
            limit,
            pages: Vec::new(),
            hot: 0,
        })
    }
    fn alloc(&mut self) -> Result<(u32, NonNull<u8>), AllocError> {
        let page = if self.pages.get(self.hot).is_some_and(Page::available) {
            self.hot
        } else if let Some(i) = self.pages.iter().position(Page::available) {
            i
        } else {
            let i = self.pages.len();
            if i * SYMBOLS_PER_PAGE >= self.limit {
                return Err(AllocError::SymbolLimit);
            }
            self.mapping.commit(i * PAGE_BYTES, PAGE_BYTES)?;
            // SAFETY: a newly committed page within the arena's reserved mapping.
            let p = unsafe {
                Page::new(
                    NonNull::new_unchecked(self.mapping.base.as_ptr().add(i * PAGE_BYTES)),
                    usize::MAX,
                    Kind::Symbol,
                    usize::MAX,
                    48,
                )
            };
            self.pages.push(p);
            self.pages[i].h_mut().capacity =
                SYMBOLS_PER_PAGE.min(self.limit - i * SYMBOLS_PER_PAGE) as u32;
            i
        };
        self.hot = page;
        let ptr = self.pages[page].alloc();
        let slot = self.pages[page]
            .index(ptr.as_ptr() as usize)
            .expect("allocated symbol cell");
        let index = page * SYMBOLS_PER_PAGE + slot as usize;
        Ok((index as u32, ptr))
    }
    fn address(&self, index: u32) -> Option<NonNull<u8>> {
        let page = self.pages.get(index as usize / SYMBOLS_PER_PAGE)?;
        let slot = index as usize % SYMBOLS_PER_PAGE;
        page.allocated(slot as u32).then(|| page.cell(slot as u32))
    }
}

pub(crate) struct Allocator {
    chunks: Vec<Option<Chunk>>,
    pages: Vec<Option<Page>>,
    large: Vec<Large>,
    symbols: SymbolArena,
    hot: MutatorHot,
    stats: AllocationStats,
    pub(crate) debt: std::rc::Rc<std::cell::Cell<usize>>,
}
impl Allocator {
    pub fn new(max_symbols: usize) -> Result<Self, AllocError> {
        let mut symbols = SymbolArena::new(max_symbols)?;
        for index in 0..crate::sym::BUILTIN_COUNT {
            assert_eq!(symbols.alloc()?.0, index);
        }
        let mut stats = AllocationStats::default();
        stats.counts.symbols = crate::sym::BUILTIN_COUNT as u64;
        stats.live_bytes = crate::sym::BUILTIN_COUNT as usize * 48;
        Ok(Self {
            chunks: Vec::new(),
            pages: Vec::new(),
            large: Vec::new(),
            symbols,
            hot: MutatorHot::default(),
            stats,
            debt: std::rc::Rc::new(std::cell::Cell::new(0)),
        })
    }
    pub(crate) fn symbol_base(&self) -> NonNull<u8> {
        self.symbols.mapping.base
    }
    /// Locate an allocated object without changing its mark state.
    pub(crate) fn address(&self, word: u64) -> Option<NonNull<u8>> {
        if word & 7 == 0 {
            return self.symbols.address(u32::try_from(word >> 3).ok()?);
        }
        let address = (word & !7) as usize;
        for page in self.pages.iter().flatten() {
            if page.base.as_ptr() as usize == address & !(PAGE_BYTES - 1) {
                let tag = match page.kind {
                    Kind::Cons => 3,
                    Kind::String => 4,
                    Kind::Vectorlike => 5,
                    Kind::Float => 7,
                    _ => return None,
                };
                if word & 7 != tag {
                    return None;
                }
                return page.index(address).map(|i| page.cell(i));
            }
        }
        self.large
            .iter()
            .find(|o| o.mapping.base.as_ptr() as usize == address && word & 7 == o.tag)
            .map(|o| o.mapping.base)
    }
    pub fn stats(&self) -> AllocationStats {
        let mut s = self.stats;
        s.pages = self.pages.iter().filter(|p| p.is_some()).count() + self.symbols.pages.len();
        s.large_objects = self.large.len();
        s.mapped_bytes = self.chunks.iter().flatten().count() * CHUNK_BYTES
            + self.large.iter().map(|o| o.mapping.len).sum::<usize>();
        s.committed_symbol_bytes = self
            .symbols
            .pages
            .iter()
            .map(|p| {
                if p.h().released {
                    HEADER_BYTES
                } else {
                    PAGE_BYTES
                }
            })
            .sum();
        s.committed_bytes = s.committed_symbol_bytes
            + self
                .pages
                .iter()
                .flatten()
                .map(|p| {
                    if p.h().released {
                        HEADER_BYTES
                    } else {
                        PAGE_BYTES
                    }
                })
                .sum::<usize>()
            + self.large.iter().map(|o| o.mapping.len).sum::<usize>();
        s
    }
    pub fn class(kind: Kind, bytes: usize) -> Result<Option<(usize, usize)>, AllocError> {
        match kind {
            Kind::Cons if bytes == 16 => Ok(Some((0, 16))),
            Kind::Float if bytes == 8 => Ok(Some((1, 8))),
            Kind::String if bytes == 48 => Ok(Some((2, 48))),
            Kind::Vectorlike if bytes >= 16 => Ok(VECTOR_CLASSES
                .iter()
                .position(|s| *s >= bytes)
                .map(|i| (3 + i, VECTOR_CLASSES[i]))),
            _ => Err(AllocError::InvalidSize),
        }
    }
    pub fn alloc(&mut self, kind: Kind, bytes: usize) -> Result<u64, AllocError> {
        let units = if kind == Kind::Vectorlike {
            bytes.saturating_sub(16) / 8
        } else {
            0
        };
        self.alloc_with_counts(kind, bytes, units)
    }
    /// Layout constructors supply logical vector slots/string characters, not
    /// rounded storage bytes, for GNU counter semantics.
    #[inline]
    pub fn alloc_with_counts(
        &mut self,
        kind: Kind,
        bytes: usize,
        units: usize,
    ) -> Result<u64, AllocError> {
        self.alloc_layout::<false>(kind, bytes, units)
    }
    pub(crate) fn alloc_foreign(
        &mut self,
        kind: crate::raw::ForeignKind,
        bytes: usize,
    ) -> Result<u64, AllocError> {
        let kind = match kind {
            crate::raw::ForeignKind::String => Kind::String,
            crate::raw::ForeignKind::Vectorlike => Kind::Vectorlike,
        };
        if kind == Kind::String {
            self.alloc_layout::<true>(kind, bytes, 0)
        } else {
            self.alloc_layout::<false>(kind, bytes, 0)
        }
    }
    /// # Safety
    /// `word` names a live string cell in this allocator. Large spans start on
    /// a page boundary; small cells follow the page's initialized header.
    pub(crate) unsafe fn foreign_string(word: u64) -> bool {
        let address = (word & !7) as usize;
        if address & (PAGE_BYTES - 1) == 0 {
            return true;
        }
        // SAFETY: validated small cell lies in a live page with this header.
        unsafe { (*((address & !(PAGE_BYTES - 1)) as *const Header)).class != 2 }
    }
    #[inline]
    fn alloc_layout<const FOREIGN_STRING: bool>(
        &mut self,
        kind: Kind,
        bytes: usize,
        units: usize,
    ) -> Result<u64, AllocError> {
        if kind == Kind::Symbol {
            if bytes != 48 {
                return Err(AllocError::InvalidSize);
            }
            let (i, _) = self.symbols.alloc()?;
            self.account(kind, 48, units);
            return Ok((i as u64) << 3);
        }
        let class = if FOREIGN_STRING {
            VECTOR_CLASSES
                .iter()
                .position(|s| *s >= bytes)
                .map(|i| (3 + VECTOR_CLASSES.len() + i, VECTOR_CLASSES[i]))
        } else {
            Self::class(kind, bytes)?
        };
        let ptr = if let Some((class, size)) = class {
            let cursor = if FOREIGN_STRING { 2 } else { class };
            let hot = self.hot.pages[cursor];
            let page = if self
                .pages
                .get(hot)
                .and_then(Option::as_ref)
                .is_some_and(|p| p.h().class == class && p.available())
            {
                hot
            } else {
                self.available_page(kind, class, size)?
            };
            self.hot.pages[cursor] = page;
            let p = self.pages[page].as_mut().expect("live page");
            let ptr = p.alloc();
            self.account(kind, size, units);
            ptr
        } else {
            let len = bytes.checked_add(4095).ok_or(AllocError::MemoryFull)? & !4095;
            let mapping = Mapping::new(len, PAGE_BYTES, false)?;
            let ptr = mapping.base;
            self.large.push(Large {
                tag: if kind == Kind::String { 4 } else { 5 },
                mapping,
                bytes,
                marked: false,
                #[cfg(feature = "verify")]
                verified: false,
            });
            self.account(kind, bytes, units);
            ptr
        };
        let tag = match kind {
            Kind::Cons => 3,
            Kind::String => 4,
            Kind::Vectorlike => 5,
            Kind::Float => 7,
            _ => unreachable!(),
        };
        Ok(ptr.as_ptr() as u64 | tag)
    }
    fn account(&mut self, kind: Kind, bytes: usize, units: usize) {
        self.stats.counts.allocated(kind, units);
        self.stats.bytes_since_cycle = self.stats.bytes_since_cycle.saturating_add(bytes);
        self.debt.set(self.debt.get().saturating_add(bytes));
        self.stats.live_bytes = self
            .stats
            .live_bytes
            .checked_add(bytes)
            .expect("live bytes overflow");
    }
    #[cold]
    #[inline(never)]
    fn available_page(
        &mut self,
        kind: Kind,
        class: usize,
        size: usize,
    ) -> Result<usize, AllocError> {
        if let Some(i) = self.pages.iter().position(|p| {
            p.as_ref()
                .is_some_and(|p| p.h().class == class && p.available())
        }) {
            Ok(i)
        } else {
            self.new_page(kind, class, size)
        }
    }
    fn new_page(&mut self, kind: Kind, class: usize, size: usize) -> Result<usize, AllocError> {
        // Released pages form a class-independent pool while their chunk still
        // contains live neighbors. Reclassify only after every old cell died.
        if let Some(i) = self
            .pages
            .iter()
            .position(|p| p.as_ref().is_some_and(|p| p.h().released))
        {
            let old = self.pages[i].as_ref().expect("released page");
            // SAFETY: released page has no live cells; header survived madvise,
            // and this allocator exclusively owns its still-mapped chunk.
            let page = unsafe { Page::new(old.base, old.chunk, kind, class, size) };
            self.pages[i] = Some(page);
            return Ok(i);
        }
        let chunk = if let Some(i) = self.chunks.iter().position(|c| {
            c.as_ref()
                .is_some_and(|c| c.next < CHUNK_BYTES / PAGE_BYTES)
        }) {
            i
        } else {
            let c = Chunk {
                mapping: Mapping::new(CHUNK_BYTES, PAGE_BYTES, false)?,
                next: 0,
            };
            if let Some(i) = self.chunks.iter().position(Option::is_none) {
                self.chunks[i] = Some(c);
                i
            } else {
                self.chunks.push(Some(c));
                self.chunks.len() - 1
            }
        };
        let c = self.chunks[chunk].as_mut().expect("live chunk");
        let offset = c.next * PAGE_BYTES;
        c.next += 1;
        // SAFETY: this previously unused page lies entirely in the chunk mapping.
        let p = unsafe {
            Page::new(
                NonNull::new_unchecked(c.mapping.base.as_ptr().add(offset)),
                chunk,
                kind,
                class,
                size,
            )
        };
        if let Some(i) = self.pages.iter().position(Option::is_none) {
            self.pages[i] = Some(p);
            Ok(i)
        } else {
            self.pages.push(Some(p));
            Ok(self.pages.len() - 1)
        }
    }
    #[cfg(feature = "verify")]
    pub(crate) fn clear_verify(&mut self) {
        for p in self
            .pages
            .iter_mut()
            .flatten()
            .chain(self.symbols.pages.iter_mut())
        {
            p.h_mut().verify.fill(0);
        }
        for o in &mut self.large {
            o.verified = false;
        }
    }
    #[cfg(feature = "verify")]
    pub(crate) fn verify_visit(&mut self, word: u64) -> bool {
        let (page, index) = if word & 7 == 0 {
            let i = (word >> 3) as usize;
            (
                &mut self.symbols.pages[i / SYMBOLS_PER_PAGE],
                (i % SYMBOLS_PER_PAGE) as u32,
            )
        } else {
            let address = (word & !7) as usize;
            if let Some(o) = self
                .large
                .iter_mut()
                .find(|o| o.mapping.base.as_ptr() as usize == address)
            {
                return !std::mem::replace(&mut o.verified, true);
            }
            let page = self
                .pages
                .iter_mut()
                .flatten()
                .find(|p| p.index(address).is_some())
                .expect("validated slot");
            let index = page.index(address).expect("validated slot");
            (page, index)
        };
        let bits = &mut page.h_mut().verify[index as usize / 64];
        let fresh = *bits & (1 << (index % 64)) == 0;
        *bits |= 1 << (index % 64);
        fresh
    }
    pub(crate) fn marked(&self, word: u64) -> bool {
        if word & 3 == 2 {
            return true;
        }
        if word & 7 == 0 {
            let i = (word >> 3) as usize;
            return self
                .symbols
                .pages
                .get(i / SYMBOLS_PER_PAGE)
                .is_some_and(|p| {
                    p.allocated((i % SYMBOLS_PER_PAGE) as u32)
                        && p.h().mark[(i % SYMBOLS_PER_PAGE) / 64].load(Ordering::Relaxed)
                            & (1 << (i % 64))
                            != 0
                });
        }
        let address = (word & !7) as usize;
        for p in self.pages.iter().flatten() {
            if let Some(i) = p.index(address) {
                return p.h().mark[i as usize / 64].load(Ordering::Relaxed) & (1 << (i % 64)) != 0;
            }
        }
        self.large
            .iter()
            .any(|o| o.mapping.base.as_ptr() as usize == address && o.marked)
    }
    pub(crate) fn symbol_word(&self, ptr: NonNull<crate::objects::Symbol>) -> u64 {
        let offset = ptr.as_ptr() as usize - self.symbols.mapping.base.as_ptr() as usize;
        let index =
            (offset / PAGE_BYTES) * SYMBOLS_PER_PAGE + (offset % PAGE_BYTES - HEADER_BYTES) / 48;
        (index as u64) << 3
    }
    pub(crate) fn symbol_words(&self) -> Vec<u64> {
        let mut words = Vec::new();
        for (page, p) in self.symbols.pages.iter().enumerate() {
            for i in 0..p.h().bump {
                if p.allocated(i) {
                    words.push(((page * SYMBOLS_PER_PAGE + i as usize) as u64) << 3);
                }
            }
        }
        words
    }
    pub(crate) fn allocated_count(&self) -> usize {
        self.pages
            .iter()
            .flatten()
            .map(|p| p.h().live as usize)
            .sum::<usize>()
            + self
                .symbols
                .pages
                .iter()
                .map(|p| p.h().live as usize)
                .sum::<usize>()
            + self.large.len()
    }
    pub fn mark(&mut self, word: u64) -> bool {
        if word & 3 == 2 {
            return true;
        }
        if word & 7 == 0 {
            let Ok(index) = u32::try_from(word >> 3) else {
                return false;
            };
            let Some(ptr) = self.symbols.address(index) else {
                return false;
            };
            let page = &self.symbols.pages[index as usize / SYMBOLS_PER_PAGE];
            let slot = page.index(ptr.as_ptr() as usize).expect("symbol address");
            page.mark(slot);
            return true;
        }
        let address = (word & !7) as usize;
        for p in self.pages.iter().flatten() {
            if (p.base.as_ptr() as usize) == (address & !(PAGE_BYTES - 1)) {
                let tag = match p.kind {
                    Kind::Cons => 3,
                    Kind::String => 4,
                    Kind::Vectorlike => 5,
                    Kind::Float => 7,
                    _ => unreachable!(),
                };
                if word & 7 != tag {
                    return false;
                }
                let Some(i) = p.index(address) else {
                    return false;
                };
                p.mark(i);
                return true;
            }
        }
        if let Some(o) = self
            .large
            .iter_mut()
            .find(|o| o.mapping.base.as_ptr() as usize == address)
        {
            o.marked = true;
            true
        } else {
            false
        }
    }
    pub fn sweep(&mut self) -> Result<(), AllocError> {
        for p in self.pages.iter_mut().flatten() {
            self.stats.live_bytes -= p.sweep(0);
        }
        for (i, p) in self.symbols.pages.iter_mut().enumerate() {
            self.stats.live_bytes -= p.sweep(if i == 0 { crate::sym::BUILTIN_COUNT } else { 0 });
            if p.h().empty_cycles >= 2 && !p.h().released {
                self.symbols
                    .mapping
                    .discard(i * PAGE_BYTES + HEADER_BYTES, PAGE_BYTES - HEADER_BYTES)?;
                p.h_mut().released = true;
                self.stats.released_pages += 1;
            }
        }
        self.large.retain_mut(|o| {
            if o.marked {
                o.marked = false;
                true
            } else {
                self.stats.live_bytes -= o.bytes;
                false
            }
        });
        for p in self.pages.iter_mut().flatten() {
            if p.h().empty_cycles >= 2 && !p.h().released {
                let c = self.chunks[p.chunk].as_ref().expect("page chunk");
                let offset = p.base.as_ptr() as usize - c.mapping.base.as_ptr() as usize;
                c.mapping
                    .discard(offset + HEADER_BYTES, PAGE_BYTES - HEADER_BYTES)?;
                p.h_mut().released = true;
                self.stats.released_pages += 1;
            }
        }
        for c in 0..self.chunks.len() {
            if self.chunks[c].is_some()
                && self
                    .pages
                    .iter()
                    .flatten()
                    .filter(|p| p.chunk == c)
                    .all(|p| p.h().released)
            {
                for p in &mut self.pages {
                    if p.as_ref().is_some_and(|p| p.chunk == c) {
                        *p = None;
                    }
                }
                self.chunks[c] = None; // Mapping::drop unmaps the entire free chunk.
            }
        }
        self.hot = MutatorHot::default();
        self.stats.bytes_since_cycle = 0;
        self.debt.set(0);
        Ok(())
    }
}

#[cfg(test)]
#[path = "release_tests.rs"]
mod release_tests;
#[cfg(test)]
#[path = "alloc_tests.rs"]
mod tests;
