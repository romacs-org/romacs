//! The compiled-Lisp store: `$XDG_CACHE_HOME/romacs/lisp/`, beside the
//! world store, private like it. One directory per compiler generation,
//! one per key in it, one artifact per set of observed inputs:
//! `GEN/KEY/INPUTS.rlc`. Writes are a private temporary file renamed into
//! place, so a reader sees a whole artifact or none.
//!
//! An artifact is `MAGIC`, the xxh3-128 of its body (a check against
//! damage, read on every load, so a fast one), then the body: the
//! status (compiled, or why the file loads as source), the source's
//! resolved path, the observed inputs (host records, `host::replay`'s
//! kind, and the files the compile loaded), the name table and the units.
use super::codec::{Reader, Writer};
use crate::host;
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MAGIC: &[u8] = b"RLC3\n";
/// What a key binds besides its inputs: this format and the compiler.
const FORMAT: &str = concat!("romacs-compiled-lisp 3\n", env!("VM2_COMPILER_ID"));
/// The default cap on the store's artifacts, in bytes.
pub(super) const BUDGET: u64 = 256 << 20;

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// This compiler generation's directory: older generations are pruned.
pub(super) fn generation(store: &Path) -> PathBuf {
    store.join(&hex(&Sha256::digest(FORMAT.as_bytes()))[..16])
}

