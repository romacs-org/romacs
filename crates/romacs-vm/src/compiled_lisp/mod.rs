//! Compiled Lisp: Romacs's analogue of `.elc` files (design note
//! lazy-libraries.md). A load GNU would satisfy with compiled code runs
//! an artifact instead of reading, expanding and compiling the source:
//! the units a compile process made from the file, in order, under GNU's
//! compiled-load semantics (`Vm::load_artifact`).
//!
//! A compile process is a batch session over the base world that walks
//! the file by bytecomp's file-compilation rules (rlisp/romacs-compile.el),
//! compiles each form the walk emits with Romacs's compiler, and
//! publishes the artifact in the store (`store`). Compile-time evaluation
//! happens there, once per compile. Its host reads are recorded
//! (`host::Mode::Record`) with the files it loads; a load checks each
//! again with Rust host calls only (`host::replay`), never Lisp, and
//! compiles afresh when one changed.
//!
//! A compile runs in the compile environment the embedder gives
//! (`Config`): its own world, HOME and locale, the same in every session,
//! so what it makes of a file depends on the file and its observed inputs
//! only, and sessions under any HOME share it, as GNU's installed `.elc`
//! files are shared. That world's identity keys every artifact.
//!
//! Which loads are compiled is decided once per process: only when
//! ROMACS_COMPILED_LISP=on, the embedder configured a store (`configure`),
//! and this is no world build. Without a store every load reads source,
//! as a GNU installation without `.elc` files.
mod background;
mod codec;
mod tree;
pub(crate) mod store;
mod unit;
#[cfg(test)]
mod compiled_lisp_tests;

use crate::*;
pub(crate) use background::{note_idle, start as start_background, stop as stop_background};
pub use tree::compile_tree;
use serde_json::{Value as Json, json};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// What a process needs to load compiled Lisp.
pub struct Config {
    /// The compile command: a program and its leading arguments, to which
    /// the source file is appended.
    /// Empty when this process compiles nothing: it loads what is
    /// compiled already, and source otherwise.
    pub compiler: Vec<OsString>,
    /// A compile process's whole environment.
    pub environment: Vec<(OsString, OsString)>,
    /// The variables of ENVIRONMENT that come from the session starting
    /// the compile: a compile's reads of them are observed inputs. The
    /// others' values are the compile environment's own, bound by its
    /// identity.
    pub inherited: &'static [&'static str],
    /// A load with no artifact reads source and leaves the file to a
    /// background compile (`background`), instead of compiling it first:
    /// an interactive session's, where no load may wait for a compile.
    pub defer_misses: bool,
    /// The store.
    pub store: PathBuf,
    /// The compile environment's identity, the world a compile process
    /// restores: asked at the first compiled load, so a process that loads
    /// nothing pays nothing, and again while there is none. None: no
    /// compile world yet; a load compiles, which makes one, or reads
    /// source.
    pub identity: Box<dyn Fn() -> Option<String> + Send + Sync>,
    /// The installation's Lisp trees, whose files have a compiled form as
    /// GNU's installed `.elc` files do.
    pub runtime: Vec<PathBuf>,
}

/// The configuration, with what this process read of its environment
/// when configured: before a compile process records its inputs, so the
/// machinery's own reads are none of them.
struct Configured {
    config: Config,
    chain: String,
    trace: bool,
    /// A compile of a tree's (`tree`): its publications leave the store's
    /// maintenance to the tree's end.
    no_prune: bool,
    identity: std::sync::OnceLock<String>,
}

impl Configured {
    fn identity(&self) -> Option<&str> {
        if let Some(identity) = self.identity.get() {
            return Some(identity);
        }
        let identity = (self.config.identity)()?;
        Some(self.identity.get_or_init(|| identity))
    }
}

static CONFIG: std::sync::OnceLock<Configured> = std::sync::OnceLock::new();

