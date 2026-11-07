//! The world store and the launch protocol (DESIGN-M1 §5, §7, §9, as the
//! addendum revises them). A world is two files in a private store, named
//! by its key: `KEY.rwi`, the arena image, and `KEY.json`, its manifest,
//! with the keyed host records the build consumed and the events the
//! embedder printed while it built.
//!
//! The key binds the executable's content, the libraries it loaded
//! (`libraries_digest`) and the embedder's recipe. The records bind every
//! host fact the build read; a launch asks the host each of them again and
//! boots from source on any difference. The manifest binds the image's
//! content by its digest (`content_digest`), which a launch checks against the file it maps,
//! hashed once and then known by its identity.
//!
//! A launch is one of two processes:
//! - the launcher, with its world valid, maps the image and allocates from
//!   it from then on (`arena::adopt`), and takes the VM
//!   (`Launch::Restored`);
//! - with no valid world, the launcher starts a builder in the pinned
//!   environment, holding the world's lock from its check through the
//!   spawn and handing it on, or waits for the one that runs. A waiting
//!   launch boots from the world it built (`wait`); one that does not wait
//!   boots from source beside it (`Launch::Chain`). The builder runs the
//!   recipe in build mode (`Launch::Build`), then `finish_build` publishes
//!   the world.
//!
//! A build that fails is recorded, and how depends on why. A deterministic
//! refusal, the fence's or a capture's (a resource of the builder, a VM not
//! at rest), is a manifest bound to the records the build read: launches
//! boot from source until one of those changes. Any other failure (an I/O
//! error, a full disk, a builder killed or timed out) is transient:
//! `KEY.failed` counts the attempts, and a launch retries one minute after
//! the first, ten after the second, an hour after the third and a day
//! after any later one. `reset` forgets both.
//!
//! A restored process holds `KEY.use` shared for its life and stamps it
//! with the time: what `prune` keeps beside the current world. One lock
//! orders them all: `store.lock`, never deleted, held shared by whoever
//! opens and locks a world's lock or use file or maps an image, and
//! exclusive by `prune` and `reset` while they unlink.
use super::{arena, image};
use crate::{Vm, host};
use serde_json::{Value as Json, json};
use std::ffi::OsString;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "romacs-world 3";
/// Set in a builder: the store it publishes to.
const BUILD_ENV: &str = "ROMACS_WORLD_BUILD";
/// Set in a builder: the descriptor of the world's lock, which its
/// launcher took and handed on as the builder's standard input.
const LOCK_ENV: &str = "ROMACS_WORLD_LOCK_FD";
/// How long a launch waits for a builder before it boots from source and
/// counts the build as failed.
const BUILD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);
/// How long after the Nth transient failure a launch builds again.
const BACKOFF: [u64; 4] = [60, 600, 3600, 86400];

/// Whether worlds are off for this process: ROMACS_WORLD=off.
pub fn disabled() -> bool {
    host::getenv("ROMACS_WORLD").is_some_and(|v| v == "off")
}

/// Whether this process is a world's builder: one `launch` makes
/// `Launch::Build`, before any window.
pub fn building() -> bool {
    host::getenv(BUILD_ENV).is_some()
}

/// One recipe's world in a store.
#[derive(Clone)]
pub struct World {
    pub store: PathBuf,
    pub key: [u8; 32],
    pub recipe: Vec<String>,
}

/// What this process does with its world. T is what the embedder carries
/// beside the VM.
pub enum Launch<T> {
    /// The image is mapped: its VM, to reinitialize, what the builder
    /// carried, and the manifest.
    Restored { vm: Box<Vm>, carry: T, manifest: Json },
    /// This process builds the world: run the recipe in build mode, then
    /// `finish_build`.
    Build,
    /// Boot from source, for the reason given.
    Chain { reason: Json },
}

/// Why the arena refused an image, for a launch's reason: a range the
/// process could not map says at which step, with what error, and whether
/// the range was reserved (not under an address-space limit, nor where
/// something was mapped before the process's first allocation).
fn refusal(refused: arena::Restore) -> Json {
    match arena::map_failure() {
        Some((step, errno, reserved)) if refused == arena::Restore::Map => {
            let error = (errno != 0).then(|| std::io::Error::from_raw_os_error(errno).to_string());
            json!({"image": "Map", "step": step, "error": error, "reserved": reserved})
        }
        _ => json!({"image": format!("{refused:?}")}),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256(data: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(data).into()
}

/// The content digest of FILE (`content_digest`), cached in STORE by the
/// file's identity and change times: hashing tens or hundreds of megabytes
/// at every launch would cost more than the rest of a restore. A write, a
/// truncation or a change of mode changes the identity.
fn file_digest(store: &Path, file: &std::fs::File) -> std::io::Result<[u8; 32]> {
    let meta = host::file_metadata(file)?;
    let cached = store.join("ids").join(identity_name(&meta));
    if let Ok(text) = host::read_store_file(&cached)
        && let Some(digest) = (0..32).map(|i| u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()).collect::<Option<Vec<u8>>>()
    {
        return Ok(digest.try_into().expect("32 bytes"));
    }
    let digest = content_digest(file, meta.len())?;
    let _ = store_subdir(&store.join("ids")).and_then(|()| host::write_store_file(&cached, hex(&digest)));
    Ok(digest)
}

/// How much of a file one digest worker reads at a time.
const DIGEST_CHUNK: u64 = 4 << 20;

/// FILE's first LEN bytes as one digest: SHA-256 over LEN, then each
/// `DIGEST_CHUNK` chunk's length and SHA-256, in order. Chunks are hashed
/// on up to eight threads, each reading its own with `pread`, so a hole
/// hashes as the zeros it reads as. A file that ends before LEN fails. The
/// thread count is fixed, not the host's: the digest never depends on it.
fn content_digest(file: &std::fs::File, len: u64) -> std::io::Result<[u8; 32]> {
    use sha2::Digest;
    use std::os::unix::fs::FileExt;
    let chunks = len.div_ceil(DIGEST_CHUNK) as usize;
    let bounds = |i: usize| (i as u64 * DIGEST_CHUNK, ((i as u64 + 1) * DIGEST_CHUNK).min(len));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let hash_chunks = || -> std::io::Result<Vec<(usize, [u8; 32])>> {
        let (mut buffer, mut hashed) = (vec![0u8; 1 << 20], Vec::new());
        loop {
            let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if i >= chunks {
                return Ok(hashed);
            }
            let (mut at, end) = bounds(i);
            let mut digest = sha2::Sha256::new();
            while at < end {
                let n = (end - at).min(buffer.len() as u64) as usize;
                file.read_exact_at(&mut buffer[..n], at)?;
                digest.update(&buffer[..n]);
                at += n as u64;
            }
            hashed.push((i, digest.finalize().into()));
        }
    };
    let threads = chunks.min(8);
    let parts: Vec<std::io::Result<Vec<(usize, [u8; 32])>>> = std::thread::scope(|scope| {
        // A worker that cannot start (an address-space limit) leaves its
        // chunks to the others; this thread takes part too.
        let workers: Vec<_> = (1..threads)
            .filter_map(|_| std::thread::Builder::new().stack_size(256 << 10).spawn_scoped(scope, hash_chunks).ok())
            .collect();
        let mine = hash_chunks();
        std::iter::once(mine)
            .chain(workers.into_iter().map(|w| w.join().unwrap_or_else(|_| Err(std::io::Error::other("digest worker panicked")))))
            .collect()
    });
    let mut digests = vec![[0u8; 32]; chunks];
    for part in parts {
        for (i, digest) in part? {
            digests[i] = digest;
        }
    }
    let mut tree = sha2::Sha256::new();
    tree.update(len.to_le_bytes());
    for (i, digest) in digests.iter().enumerate() {
        let (start, end) = bounds(i);
        tree.update((end - start).to_le_bytes());
        tree.update(digest);
    }
    Ok(tree.finalize().into())
}

/// A directory DIR of the store's own, made 0700 if missing: private
/// whatever the umask, as the store is.
pub(super) fn store_subdir(dir: &Path) -> std::io::Result<()> {
    match host::create_dir(dir, 0o700) {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        made => made,
    }
}

/// A file's identity and change times, as the digest cache names it.
fn identity_name(meta: &std::fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt;
    format!("{}-{}-{}-{}.{}-{}.{}", meta.dev(), meta.ino(), meta.size(), meta.mtime(), meta.mtime_nsec(),
            meta.ctime(), meta.ctime_nsec())
}

/// xxh3-128 of SEGMENTS, each with its file offset and length, so no two
/// layouts of the same bytes hash alike.
fn segments_hash<'a>(segments: impl Iterator<Item = (u64, &'a [u8])>) -> u128 {
    let mut hash = twox_hash::XxHash3_128::new();
    for (offset, bytes) in segments {
        hash.write(&offset.to_le_bytes());
        hash.write(&(bytes.len() as u64).to_le_bytes());
        hash.write(bytes);
    }
    hash.finish_128()
}

/// What identifies LIBRARY's code and constants, beside its name: its
/// build-id, when it has one, and the hash of its constant segments as
/// mapped (`arena::Library::constant_segments`). A build-id alone does not:
/// a file patched after linking keeps its note. The mapping is hashed at
/// every launch, from memory already resident: no cache could vouch for a
/// file without reading it. A library whose loader patched its read-only
/// segments (text relocations) hashes differently at each address, so a
/// world keyed on it is never found again, and its launches boot from
/// source. An error when a segment cannot be read.
fn library_content(library: &arena::Library) -> Result<(String, String), String> {
    let build_id = library.build_id().map_or("-".to_owned(), hex);
    let segments = library.constant_segments()
        .ok_or_else(|| format!("{}: a constant segment is unreadable", String::from_utf8_lossy(library.name)))?;
    Ok((build_id, format!("{:032x}", segments_hash(segments.into_iter()))))
}

