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
//! Which loads are compiled is decided once per process: only when
//! ROMACS_COMPILED_LISP=on, the embedder configured a store and a compile
//! command (`configure`), and this is no world build. Without a store
//! every load reads source, as a GNU installation without `.elc` files.
mod codec;
pub(crate) mod store;
mod unit;
#[cfg(test)]
mod tests;

use crate::*;
use serde_json::{Value as Json, json};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// What a process needs to load compiled Lisp.
pub struct Config {
    /// The compile command: a program and its leading arguments, to which
    /// the source file is appended.
    pub compiler: Vec<OsString>,
    /// The store (`$XDG_CACHE_HOME/romacs/lisp`).
    pub store: PathBuf,
    /// The compile environment's identity, the base world a compile
    /// process restores: asked once, at the first compiled load, so a
    /// process that loads nothing pays nothing. None: no environment, no
    /// compiled loads.
    pub environment: Box<dyn Fn() -> Option<String> + Send + Sync>,
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
    environment: std::sync::OnceLock<Option<String>>,
}

impl Configured {
    fn environment(&self) -> Option<&str> {
        self.environment.get_or_init(|| (self.config.environment)()).as_deref()
    }
}

static CONFIG: std::sync::OnceLock<Configured> = std::sync::OnceLock::new();

/// Whether this process was asked to load compiled Lisp.
pub fn requested() -> bool {
    host::getenv("ROMACS_COMPILED_LISP").is_some_and(|v| v == "on")
}

/// Load compiled Lisp with CONFIG for the rest of this process; a second
/// call changes nothing.
pub fn configure(config: Config) {
    let chain = host::getenv(CHAIN).map(|c| c.to_string_lossy().into_owned()).unwrap_or_default();
    let trace = host::getenv("ROMACS_COMPILED_LISP_TRACE").is_some();
    let _ = CONFIG.set(Configured { config, chain, trace, environment: std::sync::OnceLock::new() });
}

/// The configured store, in a process that loads compiled Lisp.
pub(crate) fn store_dir() -> Option<&'static Path> {
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

/// What a compiled load of a file runs.
pub(crate) struct Compiled {
    pub(crate) artifact: PathBuf,
    artifact_data: store::Artifact,
}

impl Compiled {
    pub(crate) fn units(&self) -> impl Iterator<Item = (usize, usize, usize, usize)> + '_ {
        self.artifact_data.units.iter().map(|u| (u.ordinal, u.byte, u.end, u.line))
    }
}

/// Facts a load found holding already, by their record's text: each is
/// asked of the host once per process.
static HOLDING: std::sync::Mutex<Option<std::collections::HashSet<String>>> = std::sync::Mutex::new(None);

