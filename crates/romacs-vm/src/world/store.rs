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
//! content by its SHA-256, which a launch checks against the file it maps,
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
//! with the time: what `prune` keeps beside the current world.
use super::{arena, image};
use crate::{Vm, host};
use serde_json::{Value as Json, json};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "romacs-world 2";
/// Set in a builder: the store it publishes to.
const BUILD_ENV: &str = "ROMACS_WORLD_BUILD";
/// Set in a builder: the descriptor of the world's lock, which its
/// launcher took and handed on.
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256(data: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(data).into()
}

/// The content digest of FILE, cached in STORE by the file's identity and
/// change times: hashing tens or hundreds of megabytes at every launch
/// would cost more than the rest of a restore. A write, a truncation or a
/// change of mode changes the identity.
fn file_digest(store: &Path, file: &std::fs::File) -> std::io::Result<[u8; 32]> {
    use sha2::Digest;
    use std::io::Read;
    let meta = host::file_metadata(file)?;
    let cached = store.join("ids").join(identity_name(&meta));
    if let Ok(text) = host::read_to_string(&cached)
        && let Some(digest) = (0..32).map(|i| u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()).collect::<Option<Vec<u8>>>()
    {
        return Ok(digest.try_into().expect("32 bytes"));
    }
    let mut digest = sha2::Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut reader = file;
    loop {
        match reader.read(&mut buffer)? {
            0 => break,
            n => digest.update(&buffer[..n]),
        }
    }
    let digest: [u8; 32] = digest.finalize().into();
    let _ = host::create_dir_all(store.join("ids")).and_then(|()| host::write(&cached, hex(&digest)));
    Ok(digest)
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

/// The store directory: made private and marked when new, and refused
/// unless this user owns it, no one else may enter (§9.2), it is no
/// symlink, and it is not $HOME or `/`. A store an earlier Romacs made has
/// no marker; it is adopted when everything in it is a store's.
fn private_store(store: &Path) -> std::io::Result<()> {
    if host::symlink_metadata(store).is_err() {
        host::create_dir_all(store)?;
        let path = std::ffi::CString::new(store.as_os_str().as_encoded_bytes()).map_err(std::io::Error::other)?;
        host::chmod(&path, 0o700, false)?;
    }
    owned_store(store, host::getenv("HOME").map(PathBuf::from).as_deref())?;
    if !marked(store) {
        if let Some(name) = foreign_entry(store)? {
            return Err(std::io::Error::other(format!("{} is no world store: it holds {name}", store.display())));
        }
        host::write(store.join(MARKER), MARKER_TEXT)?;
    }
    Ok(())
}

/// Refuse STORE unless it is a directory, no symlink, that this user owns
/// and no one else may enter, and neither HOME nor `/`.
fn owned_store(store: &Path, home: Option<&Path>) -> std::io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let refuse = |why: &str| Err(std::io::Error::other(format!("world store {}: {why}", store.display())));
    let meta = host::symlink_metadata(store)?;
    if !meta.file_type().is_dir() {
        return refuse("not a directory");
    }
    if meta.uid() != host::euid() || meta.permissions().mode() & 0o777 != 0o700 {
        return refuse("not private to this user (0700)");
    }
    let canonical = host::canonicalize(store)?;
    if canonical == Path::new("/") || home.and_then(|h| host::canonicalize(h).ok()).is_some_and(|h| h == canonical) {
        return refuse("the home or root directory");
    }
    Ok(())
}

/// Whether STORE holds Romacs's marker, a regular file with its format.
fn marked(store: &Path) -> bool {
    host::symlink_metadata(store.join(MARKER)).is_ok_and(|m| m.file_type().is_file())
        && host::read_to_string(store.join(MARKER)).is_ok_and(|t| t == MARKER_TEXT)
}