/// Whether this process was asked to load compiled Lisp.
/// Whether this process loads compiled Lisp: ROMACS_COMPILED_LISP=on or
/// off says; unset, an INTERACTIVE session does and a batch one does not.
pub fn requested(interactive: bool) -> bool {
    host::getenv("ROMACS_COMPILED_LISP").map_or(interactive, |v| v == "on")
}

/// Load compiled Lisp with CONFIG for the rest of this process; a second
/// call changes nothing.
pub fn configure(config: Config) {
    let chain = host::getenv(CHAIN).map(|c| c.to_string_lossy().into_owned()).unwrap_or_default();
    let trace = host::getenv("ROMACS_COMPILED_LISP_TRACE").is_some();
    let no_prune = host::getenv(NO_PRUNE).is_some();
    if host::getenv(BACKGROUND).is_some() {
        host::lower_priority(host::Priority::Idle);
    }
    // A compile process ends with the process that started it, and within
    // a deadline: nothing outlives a session or holds its queue forever.
    // The name is its own, not its children's (a world builder, say).
    // Read while the process is single threaded, at the start of main.
    if let Some(parent) = host::getenv(PARENT).and_then(|p| p.to_str()?.parse().ok()) {
        host::unset_env(PARENT);
        host::die_with_parent(parent);
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(600));
            eprintln!("romacs-compile: deadline passed");
            std::process::exit(124);
        });
    }
    let _ = CONFIG.set(Configured { config, chain, trace, no_prune, identity: std::sync::OnceLock::new() });
}

/// Whether this process may compile Lisp: it loads compiled Lisp and runs
/// compile processes.
pub(crate) fn compiles() -> bool {
    config().is_some_and(|c| !c.compiler.is_empty())
}

/// The configured store, in a process that loads compiled Lisp.
pub fn store_dir() -> Option<&'static Path> {
    config().map(|c| c.store.as_path())
}

fn configured() -> Option<&'static Configured> {
    (host::mode() != host::Mode::Build).then(|| CONFIG.get()).flatten()
}

fn config() -> Option<&'static Config> {
    configured().map(|c| &c.config)
}

/// The compile chain a compile process runs in: the files being compiled
/// by it and the processes that started it, which compile nothing again.
const CHAIN: &str = "ROMACS_COMPILE_CHAIN";
/// The session's `load-path`, for the compile process, one per line.
pub const LOAD_PATH: &str = "ROMACS_COMPILE_LOAD_PATH";

/// In a compile process's environment: the process that started it, which
/// it does not outlive.
const PARENT: &str = "ROMACS_COMPILE_PARENT";

/// In a compile process's environment: run at the idle priority, as a
/// background compile does (`background`).
pub const BACKGROUND: &str = "ROMACS_COMPILE_BACKGROUND";

/// In a compile process's environment, and its nested compiles': publish
/// without pruning the store (`tree`, which prunes once at its end).
const NO_PRUNE: &str = "ROMACS_COMPILE_NO_PRUNE";

/// The compile process for the source file PATH on LOAD_PATH, as
/// (PROGRAM ARGS ENVIRONMENT): None when this process compiles nothing, or
/// PATH is being compiled already up the chain (its compile-time loads
/// read source, as GNU's would with no `.elc`).
fn compile_command(configured: &Configured, path: &Path, load_path: &[String])
    -> Option<(OsString, Vec<OsString>, Vec<(OsString, OsString)>)> {
    let name = path.to_string_lossy();
    if configured.chain.lines().any(|file| file == name) {
        return None;
    }
    let (program, args) = configured.config.compiler.split_first()?;
    let mut args = args.to_vec();
    args.push(path.as_os_str().to_owned());
    let mut environment = configured.config.environment.clone();
    environment.extend([
        (CHAIN.into(), format!("{}{name}\n", configured.chain).into()),
        (LOAD_PATH.into(), load_path.join("\n").into()),
        (PARENT.into(), host::pid().to_string().into()),
        ("ROMACS_COMPILED_LISP".into(), "on".into()),
    ]);
    if configured.no_prune {
        environment.push((NO_PRUNE.into(), "1".into()));
    }
    Some((program.clone(), args, environment))
}

