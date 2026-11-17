//! The objects the loader has mapped in this process, read where they are
//! mapped: the libraries loaded before the first allocation (`libraries`,
//! noted at the arena's start), any loaded one by name (`find_library`), and this
//! process's executable (`executable`). What the arena needs of them is
//! their identity (`Library::build_id`, `Library::constant_segments`) and
//! where they lie, never a copy.
use super::{state, PAGE};
use std::ffi::CStr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// The libraries loaded as the arena started (`libraries`): for each, its
/// name, where it loaded and its program headers, the loader's own, which
/// stay for the process's life; and whether they all fit. Written once,
/// under the allocator's lock, before STATE publishes the arena.
static LOADED: [[AtomicUsize; 4]; 256] = [const { [const { AtomicUsize::new(0) }; 4] }; 256];
static LOADED_COUNT: AtomicUsize = AtomicUsize::new(0);
static LOADED_COMPLETE: AtomicBool = AtomicBool::new(false);

/// One object as dl_iterate_phdr describes it: NAME, loaded at BASE, with
/// PHNUM program headers at PHDR.
struct Object<'a> {
    name: &'a CStr,
    base: usize,
    phdr: usize,
    phnum: usize,
}

/// Call F on each object the loader has mapped, until it answers false.
/// Allocates nothing.
fn each_object<F: FnMut(&Object<'_>) -> bool>(mut f: F) {
    // UNSAFE(the loader's object list is reached only through dl_iterate_phdr's C callback)
    // SAFETY: called by dl_iterate_phdr with INFO, valid for the call, and
    // DATA, the `&mut F` each_object passed; an object's name is
    // NUL-terminated and lives while the object is loaded.
    unsafe extern "C" fn each<F: FnMut(&Object<'_>) -> bool>(info: *mut libc::dl_phdr_info, _: libc::size_t, data: *mut libc::c_void) -> libc::c_int {
        // UNSAFE(dl_iterate_phdr hands its callback raw pointers)
        // SAFETY: as above: INFO is the loader's, DATA each_object's F.
        let (info, f) = unsafe { (&*info, &mut *data.cast::<F>()) };
        if info.dlpi_name.is_null() {
            return 0;
        }
        // UNSAFE(the loader's names are C strings)
        // SAFETY: a non-null name is NUL-terminated, the loader's, for this call.
        let name = unsafe { CStr::from_ptr(info.dlpi_name) };
        let object = Object { name, base: info.dlpi_addr as usize, phdr: info.dlpi_phdr as usize, phnum: usize::from(info.dlpi_phnum) };
        libc::c_int::from(!f(&object))
    }
    // UNSAFE(the loader's object list is reached only through dl_iterate_phdr's C callback)
    // SAFETY: EACH is called with F as DATA, which lives across the call.
    unsafe { libc::dl_iterate_phdr(Some(each::<F>), (&raw mut f).cast()) };
}

/// Note the libraries loaded so far (`LOADED`): the objects themselves,
/// not paths. The executable, keyed by its file's content, and the vDSO,
/// the kernel's, which no image refers to, are left out. Allocates nothing.
pub(super) fn take_inventory() {
    LOADED_COMPLETE.store(true, Ordering::Relaxed);
    each_object(|object| {
        let name = object.name.to_bytes();
        if name.is_empty() || name.starts_with(b"linux-vdso") {
            return true;
        }
        let count = LOADED_COUNT.load(Ordering::Relaxed);
        let Some(slot) = LOADED.get(count) else {
            LOADED_COMPLETE.store(false, Ordering::Relaxed);
            return false;
        };
        for (word, value) in slot.iter().zip([object.name.as_ptr() as usize, object.base, object.phdr, object.phnum]) {
            word.store(value, Ordering::Relaxed);
        }
        LOADED_COUNT.store(count + 1, Ordering::Relaxed);
        true
    });
}

/// A library this process loaded, as the loader mapped it.
#[derive(Clone, Copy, Debug)]
pub struct Library {
    /// Its name, as the loader found it: a path.
    pub name: &'static [u8],
    base: usize,
    headers: &'static [libc::Elf64_Phdr],
}

/// The libraries this process loaded before its first allocation, or
/// None when there were more than the inventory holds.
pub fn libraries() -> Option<Vec<Library>> {
    state();
    if !LOADED_COMPLETE.load(Ordering::Relaxed) {
        return None;
    }
    let loaded = &LOADED[..LOADED_COUNT.load(Ordering::Relaxed)];
    Some(loaded.iter().map(|slot| slot.each_ref().map(|word| word.load(Ordering::Relaxed))).map(|[name, base, phdr, phnum]| {
        // UNSAFE(a library's mapped headers are memory no type owns)
        // SAFETY: the inventory noted these objects before the first
        // allocation and none was unloaded: libraries loaded that early stay.
        unsafe { Library::at(name as *const libc::c_char, base, phdr, phnum) }
    }).collect())
}