/// The digest of the libraries this process loaded before its first
/// allocation, each by its name and `library_content`, in no order, since
/// where each loaded is random. An error when one cannot be identified, or
/// more were loaded than the inventory holds.
fn libraries_digest() -> Result<[u8; 32], String> {
    let libraries = arena::libraries().ok_or("more libraries are loaded than the inventory holds")?;
    let mut entries = libraries.iter()
        .map(|l| library_content(l).map(|(id, hash)| format!("{} {id} {hash}", String::from_utf8_lossy(l.name))))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort();
    entries.dedup();
    Ok(sha256(entries.join("\n").as_bytes()))
}

/// The file that makes a directory a world store: Romacs wrote it there,
/// with the store's format. `reset` and `prune` touch no directory without
/// it.
const MARKER: &str = ".romacs-world-store";
/// The store-wide lock (`store_lock`), never deleted.
const STORE_LOCK: &str = "store.lock";
const MARKER_TEXT: &str = "romacs-world-store 1\n";

/// The store directory: made, private and marked, where no directory is
/// or an empty one is; any other is refused unless it holds a valid marker
/// (`marker_error`). It must be owned by this user, private to them (§9.2),
/// no symlink, and neither $HOME nor `/` (`owned_store`).
fn private_store(store: &Path) -> std::io::Result<()> {
    let home = host::getenv("HOME").map(PathBuf::from);
    // Made, with whatever is missing on the way, relative to directories
    // checked on the way down.
    let dir = host::open_store_dir(store, true)?;
    owned_store(&dir, store, home.as_deref())?;
    if host::store_dir_is_empty(dir.as_fd())? {
        return write_marker(&dir);
    }
    match marker_error(&dir) {
        None => Ok(()),
        Some(why) => Err(std::io::Error::other(format!("{} is no world store: {why}", store.display()))),
    }
}

/// STORE as a world knows it: absolute against this process's directory,
/// with `.` and `..` taken by name, as `expand-file-name` takes them. The
/// walk (`host::open_store_dir`) then meets no `..`, and every later open
/// names the directory it checked.
fn store_path(store: &Path) -> std::io::Result<PathBuf> {
    use std::path::Component;
    let absolute = if store.is_absolute() { store.to_owned() } else { host::current_dir()?.join(store) };
    Ok(absolute.components().fold(PathBuf::new(), |mut path, component| {
        match component {
            Component::ParentDir => { path.pop(); }
            Component::CurDir => {}
            other => path.push(other),
        }
        path
    }))
}

/// Mark the store DIR as a world store: a new marker, made in DIR itself,
/// never through a symlink.
fn write_marker(dir: &std::os::fd::OwnedFd) -> std::io::Result<()> {
    use std::io::Write;
    host::open_store_file_at(dir.as_fd(), MARKER, host::StoreOpen::New)?.write_all(MARKER_TEXT.as_bytes())
}

/// Why the store DIR's marker is no valid one, or None when it is: opened
/// in DIR without following a symlink, a regular file this user owns, of
/// this format.
fn marker_error(dir: &std::os::fd::OwnedFd) -> Option<String> {
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    let mut file = match host::open_store_file_at(dir.as_fd(), MARKER, host::StoreOpen::Read) {
        Ok(file) => file,
        Err(e) => return Some(format!("its {MARKER} cannot be opened as a file of its own: {e}")),
    };
    let meta = match host::file_metadata(&file) { Ok(meta) => meta, Err(e) => return Some(e.to_string()) };
    if !meta.file_type().is_file() || meta.uid() != host::euid() {
        return Some(format!("its {MARKER} is not a regular file of this user's"));
    }
    let mut text = String::new();
    if file.by_ref().take(256).read_to_string(&mut text).is_err() || text != MARKER_TEXT {
        return Some(format!("its {MARKER} is not of this format ({})", MARKER_TEXT.trim()));
    }
    None
}

/// Refuse the store DIR, at STORE, unless this user owns it and no one
/// else may enter, and it is neither HOME nor `/`.
fn owned_store(dir: &std::os::fd::OwnedFd, store: &Path, home: Option<&Path>) -> std::io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let refuse = |why: &str| Err(std::io::Error::other(format!("world store {}: {why}", store.display())));
    let meta = host::file_metadata(&std::fs::File::from(dir.try_clone()?))?;
    if !meta.file_type().is_dir() {
        return refuse("not a directory");
    }
    if meta.uid() != host::euid() || meta.permissions().mode() & 0o777 != 0o700 {
        return refuse("not private to this user (0700)");
    }
    let same = |other: &Path| host::metadata(other).is_ok_and(|m| (m.dev(), m.ino()) == (meta.dev(), meta.ino()));
    if same(Path::new("/")) || home.is_some_and(same) {
        return refuse("the home or root directory");
    }
    Ok(())
}

/// STORE, checked as `reset` and `prune` need it: reached by
/// `host::open_store_dir` without making anything, owned, private, not
/// HOME or `/` (`owned_store`), and marked as a store.
fn checked_store(store: &Path, home: Option<&Path>) -> std::io::Result<()> {
    let dir = host::open_store_dir(store, false)?;
    owned_store(&dir, store, home)?;
    match marker_error(&dir) {
        None => Ok(()),
        Some(why) => Err(std::io::Error::other(format!("{} is not touched: {why}", store.display()))),
    }
}

/// Whether PATH is a regular file, its own and not a symlink's target.
fn regular(path: &Path) -> bool {
    host::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file())
}

/// Whether PATH is a directory, not a symlink to one.
fn real_dir(path: &Path) -> bool {
    host::symlink_metadata(path).is_ok_and(|m| m.file_type().is_dir())
}

/// Whether NAME is a digest-cache entry's: `identity_name`'s shape.
fn identity_entry(name: &str) -> bool {
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    let fields: Vec<&str> = name.split('-').collect();
    fields.len() == 5 && fields[..3].iter().all(|f| digits(f))
        && fields[3..].iter().all(|f| f.split_once('.').is_some_and(|(a, b)| digits(a) && digits(b)))
}

impl World {
    /// RECIPE's world in STORE: the key of this executable and RECIPE.
    pub fn new(store: PathBuf, recipe: Vec<String>) -> std::io::Result<World> {
        let store = store_path(&store)?;
        private_store(&store)?;
        let executable = file_digest(&store, &host::open_world_file("/proc/self/exe")?)?;
        let libraries = libraries_digest()
            .map_err(|e| std::io::Error::other(format!("the libraries this process loaded cannot all be identified: {e}")))?;
        let mut text = format!("{FORMAT}\nexecutable {}\nlibraries {}\n", hex(&executable), hex(&libraries));
        for line in &recipe {
            text.push_str(line);
            text.push('\n');
        }
        Ok(World { store, key: sha256(text.as_bytes()), recipe })
    }

    /// The world LINES add to this one's recipe: keyed by this world's key
    /// and LINES, so it binds all this one does (an init layer,
    /// `super::layer`).
    pub fn layered(&self, lines: Vec<String>) -> World {
        let text = format!("{FORMAT}\nbase {}\n{}\n", hex(&self.key), lines.join("\n"));
        let mut recipe = self.recipe.clone();
        recipe.extend(lines);
        World { store: self.store.clone(), key: sha256(text.as_bytes()), recipe }
    }

    fn file(&self, suffix: &str) -> PathBuf {
        self.store.join(format!("{}{suffix}", hex(&self.key)))
    }

    /// The refusals of a build on these very inputs, when one stands: its
    /// records all hold, so building again would refuse the same.
    pub fn standing_refusal(&self) -> Option<Vec<Json>> {
        match self.valid_manifest() {
            Err(reason) => reason.get("refused")?.as_array().cloned(),
            Ok(_) => None,
        }
    }

    /// The identity of this world's Lisp, from its published manifest
    /// alone (no record is checked): its key, which binds the executable
    /// and the recipe, and the content of every file its build read. What
    /// a compile process restores, for the compiled-Lisp key
    /// (`crate::compiled_lisp`); a rebuild for another host fact, the
    /// environment's say, keeps it. None when no world was published.
    pub fn lisp_identity(&self) -> Option<String> {
        let text = host::read_store_file(self.file(".json")).ok()?;
        let manifest: Json = serde_json::from_str(&text).ok()?;
        let mut files: Vec<String> = manifest["records"].as_array()?.iter()
            .filter(|r| r["kind"] == "file-read")
            .map(|r| format!("{} {}", r["path"], r["sha256"]))
            .collect();
        files.sort();
        files.dedup();
        Some(format!("{} {}", hex(&self.key), hex(&sha256(files.join("\n").as_bytes()))))
    }

    /// Whether a launch would restore this world now: its manifest holds
    /// and its image is the one bound.
    pub fn ready(&self) -> bool {
        self.usable().is_ok()
    }

    /// Hold this world in use for the process's life (`prune` keeps it),
    /// as a restored launch holds the world it restored: for a world a
    /// launch boots beside, its layer's base.
    pub fn keep(&self) {
        self.mark_use();
    }

    /// The manifest, when its records still hold: every keyed fact the build
    /// read, asked again, gives the same answer.
    fn valid_manifest(&self) -> Result<Json, Json> {
        let text = host::read_store_file(self.file(".json")).map_err(|e| json!({"missing": e.to_string()}))?;
        let manifest: Json = serde_json::from_str(&text).map_err(|e| json!({"manifest": e.to_string()}))?;
        for record in manifest["records"].as_array().map(Vec::as_slice).unwrap_or_default() {
            let mut expected = record.clone();
            if let Some(fields) = expected.as_object_mut() { fields.remove("site"); }
            match host::replay(record) {
                Some(now) if now == expected => {}
                now => return Err(json!({"stale": expected, "now": now})),
            }
        }
        if let Some(refusals) = manifest["refusals"].as_array().filter(|r| !r.is_empty()) {
            return Err(json!({"refused": refusals}));
        }
        Ok(manifest)
    }