/// Run the compile process PROGRAM ARGS in ENVIRONMENT, its standard error
/// appended to STORE's compile log, within the compile deadline: its
/// standard output when it succeeded.
fn run_compile(store: &Path, program: OsString, args: Vec<OsString>, environment: Vec<(OsString, OsString)>) -> Option<String> {
    let log = store::generation(store).join("compile.log");
    let _ = host::create_dir_all(store::generation(store));
    let stderr = host::open_output(&log, host::OpenOptions::new().append(true).create(true))
        .map(std::process::Stdio::from)
        .unwrap_or_else(|_| std::process::Stdio::null());
    let mut command = host::command(program).ok()?;
    command.args(args)
        .env_clear()
        .envs(environment)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(stderr);
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut out = String::new();
        let _ = stdout.read_to_string(&mut out);
        out
    });
    let deadline = host::monotonic() + std::time::Duration::from_secs(600);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if host::monotonic() < deadline => std::thread::sleep(std::time::Duration::from_millis(2)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let out = reader.join().unwrap_or_default();
    status.is_some_and(|s| s.success()).then_some(out)
}

/// The `.el` files under DIR, its subdirectories' after its own, each
/// level sorted; names starting with a dot are skipped.
fn lisp_files(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = host::read_dir(dir).into_iter().flatten().flatten()
        .map(|e| e.path())
        .filter(|p| !p.file_name().is_some_and(|n| n.as_encoded_bytes().starts_with(b".")))
        .collect();
    entries.sort();
    let (dirs, files): (Vec<PathBuf>, Vec<PathBuf>) = entries.into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "el") || host::probe(host::Probe::IsDir, p))
        .partition(|p| host::probe(host::Probe::IsDir, p));
    files.into_iter().chain(dirs.iter().flat_map(|d| lisp_files(d))).collect()
}

/// What a compiled load of a file runs.
pub(crate) struct Compiled {
    pub(crate) artifact: PathBuf,
    artifact_data: store::Artifact,
}

impl Compiled {
    pub(crate) fn units(&self) -> impl Iterator<Item = (usize, usize, usize, usize)> + '_ {
        self.artifact_data.units.iter().map(|u| (u.ordinal, u.byte, u.end, u.line))
    }
    /// In a compile process, record what this artifact's own compile
    /// observed: the compile loading it uses what that compile-time code
    /// read (a macro's expansion, say), so it holds only while that does.
    pub(crate) fn inherit_inputs(&self) {
        for range in &self.artifact_data.inputs {
            if let Ok(Json::Object(mut record)) = serde_json::from_str::<Json>(self.artifact_data.text(range))
                && let Some(Json::String(kind)) = record.remove("kind")
            {
                host::record(&kind, Json::Object(record));
            }
        }
    }
}

/// Facts a load found holding already, by the digest of their record's
/// text (`input_memo`): each is asked of the host once per process.
static HOLDING: std::sync::Mutex<Option<rustc_hash::FxHashSet<u128>>> = std::sync::Mutex::new(None);

/// The memo of an input's record TEXT: its digest, seeded for a library
/// resolution by PATH_SEED, the `load-path' it is checked on.
fn input_memo(text: &str, path_seed: u64) -> u128 {
    let seed = if text.starts_with("{\"kind\":\"resolved\"") { path_seed } else { 0 };
    twox_hash::XxHash3_128::oneshot_with_seed(seed, text.as_bytes())
}