/// The loaded library whose name, as the loader found it, is NAME.
///
/// # Safety
/// The library stays loaded for the process's life: nothing dlcloses it.
pub unsafe fn find_library(name: &CStr) -> Option<Library> {
    let mut found = None;
    each_object(|object| {
        if object.name != name {
            return true;
        }
        // UNSAFE(a library's mapped headers are memory no type owns)
        // SAFETY: this function's contract: the library stays loaded.
        found = Some(unsafe { Library::at(object.name.as_ptr(), object.base, object.phdr, object.phnum) });
        false
    });
    found
}

impl Library {
    /// The object a dl_phdr_info describes: NAME, loaded at BASE, with
    /// PHNUM program headers at PHDR.
    ///
    /// # Safety
    /// The object stays loaded for the process's life.
    unsafe fn at(name: *const libc::c_char, base: usize, phdr: usize, phnum: usize) -> Library {
        // UNSAFE(a library's mapped name and headers are memory no type owns)
        // SAFETY: NAME is the loader's NUL-terminated name, and PHDR its
        // PHNUM program headers, both mapped while the object is loaded,
        // which the caller promises is for the process's life.
        let (name, headers) = unsafe { (CStr::from_ptr(name).to_bytes(), std::slice::from_raw_parts(phdr as *const libc::Elf64_Phdr, phnum)) };
        Library { name, base, headers }
    }

    /// LEN bytes of its image from the virtual address VADDR, as mapped.
    fn bytes(&self, vaddr: u64, len: u64) -> &'static [u8] {
        // UNSAFE(a library's mapped segments are memory no type owns)
        // SAFETY: VADDR and LEN come from one of its program headers, whose
        // segment the loader mapped readable at BASE for the object's life
        // (`at`'s contract).
        unsafe { std::slice::from_raw_parts((self.base + vaddr as usize) as *const u8, len as usize) }
    }

    /// The GNU build-id note of its PT_NOTE segments, as mapped.
    pub fn build_id(&self) -> Option<&'static [u8]> {
        self.headers.iter().filter(|h| h.p_type == libc::PT_NOTE).find_map(|h| build_id(self.bytes(h.p_vaddr, h.p_memsz)))
    }

    /// Where its first segment is mapped: an address in the object.
    pub fn address(&self) -> usize {
        self.headers.iter().find(|h| h.p_type == libc::PT_LOAD).map_or(self.base, |h| self.base + h.p_vaddr as usize)
    }

    /// Whether the loader writes its read-only segments: its dynamic
    /// section has DT_TEXTREL, or DF_TEXTREL in DT_FLAGS. Each entry is an
    /// Elf64_Dyn, a tag and its value.
    pub fn text_relocations(&self) -> bool {
        const DT_TEXTREL: i64 = 22;
        const DT_FLAGS: i64 = 30;
        const DF_TEXTREL: u64 = 0x4;
        let word = |bytes: &[u8]| <[u8; 8]>::try_from(bytes).map_or(0, u64::from_ne_bytes);
        self.headers.iter().filter(|h| h.p_type == libc::PT_DYNAMIC).any(|h| {
            self.bytes(h.p_vaddr, h.p_memsz).chunks_exact(16)
                .map(|entry| (word(&entry[..8]).cast_signed(), word(&entry[8..])))
                .take_while(|&(tag, _)| tag != 0)
                .any(|(tag, value)| tag == DT_TEXTREL || (tag == DT_FLAGS && value & DF_TEXTREL != 0))
        })
    }

    /// What the loader leaves as the file has it: each non-writable
    /// PT_LOAD segment's bytes, code and read-only data, as mapped, but for
    /// the RELRO range the loader writes, each with the file offset it maps.
    /// Position-independent code and data are not relocated there, so they
    /// are alike in every process. None when a segment is not readable.
    pub fn constant_segments(&self) -> Option<Vec<(u64, &'static [u8])>> {
        let relro = self.headers.iter().filter(|h| h.p_type == libc::PT_GNU_RELRO)
            .map(|h| (h.p_vaddr, h.p_vaddr + h.p_memsz)).collect::<Vec<_>>();
        let mut out = vec![];
        for h in self.headers.iter().filter(|h| h.p_type == libc::PT_LOAD && h.p_flags & libc::PF_W == 0) {
            if h.p_flags & libc::PF_R == 0 {
                return None;
            }
            // The segment's file bytes, less any RELRO range within them.
            let mut pieces = vec![(h.p_vaddr, h.p_vaddr + h.p_filesz)];
            for &(lo, hi) in &relro {
                pieces = pieces.into_iter().flat_map(|(a, b)| [(a, b.min(lo)), (a.max(hi), b)]).filter(|(a, b)| a < b).collect();
            }
            for (a, b) in pieces {
                out.push((h.p_offset + (a - h.p_vaddr), self.bytes(a, b - a)));
            }
        }
        (!out.is_empty()).then_some(out)
    }
}