    /// The image MANIFEST names, opened, when its content is the one it
    /// binds.
    fn checked_image(&self, manifest: &Json) -> Result<std::fs::File, Json> {
        let image = host::open_store_file(self.file(".rwi"), host::StoreOpen::Read).map_err(|e| json!({"missing": e.to_string()}))?;
        let digest = file_digest(&self.store, &image).map_err(|e| json!({"image": e.to_string()}))?;
        match manifest["image"].as_str() {
            Some(bound) if bound == hex(&digest) => Ok(image),
            bound => Err(json!({"image": {"bound": bound, "now": hex(&digest)}})),
        }
    }

    /// What this process does: see the module documentation. BUILDER_ARGS
    /// are this process's arguments as a builder, which runs elsewhere, needs
    /// them: every path absolute. With WAIT a missing or stale world is
    /// built first, or the running build waited for, and the launch then
    /// boots from it; ON_WAIT runs once before the launch blocks for that,
    /// so its embedder can say so.
    pub fn launch<T: super::safe::SnapshotSafe + 'static>(&self, builder_args: impl FnOnce() -> Vec<OsString>, wait: bool,
                                                          on_wait: impl FnOnce()) -> Launch<T> {
        if building() {
            host::unset_env(BUILD_ENV);
            return Launch::Build;
        }
        if disabled() {
            return Launch::Chain { reason: json!("ROMACS_WORLD=off") };
        }
        if !arena::active() {
            return Launch::Chain { reason: json!("no arena") };
        }
        let usable = match self.usable() {
            Ok(usable) => usable,
            Err(reason) => match self.build_for(reason, builder_args, wait, on_wait) {
                Ok(usable) => usable,
                Err(reason) => return Launch::Chain { reason },
            },
        };
        let (image, manifest) = usable;
        // Mapped and marked in use under the store's lock: `prune` then
        // finds this world held, or does not find it at all.
        let _store = self.store_lock(libc::LOCK_SH);
        match arena::adopt(&image) {
            arena::Restore::Mapped => match image::restore::<T>(&self.key) {
                Some((vm, carry)) => {
                    self.mark_use();
                    Launch::Restored { vm: Box::new(vm), carry, manifest }
                }
                None => Launch::Chain { reason: json!({"image": "made for another key"}) },
            },
            refused => Launch::Chain { reason: refusal(refused) },
        }
    }

    /// The world made usable without mapping it: its manifest when it
    /// already holds, else once a build made it so (built here, holding the
    /// lock from the check through the spawn, or waited for). The reason it
    /// could not be, otherwise, as `launch` would boot from source for.
    pub fn prepare(&self, builder_args: impl FnOnce() -> Vec<OsString>) -> Result<Json, Json> {
        if disabled() {
            return Err(json!("ROMACS_WORLD=off"));
        }
        match self.usable() {
            Ok((_, manifest)) => Ok(manifest),
            Err(reason) => self.build_for(reason, builder_args, true, || {}).map(|(_, manifest)| manifest),
        }
    }

    /// The world restored, when it is usable now, without building: for a
    /// worker isolate, which runs its spawner's exact world or nothing
    /// (rlisp/vm/isolate.el `isolate-world-unavailable').
    pub fn restore_only<T: super::safe::SnapshotSafe + 'static>(&self) -> Result<(Box<Vm>, T), Json> {
        if !arena::active() {
            return Err(json!("no arena"));
        }
        let (image, _manifest) = self.usable()?;
        let _store = self.store_lock(libc::LOCK_SH);
        match arena::adopt(&image) {
            arena::Restore::Mapped => match image::restore::<T>(&self.key) {
                Some((vm, carry)) => {
                    self.mark_use();
                    Ok((Box::new(vm), carry))
                }
                None => Err(json!({"image": "made for another key"})),
            },
            refused => Err(json!({"image": format!("{refused:?}")})),
        }
    }

    /// The world's image, opened, and its manifest, when both hold: every
    /// record and the image's content.
    fn usable(&self) -> Result<(std::fs::File, Json), Json> {
        let manifest = self.valid_manifest()?;
        Ok((self.checked_image(&manifest)?, manifest))
    }

    /// The world's image and manifest once a build made them usable, for a
    /// launch that found them not for REASON: built here, holding the lock from its
    /// check through the spawn; or waited for while another builds. Without
    /// WAIT a builder is started, and the launch boots from source. The
    /// reason to boot from source otherwise: a refusal on these inputs, a
    /// transient failure not yet due a retry, or a failed or timed-out
    /// build.
    fn build_for(&self, reason: Json, builder_args: impl FnOnce() -> Vec<OsString>, wait: bool,
                 on_wait: impl FnOnce()) -> Result<(std::fs::File, Json), Json> {
        let mut on_wait = Some(on_wait);
        let mut announce = || if let Some(f) = on_wait.take() { f() };
        let mut reason = reason;
        let lock = loop {
            if reason.get("refused").is_some() {
                return Err(json!({"world": reason}));
            }
            if let Some(seconds) = self.retry_in() {
                return Err(json!({"world": reason, "retry_in": seconds}));
            }
            match self.try_lock() {
                Ok(Some(lock)) => break lock,
                Ok(None) if wait => {
                    // Another launch's builder runs: wait for it, then look
                    // at what it left.
                    announce();
                    if !self.wait_unlocked(BUILD_TIMEOUT) {
                        return Err(json!({"world": reason, "builder": "another builder still runs"}));
                    }
                    match self.usable() {
                        Ok(usable) => return Ok(usable),
                        Err(now) => reason = now,
                    }
                }
                Ok(None) => return Err(json!({"world": reason, "builder": "a builder runs"})),
                Err(e) => return Err(json!({"world": reason, "builder": e.to_string()})),
            }
        };
        // With the lock held, no builder runs: what one left is final.
        if let Ok(usable) = self.usable() {
            return Ok(usable);
        }
        let attempts = self.failures().map(|f| f.0);
        if wait {
            announce();
        }
        let child = match self.start_builder(builder_args(), lock) {
            Ok(child) => child,
            Err(e) => return Err(json!({"world": reason, "builder": e.to_string()})),
        };
        if !wait {
            let pid = child.id();
            let world = self.clone();
            // Reap it, whoever outlives whom, and count a failure it could
            // not record itself.
            std::thread::spawn(move || {
                let mut child = child;
                let status = child.wait();
                world.note_builder_end(status.map(|s| s.to_string()).map_err(|e| e.to_string()), attempts);
            });
            return Err(json!({"world": reason, "builder": pid}));
        }
        let ended = wait_bounded(child, BUILD_TIMEOUT);
        self.note_builder_end(ended.clone(), attempts);
        self.usable().map_err(|now| json!({"world": now, "builder": ended.err().unwrap_or_else(|| "built".into())}))
    }

    /// After a builder ended (ENDED, its status or why there is none):
    /// record a failure it could not record itself, unless it published or
    /// recorded one since ATTEMPTS.
    fn note_builder_end(&self, ended: Result<String, String>, attempts: Option<u64>) {
        let recorded = self.failures().map(|f| f.0) != attempts || self.usable().is_ok()
            || self.valid_manifest().is_err_and(|r| r.get("refused").is_some());
        if !recorded {
            let why = match ended { Ok(status) => format!("the builder ended: {status}"), Err(e) => e };
            let _ = self.record_transient(&why);
        }
    }

    /// The world's lock, if no builder holds it.
    fn try_lock(&self) -> std::io::Result<Option<std::fs::File>> {
        use std::os::fd::AsRawFd;
        let _store = self.store_lock(libc::LOCK_SH)?;
        let file = host::open_store_file(self.file(".lock"), host::StoreOpen::Write { create: true, truncate: false })?;
        pause_for_test();
        Ok((unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0).then_some(file))
    }

    /// The store-wide lock, `STORE/store.lock`, taken as KIND and held
    /// while the file lives. Never deleted, so every process locks the same
    /// one. Every process holds it shared while it opens and locks a
    /// world's own lock or use file, or maps an image; `prune` and `reset`
    /// hold it exclusive while they unlink: no process is then between
    /// opening such a file and locking it.
    fn store_lock(&self, kind: i32) -> std::io::Result<std::fs::File> {
        lock_store(&self.store, kind)
    }

    /// Wait up to TIMEOUT for the builder holding the lock to end: whether
    /// it did.
    fn wait_unlocked(&self, timeout: std::time::Duration) -> bool {
        let deadline = host::monotonic() + timeout;
        loop {
            match self.try_lock() {
                Ok(Some(_)) => return true,
                Ok(None) if host::monotonic() < deadline => std::thread::sleep(std::time::Duration::from_millis(50)),
                _ => return false,
            }
        }
    }

    /// Start a builder with ARGS, handing it LOCK as its standard input: in
    /// the pinned environment under the store, its output in `KEY.log`.
    ///
    /// The builder is spawned, never forked: a launch forks while its GPU
    /// thread may be unloading a Vulkan driver, and fork runs the driver's
    /// atfork handlers, which glibc unregisters only after the driver's
    /// destructors ran (NVIDIA's then lock a mutex that is gone, at NULL).
    /// With no pre_exec the standard library uses posix_spawn, which runs
    /// no handlers (`host::spawn_isolate` does the same).
    fn start_builder(&self, args: Vec<OsString>, lock: std::fs::File) -> std::io::Result<std::process::Child> {
        use std::os::unix::process::CommandExt;
        let pinned = self.pinned_dir()?;
        let log = host::open_store_file(self.file(".log"), host::StoreOpen::Write { create: true, truncate: true })?;
        let mut env = pinned_env(&host::env_snapshot(), &pinned);
        env.push((BUILD_ENV.into(), self.store.clone().into_os_string()));
        env.push((LOCK_ENV.into(), "0".into()));
        let mut command = host::command(host::current_exe()?)?;
        // The builder holds the lock this process took, on its descriptor 0;
        // this one's copy closes once the builder runs.
        command.args(args).env_clear().envs(env).current_dir(pinned.join("cwd"))
            .stdin(lock).stdout(log.try_clone()?).stderr(log).process_group(0);
        command.spawn()
    }

    /// The transient failures recorded since the last success: how many,
    /// and when the last one was (seconds since the epoch).
    fn failures(&self) -> Option<(u64, u64)> {
        let text = host::read_store_file(self.file(".failed")).ok()?;
        let record: Json = serde_json::from_str(&text).ok()?;
        Some((record["attempts"].as_u64()?, record["at"].as_u64()?))
    }

    /// How many seconds until a transient failure is due a retry, if one is
    /// recorded and not yet due.
    fn retry_in(&self) -> Option<u64> {
        let (attempts, at) = self.failures()?;
        let wait = BACKOFF[(attempts.max(1) - 1).min(BACKOFF.len() as u64 - 1) as usize];
        (at + wait).checked_sub(now_seconds()).filter(|&left| left > 0)
    }

    /// Count a transient failure, ERROR, after those recorded.
    fn record_transient(&self, error: &str) -> std::io::Result<()> {
        let attempts = self.failures().map_or(0, |f| f.0) + 1;
        let record = json!({"kind": "transient", "error": error, "attempts": attempts, "at": now_seconds()});
        self.publish(".failed", &serde_json::to_vec(&record).map_err(std::io::Error::other)?)
    }

    /// Hold `KEY.use` shared for this process's life, stamped with the time:
    /// the world is in use, and was just now (`prune`).
    fn mark_use(&self) {
        use std::os::fd::AsRawFd;
        let Ok(_store) = self.store_lock(libc::LOCK_SH) else { return };
        if let Ok(file) = host::open_store_file(self.file(".use"), host::StoreOpen::Write { create: true, truncate: false }) {
            let _ = file.set_modified(host::clock());
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } == 0 {
                std::mem::forget(file);
            }
        }
    }

    /// The pinned table's directories, `STORE/p/{cwd,tmp,bin}`, empty and
    /// read-only (§5 step 1). They persist, so a record naming them holds.
    fn pinned_dir(&self) -> std::io::Result<PathBuf> {
        let p = self.store.join("p");
        store_subdir(&p)?;
        for name in ["cwd", "tmp", "bin"] {
            if !host::probe(host::Probe::IsDir, p.join(name)) {
                host::create_dir(p.join(name), 0o500)?;
            }
        }
        Ok(p)
    }

    /// A builder's read roots beside RECIPE's: the pinned table and the
    /// binary's own (`sandbox::build_roots`).
    pub fn build_roots(&self, recipe: Vec<PathBuf>) -> Vec<PathBuf> {
        super::sandbox::build_roots(recipe, &self.store)
    }

    /// A builder, once it holds the lock: confine this thread to ROOTS,
    /// writing only in the store, and record in build mode. The lock is
    /// held until the process exits.
    pub fn begin_build(&self, roots: &[PathBuf]) -> std::io::Result<()> {
        // The launcher's lock, handed on; taking it again on the same open
        // file only confirms it.
        use std::os::fd::{AsRawFd, FromRawFd};
        let handed = host::getenv(LOCK_ENV).and_then(|fd| fd.to_str()?.parse::<i32>().ok());
        host::unset_env(LOCK_ENV);
        // Without one, the world's lock is opened and locked under the
        // store's, held until the flock is: `prune` cannot unlink it between.
        let mut store = None;
        let lock = match handed {
            Some(fd) => unsafe { std::fs::File::from_raw_fd(fd) },
            None => {
                store = Some(self.store_lock(libc::LOCK_SH)?);
                let lock = host::open_store_file(self.file(".lock"), host::StoreOpen::Write { create: true, truncate: false })?;
                pause_for_test();
                lock
            }
        };
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(std::io::Error::other("another builder holds the lock"));
        }
        drop(store);
        unsafe { libc::fcntl(lock.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
        std::mem::forget(lock);
        self.remove_abandoned()?;
        super::sandbox::confine_publishing(roots, &self.store)?;
        let seed = self.key[..4].try_into().expect("4 bytes");
        host::enter_build(host::Pinned { clock: host::clock(), pid: 4242, seed });
        Ok(())
    }

    /// The manifest of a build that read TRACE, with REFUSALS and EVENTS.
    fn manifest(&self, trace: &Json, refusals: Json, events: Json) -> Json {
        let records: Vec<&Json> = trace["records"].as_array().map(|r| r.iter().filter(|r| host::keyed(r)).collect()).unwrap_or_default();
        json!({"format": FORMAT, "key": hex(&self.key), "recipe": self.recipe,
               "records": records, "refusals": refusals, "events": events,
               "built": host::clock().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())})
    }

    /// Publish BYTES as this world's SUFFIX file, read-only.
    fn publish(&self, suffix: &str, bytes: &[u8]) -> std::io::Result<()> {
        use std::io::Write;
        let (temporary, mut file) = self.temporary(suffix)?;
        file.write_all(bytes)?;
        read_only(&temporary)?;
        host::rename(&temporary, self.file(suffix))
    }

    /// A builder's work once `begin_build` returned: STEPS, in build mode,
    /// give the VM, what the embedder carries beside it and the events to
    /// keep, and `finish_build` publishes them. The one owner of what a
    /// failure leaves: a step's failure is transient, retried after a while
    /// (`retry_in`), and `finish_build` records its own. The image's
    /// length, or the failure to report.
    pub fn build<T: super::safe::SnapshotSafe + 'static>(&self, steps: impl FnOnce() -> Result<(Vm, T, Json), String>)
        -> Result<usize, String> {
        match steps() {
            Ok((vm, carry, events)) => self.finish_build(vm, carry, events),
            Err(error) => {
                let _ = host::leave_build();
                let _ = self.record_transient(&error);
                Err(error)
            }
        }
    }

    /// Record a build's refusal on its inputs, REFUSALS with the records of
    /// TRACE: launches boot from source until a record changes.
    fn record_refusal(&self, trace: &Json, refusals: Json) -> std::io::Result<()> {
        let manifest = self.manifest(trace, refusals, json!({}));
        self.publish(".json", &serde_json::to_vec(&manifest).map_err(std::io::Error::other)?)
    }

    /// A new file for what will be published as SUFFIX: a unique name, so
    /// no file a builder left behind stands in its way.
    fn temporary(&self, suffix: &str) -> std::io::Result<(PathBuf, std::fs::File)> {
        host::temp_file(&self.file(&format!("{suffix}.XXXXXX")))
    }

    /// Remove what interrupted builders of this world left unpublished: its
    /// temporaries, read-only once they were written. Only the holder of
    /// the lock runs this, so no builder is still writing one.
    fn remove_abandoned(&self) -> std::io::Result<()> {
        let prefix = format!("{}.", hex(&self.key));
        for entry in host::read_dir(&self.store)? {
            let name = entry?.file_name();
            let Some(rest) = name.to_str().and_then(|n| n.strip_prefix(&prefix)) else { continue };
            let temporary = match rest.split_once('.') {
                Some(("rwi" | "json", tail)) => tail == "new" || (tail.len() == 6 && tail.bytes().all(|b| b.is_ascii_alphanumeric())),
                _ => false,
            };
            if temporary {
                host::remove_file(self.store.join(&name))?;
            }
        }
        Ok(())
    }

    /// The builder's end: check what startup read, then publish VM, with
    /// CARRY beside it, as this world's image, and EVENTS, what the recipe
    /// printed, in its manifest. A build with any refusal publishes its
    /// manifest only, so launches boot from source until its records
    /// change. The image's length.
    fn finish_build<T: super::safe::SnapshotSafe + 'static>(&self, mut vm: Vm, carry: T, events: Json) -> Result<usize, String> {
        vm.world_check_inputs();
        // Kept to the end: whatever fails from here on is recorded with it.
        let trace = host::leave_build().ok_or("no world build was recording")?;
        if trace["refusals"].as_array().is_some_and(|r| !r.is_empty()) {
            self.record_refusal(&trace, trace["refusals"].clone()).map_err(|e| e.to_string())?;
            return Err(format!("world build refused: {}", trace["refusals"]));
        }
        match self.publish_world(vm, carry, &trace, events) {
            Ok(len) => {
                let _ = host::remove_file(self.file(".failed"));
                Ok(len)
            }
            // A capture refusal is the VM's state, which the same inputs
            // make again; anything else (a full disk, an I/O error) may pass.
            Err(error) if error.starts_with("world capture refused") => {
                let _ = self.record_refusal(&trace, json!([{"kind": "capture", "error": error}]));
                Err(error)
            }
            Err(error) => {
                let _ = self.record_transient(&error);
                Err(error)
            }
        }
    }

    /// Capture VM and CARRY as this world's image and publish it with the
    /// manifest of TRACE and EVENTS: the image's length.
    fn publish_world<T: super::safe::SnapshotSafe + 'static>(&self, mut vm: Vm, carry: T, trace: &Json, events: Json) -> Result<usize, String> {
        let mut manifest = self.manifest(trace, trace["refusals"].clone(), events);
        vm.world_lift();
        vm.world_normalize();
        vm.collect_compacting();
        if vm.conses.unused() + vm.closures.unused() + vm.cells.unused() + vm.objects.unused() != 0 {
            return Err("world image: the tables did not compact".into());
        }
        let (temporary, file) = self.temporary(".rwi").map_err(|e| e.to_string())?;
        let len = image::capture(vm, carry, &file, self.key)?;
        file.sync_all().map_err(|e| e.to_string())?;
        read_only(&temporary).map_err(|e| e.to_string())?;
        host::rename(&temporary, self.file(".rwi")).map_err(|e| e.to_string())?;
        // Hashed from the file as published, which also caches it.
        let image = host::open_store_file(self.file(".rwi"), host::StoreOpen::Read).map_err(|e| e.to_string())?;
        let digest = file_digest(&self.store, &image).map_err(|e| e.to_string())?;
        manifest["image"] = json!(hex(&digest));
        self.publish(".json", &serde_json::to_vec(&manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        Ok(len)
    }
}

