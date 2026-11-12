//! Immutable cons graphs (v1) and UTF-8 string graphs (v2).
//! Indices are typed, never process pointers. Authenticated regions precede access.
use super::*;
use sha2::{Digest, Sha256};
use std::{fs::File, os::fd::AsRawFd, path::Path, sync::Arc};
const PAGE: usize = 4096;
const MANIFEST: &[u8; 8] = b"VM2PAGE1";
struct PageIntegrity {
    hashes: Vec<[u8; 32]>,
    verified: Vec<std::sync::atomic::AtomicBool>,
}
pub(super) const IMAGE: usize = 1 << 60;
const MAGIC: &[u8; 8] = b"VM2IMG03";
const LEGACY_MAGIC: &[u8; 8] = b"VM2IMG02";
fn content_key(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut key = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(key, "{byte:02x}").unwrap();
    }
    key
}
unsafe extern "C" {
    fn mmap(
        addr: *mut std::ffi::c_void,
        len: usize,
        prot: i32,
        flags: i32,
        fd: i32,
        offset: i64,
    ) -> *mut std::ffi::c_void;
    fn munmap(addr: *mut std::ffi::c_void, len: usize) -> i32;
    fn madvise(addr: *mut std::ffi::c_void, len: usize, advice: i32) -> i32;
}
pub struct ImageRegion {
    ptr: *const u8,
    bytes: usize,
    offset: usize,
    legacy_fixnums: bool,
    len: usize,
    strings: usize,
    blob: usize,
    integrity: Option<PageIntegrity>,
    pub(super) symbol_slots: std::sync::atomic::AtomicUsize,
}
unsafe impl Send for ImageRegion {}
unsafe impl Sync for ImageRegion {}
impl Drop for ImageRegion {
    fn drop(&mut self) {
        unsafe {
            munmap(self.ptr as *mut _, self.bytes);
        }
    }
}
impl ImageRegion {
    /// # Safety
    /// The backing inode must not change or be truncated for the mapping's lifetime.
    pub unsafe fn map(file: &File) -> Result<Arc<Self>> {
        unsafe { Self::map_inner(file, None) }
    }
    unsafe fn map_inner(file: &File, integrity: Option<PageIntegrity>) -> Result<Arc<Self>> {
        if !cfg!(target_endian = "little") {
            return Err("image ABI requires little endian".into());
        }
        // HOST(metadata): the code-image store, reached only through the vm2 adapter's --image,
        // --code-image and --cache-store; world builds refuse images (M1)
        let bytes = usize::try_from(file.metadata().map_err(|e| e.to_string())?.len())
            .map_err(|_| "image too large")?;
        if bytes == 0 || bytes > isize::MAX as usize {
            return Err("empty or oversized image".into());
        }
        let ptr = unsafe { mmap(std::ptr::null_mut(), bytes, 1, 2, file.as_raw_fd(), 0) };
        if ptr as isize == -1 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut image = Self {
            ptr: ptr.cast(),
            bytes,
            offset: 0,
            legacy_fixnums: true,
            len: bytes / 16,
            strings: 0,
            blob: bytes,
            integrity,
            symbol_slots: std::sync::atomic::AtomicUsize::new(0),
        };
        let result: Result<()> = (|| {
            if image.integrity.is_some() && unsafe { madvise(image.ptr as *mut _, bytes, 1) } != 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            image.verify(0, bytes.min(64))?;
            let data = image.data();
            if data.starts_with(MAGIC) || data.starts_with(LEGACY_MAGIC) {
                let legacy_fixnums = data.starts_with(LEGACY_MAGIC);
                if bytes < 64 {
                    return Err("truncated v2 image header".into());
                }
                let word = |at| {
                    usize::try_from(u64::from_le_bytes(data[at..at + 8].try_into().unwrap()))
                        .map_err(|_| "image size overflow")
                };
                let conses = word(8)?;
                let strings = word(16)?;
                let blob_bytes = word(24)?;
                let blob = conses
                    .checked_add(strings)
                    .and_then(|n| n.checked_mul(16))
                    .and_then(|n| n.checked_add(64))
                    .ok_or("image size overflow")?;
                if conses >= IMAGE
                    || strings >= IMAGE
                    || blob.checked_add(blob_bytes) != Some(bytes)
                {
                    return Err("invalid v2 image lengths".into());
                }
                image.legacy_fixnums = legacy_fixnums;
                image.offset = 64;
                image.len = conses;
                image.strings = strings;
                image.blob = blob;
            } else if bytes % 16 != 0 {
                return Err("image must contain complete cons records".into());
            }
            if image.integrity.is_some() {
                return Ok(());
            }
            for i in 0..image.strings {
                image.string(i)?;
            }
            for i in 0..image.len {
                let pair = image.pair(i)?;
                for v in [pair.0, pair.1] {
                    match v.tag() {
                        1 => (),
                        0 if v == Value::NIL => (),
                        2 if v == Value::TRUE => (),
                        3 if Vm::image_value(v) => {
                            if ((v.payload() as usize) & !IMAGE) >= image.len {
                                return Err("image cons index out of range".into());
                            }
                        }
                        0 if Vm::image_value(v) => {
                            if ((v.payload() as usize) & !IMAGE) >= image.strings {
                                return Err("image string index out of range".into());
                            }
                        }
                        _ => {
                            return Err(
                                "image edge is not an immutable image object or immediate".into()
                            );
                        }
                    }
                }
            }
            Ok(())
        })();
        result?;
        Ok(Arc::new(image))
    }
    fn data(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.bytes) }
    }
    /// Content lookup is exact and fails closed; it never boots from source.
    /// # Safety
    /// Store entries must remain immutable, including against other processes.
    pub unsafe fn open(store: &Path, key: &str) -> Result<Arc<Self>> {
        if key.len() != 64
            || !key
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("image key must be 64 lowercase hexadecimal digits".into());
        }
        let manifest =
            // HOST(std::fs): the code-image store, reached only through the vm2 adapter's --image,
            // --code-image and --cache-store; world builds refuse images (M1)
            std::fs::read(store.join(format!("{key}.pages"))).map_err(|e| e.to_string())?;
        if content_key(&manifest) != key || manifest.len() < 16 || &manifest[..8] != MANIFEST {
            return Err("image page manifest hash mismatch".into());
        }
        let bytes = usize::try_from(u64::from_le_bytes(manifest[8..16].try_into().unwrap()))
            .map_err(|_| "image size overflow")?;
        let pages = bytes.div_ceil(PAGE);
        if bytes == 0
            || pages.checked_mul(32).and_then(|n| n.checked_add(16)) != Some(manifest.len())
        {
            return Err("invalid page manifest length".into());
        }
        // HOST(File::open): the code-image store, reached only through the vm2 adapter's --image,
        // --code-image and --cache-store; world builds refuse images (M1)
        let file = File::open(store.join(key)).map_err(|e| e.to_string())?;
        // HOST(metadata): the code-image store, reached only through the vm2 adapter's --image,
        // --code-image and --cache-store; world builds refuse images (M1)
        if file.metadata().map_err(|e| e.to_string())?.len() != bytes as u64 {
            return Err("image length differs from authenticated manifest".into());
        }
        let integrity = PageIntegrity {
            hashes: manifest[16..]
                .chunks_exact(32)
                .map(|h| h.try_into().unwrap())
                .collect(),
            verified: (0..pages)
                .map(|_| std::sync::atomic::AtomicBool::new(false))
                .collect(),
        };
        let image = unsafe { Self::map_inner(&file, Some(integrity)) }?;
        Ok(image)
    }
    fn verify(&self, start: usize, end: usize) -> Result<()> {
        use std::sync::atomic::Ordering;
        if start > end || end > self.bytes {
            return Err("image access outside mapping".into());
        }
        if let Some(integrity) = &self.integrity {
            if start == end {
                return Ok(());
            }
            for page in start / PAGE..=(end - 1) / PAGE {
                if !integrity.verified[page].load(Ordering::Acquire) {
                    let bytes = &self.data()[page * PAGE..self.bytes.min((page + 1) * PAGE)];
                    let actual: [u8; 32] = Sha256::digest(bytes).into();
                    if actual != integrity.hashes[page] {
                        return Err(format!("image page {page} hash mismatch"));
                    }
                    integrity.verified[page].store(true, Ordering::Release);
                }
            }
        }
        Ok(())
    }
    fn validate_edge(&self, v: Value) -> Result<()> {
        match v.tag() {
            7 if v != Value::UNBOUND
                && v.payload() as usize
                    <= self.symbol_slots.load(std::sync::atomic::Ordering::Acquire) =>
            {
                Ok(())
            }
            1 => Ok(()),
            0 if v == Value::NIL => Ok(()),
            2 if v == Value::TRUE => Ok(()),
            3 if Vm::image_value(v) && ((v.payload() as usize) & !IMAGE) < self.len => Ok(()),
            0 if Vm::image_value(v) && ((v.payload() as usize) & !IMAGE) < self.strings => Ok(()),
            _ => Err("image edge is not a valid immutable object or immediate".into()),
        }
    }
    pub(crate) fn reference_raw(&self, index: usize) -> Result<Value> {
        if index >= self.len {
            return Err("image cons index out of range".into());
        }
        Ok(Value::reference(IMAGE | index, 3))
    }
    pub(crate) fn string_reference_raw(&self, index: usize) -> Result<Value> {
        if index >= self.strings {
            return Err("image string index out of range".into());
        }
        Ok(Value::reference(IMAGE | index, 0))
    }
    /// Authenticated pages actually accessed, not a count of OS page faults.
    pub fn verified_pages(&self) -> usize {
        self.integrity.as_ref().map_or(0, |i| {
            i.verified
                .iter()
                .filter(|v| v.load(std::sync::atomic::Ordering::Acquire))
                .count()
        })
    }
    pub fn mapped_bytes(&self) -> usize {
        self.bytes
    }
    fn pair(&self, index: usize) -> Result<(Value, Value)> {
        if index >= self.len {
            return Err("image cons index out of range".into());
        }
        let at = self.offset + index * 16;
        self.verify(at, at + 16)?;
        let data = self.data();
        let decode = |word: u64| -> Result<Value> {
            if self.legacy_fixnums {
                match word & 7 {
                    1 => return Value::integer((word as i64) >> 3),
                    5 => return Err("legacy image edge contains private cell".into()),
                    _ => {}
                }
            }
            Ok(Value(word))
        };
        let pair = (
            decode(u64::from_le_bytes(data[at..at + 8].try_into().unwrap()))?,
            decode(u64::from_le_bytes(
                data[at + 8..at + 16].try_into().unwrap(),
            ))?,
        );
        self.validate_edge(pair.0)?;
        self.validate_edge(pair.1)?;
        Ok(pair)
    }
    pub(super) fn string(&self, index: usize) -> Result<&str> {
        if index >= self.strings {
            return Err("image string index out of range".into());
        }
        let at = self.offset + (self.len + index) * 16;
        self.verify(at, at + 16)?;
        let data = self.data();
        let offset = usize::try_from(u64::from_le_bytes(data[at..at + 8].try_into().unwrap()))
            .map_err(|_| "string offset overflow")?;
        let len = usize::try_from(u64::from_le_bytes(
            data[at + 8..at + 16].try_into().unwrap(),
        ))
        .map_err(|_| "string length overflow")?;
        let start = self
            .blob
            .checked_add(offset)
            .ok_or("string offset overflow")?;
        let end = start.checked_add(len).ok_or("string length overflow")?;
        self.verify(start, end)?;
        std::str::from_utf8(data.get(start..end).ok_or("string outside image blob")?)
            .map_err(|_| "image string is not UTF-8".into())
    }
}

