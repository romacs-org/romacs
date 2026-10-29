//! The world store and the launch protocol (DESIGN-M1 §5, §7, §9, as the
//! addendum revises them). A world is two files in a private store, named
//! by its key: `KEY.rwi`, the arena image, and `KEY.json`, its manifest,
//! with the keyed host records the build consumed and the events the
//! embedder printed while it built.
//!
//! The key binds the executable's content, the libraries it loads
//! (`arena::libraries`) and the embedder's recipe. The records bind every
//! host fact the build read; a launch asks the host each of them again and
//! boots from source on any difference. The manifest binds the image's
//! content by its SHA-256, which a launch checks against the file it maps,
//! hashed once and then known by its identity.
//!
//! A launch is one of two processes:
//! - the launcher, with its world valid, maps the image and allocates from
//!   it from then on (`arena::adopt`), and takes the VM
//!   (`Launch::Restored`);
//! - with no valid world, the launcher boots from source and starts a
//!   builder beside itself, in the pinned environment, unless one runs
//!   (`Launch::Chain`). The builder runs the recipe in build mode
//!   (`Launch::Build`), then `finish_build` publishes the world.
use super::{arena, image};
use crate::{Vm, host};
use serde_json::{Value as Json, json};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "romacs-world 2";
/// Set in a builder: the store it publishes to.
const BUILD_ENV: &str = "ROMACS_WORLD_BUILD";

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
    use std::os::unix::fs::MetadataExt;
    let meta = host::file_metadata(file)?;
    let name = format!("{}-{}-{}-{}.{}-{}.{}", meta.dev(), meta.ino(), meta.size(),
                       meta.mtime(), meta.mtime_nsec(), meta.ctime(), meta.ctime_nsec());
    let cached = store.join("ids").join(name);
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

/// The store directory: made private when new, and refused unless this
/// user owns it and no one else may enter (§9.2).
fn private_store(store: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    if !host::probe(host::Probe::IsDir, store) {
        host::create_dir_all(store)?;
        let path = std::ffi::CString::new(store.as_os_str().as_encoded_bytes()).map_err(std::io::Error::other)?;
        host::chmod(&path, 0o700, false)?;
    }
    let meta = host::metadata(store)?;
    if meta.uid() != host::euid() || meta.permissions().mode() & 0o077 != 0 {
        return Err(std::io::Error::other(format!("world store {} is not private", store.display())));
    }
    Ok(())
}

impl World {
    /// RECIPE's world in STORE: the key of this executable and RECIPE.
    pub fn new(store: PathBuf, recipe: Vec<String>) -> std::io::Result<World> {
        private_store(&store)?;
        let executable = file_digest(&store, &host::open_world_file("/proc/self/exe")?)?;
        let mut text = format!("{FORMAT}\nexecutable {}\nlibraries {}\n", hex(&executable), hex(&arena::libraries()));
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
    /// built first, and the launch then boots from it.
    pub fn launch<T: 'static>(&self, builder_args: impl FnOnce() -> Vec<OsString>, wait: bool) -> Launch<T> {
        if building() {
            host::unset_env(BUILD_ENV);
            return Launch::Build;
        }
        if host::getenv("ROMACS_WORLD").is_some_and(|v| v == "off") {
            return Launch::Chain { reason: json!("ROMACS_WORLD=off") };
        }
        if !arena::active() {
            return Launch::Chain { reason: json!("no arena") };
        }
        let mut builder_args = Some(builder_args);
        let mut args = move || builder_args.take().map(|f| f()).unwrap_or_default();
        let mut valid = self.valid_manifest();
        if let (Err(_), true) = (&valid, wait) {
            let built = self.start_builder(args()).and_then(|mut child| child.wait());
            valid = match built {
                Ok(status) if status.success() => self.valid_manifest(),
                Ok(status) => Err(json!({"builder": status.to_string()})),
                Err(e) => Err(json!({"builder": e.to_string()})),
            };
            if let Err(reason) = valid {
                return Launch::Chain { reason: json!({"world": reason}) };
            }
        }
        match valid.and_then(|manifest| Ok((self.checked_image(&manifest)?, manifest))) {
            Ok((image, manifest)) => match arena::adopt(&image) {
                arena::Restore::Mapped => match image::restore::<T>(&self.key) {
                    Some((vm, carry)) => Launch::Restored { vm: Box::new(vm), carry, manifest },
                    None => Launch::Chain { reason: json!({"image": "made for another key"}) },
                },
                refused => Launch::Chain { reason: json!({"image": format!("{refused:?}")}) },
            },
            Err(reason) => {
                let started = self.start_builder(args()).map(|child| {
                    let pid = child.id();
                    // Reap it, whoever outlives whom.
                    std::thread::spawn(move || { let mut child = child; let _ = child.wait(); });
                    pid
                });
                Launch::Chain { reason: json!({"world": reason, "builder": started.map_err(|e| e.to_string()).ok()}) }
            }
        }
    }

