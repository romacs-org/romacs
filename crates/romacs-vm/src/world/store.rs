//! The world store and the launch protocol (DESIGN-M1 §5, §7, §9, as the
//! addendum revises them). A world is two files in a private store, named
//! by its key: `KEY.rwi`, the arena image, and `KEY.json`, its manifest,
//! with the keyed host records the build consumed and the events the
//! embedder printed while it built.
//!
//! The key binds the executable's content and the embedder's recipe. The
//! records bind every host fact the build read; a launch asks the host each
//! of them again and boots from source on any difference. The image binds
//! the executable mappings it needs (`arena::layout`).
//!
//! A launch is one of three processes:
//! - the launcher, with its world valid, runs itself again without address
//!   randomization, naming the image; it never returns;
//! - that process maps the image at its first allocation and takes the VM
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

pub const FORMAT: &str = "romacs-world 1";
/// Set in the process that maps an image: the image's path.
const IMAGE_ENV: &str = "ROMACS_WORLD_IMAGE";
/// Set in a builder: the store it publishes to.
const BUILD_ENV: &str = "ROMACS_WORLD_BUILD";

/// One recipe's world in a store.
pub struct World {
    pub store: PathBuf,
    pub key: [u8; 32],
    pub recipe: Vec<String>,
}

/// What this process does with its world.
pub enum Launch {
    /// The image is mapped: its VM, to reinitialize, and its manifest.
    Restored { vm: Box<Vm>, manifest: Json },
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

/// The content digest of this executable, cached in STORE by the file's
/// identity and change times: hashing tens of megabytes at every launch
/// would cost more than the rest of a restore.
fn executable_digest(store: &Path) -> std::io::Result<[u8; 32]> {
    use std::os::unix::fs::MetadataExt;
    let meta = host::metadata("/proc/self/exe")?;
    let name = format!("{}-{}-{}-{}.{}-{}.{}", meta.dev(), meta.ino(), meta.size(),
                       meta.mtime(), meta.mtime_nsec(), meta.ctime(), meta.ctime_nsec());
    let cached = store.join("ids").join(name);
    if let Ok(text) = host::read_to_string(&cached)
        && let Some(digest) = (0..32).map(|i| u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()).collect::<Option<Vec<u8>>>()
    {
        return Ok(digest.try_into().expect("32 bytes"));
    }
    let digest = sha256(&host::read_file("/proc/self/exe")?);
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
        let mut text = format!("{FORMAT}\nexecutable {}\n", hex(&executable_digest(&store)?));
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
        if !host::probe(host::Probe::IsFile, self.file(".rwi")) {
            return Err(json!({"missing": "image"}));
        }
        Ok(manifest)
    }

    /// What this process does: see the module documentation. BUILDER_ARGS
    /// are this process's arguments as a builder, which runs elsewhere, needs
    /// them: every path absolute.
    pub fn launch(&self, builder_args: impl FnOnce() -> Vec<OsString>) -> Launch {
        if host::getenv(IMAGE_ENV).is_some() {
            host::unset_env(IMAGE_ENV);
            return match image::restore(&self.key) {
                Some(vm) => {
                    let manifest = host::read_to_string(self.file(".json")).ok()
                        .and_then(|text| serde_json::from_str(&text).ok()).unwrap_or(Json::Null);
                    Launch::Restored { vm: Box::new(vm), manifest }
                }
                None => Launch::Chain { reason: json!({"image": format!("{:?}", arena::restore_outcome())}) },
            };
        }
        if host::getenv(BUILD_ENV).is_some() {
            host::unset_env(BUILD_ENV);
            return Launch::Build;
        }
        if host::getenv("ROMACS_WORLD").is_some_and(|v| v == "off") {
            return Launch::Chain { reason: json!("ROMACS_WORLD=off") };
        }
        if !arena::active() {
            return Launch::Chain { reason: json!("no arena") };
        }
        match self.valid_manifest() {
            Ok(_) => {
                let mut env = host::env_snapshot();
                env.push((IMAGE_ENV.into(), self.file(".rwi").into_os_string()));
                let args: Vec<OsString> = std::env::args_os().skip(1).collect();
                let error = host::exec_unrandomized(&args, &env);
                Launch::Chain { reason: json!({"exec": error.to_string()}) }
            }
            Err(reason) => {
                let started = self.start_builder(builder_args());
                Launch::Chain { reason: json!({"world": reason, "builder": started.map_err(|e| e.to_string()).ok()}) }
            }
        }
    }

    /// Start a builder with ARGS unless one runs: detached, in the pinned
    /// environment under the store, its output in `KEY.log`. The pid.
    fn start_builder(&self, args: Vec<OsString>) -> std::io::Result<u32> {
        use std::os::unix::process::CommandExt;
        if !self.lock()? {
            return Err(std::io::Error::other("a builder runs"));
        }
        let pinned = self.pinned_dir()?;
        let log = host::open_output(self.file(".log"), host::OpenOptions::new().write(true).create(true).truncate(true))?;
        let mut env = pinned_env(&host::env_snapshot(), &pinned);
        env.push((BUILD_ENV.into(), self.store.clone().into_os_string()));
        let child = host::unrandomized_command(host::current_exe()?)?
            .args(args).env_clear().envs(env).current_dir(pinned.join("cwd"))
            .stdin(std::process::Stdio::null()).stdout(log.try_clone()?).stderr(log)
            .process_group(0).spawn()?;
        let pid = child.id();
        // Reap it, whoever outlives whom.
        std::thread::spawn(move || { let mut child = child; let _ = child.wait(); });
        Ok(pid)
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

    /// The builder's end: check what startup read, then publish VM as this
    /// world's image with EVENTS, what the recipe printed, in its manifest.
    /// A build with any refusal publishes its manifest only, so launches
    /// boot from source until its records change. The image's length.
    pub fn finish_build(&self, mut vm: Vm, events: Json) -> Result<usize, String> {
        vm.world_check_inputs();
        let trace = host::leave_build().ok_or("no world build was recording")?;
        let records: Vec<&Json> = trace["records"].as_array().map(|r| r.iter().filter(|r| host::keyed(r)).collect()).unwrap_or_default();
        let manifest = json!({"format": FORMAT, "key": hex(&self.key), "recipe": self.recipe,
                              "records": records, "refusals": trace["refusals"], "events": events,
                              "built": host::clock().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())});
        let text = serde_json::to_vec(&manifest).map_err(|e| e.to_string())?;
        let publish = |suffix: &str, bytes: &[u8]| -> std::io::Result<()> {
            let temporary = self.file(&format!("{suffix}.new"));
            host::write(&temporary, bytes)?;
            host::rename(&temporary, self.file(suffix))
        };
        if trace["refusals"].as_array().is_some_and(|r| !r.is_empty()) {
            publish(".json", &text).map_err(|e| e.to_string())?;
            return Err(format!("world build refused: {}", trace["refusals"]));
        }
        vm.world_lift();
        vm.world_normalize();
        vm.collect_garbage();
        let temporary = self.file(".rwi.new");
        let file = host::open_output(&temporary, host::OpenOptions::new().write(true).create(true).truncate(true))
            .map_err(|e| e.to_string())?;
        let len = image::capture(vm, &file, self.key)?;
        file.sync_all().map_err(|e| e.to_string())?;
        host::rename(&temporary, self.file(".rwi")).map_err(|e| e.to_string())?;
        publish(".json", &text).map_err(|e| e.to_string())?;
        Ok(len)
    }
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