/// Build-time producer. Source identity is supplied by the compiler/reader owner;
/// it is an ordered declared-input digest, not a path or timestamp.
#[derive(Default)]
pub struct ImageBuilder {
    pub(super) conses: Vec<(Value, Value)>,
    pub(super) strings: Vec<String>,
}
impl ImageBuilder {
    pub(crate) fn string_raw(&mut self, text: &str) -> Value {
        let v = Value::reference(IMAGE | self.strings.len(), 0);
        self.strings.push(text.into());
        v
    }
    pub(crate) fn cons_raw(&mut self, car: Value, cdr: Value) -> Value {
        let v = Value::reference(IMAGE | self.conses.len(), 3);
        self.conses.push((car, cdr));
        v
    }
    pub fn encode(&self, source_digest: [u8; 32]) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        for n in [
            self.conses.len(),
            self.strings.len(),
            self.strings.iter().map(|s| s.len()).sum(),
        ] {
            bytes.extend_from_slice(&(n as u64).to_le_bytes());
        }
        bytes.extend_from_slice(&source_digest);
        for &(a, b) in &self.conses {
            bytes.extend_from_slice(&a.0.to_le_bytes());
            bytes.extend_from_slice(&b.0.to_le_bytes());
        }
        let mut offset = 0u64;
        for s in &self.strings {
            bytes.extend_from_slice(&offset.to_le_bytes());
            bytes.extend_from_slice(&(s.len() as u64).to_le_bytes());
            offset += s.len() as u64;
        }
        for s in &self.strings {
            bytes.extend_from_slice(s.as_bytes());
        }
        bytes
    }
    /// Publish a new immutable inode atomically. Never overwrite an existing key.
    pub fn publish(&self, store: &Path, source_digest: [u8; 32]) -> Result<String> {
        use std::io::Write;
        let bytes = self.encode(source_digest);
        let mut manifest = MANIFEST.to_vec();
        manifest.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        for page in bytes.chunks(PAGE) {
            manifest.extend_from_slice(&Sha256::digest(page));
        }
        let key = content_key(&manifest);
        // HOST(std::fs): the code-image store, reached only through the vm2 adapter's --image,
        // --code-image and --cache-store; world builds refuse images (M1)
        std::fs::create_dir_all(store).map_err(|e| e.to_string())?;
        for (name, bytes) in [
            (format!("{key}.pages"), manifest.as_slice()),
            (key.clone(), bytes.as_slice()),
        ] {
            // HOST(std::process::id): the code-image store, reached only through the vm2 adapter's
            // --image, --code-image and --cache-store; world builds refuse images (M1)
            let temporary = store.join(format!(".{name}-{}", std::process::id()));
            // HOST(OpenOptions, std::fs): the code-image store, reached only through the vm2
            // adapter's --image, --code-image and --cache-store; world builds refuse images (M1)
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|e| e.to_string())?;
            let result = (|| {
                file.write_all(bytes).map_err(|e| e.to_string())?;
                file.sync_all().map_err(|e| e.to_string())?;
                file.set_permissions(std::fs::Permissions::from_mode(0o444))
                    .map_err(|e| e.to_string())?;
                // HOST(std::fs): the code-image store, reached only through the vm2 adapter's
                // --image, --code-image and --cache-store; world builds refuse images (M1)
                match std::fs::hard_link(&temporary, store.join(&name)) {
                    Ok(()) => (),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        // HOST(std::fs): the code-image store, reached only through the vm2
                        // adapter's --image, --code-image and --cache-store; world builds refuse
                        // images (M1)
                        if std::fs::read(store.join(&name)).map_err(|e| e.to_string())? != bytes {
                            return Err("existing image content mismatch".into());
                        }
                    }
                    Err(e) => return Err(e.to_string()),
                }
                // HOST(File::open): the code-image store, reached only through the vm2 adapter's
                // --image, --code-image and --cache-store; world builds refuse images (M1)
                File::open(store)
                    .and_then(|d| d.sync_all())
                    .map_err(|e| e.to_string())?;
                Ok(())
            })();
            // HOST(std::fs): the code-image store, reached only through the vm2 adapter's --image,
            // --code-image and --cache-store; world builds refuse images (M1)
            let cleanup = std::fs::remove_file(temporary).map_err(|e| e.to_string());
            result?;
            cleanup?;
        }
        Ok(key)
    }
}
use std::os::unix::fs::PermissionsExt;
impl Vm {
    pub fn attach_image(&mut self, image: Arc<ImageRegion>) -> Result<()> {
        if self.image.is_some() {
            return Err("image is already attached".into());
        }
        self.image = Some(image);
        Ok(())
    }
    #[inline(always)]
    pub(crate) fn cons_pair(&self, v: Value) -> Result<(Value, Value)> {
        let index = v.index(3)?;
        if index & IMAGE != 0 {
            if crate::frozen::region_of(index) != 0 {
                return self.frozen_cons(index);
            }
            self.image
                .as_ref()
                .ok_or("no image attached")?
                .pair(index & !IMAGE)
        } else {
            self.conses
                .get(index)
                .copied()
                .ok_or_else(|| "invalid private cons".into())
        }
    }
    pub(crate) fn string_value(&self, v: Value) -> Result<&str> {
        let index = v.index(0)?;
        if index & IMAGE == 0 || crate::frozen::region_of(index) != 0 {
            return Err("string required".into());
        }
        self.image
            .as_ref()
            .ok_or("no image attached")?
            .string(index & !IMAGE)
    }
    /// V is not a private heap object: a reference into the executable
    /// image (region 0) or a frozen region (`frozen.rs`). Such a value is
    /// never traced, moved or written.
    #[inline(always)]
    pub(super) fn image_value(v: Value) -> bool {
        matches!(v.tag(), 0 | 3) && v.payload() as usize & IMAGE != 0
    }
    /// V is a string of the executable image, which `string_value` reads;
    /// frozen strings are objects (`Vm::object`).
    #[inline(always)]
    pub(super) fn image_string(v: Value) -> bool {
        v.tag() == 0 && v.payload() as usize & IMAGE != 0 && crate::frozen::region_of(v.payload() as usize) == 0
    }
    #[inline(always)]
    pub(super) fn list_limit(&self) -> usize {
        self.conses.len() + self.image.as_ref().map_or(0, |i| i.len) + self.region_conses()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    unsafe extern "C" {
        fn mprotect(addr: *mut std::ffi::c_void, len: usize, prot: i32) -> i32;
    }
    fn mapped(bytes: &[u8], label: &str) -> Result<Arc<ImageRegion>> {
        let path = std::env::temp_dir().join(format!("vm2-{label}-{}", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.write_all(bytes).unwrap();
        file.flush().unwrap();
        let result = unsafe { ImageRegion::map(&file) };
        std::fs::remove_file(path).unwrap();
        result
    }
    #[test]
    fn invalid_image_edges_and_partial_records_fail_closed() {
        for (i, words) in [
            vec![3u64, 0],
            vec![(1u64 << 63) | 4, 0],
            vec![(1u64 << 63) | 5, 0],
            vec![(1u64 << 63) | (2 << 3) | 3, 0],
            vec![1],
        ]
        .into_iter()
        .enumerate()
        {
            let bytes: Vec<u8> = words.into_iter().flat_map(u64::to_le_bytes).collect();
            assert!(mapped(&bytes, &format!("invalid-{i}")).is_err());
        }
    }
    #[test]
    fn collector_never_reads_image_pages_even_on_first_collection() {
        let words = [(1u64 << 63) | 3, 0];
        let bytes: Vec<u8> = words.into_iter().flat_map(u64::to_le_bytes).collect();
        let image = mapped(&bytes, "untraced").unwrap();
        let root = image.reference_raw(0).unwrap();
        let mut vm = Vm::new();
        vm.attach_image(image.clone()).unwrap();
        vm.declare_special_raw("world", root);
        assert_eq!(unsafe { mprotect(image.ptr as *mut _, image.bytes, 0) }, 0);
        vm.collect(&mut [root]);
        vm.collect(&mut [root]);
        assert_eq!(unsafe { mprotect(image.ptr as *mut _, image.bytes, 1) }, 0);
        assert_eq!(vm.cons_pair(root).unwrap().0, root);
        assert_eq!(vm.heap_stats().collections, 2);
    }
    #[test]
    fn strings_in_graphs_are_untraced_and_survive_private_compaction() {
        let mut b = ImageBuilder::default();
        let text = b.string_raw("Greek Ελληνικά 🦀");
        b.cons_raw(text, Value::NIL);
        let image = mapped(&b.encode([42; 32]), "strings").unwrap();
        let root = image.reference_raw(0).unwrap();
        let mut vm = Vm::new();
        vm.attach_image(image.clone()).unwrap();
        vm.declare_special_raw("world", root);
        vm.declare_special_raw("message", text);
        vm.conses.push((Value::NIL, Value::NIL));
        vm.conses.push((root, text));
        let mut roots = [Value::reference(1, 3), text];
        assert_eq!(unsafe { mprotect(image.ptr as *mut _, image.bytes, 0) }, 0);
        vm.collect(&mut roots);
        vm.collect(&mut roots);
        assert_eq!(unsafe { mprotect(image.ptr as *mut _, image.bytes, 1) }, 0);
        assert_eq!(vm.cons_pair(roots[0]).unwrap(), (root, text));
        assert_eq!(vm.string_value(roots[1]).unwrap(), "Greek Ελληνικά 🦀");
        assert_eq!(vm.heap_stats().reclaimed, 1);
        assert!(image.reference_raw(1).is_err());
        assert!(image.string_reference_raw(1).is_err());
        assert!(vm.string_value(root).is_err());
        assert!(vm.cons_pair(text).is_err());
    }
    #[test]
    fn string_bounds_utf8_and_header_validation() {
        let mut b = ImageBuilder::default();
        let text = b.string_raw("ok");
        b.cons_raw(text, Value::NIL);
        let valid = b.encode([0; 32]);
        for i in 0..4 {
            let mut bytes = valid.clone();
            match i {
                0 => bytes[16..24].copy_from_slice(&u64::MAX.to_le_bytes()),
                1 => bytes[80..88].copy_from_slice(&u64::MAX.to_le_bytes()),
                2 => bytes[88..96].copy_from_slice(&u64::MAX.to_le_bytes()),
                _ => bytes[96] = 255,
            }
            assert!(mapped(&bytes, &format!("bad-string-{i}")).is_err());
        }
        assert!(mapped(&valid[..63], "short-header").is_err());
        assert_eq!(b.encode([0; 32]), valid);
        assert_ne!(b.encode([1; 32]), valid);
    }
    #[test]
    fn authenticated_pages_still_require_valid_edges_and_utf8() {
        let store = std::env::temp_dir().join(format!("vm2-semantics-{}", std::process::id()));
        let mut b = ImageBuilder::default();
        b.cons_raw(Value(7), Value::NIL);
        let key = b.publish(&store, [0; 32]).unwrap();
        let image = unsafe { ImageRegion::open(&store, &key) }.unwrap();
        assert!(image.reference_raw(0).is_ok());
        assert!(image.pair(0).is_err());
        drop(image);
        let mut b = ImageBuilder::default();
        b.string_raw(&"x".repeat(PAGE * 3));
        let key = b.publish(&store, [0; 32]).unwrap();
        let path = store.join(&key);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        use std::os::unix::fs::FileExt;
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.write_all_at(&[255], (PAGE * 2) as u64).unwrap();
        let image = unsafe { ImageRegion::open(&store, &key) }.unwrap();
        assert!(image.string_reference_raw(0).is_ok());
        assert!(image.string(0).unwrap_err().contains("hash mismatch"));
        drop(image);
        std::fs::remove_dir_all(store).unwrap();
    }
    #[test]
    fn lazy_corruption_truncation_and_manifest_tampering_fail_closed() {
        use std::os::unix::fs::FileExt;
        let store = std::env::temp_dir().join(format!("vm2-corrupt-{}", std::process::id()));
        let mut b = ImageBuilder::default();
        for _ in 0..32768 {
            b.cons_raw(Value::NIL, Value::NIL);
        }
        let key = b.publish(&store, [7; 32]).unwrap();
        let path = store.join(&key);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.write_all_at(&[255], 64 + 32767 * 16).unwrap();
        let image = unsafe { ImageRegion::open(&store, &key) }.unwrap();
        assert_eq!(image.pair(0).unwrap(), (Value::NIL, Value::NIL));
        assert!(image.pair(32767).unwrap_err().contains("hash mismatch"));
        assert!(image.pair(32767).is_err());
        drop(image);
        file.set_len(8192).unwrap();
        assert!(unsafe { ImageRegion::open(&store, &key) }.is_err());
        let manifest = store.join(format!("{key}.pages"));
        std::fs::remove_file(&manifest).unwrap();
        std::fs::write(&manifest, b"VM2PAGE1").unwrap();
        assert!(unsafe { ImageRegion::open(&store, &key) }.is_err());
        std::fs::remove_dir_all(store).unwrap();
    }
    #[test]
    fn opening_large_image_faults_only_header_pages() {
        let store = std::env::temp_dir().join(format!("vm2-demand-{}", std::process::id()));
        let mut b = ImageBuilder::default();
        for _ in 0..524288 {
            b.cons_raw(Value::NIL, Value::NIL);
        }
        let key = b.publish(&store, [9; 32]).unwrap();
        let image = unsafe { ImageRegion::open(&store, &key) }.unwrap();
        let address = image.ptr as usize;
        let smaps = std::fs::read_to_string("/proc/self/smaps").unwrap();
        let mut selected = false;
        let mut rss = None;
        for line in smaps.lines() {
            if let Some(range) = line.split_whitespace().next().filter(|s| s.contains('-')) {
                let (a, z) = range.split_once('-').unwrap();
                if let (Ok(a), Ok(z)) = (usize::from_str_radix(a, 16), usize::from_str_radix(z, 16))
                {
                    selected = a <= address && address < z;
                }
            }
            if selected && line.starts_with("Rss:") {
                rss = Some(
                    line.split_whitespace()
                        .nth(1)
                        .unwrap()
                        .parse::<usize>()
                        .unwrap(),
                );
                break;
            }
        }
        println!(
            "mapping bytes={} startup_rss_kib={}",
            image.bytes,
            rss.unwrap()
        );
        assert!(
            rss.unwrap() <= 256,
            "mapping RSS {} KiB exceeds header fault budget",
            rss.unwrap()
        );
        assert_eq!(image.pair(524287).unwrap(), (Value::NIL, Value::NIL));
        drop(image);
        std::fs::remove_dir_all(store).unwrap();
    }
    #[test]
    fn executable_image_is_untraced_before_and_after_lazy_lowering() {
        let store = std::env::temp_dir().join(format!("vm2-code-untraced-{}", std::process::id()));
        let mut source = Vm::new();
        let cold = source
            .compile(
                &Form::from_json(
                    serde_json::json!(["quote", ["cold-symbol", {"string":"cold text"}]] ),
                )
                .unwrap(),
            )
            .unwrap();
        let entry = source
            .compile(
                &Form::from_json(
                    serde_json::json!(["quote", ["live-symbol", {"string":"live text"}]] ),
                )
                .unwrap(),
            )
            .unwrap();
        let key = source.publish_code_image(&store, entry, [0; 32]).unwrap();
        let image = unsafe { ImageRegion::open(&store, &key) }.unwrap();
        let (mut vm, entry) = Vm::from_code_image(image.clone(), [0; 32]).unwrap();
        assert_eq!(vm.image_functions_materialized(), 0);
        assert_eq!(unsafe { mprotect(image.ptr as *mut _, image.bytes, 0) }, 0);
        vm.collect(&mut []);
        assert_eq!(unsafe { mprotect(image.ptr as *mut _, image.bytes, 1) }, 0);
        let root = vm.execute_raw(entry).unwrap();
        assert!(!Vm::image_value(root), "EXECUTABLE_ROOT_PRIVATE");
        vm.conses.push((Value::NIL, Value::NIL));
        let holder = Value::reference(vm.conses.len(), 3);
        vm.conses.push((root, Value::NIL));
        let mut roots = [holder, root];
        assert_eq!(unsafe { mprotect(image.ptr as *mut _, image.bytes, 0) }, 0);
        vm.collect(&mut roots);
        vm.collect(&mut roots);
        assert_eq!(unsafe { mprotect(image.ptr as *mut _, image.bytes, 1) }, 0);
        assert_eq!(vm.image_functions_materialized(), 1);
        assert_eq!(vm.cons_pair(roots[0]).unwrap().0, roots[1], "HOLDER_FORWARDED_CHILD");
        assert_eq!(vm.display(roots[1]).unwrap(), "(live-symbol \"live text\")");
        let result = vm.execute_raw(cold).unwrap();
        assert_eq!(vm.display(result).unwrap(), "(cold-symbol \"cold text\")");
        std::fs::remove_dir_all(store).unwrap();
    }
    #[test]
    fn content_addressed_publication_reuse_and_corruption_fail_closed() {
        let store = std::env::temp_dir().join(format!("vm2-store-{}", std::process::id()));
        let mut b = ImageBuilder::default();
        b.string_raw("sentinel");
        let key = b.publish(&store, [1; 32]).unwrap();
        assert_eq!(b.publish(&store, [1; 32]).unwrap(), key);
        assert!(unsafe { ImageRegion::open(&store, "../escape") }.is_err());
        assert!(unsafe { ImageRegion::open(&store, &"0".repeat(64)) }.is_err());
        let image = unsafe { ImageRegion::open(&store, &key) }.unwrap();
        assert_eq!(image.string(0).unwrap(), "sentinel");
        drop(image);
        std::fs::remove_file(store.join(&key)).unwrap();
        std::fs::write(store.join(&key), b.encode([2; 32])).unwrap();
        assert!(unsafe { ImageRegion::open(&store, &key) }.is_err());
        assert!(b.publish(&store, [1; 32]).is_err());
        std::fs::remove_dir_all(store).unwrap();
    }
}