    /// Start a builder with ARGS unless one runs: detached, in the pinned
    /// environment under the store, its output in `KEY.log`.
    fn start_builder(&self, args: Vec<OsString>) -> std::io::Result<std::process::Child> {
        use std::os::unix::process::CommandExt;
        if !self.lock()? {
            return Err(std::io::Error::other("a builder runs"));
        }
        let pinned = self.pinned_dir()?;
        let log = host::open_output(self.file(".log"), host::OpenOptions::new().write(true).create(true).truncate(true))?;
        let mut env = pinned_env(&host::env_snapshot(), &pinned);
        env.push((BUILD_ENV.into(), self.store.clone().into_os_string()));
        host::command(host::current_exe()?)?
            .args(args).env_clear().envs(env).current_dir(pinned.join("cwd"))
            .stdin(std::process::Stdio::null()).stdout(log.try_clone()?).stderr(log)
            .process_group(0).spawn()
    }

    /// Whether no builder holds this world's lock. A builder takes it for
    /// its life (`hold_lock`).
    fn lock(&self) -> std::io::Result<bool> {
        let file = host::open_output(self.file(".lock"), host::OpenOptions::new().write(true).create(true))?;
        use std::os::fd::AsRawFd;
        let free = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
        Ok(free)
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
        let lock = host::open_output(self.file(".lock"), host::OpenOptions::new().write(true).create(true))?;
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(std::io::Error::other("another builder holds the lock"));
        }
        std::mem::forget(lock);
        super::sandbox::confine_publishing(roots, &self.store)?;
        let seed = self.key[..4].try_into().expect("4 bytes");
        host::enter_build(host::Pinned { clock: host::clock(), pid: 4242, seed });
        Ok(())
    }

    /// The builder's end: check what startup read, then publish VM, with
    /// CARRY beside it, as this world's image, and EVENTS, what the recipe
    /// printed, in its manifest. A build with any refusal publishes its
    /// manifest only, so launches boot from source until its records
    /// change. The image's length.
    pub fn finish_build<T: 'static>(&self, mut vm: Vm, carry: T, events: Json) -> Result<usize, String> {
        vm.world_check_inputs();
        let trace = host::leave_build().ok_or("no world build was recording")?;
        let records: Vec<&Json> = trace["records"].as_array().map(|r| r.iter().filter(|r| host::keyed(r)).collect()).unwrap_or_default();
        let manifest = json!({"format": FORMAT, "key": hex(&self.key), "recipe": self.recipe,
                              "records": records, "refusals": trace["refusals"], "events": events,
                              "built": host::clock().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())});
        let publish = |suffix: &str, bytes: &[u8]| -> std::io::Result<()> {
            let temporary = self.file(&format!("{suffix}.new"));
            host::write(&temporary, bytes)?;
            read_only(&temporary)?;
            host::rename(&temporary, self.file(suffix))
        };
        let text = |manifest: &Json| serde_json::to_vec(manifest).map_err(|e| e.to_string());
        if trace["refusals"].as_array().is_some_and(|r| !r.is_empty()) {
            publish(".json", &text(&manifest)?).map_err(|e| e.to_string())?;
            return Err(format!("world build refused: {}", trace["refusals"]));
        }
        vm.world_lift();
        vm.world_normalize();
        vm.collect_garbage();
        let temporary = self.file(".rwi.new");
        let file = host::open_output(&temporary, host::OpenOptions::new().write(true).create(true).truncate(true))
            .map_err(|e| e.to_string())?;
        let len = image::capture(vm, carry, &file, self.key)?;
        file.sync_all().map_err(|e| e.to_string())?;
        read_only(&temporary).map_err(|e| e.to_string())?;
        host::rename(&temporary, self.file(".rwi")).map_err(|e| e.to_string())?;
        // Hashed from the file as published, which also caches it.
        let image = host::open_world_file(self.file(".rwi")).map_err(|e| e.to_string())?;
        let digest = file_digest(&self.store, &image).map_err(|e| e.to_string())?;
        let mut manifest = manifest;
        manifest["image"] = json!(hex(&digest));
        publish(".json", &text(&manifest)?).map_err(|e| e.to_string())?;
        Ok(len)
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