/// A world unused this long is pruned (`prune`).
const UNUSED: u64 = 7 * 86400;
/// The most worlds, and bytes on disk, a store keeps beside those in use.
const MOST_WORLDS: usize = 3;
const MOST_BYTES: u64 = 4 << 30;

/// One world in a store, as `prune` sees it.
#[derive(Clone, Debug, PartialEq)]
struct Stored {
    key: String,
    /// Its family (`family`), which `prune_plan` bounds alone.
    family: String,
    /// When it was last used (seconds since the epoch).
    used: u64,
    /// Its files' size on disk.
    bytes: u64,
    /// A builder or a restored process holds it.
    busy: bool,
    /// It has a manifest and no image: a build's refusal on its inputs,
    /// which a launch resolving a layer reads (romacs-vm-gui world.rs).
    refusal: bool,
}

/// The worlds of WORLDS to delete, family by family (`plan_family`).
fn prune_plan(worlds: &[Stored], current: &str, now: u64) -> Vec<String> {
    let mut families: Vec<&str> = worlds.iter().map(|w| w.family.as_str()).collect();
    families.sort_unstable();
    families.dedup();
    families.into_iter()
        .flat_map(|family| plan_family(&worlds.iter().filter(|w| w.family == family).cloned().collect::<Vec<_>>(), current, now))
        .collect()
}