/// The key of SOURCE (its resolved path and content DIGEST) compiled in
/// the environment ENV. `load-path' is not keyed: the compile's library
/// resolutions are observed inputs, checked on the `load-path' of the load.
pub(super) fn key(env: &str, source: &Path, digest: &[u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(FORMAT.as_bytes());
    hash.update([0]);
    hash.update(env.as_bytes());
    hash.update([1]);
    hash.update(source.as_os_str().as_encoded_bytes());
    hash.update([0]);
    hash.update(digest);
    hash.finalize().into()
}

/// Why an artifact's file loads as source, or that it is compiled.
pub(super) enum Status {
    Compiled,
    Source(String),
}

/// One unit's place: the top-level form it came from and its bytes.
pub(super) struct UnitEntry {
    pub(super) ordinal: usize,
    pub(super) byte: usize,
    pub(super) end: usize,
    pub(super) line: usize,
    pub(super) range: std::ops::Range<usize>,
}

pub(super) struct Artifact {
    /// Shared with the bodies a load leaves pending (`PendingBody`).
    pub(super) bytes: std::sync::Arc<Vec<u8>>,
    pub(super) status: Status,
    /// The observed inputs, each a record's text, and the names, as
    /// places in BYTES (`text`).
    pub(super) inputs: Vec<std::ops::Range<usize>>,
    pub(super) names: Vec<std::ops::Range<usize>>,
    pub(super) units: Vec<UnitEntry>,
}

impl Artifact {
    /// The text at RANGE, one of `inputs` or `names`: UTF-8, as `decode` checked.
    pub(super) fn text(&self, range: &std::ops::Range<usize>) -> &str {
        std::str::from_utf8(&self.bytes[range.clone()]).unwrap_or_default()
    }
}

/// An artifact's bytes, its inputs part's digest naming it.
pub(super) fn encode(status: &Status, source: &Path, inputs: &[Json], names: &[String],
                     units: &[(usize, usize, usize, usize, Vec<u8>)]) -> (Vec<u8>, [u8; 32]) {
    let mut body = Writer::default();
    match status {
        Status::Compiled => {
            body.byte(0);
            body.str("");
        }
        Status::Source(reason) => {
            body.byte(1);
            body.str(reason);
        }
    }
    body.str(&source.to_string_lossy());
    // In one order, so concurrent compiles of a file that saw the same
    // inputs publish the same artifact.
    let mut inputs_text: Vec<String> = inputs.iter().map(Json::to_string).collect();
    inputs_text.sort();
    inputs_text.dedup();
    let mut inputs_hash = Sha256::new();
    body.n(inputs_text.len());
    for text in &inputs_text {
        inputs_hash.update(text.as_bytes());
        inputs_hash.update([0]);
        body.str(text);
    }
    body.n(names.len());
    for name in names {
        body.str(name);
    }
    body.n(units.len());
    for (ordinal, byte, end, line, data) in units {
        for n in [*ordinal, *byte, *end, *line] {
            body.n(n);
        }
        body.data(data);
    }
    let mut out = Writer::default();
    out.raw(MAGIC);
    out.raw(&checksum(&body.bytes));
    out.raw(&body.bytes);
    (out.bytes, inputs_hash.finalize().into())
}

fn checksum(body: &[u8]) -> [u8; 16] {
    twox_hash::XxHash3_128::oneshot(body).to_le_bytes()
}

/// The artifact BYTES hold, when they are whole.
pub(super) fn decode(bytes: Vec<u8>) -> crate::Result<Artifact> {
    let body_at = MAGIC.len() + 16;
    if bytes.len() < body_at || &bytes[..MAGIC.len()] != MAGIC {
        return Err("not a compiled Lisp artifact".into());
    }
    if checksum(&bytes[body_at..])[..] != bytes[MAGIC.len()..body_at] {
        return Err("compiled Lisp artifact corrupt".into());
    }
    let mut r = Reader::new(&bytes[body_at..]);
    let status = match (r.byte()?, r.str()?) {
        (0, _) => Status::Compiled,
        (1, reason) => Status::Source(reason.to_owned()),
        _ => return Err("compiled Lisp artifact: bad status".into()),
    };
    let _source = r.str()?;
    let place = |s: &str| {
        let start = s.as_ptr() as usize - bytes.as_ptr() as usize;
        start..start + s.len()
    };
    let n = r.count()?;
    let inputs = (0..n).map(|_| r.str().map(place)).collect::<crate::Result<Vec<_>>>()?;
    let n = r.count()?;
    let names = (0..n).map(|_| r.str().map(place)).collect::<crate::Result<Vec<_>>>()?;
    let n = r.count()?;
    let mut units = Vec::with_capacity(n);
    for _ in 0..n {
        let (ordinal, byte, end, line) = (r.n()?, r.n()?, r.n()?, r.n()?);
        let data = r.data()?;
        let start = data.as_ptr() as usize - bytes.as_ptr() as usize;
        units.push(UnitEntry { ordinal, byte, end, line, range: start..start + data.len() });
    }
    if !r.done() {
        return Err("compiled Lisp artifact: trailing data".into());
    }
    Ok(Artifact { bytes: std::sync::Arc::new(bytes), status, inputs, names, units })
}

/// A store directory, made private when missing.
fn subdir(dir: &Path) -> std::io::Result<()> {
    match host::create_dir(dir, 0o700) {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        made => made,
    }
}

fn read(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = vec![];
    host::open_store_file(path, host::StoreOpen::Read)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The first artifact for KEY whose inputs VALID accepts. A damaged one
/// is deleted: its generation fixes its format, so it can never load.
pub(super) fn lookup(store: &Path, key: &[u8; 32], mut valid: impl FnMut(&Artifact) -> bool) -> Option<(PathBuf, Artifact)> {
    let dir = generation(store).join(hex(key));
    let mut candidates: Vec<PathBuf> = host::read_dir(&dir).ok()?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "rlc"))
        .collect();
    candidates.sort();
    candidates.into_iter().find_map(|path| {
        let Ok(artifact) = decode(read(&path).ok()?) else {
            let _ = host::remove_file(&path);
            return None;
        };
        valid(&artifact).then_some((path, artifact))
    })
}

/// Whether KEY has any artifact, whatever its inputs.
pub(super) fn has_artifact(store: &Path, key: &[u8; 32]) -> bool {
    host::read_dir(generation(store).join(hex(key))).is_ok_and(|mut entries| {
        entries.any(|e| e.is_ok_and(|e| e.path().extension().is_some_and(|x| x == "rlc")))
    })
}

/// The artifact at PATH, as a compile process published it.
pub(super) fn open(path: &Path) -> crate::Result<Artifact> {
    decode(read(path).map_err(|e| format!("{}: {e}", path.display()))?)
}