impl Vm {
/// Whether every observed input of an artifact, each a record's text,
/// holds now. A library resolution holds on the `load-path' in force,
/// LOAD_PATH, which keys its memo.
fn inputs_hold(&mut self, store: &Path, inputs: &[String], load_path: &str) -> bool {
    inputs.iter().all(|text| {
        let resolution = text.starts_with("{\"kind\":\"resolved\"");
        let memo = if resolution { format!("{text}\0{load_path}") } else { text.clone() };
        if HOLDING.lock().unwrap().get_or_insert_default().contains(&memo) {
            return true;
        }
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
    /// now. None loads PATH as source: no store, no compiled form, or a
    /// compile refused or failed.
    pub(crate) fn compiled_load(&mut self, path: &Path) -> Result<Option<Compiled>> {
        let Some(configured) = configured() else { return Ok(None) };
        let config = &configured.config;
        if !self.has_compiled_form(config, path) {
            return Ok(None);
        }
        let Some(environment) = configured.environment() else { return Ok(None) };
        let Ok(digest) = store::source_digest(&config.store, path) else { return Ok(None) };
        let load_path = self.load_path_strings()?;
        let key = store::key(environment, path, &digest);
        let joined = load_path.join("\n");
        let found = store::lookup(&config.store, &key, |inputs| self.inputs_hold(&config.store, inputs, &joined));
        let (artifact, data) = match found {
            Some(found) => found,
            None => match self.compile_now(config, path, &load_path)? {
                Some(artifact) => match store::open(&artifact) {
                    Ok(data) => (artifact, data),
                    Err(_) => return Ok(None),
                },
                None => return Ok(None),
            },
        };
        if let store::Status::Source(_) = data.status {
            return Ok(None);
        }
        store::note_use(&config.store, &artifact);
        Ok(Some(Compiled { artifact, artifact_data: data }))
    }

    /// Compile PATH in a compile process now: the artifact it published,
    /// or None when it did not, or PATH is being compiled already up the
    /// chain (its compile-time loads read source, as GNU's would with no
    /// `.elc`).
    fn compile_now(&mut self, config: &Config, path: &Path, load_path: &[String]) -> Result<Option<PathBuf>> {
        let chain = configured().map_or("", |c| c.chain.as_str());
        let name = path.to_string_lossy();
        if chain.lines().any(|file| file == name) {
            return Ok(None);
        }
        let Some((program, args)) = config.compiler.split_first() else { return Ok(None) };
        let log = store::generation(&config.store).join("compile.log");
        let _ = host::create_dir_all(store::generation(&config.store));
        let stderr = host::open_output(&log, host::OpenOptions::new().append(true).create(true))
            .map(std::process::Stdio::from)
            .unwrap_or_else(|_| std::process::Stdio::null());
        let mut command = host::command(program).map_err(|e| e.to_string())?;
        command.args(args).arg(path)
            .env(CHAIN, format!("{chain}{name}\n"))
            .env(LOAD_PATH, load_path.join("\n"))
            .env("ROMACS_COMPILED_LISP", "on")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(stderr);
        let Ok(mut child) = command.spawn() else { return Ok(None) };
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
        if !status.is_some_and(|s| s.success()) {
            return Ok(None);
        }
        Ok(out.lines().rev().find_map(|line| line.strip_prefix("romacs-compiled: ")).map(PathBuf::from))
    }

    /// Install unit INDEX of a compiled load, with SLOTS for its names:
    /// the entry function to run.
    pub(crate) fn install_compiled_unit(&mut self, compiled: &Compiled, slots: &[usize], index: usize) -> Result<usize> {
        let unit = &compiled.artifact_data.units[index];
        let data = &compiled.artifact_data.bytes[unit.range.clone()];
        let mut reader = codec::Reader::new(data);
        let entry = unit::Install { slots }.unit(self, &mut reader)?;
        if !reader.done() {
            return Err("compiled Lisp artifact: unit has trailing data".into());
        }
        Ok(entry)
    }

    /// The slots of a compiled load's names, interned.
    pub(crate) fn compiled_slots(&mut self, compiled: &Compiled) -> Vec<usize> {
        compiled.artifact_data.names.iter().map(|name| self.slot(name)).collect()
    }

    /// The compile process: compile SOURCE, publish its artifact in the
    /// configured store and say where on standard output. The walk's
    /// errors and refusals make a source-only artifact, so the next load
    /// reads source without compiling again until an input changes.
    pub fn compile_lisp_file(&mut self, source: &Path) -> Result<PathBuf> {
        let configured = configured().ok_or("compiled Lisp is not configured in this process")?;
        let config = &configured.config;
        let environment = configured.environment().ok_or("no compile environment")?;
        let source = host::canonicalize(source).map_err(|e| format!("{}: {e}", source.display()))?;
        let digest = store::source_digest(&config.store, &source).map_err(|e| e.to_string())?;
        let key = store::key(environment, &source, &digest);
        host::enter_record();
        let mut names = unit::Names::default();
        let walked = self.compile_walk(&source, &mut names);
        let records = host::leave_record();
        let inputs: Vec<Json> = records.into_iter()
            .filter(|r| {
                let path = r["path"].as_str().map(Path::new);
                !path.is_some_and(|p| p.starts_with(&config.store) || p == source)
            })
            .collect();
        let (status, units) = match walked {
            Ok(Ok(units)) => (store::Status::Compiled, units),
            Ok(Err(reason)) | Err(reason) => {
                eprintln!("romacs-compile: {}: {reason}", source.display());
                (store::Status::Source(reason), vec![])
            }
        };
        let (bytes, inputs_digest) = store::encode(&status, &source, &inputs, &names.names, &units);
        let path = store::publish(&config.store, &key, &inputs_digest, &bytes).map_err(|e| e.to_string())?;
        if let Ok(removed) = store::prune(&config.store, store::BUDGET)
            && !removed.is_empty()
        {
            eprintln!("romacs-compile: {}", json!({"pruned": removed.len()}));
        }
        Ok(path)
    }

    /// Walk SOURCE and compile what it emits: the units, or why it is not
    /// compiled (Ok(Err)); Err for a signal.
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
                    Err(reason) => return Ok(Err(format!("line {}: {reason}", line?))),
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