/// The worlds of WORLDS, one family's, to delete, CURRENT's kept: those a
/// builder or a restored process holds are kept too, and the others, most
/// recently used first, while they were used within `UNUSED` and the family
/// stays within `MOST_WORLDS` worlds and `MOST_BYTES` bytes. A refusal is
/// kept while it is within `UNUSED`, outside the bounds: forgotten, the
/// next launch would build its refused inputs again, as an init layer's
/// resolution does after each one it forgets.
fn plan_family(worlds: &[Stored], current: &str, now: u64) -> Vec<String> {
    let kept = |w: &&Stored| w.key == current || w.busy;
    let refusal = |w: &&Stored| w.refusal && !kept(w) && now.saturating_sub(w.used) <= UNUSED;
    let (mut count, mut bytes) = worlds.iter().filter(kept).fold((0, 0), |(c, b), w| (c + 1, b + w.bytes));
    let mut others: Vec<&Stored> = worlds.iter().filter(|w| !kept(w) && !refusal(w)).collect();
    others.sort_by(|a, b| b.used.cmp(&a.used).then_with(|| a.key.cmp(&b.key)));
    let mut delete = vec![];
    for world in others {
        if now.saturating_sub(world.used) <= UNUSED && count < MOST_WORLDS && bytes + world.bytes <= MOST_BYTES {
            count += 1;
            bytes += world.bytes;
        } else {
            delete.push(world.key.clone());
        }
    }
    delete
}

/// Whether NAME is a world's file in a store: a key, a dot and a suffix.
fn world_file(name: &str) -> Option<(&str, &str)> {
    let (key, suffix) = name.split_at_checked(64)?;
    (key.bytes().all(|b| b.is_ascii_hexdigit()) && suffix.starts_with('.')).then_some((key, &suffix[1..]))
}

/// Forget STORE's failure records: every world's transient failures and
/// refused manifests, so its next launch builds again. With EVERYTHING,
/// every world, the plans, the digest cache and the pinned table too,
/// leaving the marker and the store lock. Only a marked store is touched
/// (`checked_store`), and in it only regular files and real directories
/// with a store's names: no symlink is followed. How many it removed.
pub fn reset(store: &Path, everything: bool) -> std::io::Result<usize> {
    reset_in(store, everything, host::getenv("HOME").map(PathBuf::from).as_deref())
}

fn reset_in(store: &Path, everything: bool, home: Option<&Path>) -> std::io::Result<usize> {
    let store = &store_path(store)?;
    if host::symlink_metadata(store).is_err() {
        return Ok(0);
    }
    checked_store(store, home)?;
    let _store = lock_store(store, libc::LOCK_EX)?;
    let mut removed = 0;
    for entry in host::read_dir(store)? {
        let name = entry?.file_name();
        let Some(name) = name.to_str() else { continue };
        let path = store.join(name);
        let refused = || host::read_store_file(&path).ok()
            .and_then(|text| serde_json::from_str::<Json>(&text).ok())
            .is_some_and(|m| m["refusals"].as_array().is_some_and(|r| !r.is_empty()));
        // A world's lock or use file goes only when no process holds it.
        let held = |suffix: &str| ["lock", "use"].contains(&suffix) && host::open_store_file(&path, host::StoreOpen::Write { create: false, truncate: false })
            .is_ok_and(|file| { use std::os::fd::AsRawFd; (unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }) != 0 });
        match world_file(name) {
            Some((_, suffix)) if regular(&path) && !held(suffix) && (everything || suffix == "failed" || (suffix == "json" && refused())) => {
                host::remove_file(&path)?;
                removed += 1;
            }
            _ => {}
        }
    }
    if everything {
        let ids = store.join("ids");
        if real_dir(&ids) {
            for entry in host::read_dir(&ids)? {
                let name = entry?.file_name().to_string_lossy().into_owned();
                if identity_entry(&name) && regular(&ids.join(&name)) {
                    host::remove_file(ids.join(&name))?;
                    removed += 1;
                }
            }
        }
        let p = store.join("p");
        if real_dir(&p) {
            for name in ["cwd", "tmp", "bin"] {
                if real_dir(&p.join(name)) && host::remove_dir(p.join(name)).is_ok() {
                    removed += 1;
                }
            }
        }
        let plans = store.join("plans");
        if real_dir(&plans) {
            for entry in host::read_dir(&plans)? {
                let name = entry?.file_name().to_string_lossy().into_owned();
                if super::layer::plan_file(&name) && regular(&plans.join(&name)) {
                    host::remove_file(plans.join(&name))?;
                    removed += 1;
                }
            }
        }
    }
    Ok(removed)
}

impl World {
    /// Bound the store: delete the worlds `prune_plan` gives up, holding
    /// each one's locks while its files go, and the digest cache's entries
    /// for files no kept world or executable is, once they are a week old.
    /// The worlds deleted.
    pub fn prune(&self) -> std::io::Result<Vec<String>> {
        self.prune_in(host::getenv("HOME").map(PathBuf::from).as_deref(), false)
    }

    /// `prune` within this world's family only: what a batch builder
    /// bounds, so it never deletes a GUI's world, however long unused.
    pub fn prune_family(&self) -> std::io::Result<Vec<String>> {
        self.prune_in(host::getenv("HOME").map(PathBuf::from).as_deref(), true)
    }

    fn prune_in(&self, home: Option<&Path>, own_family: bool) -> std::io::Result<Vec<String>> {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::MetadataExt;
        checked_store(&self.store, home)?;
        // No process is between opening a world's lock or use file and
        // locking it while this is held: each lock below is as it is.
        let _store = self.store_lock(libc::LOCK_EX)?;
        let mut files: std::collections::BTreeMap<String, Vec<(String, std::fs::Metadata)>> = Default::default();
        for entry in host::read_dir(&self.store)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some((key, _)) = world_file(&name) && let Ok(meta) = host::symlink_metadata(entry.path())
                && meta.file_type().is_file()
            {
                files.entry(key.to_owned()).or_default().push((name, meta));
            }
        }
        let held = |path: PathBuf| -> bool {
            host::open_store_file(&path, host::StoreOpen::Write { create: false, truncate: false })
                .is_ok_and(|file| (unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }) != 0)
        };
        let worlds: Vec<Stored> = files.iter().map(|(key, files)| Stored {
            key: key.clone(),
            family: self.family(key),
            used: files.iter().filter(|(n, _)| [".use", ".json", ".rwi"].iter().any(|s| n.ends_with(s))).map(|(_, m)| m.mtime().max(0) as u64).max().unwrap_or(0),
            bytes: files.iter().map(|(_, m)| m.blocks() * 512).sum(),
            busy: held(self.store.join(format!("{key}.lock"))) || held(self.store.join(format!("{key}.use"))),
            refusal: files.iter().any(|(n, _)| n.ends_with(".json")) && !files.iter().any(|(n, _)| n.ends_with(".rwi")),
        }).collect();
        let current = hex(&self.key);
        let mine = self.family(&current);
        let worlds: Vec<Stored> = worlds.into_iter().filter(|w| !own_family || w.family == mine).collect();
        let mut deleted = vec![];
        for key in prune_plan(&worlds, &current, now_seconds()) {
            // Taken now, these say no builder or restored process came since.
            let locks: Vec<std::fs::File> = ["lock", "use"].iter()
                .filter_map(|s| host::open_store_file(self.store.join(format!("{key}.{s}")), host::StoreOpen::Write { create: false, truncate: false }).ok())
                .collect();
            if locks.iter().any(|f| unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0) {
                continue;
            }
            for (name, _) in &files[&key] {
                host::remove_file(self.store.join(name))?;
            }
            deleted.push(key);
        }
        // The digest cache: the executable's entry, and each kept image's.
        let mut referenced: std::collections::HashSet<String> = Default::default();
        referenced.extend(host::metadata("/proc/self/exe").ok().map(|m| identity_name(&m)));
        for key in files.keys().filter(|k| !deleted.contains(k)) {
            referenced.extend(host::metadata(self.store.join(format!("{key}.rwi"))).ok().map(|m| identity_name(&m)));
        }
        if real_dir(&self.store.join("ids")) && let Ok(entries) = host::read_dir(self.store.join("ids")) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let old = host::symlink_metadata(entry.path())
                    .is_ok_and(|m| m.file_type().is_file() && now_seconds().saturating_sub(m.mtime().max(0) as u64) > UNUSED);
                if old && identity_entry(&name) && !referenced.contains(&name) {
                    host::remove_file(entry.path())?;
                }
            }
        }
        Ok(deleted)
    }
}