/// Whether the store's filesystem has room for writing: at least the
/// smaller of 5% of it and 2 GB free.
fn room(store: &Path) -> bool {
    let Ok(path) = std::ffi::CString::new(store.as_os_str().as_encoded_bytes()) else { return false };
    let Ok(stat) = host::statvfs(&path) else { return false };
    let free = stat.f_bavail as u64 * stat.f_frsize as u64;
    let size = stat.f_blocks as u64 * stat.f_frsize as u64;
    free >= (size / 20).min(2 << 30)
}

/// Publish BYTES as KEY's artifact named by INPUTS: its path, or an error
/// when the store has no room or cannot be written.
pub(super) fn publish(store: &Path, key: &[u8; 32], inputs: &[u8; 32], bytes: &[u8]) -> std::io::Result<PathBuf> {
    if !room(store) {
        return Err(std::io::Error::other("no room on the store's filesystem"));
    }
    let generation = generation(store);
    subdir(store)?;
    subdir(&generation)?;
    let dir = generation.join(hex(key));
    subdir(&dir)?;
    let path = dir.join(format!("{}.rlc", hex(inputs)));
    let temporary = dir.join(format!(".{}.{}", hex(inputs), host::pid()));
    host::write_store_file(&temporary, bytes)?;
    host::rename(&temporary, &path)?;
    Ok(path)
}

/// Note that the artifact at PATH, of the source file SOURCE, was used:
/// for pruning, and for what a background compile does first (`history`).
/// A line is the artifact's path in its generation, a tab, the source's.
pub(super) fn note_use(store: &Path, path: &Path, source: &Path) {
    use std::io::Write;
    let log = generation(store).join("use.log");
    if let Ok(mut file) = host::open_store_file(&log, host::StoreOpen::Write { create: true, truncate: false }) {
        let line = format!("{}\t{}\n", path.strip_prefix(generation(store)).unwrap_or(path).display(), source.display());
        let _ = file.seek_end().and_then(|()| file.write_all(line.as_bytes()));
    }
}

trait SeekEnd {
    fn seek_end(&mut self) -> std::io::Result<()>;
}
impl SeekEnd for std::fs::File {
    fn seek_end(&mut self) -> std::io::Result<()> {
        use std::io::Seek;
        self.seek(std::io::SeekFrom::End(0)).map(drop)
    }
}

/// Keep the store within BUDGET: other generations go first, then this
/// generation's least recently used artifacts by the use log (atime is not
/// trusted), which is then compacted. Runs after a compile publishes,
/// never inside a load.
pub(super) fn prune(store: &Path, budget: u64) -> std::io::Result<Vec<PathBuf>> {
    let current = generation(store);
    let mut removed = vec![];
    // Every older generation: at most two are kept, the current and the one
    // before it, until the current one has a runtime tree's worth.
    let mut generations: Vec<(std::time::SystemTime, PathBuf)> = host::read_dir(store)?
        .filter_map(|e| {
            let path = e.ok()?.path();
            let name = path.file_name()?.to_str()?.to_owned();
            (name.len() == 16 && path != current && host::probe(host::Probe::IsDir, &path))
                .then(|| (host::metadata(&path).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), path))
        })
        .collect();
    generations.sort();
    let artifacts = |dir: &Path| -> Vec<(PathBuf, u64)> {
        let mut out = vec![];
        for key in host::read_dir(dir).into_iter().flatten().flatten() {
            for file in host::read_dir(key.path()).into_iter().flatten().flatten() {
                let path = file.path();
                if path.extension().is_some_and(|e| e == "rlc")
                    && let Ok(meta) = host::symlink_metadata(&path)
                {
                    out.push((path, meta.len()));
                }
            }
        }
        out
    };
    let mut total: u64 = artifacts(&current).iter().map(|(_, n)| n).sum();
    let older: u64 = generations.iter().flat_map(|(_, g)| artifacts(g)).map(|(_, n)| n).sum();
    let keep_previous = generations.len() <= 1 && total + older <= budget;
    for (index, (_, dir)) in generations.iter().enumerate() {
        if keep_previous && index + 1 == generations.len() {
            total += older;
            break;
        }
        for (path, _) in artifacts(dir) {
            if host::remove_file(&path).is_ok() {
                removed.push(path);
            }
        }
        for key in host::read_dir(dir).into_iter().flatten().flatten() {
            let _ = host::remove_dir(key.path());
        }
        let _ = host::remove_file(dir.join("use.log"));
        let _ = host::remove_dir(dir);
    }
    if total <= budget {
        return Ok(removed);
    }
    // Least recently used first: the use log's last mention of each.
    let log = read(&current.join("use.log")).unwrap_or_default();
    let mut last_use = std::collections::HashMap::new();
    for (i, line) in String::from_utf8_lossy(&log).lines().enumerate() {
        let artifact = line.split('\t').next().unwrap_or(line);
        last_use.insert(current.join(artifact), (i, line.to_owned()));
    }
    let mut files = artifacts(&current);
    files.sort_by_key(|(path, _)| last_use.get(path).map_or(0, |(i, _)| i + 1));
    for (path, size) in files {
        if total <= budget * 9 / 10 {
            break;
        }
        if host::remove_file(&path).is_ok() {
            total -= size;
            last_use.remove(&path);
            removed.push(path);
        }
    }
    let mut kept: Vec<(usize, String)> = last_use.into_values().collect();
    kept.sort();
    let text: String = kept.iter().map(|(_, line)| format!("{line}\n")).collect();
    let temporary = current.join(format!(".use.log.{}", host::pid()));
    host::write_store_file(&temporary, text)?;
    host::rename(&temporary, current.join("use.log"))?;
    Ok(removed)
}