impl Vm {
/// Whether every observed input of ARTIFACT holds now. A library
/// resolution holds on the `load-path' in force, LOAD_PATH, which keys
/// its memo. Records held already are not parsed again.
fn inputs_hold(&mut self, store: &Path, artifact: &store::Artifact, load_path: &str) -> bool {
    let path_seed = twox_hash::XxHash3_128::oneshot(load_path.as_bytes()) as u64;
    let unknown: Vec<(&str, u128)> = {
        let mut held = HOLDING.lock().unwrap();
        let held = held.get_or_insert_default();
        artifact.inputs.iter()
            .map(|range| artifact.text(range))
            .map(|text| (text, input_memo(text, path_seed)))
            .filter(|(_, memo)| !held.contains(memo))
            .collect()
    };
    unknown.into_iter().all(|(text, memo)| {
        let Ok(record) = serde_json::from_str::<Json>(text) else { return false };
        let record = &record;
        let holds = match record["kind"].as_str() {
            Some("loaded") => record["path"].as_str().is_some_and(|path| {
                store::source_digest(store, Path::new(path)).is_ok_and(|d| record["sha256"] == store::hex(&d))
            }),
            Some("resolved") => record["name"].as_str().is_some_and(|name| {
                let nosuffix = record["nosuffix"].as_bool().unwrap_or(false);
                self.source_path(name, nosuffix).ok().is_some_and(|found| {
                    json!(found.map(|(p, _)| p.to_string_lossy().into_owned())) == record["path"]
                })
            }),
            _ => {
                let now = host::replay(record);
                let holds = now.as_ref() == Some(record);
                if !holds && configured().is_some_and(|c| c.trace) {
                    eprintln!("romacs-compiled-lisp: {}", json!({"stale": record, "now": now}));
                }
                holds
            }
        };
        if holds {
            HOLDING.lock().unwrap().get_or_insert_default().insert(memo);
        }
        holds
    })
}

    /// Whether PATH has a compiled form GNU's suffix rules would pick:
    /// a file of the installation's Lisp trees, or one beside an `.elc`
    /// that is not older than it when `load-prefer-newer` asks.
    fn has_compiled_form(&self, config: &Config, path: &Path) -> bool {
        if config.runtime.iter().any(|root| path.starts_with(root)) {
            return true;
        }
        let elc = path.with_extension("elc");
        let Ok(compiled) = host::metadata(&elc) else { return false };
        let prefer_newer = self.names.get("load-prefer-newer").is_some_and(|&s| self.slots[s].value != Value::NIL);
        if !prefer_newer {
            return true;
        }
        let modified = |m: std::fs::Metadata| m.modified().ok();
        modified(compiled) >= host::metadata(path).ok().and_then(modified)
    }

    /// The compiled load of the source file PATH (resolved), when this
    /// load is one: a stored artifact whose inputs hold, else one compiled
    /// now. None loads PATH as source: no store, no compiled form, a
    /// compile refused, failed or not ours to run, or PATH being compiled
    /// up this compile's chain (a cycle: gnus-sum's compile requires
    /// gnus-art, which requires gnus-sum), whatever the store holds, so a
    /// compile's result never depends on whether another one published
    /// first.
    pub(crate) fn compiled_load(&mut self, path: &Path) -> Result<Option<Compiled>> {
        let Some(configured) = configured() else { return Ok(None) };
        let config = &configured.config;
        if !self.has_compiled_form(config, path) || configured.chain.lines().any(|file| Path::new(file) == path) {
            return Ok(None);
        }
        let load_path = self.load_path_strings()?;
        let Ok(found) = self.stored(configured, path, &load_path) else { return Ok(None) };
        let (artifact, data) = match found {
            Some(found) => found,
            None if config.defer_misses => {
                if !config.compiler.is_empty() {
                    background::note_miss(path, &load_path);
                }
                return Ok(None);
            }
            None => match self.compile_now(config, path, &load_path)? {
                Some(artifact) => match store::open(&artifact) {
                    Ok(data) => (artifact, data),
                    Err(_) => return Ok(None),
                },
                None => return Ok(None),
            },
        };
        if !matches!(data.status, store::Status::Compiled) {
            return Ok(None);
        }
        store::note_use(&config.store, &artifact, path);
        Ok(Some(Compiled { artifact, artifact_data: data }))
    }