/// The GNU build-id in NOTES, an ELF note section.
fn build_id(notes: &[u8]) -> Option<&[u8]> {
    const NT_GNU_BUILD_ID: u32 = 3;
    let word = |at: usize| notes.get(at..at + 4).map(|b| u32::from_ne_bytes(b.try_into().unwrap()));
    let mut at = 0;
    while at + 12 <= notes.len() {
        let (name_size, desc_size, kind) = (word(at)? as usize, word(at + 4)? as usize, word(at + 8)?);
        let name = at + 12;
        let desc = name + name_size.next_multiple_of(4);
        let next = desc + desc_size.next_multiple_of(4);
        if next > notes.len() {
            return None;
        }
        if kind == NT_GNU_BUILD_ID && &notes[name..name + name_size] == b"GNU\0" && desc_size > 0 {
            return Some(&notes[desc..desc + desc_size]);
        }
        at = next;
    }
    None
}

/// Where this process's executable loaded, and the span of its loaded
/// segments from there, from its program headers, without allocating.
pub(super) fn executable() -> Option<(usize, std::ops::Range<usize>)> {
    // UNSAFE(no crate reads the auxiliary vector's AT_PHDR: getauxval is a C call)
    // SAFETY: getauxval reads the auxiliary vector the kernel gave this
    // process; it has no precondition.
    let (phdr, count) = unsafe { (libc::getauxval(libc::AT_PHDR) as usize, libc::getauxval(libc::AT_PHNUM) as usize) };
    if phdr == 0 {
        return None;
    }
    // UNSAFE(the executable's mapped headers are memory no type owns)
    // SAFETY: AT_PHDR and AT_PHNUM give the address and number of this
    // executable's program headers, which stay mapped for the process's life.
    let headers = unsafe { std::slice::from_raw_parts(phdr as *const libc::Elf64_Phdr, count) };
    let base = phdr - headers.iter().find(|h| h.p_type == libc::PT_PHDR)?.p_vaddr as usize;
    let loads = || headers.iter().filter(|h| h.p_type == libc::PT_LOAD);
    let low = loads().map(|h| h.p_vaddr as usize / PAGE * PAGE).min()?;
    let high = loads().map(|h| (h.p_vaddr + h.p_memsz) as usize).max()?.next_multiple_of(PAGE);
    Some((base, low..high))
}

/// Where this process's executable loaded: what a capture records.
pub fn executable_base() -> usize {
    executable().map_or(0, |(base, _)| base)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A note section of NOTES, (name, type, desc).
    fn notes(notes: &[(&[u8], u32, &[u8])]) -> Vec<u8> {
        let mut out = vec![];
        for (name, kind, desc) in notes {
            out.extend((name.len() as u32).to_ne_bytes());
            out.extend((desc.len() as u32).to_ne_bytes());
            out.extend(kind.to_ne_bytes());
            out.extend(*name);
            out.resize(out.len().next_multiple_of(4), 0);
            out.extend(*desc);
            out.resize(out.len().next_multiple_of(4), 0);
        }
        out
    }

    #[test]
    fn a_build_id_is_read_from_its_note() {
        let id = [0xab, 0xcd, 0xef, 0x01, 0x23];
        let section = notes(&[(b"GNU\0", 1, b"abi-tag!"), (b"GNU\0", 3, &id)]);
        assert_eq!(build_id(&section), Some(&id[..]));
        assert_eq!(build_id(&notes(&[(b"FDO\0", 3, &id)])), None);
        assert_eq!(build_id(&section[..section.len() - 4]), None);
    }

    /// The build-id the file at PATH carries, read from its ELF section
    /// headers: what the mapping's note must agree with.
    fn file_build_id(path: &str) -> Option<Vec<u8>> {
        let bytes = std::fs::read(path).ok()?;
        let at = |o: usize, n: usize| u64::from_le_bytes(bytes[o..o + n].iter().chain([0u8; 8].iter()).take(8).copied().collect::<Vec<_>>().try_into().unwrap()) as usize;
        let (shoff, shentsize, shnum) = (at(0x28, 8), at(0x3a, 2), at(0x3c, 2));
        (0..shnum).find_map(|i| {
            let h = shoff + i * shentsize;
            (at(h + 4, 4) == 7).then(|| build_id(&bytes[at(h + 0x18, 8)..at(h + 0x18, 8) + at(h + 0x20, 8)]).map(<[u8]>::to_vec)).flatten()
        })
    }

    #[test]
    fn this_process_knows_libc_by_the_build_id_it_mapped() {
        let libraries = libraries().expect("an inventory");
        let libc = libraries.iter().find(|l| std::str::from_utf8(l.name).is_ok_and(|n| n.contains("libc.so"))).expect("libc is loaded");
        let path = std::str::from_utf8(libc.name).unwrap();
        assert_eq!(file_build_id(path).as_deref(), libc.build_id(), "{path}");
        let segments = libc.constant_segments().expect("readable");
        // Read-only data and code, as the file has them at those offsets.
        let file = std::fs::read(path).unwrap();
        for (offset, bytes) in segments {
            assert_eq!(&file[offset as usize..offset as usize + bytes.len()], bytes);
        }
    }
}