impl World {
    /// The family of the world KEY names in this store: this world's own,
    /// whose manifest a build may not have published yet, or its manifest
    /// recipe's (none without a manifest). A GUI's worlds and a batch
    /// session's, one per locale or HOME it ran under, are pruned apart, so
    /// neither evicts the other's.
    fn family(&self, key: &str) -> String {
        if key == hex(&self.key) { return family(self.recipe.iter().map(String::as_str)); }
        let Ok(text) = host::read_store_file(self.store.join(format!("{key}.json"))) else { return String::new() };
        let manifest: Json = serde_json::from_str(&text).unwrap_or_default();
        family(manifest["recipe"].as_array().map(Vec::as_slice).unwrap_or_default().iter().filter_map(Json::as_str))
    }
}

/// The lines of RECIPE that say which consumer, launch and executable a
/// world is for: its family.
fn family<'a>(recipe: impl Iterator<Item = &'a str>) -> String {
    recipe.filter(|line| ["consumer ", "launch ", "exe "].iter().any(|p| line.starts_with(p))).collect::<Vec<_>>().join("\n")
}

/// STORE's store-wide lock (`World::store_lock`), taken as KIND.
fn lock_store(store: &Path, kind: i32) -> std::io::Result<std::fs::File> {
    use std::os::fd::AsRawFd;
    let file = host::open_store_file(store.join(STORE_LOCK), host::StoreOpen::Write { create: true, truncate: false })?;
    if unsafe { libc::flock(file.as_raw_fd(), kind) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(file)
}

/// A test's hook between opening a world's lock and locking it: with
/// ROMACS_WORLD_TEST_PAUSE_LOCK=DIR, say so in DIR/paused and wait for
/// DIR/resume. Only under the `world-test-hooks` feature, which only test
/// targets enable; a shipped binary has no such hook.
#[cfg(not(feature = "world-test-hooks"))]
fn pause_for_test() {}

#[cfg(feature = "world-test-hooks")]
fn pause_for_test() {
    let Some(dir) = host::getenv("ROMACS_WORLD_TEST_PAUSE_LOCK").map(PathBuf::from) else { return };
    let _ = host::write_store_file(dir.join("paused"), b"");
    let deadline = host::monotonic() + std::time::Duration::from_secs(60);
    while !host::probe(host::Probe::Exists, dir.join("resume")) && host::monotonic() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// The clock, in seconds since the epoch.
fn now_seconds() -> u64 {
    host::clock().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Wait for CHILD for up to TIMEOUT: its status, or why there is none, after
/// killing it at the deadline.
fn wait_bounded(mut child: std::process::Child, timeout: std::time::Duration) -> Result<String, String> {
    let deadline = host::monotonic() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(status.to_string()),
            Ok(Some(status)) => return Err(format!("the builder ended: {status}")),
            Ok(None) if host::monotonic() < deadline => std::thread::sleep(std::time::Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("the builder ran past {} s and was killed", timeout.as_secs()));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Make the file at PATH readable by its owner only, as a published world's
/// files are (§9.2).
fn read_only(path: &Path) -> std::io::Result<()> {
    let path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).map_err(std::io::Error::other)?;
    host::chmod(&path, 0o400, false)
}

/// The builder's environment: INVOKING without its per-process names, which
/// the pinned table under P supplies (§5 step 1). Keyed names keep their
/// values.
pub fn pinned_env(invoking: &[(OsString, OsString)], p: &Path) -> Vec<(OsString, OsString)> {
    let pinned = [
        ("PWD", p.join("cwd")), ("TMPDIR", p.join("tmp")), ("TMP", p.join("tmp")),
        ("TEMP", p.join("tmp")), ("PATH", p.join("bin")), ("TERM", PathBuf::from("dumb")),
    ];
    invoking.iter()
        .filter(|(name, _)| !host::P_ENV.iter().any(|p| name == p))
        .cloned()
        .chain(pinned.map(|(name, value)| (OsString::from(name), value.into_os_string())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a shared library at OUT from SOURCE with ARGS, or None when no
    /// C compiler is here.
    fn build(dir: &Path, name: &str, source: &str, args: &[&str]) -> PathBuf {
        let c = dir.join(format!("{name}.c"));
        std::fs::write(&c, source).unwrap();
        let out = dir.join(format!("{name}.so"));
        let status = std::process::Command::new("cc").args(["-shared", "-fPIC", "-O1", "-o"]).arg(&out).arg(&c).args(args)
            .status().expect("cc runs");
        assert!(status.success());
        out
    }

    /// Load PATH and the library as this process mapped it.
    fn load(path: &Path) -> arena::Library {
        let name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        assert!(!unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) }.is_null(), "{}", path.display());
        let mut found: Option<arena::Library> = None;
        unsafe extern "C" fn each(info: *mut libc::dl_phdr_info, _: libc::size_t, data: *mut libc::c_void) -> libc::c_int {
            let (want, found) = unsafe { &mut *data.cast::<(&std::ffi::CStr, &mut Option<arena::Library>)>() };
            let info = unsafe { &*info };
            if !info.dlpi_name.is_null() && unsafe { std::ffi::CStr::from_ptr(info.dlpi_name) } == *want {
                **found = Some(unsafe { arena::Library::at(info.dlpi_name, info.dlpi_addr as usize, info.dlpi_phdr as usize, info.dlpi_phnum as usize) });
            }
            0
        }
        let mut data = (name.as_c_str(), &mut found);
        unsafe { libc::dl_iterate_phdr(Some(each), (&raw mut data).cast()) };
        found.expect("loaded")
    }

    /// The old identity of a library without a build-id: its executable
    /// segments only.
    fn code_only(library: &arena::Library, path: &Path) -> u128 {
        let file = std::fs::read(path).unwrap();
        let headers = |i: usize| {
            let at = |o: usize, n: usize| { let mut b = [0u8; 8]; b[..n].copy_from_slice(&file[o..o + n]); u64::from_le_bytes(b) as usize };
            let h = at(0x20, 8) + i * at(0x36, 2);
            (at(h, 4), at(h + 4, 4), at(h + 8, 8), at(h + 0x20, 8))
        };
        let count = u16::from_le_bytes([file[0x38], file[0x39]]) as usize;
        let _ = library;
        segments_hash((0..count).map(headers).filter(|&(kind, flags, _, _)| kind == 1 && flags & 1 != 0)
            .map(|(_, _, offset, size)| (offset as u64, &file[offset..offset + size])))
    }

    /// A fresh store for test NAME, as `private_store` makes one.
    fn marked_store(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("romacs-store-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        private_store(&dir).unwrap();
        dir
    }

    #[test]
    fn reset_and_prune_touch_only_a_marked_private_store_and_follow_no_symlink() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let world = |store: &Path| World { store: store.to_owned(), key: [0xaa; 32], recipe: vec![] };
        let key = "b".repeat(64);
        // Without the marker: refused, every file left.
        let plain = std::env::temp_dir().join(format!("romacs-store-{}-plain", std::process::id()));
        let _ = std::fs::remove_dir_all(&plain);
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(plain.join(format!("{key}.failed")), "{}").unwrap();
        assert!(reset_in(&plain, true, None).is_err());
        assert!(world(&plain).prune_in(None, false).is_err());
        assert!(plain.join(format!("{key}.failed")).exists());
        // Marked, but open to others: refused.
        let open = marked_store("open");
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(reset_in(&open, true, None).is_err());
        // The home directory, or the root, even marked: refused.
        let home = marked_store("home");
        assert!(reset_in(&home, true, Some(&home)).is_err());
        assert!(world(&home).prune_in(Some(&home), false).is_err());
        assert!(reset_in(Path::new("/"), true, None).is_err());
        // Symlinks: never followed, their targets intact.
        let outside = std::env::temp_dir().join(format!("romacs-store-{}-outside", std::process::id()));
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(outside.join("cwd")).unwrap();
        std::fs::write(outside.join("1-2-3-4.5-6.7"), "00").unwrap();
        std::fs::write(outside.join("precious"), "mine").unwrap();
        let store = marked_store("links");
        symlink(&outside, store.join("ids")).unwrap();
        symlink(&outside, store.join("p")).unwrap();
        symlink(outside.join("precious"), store.join(format!("{key}.rwi"))).unwrap();
        std::fs::write(store.join(format!("{key}.json")), "{}").unwrap();
        assert_eq!(reset_in(&store, true, None).unwrap(), 1, "only the regular world file");
        assert!(outside.join("1-2-3-4.5-6.7").exists() && outside.join("cwd").exists());
        assert_eq!(std::fs::read_to_string(outside.join("precious")).unwrap(), "mine");
        assert!(!store.join(format!("{key}.json")).exists() && store.join(MARKER).exists());
        world(&store).prune_in(None, false).unwrap();
        assert_eq!(std::fs::read_to_string(outside.join("precious")).unwrap(), "mine");
        for dir in [plain, open, home, outside, store] {
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn a_store_path_is_absolute_and_takes_dot_and_dot_dot_by_name() {
        let cwd = host::current_dir().unwrap();
        assert_eq!(store_path(Path::new("/a/./b/../c/")).unwrap(), Path::new("/a/c"));
        assert_eq!(store_path(Path::new("/../a")).unwrap(), Path::new("/a"));
        assert_eq!(store_path(Path::new("worlds")).unwrap(), cwd.join("worlds"));
        let up: PathBuf = cwd.components().skip(1).map(|_| "..").collect();
        assert_eq!(store_path(&up.join("x/y")).unwrap(), Path::new("/x/y"));
    }

    #[test]
    fn a_store_is_reached_only_through_directories_no_one_else_may_replace() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = std::env::temp_dir().join(format!("romacs-store-{}-ancestors", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mode = |dir: &Path, mode: u32| std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode)).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        mode(&root, 0o700);
        // Missing parents are made private, whatever the umask allows.
        let made = root.join("made/a/b/store");
        private_store(&made).unwrap();
        for dir in [root.join("made"), root.join("made/a"), root.join("made/a/b"), made.clone()] {
            assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o7777, 0o700, "{}", dir.display());
        }
        // An ancestor others may write, and not sticky: refused, nothing made.
        let shared = root.join("shared");
        std::fs::create_dir(&shared).unwrap();
        mode(&shared, 0o770);
        let error = private_store(&shared.join("store")).unwrap_err().to_string();
        assert!(error.contains("writable by others"), "{error}");
        assert!(!shared.join("store").exists());
        mode(&shared, 0o702);
        assert!(private_store(&shared.join("store")).is_err());
        assert!(!shared.join("store").exists());
        // A sticky one, as /tmp is: others may not replace what is ours.
        let sticky = root.join("sticky");
        std::fs::create_dir(&sticky).unwrap();
        mode(&sticky, 0o1777);
        private_store(&sticky.join("store")).unwrap();
        assert!(checked_store(&sticky.join("store"), None).is_ok());
        // A symlinked ancestor: refused, and nothing made at its target.
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        mode(&elsewhere, 0o700);
        symlink(&elsewhere, root.join("linked")).unwrap();
        let error = private_store(&root.join("linked/store")).unwrap_err().to_string();
        assert!(error.contains("symlink"), "{error}");
        assert!(std::fs::read_dir(&elsewhere).unwrap().next().is_none());
        // Nor are reset and prune led through one to a real store.
        private_store(&elsewhere.join("store")).unwrap();
        assert!(reset_in(&root.join("linked/store"), true, None).is_err());
        assert!(World { store: root.join("linked/store"), key: [0; 32], recipe: vec![] }.prune_in(None, false).is_err());
        assert!(elsewhere.join("store").join(MARKER).exists());
        mode(&shared, 0o700);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_store_is_made_only_where_nothing_is_and_kept_only_with_a_valid_marker() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = |name: &str| {
            let dir = std::env::temp_dir().join(format!("romacs-store-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            dir
        };
        let private = |dir: &Path| {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        };
        // Nothing there, or an empty directory: made a store.
        let fresh = dir("fresh");
        private_store(&fresh).unwrap();
        assert_eq!(std::fs::read_to_string(fresh.join(MARKER)).unwrap(), MARKER_TEXT);
        let empty = dir("empty");
        private(&empty);
        private_store(&empty).unwrap();
        assert!(checked_store(&empty, None).is_ok());
        // A directory holding anything without the marker, even only names
        // a store would have: refused, untouched.
        let unmarked = dir("unmarked");
        private(&unmarked);
        std::fs::write(unmarked.join(format!("{}.notes", "a".repeat(64))), "mine").unwrap();
        assert!(private_store(&unmarked).is_err());
        assert!(!unmarked.join(MARKER).exists());
        assert!(reset_in(&unmarked, true, None).is_err());
        // A marker of another format: refused.
        let old = dir("version");
        private(&old);
        std::fs::write(old.join(MARKER), "romacs-world-store 0\n").unwrap();
        assert!(private_store(&old).unwrap_err().to_string().contains("format"));
        assert!(reset_in(&old, true, None).is_err());
        // A marker that is a symlink: refused, and its target never written.
        let linked = dir("symlinked");
        private(&linked);
        let target = dir("marker-target");
        std::fs::write(&target, MARKER_TEXT).unwrap();
        symlink(&target, linked.join(MARKER)).unwrap();
        assert!(private_store(&linked).is_err());
        assert!(reset_in(&linked, true, None).is_err());
        assert!(World { store: linked.clone(), key: [0; 32], recipe: vec![] }.prune_in(None, false).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), MARKER_TEXT);
        // A dangling one: never created through.
        let dangling = dir("dangling");
        private(&dangling);
        let nowhere = dir("nowhere");
        symlink(&nowhere, dangling.join(MARKER)).unwrap();
        assert!(private_store(&dangling).is_err());
        assert!(!nowhere.exists());
        for d in [fresh, empty, unmarked, old, linked, dangling] {
            let _ = std::fs::remove_dir_all(d);
        }
        let _ = std::fs::remove_file(target);
    }

    fn fifo(path: &Path) {
        let name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0, "{}", path.display());
    }

    /// Run F on a thread of its own: its result, unless it blocks.
    fn bounded<T: Send + 'static>(what: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || { let _ = send.send(f()); });
        receive.recv_timeout(std::time::Duration::from_secs(5)).unwrap_or_else(|_| panic!("{what} blocked"))
    }

    #[test]
    fn a_fifo_in_place_of_a_store_file_is_refused_without_blocking() {
        // As the marker: launch, reset and prune refuse the store at once.
        let marked = marked_store("fifo-marker");
        std::fs::remove_file(marked.join(MARKER)).unwrap();
        fifo(&marked.join(MARKER));
        let store = marked.clone();
        assert!(bounded("validating the store", move || private_store(&store)).is_err());
        let store = marked.clone();
        assert!(bounded("reset", move || reset_in(&store, true, None)).is_err());
        // As a world's files and the store lock: each open refuses it.
        let dir = marked_store("fifo-files");
        let world = World { store: dir.clone(), key: [0xdd; 32], recipe: vec![] };
        for suffix in ["json", "lock", "use", "failed", "rwi", "log"] {
            fifo(&world.file(&format!(".{suffix}")));
        }
        let w = world.clone();
        assert!(bounded("the manifest", move || w.valid_manifest()).is_err());
        let w = world.clone();
        assert!(bounded("the image", move || w.checked_image(&json!({"image": "00"}))).is_err());
        // As the digest cache's entry for a real file: not read, and the
        // digest made from the file itself.
        let input = dir.join("input");
        std::fs::write(&input, "bytes").unwrap();
        let meta = std::fs::metadata(&input).unwrap();
        std::fs::create_dir_all(dir.join("ids")).unwrap();
        fifo(&dir.join("ids").join(identity_name(&meta)));
        let store = dir.clone();
        let digest = bounded("the digest cache", move || file_digest(&store, &std::fs::File::open(input).unwrap())).unwrap();
        assert_eq!(digest, tree_digest(b"bytes"));
        let w = world.clone();
        assert!(bounded("the world's lock", move || w.try_lock()).is_err());
        let w = world.clone();
        bounded("the use file", move || w.mark_use());
        let w = world.clone();
        assert!(bounded("the failure record", move || w.failures()).is_none());
        let w = world.clone();
        assert!(bounded("the builder's log", move || w.start_builder(vec![], tempfile_lock(&w))).is_err());
        let w = world.clone();
        bounded("pruning", move || w.prune_in(None, false)).unwrap();
        let store = dir.clone();
        bounded("reset", move || reset_in(&store, true, None)).unwrap();
        std::fs::remove_file(dir.join(STORE_LOCK)).unwrap();
        fifo(&dir.join(STORE_LOCK));
        let store = dir.clone();
        assert!(bounded("the store lock", move || lock_store(&store, libc::LOCK_SH)).is_err());
        let _ = std::fs::remove_dir_all(marked);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// `content_digest` of BYTES, made sequentially from its definition.
    fn tree_digest(bytes: &[u8]) -> [u8; 32] {
        let mut tree = (bytes.len() as u64).to_le_bytes().to_vec();
        for chunk in bytes.chunks(DIGEST_CHUNK as usize) {
            tree.extend((chunk.len() as u64).to_le_bytes());
            tree.extend(sha256(chunk));
        }
        sha256(&tree)
    }

    fn digest_of(path: &Path) -> std::io::Result<[u8; 32]> {
        let file = std::fs::File::open(path)?;
        content_digest(&file, file.metadata()?.len())
    }

    #[test]
    fn an_image_digest_binds_every_byte_and_the_length() {
        let dir = std::env::temp_dir().join(format!("romacs-digest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("image");
        let chunk = DIGEST_CHUNK as usize;
        let bytes: Vec<u8> = (0..3 * chunk + 17).map(|i| (i * 31 % 251) as u8).collect();
        for len in [0, 1, chunk - 1, chunk, chunk + 1, bytes.len()] {
            std::fs::write(&path, &bytes[..len]).unwrap();
            assert_eq!(digest_of(&path).unwrap(), tree_digest(&bytes[..len]), "{len} bytes");
        }
        let original = tree_digest(&bytes);
        let changed = |at: usize| {
            let mut bytes = bytes.clone();
            bytes[at] ^= 1;
            std::fs::write(&path, &bytes).unwrap();
            digest_of(&path).unwrap()
        };
        for at in [0, chunk - 1, chunk, 2 * chunk + 5, bytes.len() - 1] {
            assert_ne!(changed(at), original, "a flipped byte at {at}");
        }
        std::fs::write(&path, &bytes).unwrap();
        let file = std::fs::OpenOptions::new().read(true).write(true).open(&path).unwrap();
        file.set_len(bytes.len() as u64 - 1).unwrap();
        assert!(content_digest(&file, bytes.len() as u64).is_err(), "a file shorter than its length");
        let mut extended = bytes.clone();
        extended.extend([0; 4096]);
        std::fs::write(&path, &extended).unwrap();
        assert_ne!(digest_of(&path).unwrap(), original, "a page more of zeros");
        // A sparse file hashes as the zeros its hole reads as, and a write
        // into the hole changes it.
        let sparse = dir.join("sparse");
        let file = std::fs::File::create(&sparse).unwrap();
        file.set_len(2 * DIGEST_CHUNK + 9).unwrap();
        {
            use std::os::unix::fs::FileExt;
            file.write_all_at(b"head", 0).unwrap();
        }
        let mut dense = vec![0; 2 * chunk + 9];
        dense[..4].copy_from_slice(b"head");
        assert_eq!(digest_of(&sparse).unwrap(), tree_digest(&dense));
        {
            use std::os::unix::fs::FileExt;
            file.write_all_at(b"x", DIGEST_CHUNK + 3).unwrap();
        }
        assert_ne!(digest_of(&sparse).unwrap(), tree_digest(&dense), "a write into the hole");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A lock file to hand `start_builder`, which never gets to use it.
    fn tempfile_lock(world: &World) -> std::fs::File {
        std::fs::File::create(world.store.join("handed.lock")).unwrap()
    }

    fn stored(key: &str, days_ago: u64, mib: u64, busy: bool) -> Stored {
        Stored { key: key.repeat(64 / key.len()), family: String::new(), used: 100 * 86400 - days_ago * 86400, bytes: mib << 20, busy, refusal: false }
    }

    #[test]
    fn pruning_keeps_the_current_and_the_held_and_bounds_the_rest() {
        let now = 100 * 86400;
        let key = |k: &str| k.repeat(64 / k.len());
        // Unused for over a week: gone. Within it: the most recent kept, up
        // to three worlds in all.
        let worlds = [stored("a", 0, 300, false), stored("b", 1, 300, false), stored("c", 2, 300, false),
                      stored("d", 3, 300, false), stored("e", 8, 300, false)];
        assert_eq!(prune_plan(&worlds, &key("a"), now), [key("d"), key("e")]);
        // The current world and held ones stay, whatever their age or the
        // cap; they count toward it.
        let worlds = [stored("a", 30, 300, false), stored("b", 20, 300, true), stored("c", 1, 300, false), stored("d", 0, 300, false)];
        assert_eq!(prune_plan(&worlds, &key("a"), now), [key("c")]);
        // Four GiB at most: the older of two large worlds goes.
        let worlds = [stored("a", 0, 1024, false), stored("b", 1, 2048, false), stored("c", 2, 2048, false)];
        assert_eq!(prune_plan(&worlds, &key("a"), now), [key("c")]);
        // Each family is bounded alone: three batch worlds beside three of
        // a GUI's, whichever family the current world is of.
        let batch = |k, days| Stored { family: "launch batch".into(), ..stored(k, days, 300, false) };
        let worlds = [stored("a", 0, 300, false), stored("b", 1, 300, false), stored("c", 2, 300, false),
                      batch("d", 0), batch("e", 1), batch("f", 2), batch("g", 3)];
        assert_eq!(prune_plan(&worlds, &key("a"), now), [key("g")]);
        assert_eq!(prune_plan(&worlds, &key("g"), now), [key("f")]);
        // A refusal within a week stays beside three worlds and counts toward
        // nothing; an older one goes.
        let refusal = |k, days| Stored { refusal: true, ..stored(k, days, 1, false) };
        let worlds = [stored("a", 0, 300, false), stored("b", 1, 300, false), stored("c", 2, 300, false),
                      refusal("d", 0), refusal("e", 8)];
        assert_eq!(prune_plan(&worlds, &key("a"), now), [key("e")]);
    }

    #[test]
    fn pruning_deletes_an_old_worlds_files_and_spares_one_in_use() {
        use std::os::fd::AsRawFd;
        let dir = marked_store("prune");
        std::fs::create_dir_all(dir.join("ids")).unwrap();
        let current = World { store: dir.clone(), key: [0xaa; 32], recipe: vec![] };
        let (old, held) = ("b".repeat(64), "c".repeat(64));
        let week_ago = host::clock() - std::time::Duration::from_secs(8 * 86400);
        for key in [hex(&current.key), old.clone(), held.clone()] {
            for suffix in ["rwi", "json", "log", "lock", "use", "failed"] {
                let file = std::fs::File::create(dir.join(format!("{key}.{suffix}"))).unwrap();
                std::io::Write::write_all(&mut &file, &[1; 8192]).unwrap();
                if key != hex(&current.key) { file.set_modified(week_ago).unwrap(); }
            }
        }
        let stale_id = dir.join("ids/1-2-3-4.5-6.7");
        std::fs::write(&stale_id, "00").unwrap();
        std::fs::File::options().write(true).open(&stale_id).unwrap().set_modified(week_ago).unwrap();
        // A restored process maps HELD: it holds its use lock shared.
        let in_use = std::fs::File::open(dir.join(format!("{held}.use"))).unwrap();
        assert_eq!(unsafe { libc::flock(in_use.as_raw_fd(), libc::LOCK_SH) }, 0);
        assert_eq!(current.prune().unwrap(), [old.clone()]);
        let names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert!(!names.iter().any(|n| n.starts_with(&old)), "{names:?}");
        assert_eq!(names.iter().filter(|n| n.starts_with(&held)).count(), 6);
        assert_eq!(names.iter().filter(|n| n.starts_with(&hex(&current.key))).count(), 6);
        assert!(!stale_id.exists(), "an old identity entry no kept file has");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_world_being_built_counts_in_its_familys_bound() {
        let dir = marked_store("prune-family");
        let recipe = |launch: &str, more: &str| vec!["consumer t".to_owned(), format!("launch {launch}"), more.to_owned()];
        // The world a builder builds: its lock and log, no manifest yet.
        let current = World { store: dir.clone(), key: [0xaa; 32], recipe: recipe("batch", "environment C") };
        for suffix in ["lock", "log"] { std::fs::write(dir.join(format!("{}.{suffix}", hex(&current.key))), "").unwrap(); }
        // Three batch worlds of other locales, the oldest first, and a GUI's.
        let now = host::clock();
        let worlds = [("b", "batch", 3), ("c", "batch", 2), ("d", "batch", 1), ("e", "gui", 4), ("f", "gui", 9 * 24)];
        for (key, launch, hours) in worlds {
            let manifest = json!({"recipe": recipe(launch, key)}).to_string();
            for (suffix, text) in [("json", manifest.as_str()), ("rwi", "image")] {
                let file = std::fs::File::create(dir.join(format!("{}.{suffix}", key.repeat(64)))).unwrap();
                std::io::Write::write_all(&mut &file, text.as_bytes()).unwrap();
                file.set_modified(now - std::time::Duration::from_secs(hours * 3600)).unwrap();
            }
        }
        // A batch builder's prune leaves the GUI's family alone, even a
        // world of it unused for over a week, which `prune` deletes.
        assert_eq!(current.prune_family().unwrap(), ["b".repeat(64)]);
        assert_eq!(current.prune().unwrap(), ["f".repeat(64)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_library_the_loader_patched_is_known_by_where_it_loaded() {
        let dir = std::env::temp_dir().join(format!("romacs-library-textrel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A pointer in read-only data: the loader relocates it in place.
        let asm = dir.join("textrel.s");
        std::fs::write(&asm, "\t.section .rodata,\"a\"\n\t.globl romacs_table\n\t.p2align 3\nromacs_table:\n\t.quad romacs_string\nromacs_string:\n\t.asciz \"one\"\n").unwrap();
        let first = dir.join("textrel-one.so");
        let status = std::process::Command::new("cc").args(["-shared", "-fPIC", "-Wl,-z,notext", "-o"]).arg(&first).arg(&asm).status().unwrap();
        assert!(status.success());
        let second = dir.join("textrel-two.so");
        std::fs::copy(&first, &second).unwrap();
        let (a, b) = (load(&first), load(&second));
        assert!(a.text_relocations() && b.text_relocations());
        assert!(!load(&build(&dir, "plain", SOURCE, &["-DCONSTANT=\"PLAIN\""])).text_relocations());
        // The same file, loaded at two addresses, hashes differently: a
        // world keyed on it is not found again, and launches boot from source.
        assert_ne!(library_content(&a).unwrap(), library_content(&b).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    const SOURCE: &str = "const char romacs_constant[] = CONSTANT;\nconst char *romacs_value(void) { return romacs_constant; }\n";

    #[test]
    fn a_library_is_known_by_its_code_and_constants_not_its_build_id_alone() {
        let dir = std::env::temp_dir().join(format!("romacs-library-identity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // One build, and a copy whose constant was patched after linking:
        // the build-id note is the same.
        let original = build(&dir, "noted", SOURCE, &["-DCONSTANT=\"ORIGINAL_CONSTANT_0123\"", "-Wl,--build-id=sha1"]);
        let mut bytes = std::fs::read(&original).unwrap();
        let at = bytes.windows(22).position(|w| w == b"ORIGINAL_CONSTANT_0123").unwrap();
        bytes[at..at + 22].copy_from_slice(b"PATCHED!_CONSTANT_0123");
        let patched = dir.join("patched.so");
        std::fs::write(&patched, bytes).unwrap();
        let (a, b) = (load(&original), load(&patched));
        assert!(a.build_id().is_some());
        assert_eq!(a.build_id(), b.build_id(), "the note survives the patch");
        let (ca, cb) = (library_content(&a).unwrap(), library_content(&b).unwrap());
        assert_eq!(ca.0, cb.0);
        assert_ne!(ca.1, cb.1, "the constants differ");

        // Two builds with no build-id, the same code and other constants.
        let one = build(&dir, "plain-one", SOURCE, &["-DCONSTANT=\"FIRST_CONSTANT_ABCDEF\"", "-Wl,--build-id=none"]);
        let two = build(&dir, "plain-two", SOURCE, &["-DCONSTANT=\"OTHER_CONSTANT_ABCDEF\"", "-Wl,--build-id=none"]);
        let (p1, p2) = (load(&one), load(&two));
        assert!(p1.build_id().is_none() && p2.build_id().is_none());
        assert_eq!(code_only(&p1, &one), code_only(&p2, &two), "their code is the same");
        assert_ne!(library_content(&p1).unwrap(), library_content(&p2).unwrap());

        // Hashed again, alike: the mapping, not a record of it.
        assert_eq!(library_content(&a).unwrap(), ca);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