    /// PATH's stored artifact whose inputs hold on LOAD_PATH, if any; Err
    /// when PATH cannot be read.
    fn stored(&mut self, configured: &Configured, path: &Path, load_path: &[String])
        -> std::io::Result<Option<(PathBuf, store::Artifact)>> {
        let config = &configured.config;
        let digest = store::source_digest(&config.store, path)?;
        let joined = load_path.join("\n");
        Ok(configured.identity().and_then(|identity| {
            let key = store::key(identity, path, &digest);
            store::lookup(&config.store, &key, |artifact| self.inputs_hold(&config.store, artifact, &joined))
        }))
    }

    /// Compile PATH in a compile process now: the artifact it published,
    /// or None when it did not, or PATH is being compiled already up the
    /// chain (its compile-time loads read source, as GNU's would with no
    /// `.elc`).
    fn compile_now(&mut self, config: &Config, path: &Path, load_path: &[String]) -> Result<Option<PathBuf>> {
        let Some(configured) = configured() else { return Ok(None) };
        let Some((program, args, environment)) = compile_command(configured, path, load_path) else { return Ok(None) };
        Ok(run_compile(&config.store, program, args, environment)
            .and_then(|out| out.lines().rev().find_map(|line| line.strip_prefix("romacs-compiled: ")).map(PathBuf::from)))
    }

    /// Install unit INDEX of a compiled load, with SLOTS for its names:
    /// the entry function to run.
    pub(crate) fn install_compiled_unit(&mut self, compiled: &Compiled, slots: &Arc<Vec<usize>>, index: usize) -> Result<usize> {
        let unit = &compiled.artifact_data.units[index];
        let bytes = &compiled.artifact_data.bytes;
        let mut reader = codec::Reader::new(&bytes[unit.range.clone()]);
        let entry = unit::Install { bytes, slots }.unit(self, &mut reader)?;
        if !reader.done() {
            return Err("compiled Lisp artifact: unit has trailing data".into());
        }
        Ok(entry)
    }

    /// The slots of a compiled load's names, interned.
    pub(crate) fn compiled_slots(&mut self, compiled: &Compiled) -> Arc<Vec<usize>> {
        let artifact = &compiled.artifact_data;
        Arc::new(artifact.names.iter().map(|name| self.slot(artifact.text(name))).collect())
    }

    /// The compile process: compile SOURCE, publish its artifact in the
    /// configured store and say where on standard output, with whether an
    /// artifact whose inputs hold was there already (then nothing compiles).
    /// The walk's refusals and errors make a source-only artifact, so the
    /// next load reads source without compiling again until an input
    /// changes.
    pub fn compile_lisp_file(&mut self, source: &Path) -> Result<(PathBuf, bool)> {
        let configured = configured().ok_or("compiled Lisp is not configured in this process")?;
        let config = &configured.config;
        let identity = configured.identity().ok_or("no compile world")?;
        let source = host::canonicalize(source).map_err(|e| format!("{}: {e}", source.display()))?;
        let load_path = self.load_path_strings()?;
        if let Ok(Some((held, _))) = self.stored(configured, &source, &load_path) {
            return Ok((held, true));
        }
        let digest = store::source_digest(&config.store, &source).map_err(|e| e.to_string())?;
        let key = store::key(identity, &source, &digest);
        host::enter_record();
        let mut names = unit::Names::default();
        let walked = self.compile_walk(&source, &mut names);
        let records = host::leave_record();
        let inputs: Vec<Json> = records.into_iter()
            .filter(|r| {
                let path = r["path"].as_str().map(Path::new);
                !path.is_some_and(|p| p.starts_with(&config.store) || p == source)
            })
            .filter(|r| r["kind"] != "env" || r["name"].as_str().is_some_and(|name| config.inherited.contains(&name)))
            .collect();
        let (status, units) = match walked {
            Ok(Ok(units)) => (store::Status::Compiled, units),
            Ok(Err(reason)) => {
                eprintln!("romacs-compile: {}: {reason}", source.display());
                (store::Status::Source(reason), vec![])
            }
            Err(reason) => {
                eprintln!("romacs-compile: {}: {reason}", source.display());
                (store::Status::Error(reason.to_string()), vec![])
            }
        };
        let (bytes, inputs_digest) = store::encode(&status, &source, &inputs, &names.names, &units);
        let path = store::publish(&config.store, &key, &inputs_digest, &bytes).map_err(|e| e.to_string())?;
        if !configured.no_prune
            && let Ok(removed) = store::prune(&config.store, store::BUDGET, &Default::default())
            && !removed.is_empty()
        {
            eprintln!("romacs-compile: {}", json!({"pruned": removed.len()}));
        }
        Ok((path, false))
    }