/// The first entry of STORE no store has, if any.
fn foreign_entry(store: &Path) -> std::io::Result<Option<String>> {
    for entry in host::read_dir(store)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if world_file(&name).is_none() && !["ids", "p", MARKER, STORE_LOCK].contains(&name.as_str()) {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

/// STORE, checked as `reset` and `prune` need it: owned, private, no
/// symlink, not HOME or `/` (`owned_store`), and marked as a store.
fn checked_store(store: &Path, home: Option<&Path>) -> std::io::Result<()> {
    owned_store(store, home)?;
    if !marked(store) {
        return Err(std::io::Error::other(format!("{} has no {MARKER}: not touched", store.display())));
    }
    Ok(())
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

    fn file(&self, suffix: &str) -> PathBuf {
        self.store.join(format!("{}{suffix}", hex(&self.key)))
    }

    /// The manifest, when its records still hold: every keyed fact the build
    /// read, asked again, gives the same answer.
    fn valid_manifest(&self) -> Result<Json, Json> {
        let text = host::read_to_string(self.file(".json")).map_err(|e| json!({"missing": e.to_string()}))?;
        let manifest: Json = serde_json::from_str(&text).map_err(|e| json!({"manifest": e.to_string()}))?;
        let case = crate::prim::prims::savejourney::filesystem_case_insensitive;
        for record in manifest["records"].as_array().map(Vec::as_slice).unwrap_or_default() {
            let mut expected = record.clone();
            if let Some(fields) = expected.as_object_mut() { fields.remove("site"); }
            match host::replay(record, case) {
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
        let image = host::open_world_file(self.file(".rwi")).map_err(|e| json!({"missing": e.to_string()}))?;
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
        match arena::adopt(&image) {
            arena::Restore::Mapped => match image::restore::<T>(&self.key) {
                Some((vm, carry)) => {
                    self.mark_use();
                    Launch::Restored { vm: Box::new(vm), carry, manifest }
                }
                None => Launch::Chain { reason: json!({"image": "made for another key"}) },
            },
            refused => Launch::Chain { reason: json!({"image": format!("{refused:?}")}) },
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
        let file = host::open_output(self.file(".lock"), host::OpenOptions::new().write(true).create(true))?;
        Ok((unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0).then_some(file))
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

    /// Start a builder with ARGS, handing it LOCK: in the pinned environment
    /// under the store, its output in `KEY.log`.
    fn start_builder(&self, args: Vec<OsString>, lock: std::fs::File) -> std::io::Result<std::process::Child> {
        use std::os::fd::AsRawFd;
        use std::os::unix::process::CommandExt;
        let pinned = self.pinned_dir()?;
        let log = host::open_output(self.file(".log"), host::OpenOptions::new().write(true).create(true).truncate(true))?;
        let mut env = pinned_env(&host::env_snapshot(), &pinned);
        env.push((BUILD_ENV.into(), self.store.clone().into_os_string()));
        let fd = lock.as_raw_fd();
        env.push((LOCK_ENV.into(), fd.to_string().into()));
        let mut command = host::command(host::current_exe()?)?;
        command.args(args).env_clear().envs(env).current_dir(pinned.join("cwd"))
            .stdin(std::process::Stdio::null()).stdout(log.try_clone()?).stderr(log).process_group(0);
        // The lock's descriptor stays open across the exec: the builder holds
        // the lock this process took, and this one's copy closes after.
        unsafe { command.pre_exec(move || if libc::fcntl(fd, libc::F_SETFD, 0) == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }) };
        let child = command.spawn();
        drop(lock);
        child
    }

    /// The transient failures recorded since the last success: how many,
    /// and when the last one was (seconds since the epoch).
    fn failures(&self) -> Option<(u64, u64)> {
        let text = host::read_to_string(self.file(".failed")).ok()?;
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
        if let Ok(file) = host::open_output(self.file(".use"), host::OpenOptions::new().write(true).create(true)) {
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
        host::create_dir_all(&p)?;
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
        let lock = match handed {
            Some(fd) => unsafe { std::fs::File::from_raw_fd(fd) },
            None => host::open_output(self.file(".lock"), host::OpenOptions::new().write(true).create(true))?,
        };
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(std::io::Error::other("another builder holds the lock"));
        }
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
        vm.collect_garbage();
        let (temporary, file) = self.temporary(".rwi").map_err(|e| e.to_string())?;
        let len = image::capture(vm, carry, &file, self.key)?;
        file.sync_all().map_err(|e| e.to_string())?;
        read_only(&temporary).map_err(|e| e.to_string())?;
        host::rename(&temporary, self.file(".rwi")).map_err(|e| e.to_string())?;
        // Hashed from the file as published, which also caches it.
        let image = host::open_world_file(self.file(".rwi")).map_err(|e| e.to_string())?;
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
    /// When it was last used (seconds since the epoch).
    used: u64,
    /// Its files' size on disk.
    bytes: u64,
    /// A builder or a restored process holds it.
    busy: bool,
}

/// The worlds of WORLDS to delete, CURRENT's kept: those a builder or a
/// restored process holds are kept too, and the others, most recently used
/// first, while they were used within `UNUSED` and the store stays within
/// `MOST_WORLDS` worlds and `MOST_BYTES` bytes.
fn prune_plan(worlds: &[Stored], current: &str, now: u64) -> Vec<String> {
    let kept = |w: &&Stored| w.key == current || w.busy;
    let (mut count, mut bytes) = worlds.iter().filter(kept).fold((0, 0), |(c, b), w| (c + 1, b + w.bytes));
    let mut others: Vec<&Stored> = worlds.iter().filter(|w| !kept(w)).collect();
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
/// every world and the digest cache and pinned table too, leaving the
/// marker and the store lock. Only a marked store is touched
/// (`checked_store`), and in it only regular files and real directories
/// with a store's names: no symlink is followed. How many it removed.
pub fn reset(store: &Path, everything: bool) -> std::io::Result<usize> {
    reset_in(store, everything, host::getenv("HOME").map(PathBuf::from).as_deref())
}

fn reset_in(store: &Path, everything: bool, home: Option<&Path>) -> std::io::Result<usize> {
    if host::symlink_metadata(store).is_err() {
        return Ok(0);
    }
    checked_store(store, home)?;
    let mut removed = 0;
    for entry in host::read_dir(store)? {
        let name = entry?.file_name();
        let Some(name) = name.to_str() else { continue };
        let path = store.join(name);
        let refused = || host::read_to_string(&path).ok()
            .and_then(|text| serde_json::from_str::<Json>(&text).ok())
            .is_some_and(|m| m["refusals"].as_array().is_some_and(|r| !r.is_empty()));
        match world_file(name) {
            Some((_, suffix)) if regular(&path) && (everything || suffix == "failed" || (suffix == "json" && refused())) => {
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
    }
    Ok(removed)
}

impl World {
    /// Bound the store: delete the worlds `prune_plan` gives up, holding
    /// each one's locks while its files go, and the digest cache's entries
    /// for files no kept world or executable is, once they are a week old.
    /// The worlds deleted.
    pub fn prune(&self) -> std::io::Result<Vec<String>> {
        self.prune_in(host::getenv("HOME").map(PathBuf::from).as_deref())
    }

    fn prune_in(&self, home: Option<&Path>) -> std::io::Result<Vec<String>> {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::MetadataExt;
        checked_store(&self.store, home)?;
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
            host::open_output(&path, host::OpenOptions::new().write(true))
                .is_ok_and(|file| (unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }) != 0)
        };
        let worlds: Vec<Stored> = files.iter().map(|(key, files)| Stored {
            key: key.clone(),
            used: files.iter().filter(|(n, _)| [".use", ".json", ".rwi"].iter().any(|s| n.ends_with(s))).map(|(_, m)| m.mtime().max(0) as u64).max().unwrap_or(0),
            bytes: files.iter().map(|(_, m)| m.blocks() * 512).sum(),
            busy: held(self.store.join(format!("{key}.lock"))) || held(self.store.join(format!("{key}.use"))),
        }).collect();
        let current = hex(&self.key);
        let mut deleted = vec![];
        for key in prune_plan(&worlds, &current, now_seconds()) {
            // Taken now, these say no builder or restored process came since.
            let locks: Vec<std::fs::File> = ["lock", "use"].iter()
                .filter_map(|s| host::open_output(self.store.join(format!("{key}.{s}")), host::OpenOptions::new().write(true)).ok())
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
        assert!(world(&plain).prune_in(None).is_err());
        assert!(plain.join(format!("{key}.failed")).exists());
        // Marked, but open to others: refused.
        let open = marked_store("open");
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(reset_in(&open, true, None).is_err());
        // The home directory, or the root, even marked: refused.
        let home = marked_store("home");
        assert!(reset_in(&home, true, Some(&home)).is_err());
        assert!(world(&home).prune_in(Some(&home)).is_err());
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
        world(&store).prune_in(None).unwrap();
        assert_eq!(std::fs::read_to_string(outside.join("precious")).unwrap(), "mine");
        for dir in [plain, open, home, outside, store] {
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn a_store_an_earlier_romacs_made_is_adopted_and_a_foreign_directory_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("romacs-store-{}-adopt", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("ids")).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(dir.join(format!("{}.json", "c".repeat(64))), "{}").unwrap();
        private_store(&dir).unwrap();
        assert!(marked(&dir));
        std::fs::remove_file(dir.join(MARKER)).unwrap();
        std::fs::write(dir.join("notes.txt"), "mine").unwrap();
        assert!(private_store(&dir).is_err(), "a directory with other files is no store");
        assert!(!marked(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn stored(key: &str, days_ago: u64, mib: u64, busy: bool) -> Stored {
        Stored { key: key.repeat(64 / key.len()), used: 100 * 86400 - days_ago * 86400, bytes: mib << 20, busy }
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