/// The source files compiled loads used, most recent first, from the use
/// logs of every generation in the store: a new compiler's generation
/// starts from what the one before it served.
pub(super) fn history(store: &Path) -> Vec<PathBuf> {
    let mut logs: Vec<(std::time::SystemTime, PathBuf)> = host::read_dir(store).into_iter().flatten().flatten()
        .map(|e| e.path().join("use.log"))
        .filter_map(|log| Some((host::metadata(&log).ok()?.modified().ok()?, log)))
        .collect();
    logs.sort();
    let mut sources: Vec<PathBuf> = vec![];
    let mut seen = std::collections::HashSet::new();
    for (_, log) in logs.iter().rev() {
        let text = read(log).unwrap_or_default();
        for line in String::from_utf8_lossy(&text).lines().rev() {
            if let Some((_, source)) = line.split_once('\t')
                && seen.insert(source.to_owned())
            {
                sources.push(PathBuf::from(source));
            }
        }
    }
    sources
}

/// SOURCE's content digest, memoised by its identity and change times in
/// the store (as the world store keeps the executable's) and in this
/// process.
pub(crate) fn source_digest(store: &Path, source: &Path) -> std::io::Result<[u8; 32]> {
    use std::os::unix::fs::MetadataExt;
    static MEMO: std::sync::Mutex<Option<std::collections::HashMap<PathBuf, (String, [u8; 32])>>> = std::sync::Mutex::new(None);
    let file = host::open_world_file(source)?;
    let meta = host::file_metadata(&file)?;
    let identity = format!("{}-{}-{}-{}.{}-{}.{}", meta.dev(), meta.ino(), meta.size(), meta.mtime(), meta.mtime_nsec(),
                           meta.ctime(), meta.ctime_nsec());
    if let Some((id, digest)) = MEMO.lock().unwrap().get_or_insert_default().get(source)
        && *id == identity
    {
        return Ok(*digest);
    }
    let ids = store.join("ids");
    let cached = ids.join(&identity);
    let digest = match host::read_store_file(&cached).ok().and_then(|text| {
        (0..32).map(|i| u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()).collect::<Option<Vec<u8>>>()
    }) {
        Some(digest) => digest.try_into().expect("32 bytes"),
        None => {
            use std::io::Read;
            let mut bytes = vec![];
            (&file).read_to_end(&mut bytes)?;
            let digest: [u8; 32] = Sha256::digest(&bytes).into();
            if subdir(store).and_then(|()| subdir(&ids)).is_ok() {
                let _ = host::write_store_file(&cached, hex(&digest));
            }
            digest
        }
    };
    MEMO.lock().unwrap().get_or_insert_default().insert(source.to_owned(), (identity, digest));
    Ok(digest)
}