    /// Walk SOURCE and compile what it emits: the units, or why the walk
    /// refused it (Ok(Err)); Err for a signal or a form that failed to
    /// compile.
    fn compile_walk(&mut self, source: &Path, names: &mut unit::Names)
        -> Result<std::result::Result<Vec<(usize, usize, usize, usize, Vec<u8>)>, String>> {
        self.eval_source_raw("(require 'romacs-compile)")?;
        let file = serde_json::to_string(&source.to_string_lossy()).map_err(|e| e.to_string())?;
        let begun = self.eval_source_raw(&format!("(romacs-compile--begin {file})"))?;
        if begun != Value::NIL {
            return Ok(Err(self.source_string(begun).unwrap_or("not compiled").to_owned()));
        }
        let eof = self.intern(":eof");
        let mut units = vec![];
        loop {
            let next = self.eval_source_raw("(romacs-compile--next)")?;
            if next == eof {
                break;
            }
            // Every emission becomes Rust data before anything compiles:
            // compiling can run Lisp, which may move the heap.
            let mut emitted = vec![];
            for emission in self.list_values(next)? {
                let parts = self.list_values(emission)?;
                let [form, byte, end, line, ordinal, specials] = parts.as_slice() else {
                    return Err("romacs-compile--next: malformed emission".into());
                };
                let form = self.expansion_form(*form, 0)?;
                let numbers = [*byte, *end, *line, *ordinal].map(|v| v.as_integer().map(|n| n as usize));
                let specials: NameSet<String> = self.list_values(*specials)?
                    .into_iter()
                    .filter_map(|s| s.index_if(7).map(|slot| self.symbol_names[slot - 1].clone()))
                    .collect();
                emitted.push((form, numbers, specials));
            }
            for (form, [byte, end, line, ordinal], specials) in emitted {
                let data = match self.compile_unit(&form, specials, names) {
                    Ok(data) => data,
                    Err(reason) => return Err(format!("line {}: {reason}", line?).into()),
                };
                units.push((ordinal?, byte?, end?, line?, data));
            }
        }
        let report = self.eval_source_raw("(romacs-compile--end)")?;
        let not_inlined: Vec<String> = self.list_values(report)?.get(1).map_or(Ok(vec![]), |v| self.list_values(*v))?
            .into_iter()
            .filter_map(|s| s.index_if(7).map(|slot| self.symbol_names[slot - 1].clone()))
            .collect();
        if !not_inlined.is_empty() {
            eprintln!("romacs-compile: {}: {}", source.display(), json!({"not-inlined": not_inlined}));
        }
        Ok(Ok(units))
    }

    /// FORM compiled into one unit's bytes, with SPECIALS the names the
    /// file declared special so far.
    fn compile_unit(&mut self, form: &Form, specials: NameSet<String>, names: &mut unit::Names) -> Result<Vec<u8>> {
        let saved = (std::mem::replace(&mut self.file_specials, specials),
                     std::mem::replace(&mut self.lazy_regime, false),
                     std::mem::replace(&mut self.defer_expansion_errors, false));
        self.seed_file_specials = true;
        let compiled = self.compile_request(form);
        self.seed_file_specials = false;
        (self.file_specials, self.lazy_regime, self.defer_expansion_errors) = saved;
        let (entry, _) = compiled?;
        let mut out = codec::Writer::default();
        let encoded = unit::Encoder::new(self, names).unit(entry, &mut out);
        self.release_request_entry(entry);
        encoded.map(|()| out.bytes)
    }
}
