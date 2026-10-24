//! GNU differential tests for primitive crates, without the old evaluator.
//!
//! Case files contain `(case NAME FN ARG...)`. GNU evaluates arguments and
//! encodes them; a crate dispatches directly on typed values. No Lisp reader,
//! callback, buffer, identity/GC-timing or evaluator emulation lives here.
//! Those require later stages. A pinned inventory is mandatory: deleting a
//! file or silently selecting fewer cases is an error, not a smaller green.
//! `Runner::refresh` is separate from `Runner::run`; warm runs start no Emacs.
//! Strings contain GNU internal bytes, not UTF-8, and floats contain IEEE bits.
//! NaN payloads, bignums and circular values are explicitly unsupported (RED).

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

// Bump when Rust encoding/decoding semantics change, not only wire shape.
const CODEC_VERSION: &str = "flat-p-2";
const DRIVER: &str = include_str!("../lisp/gnudiff.el");

/// The flat transport's value tree. Cons cells preserve dotted lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arg {
    Fixnum(i64),
    Float(u64),
    String { multibyte: bool, bytes: Vec<u8> },
    Symbol(String),
    Cons(Box<Arg>, Box<Arg>),
    Vector(Vec<Arg>),
}

/// A GNU-visible condition, compared by both symbol and data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Err {
    pub symbol: String,
    pub data: Arg,
}

/// A crate's primitive dispatch function; it owns no evaluator.
pub type Dispatch<'a> = dyn FnMut(&str, &[Arg]) -> Result<Arg, Err> + 'a;

/// A verified executable pin. On a cache-only host, supply the retained sha.
#[derive(Clone, Debug)]
pub struct Oracle {
    pub binary: PathBuf,
    pub sha256: String,
}

impl Oracle {
    /// Hash the actual GNU executable (the default oracle is `/usr/bin/emacs`).
    pub fn pin(binary: &Path) -> io::Result<Self> {
        Ok(Self {
            binary: binary.into(),
            sha256: sha256(binary)?,
        })
    }
}

/// Per-run evidence, written even for a failed or incomplete selection.
#[derive(Debug, Default)]
pub struct Receipt {
    pub gnu_sha256: String,
    pub gnu_binary: String,
    pub selected: usize,
    pub executed: usize,
    pub skipped: usize,
    pub cache_hits: usize,
    pub oracle_starts: usize,
    pub cases: Vec<String>,
    pub failures: Vec<String>,
}

impl Receipt {
    pub fn green(&self) -> bool {
        self.selected > 0
            && self.executed == self.selected
            && self.skipped == 0
            && self.failures.is_empty()
    }

    fn write(&self, path: &Path) -> io::Result<()> {
        let strings = |values: &[String]| {
            values
                .iter()
                .map(|v| json_string(v))
                .collect::<Vec<_>>()
                .join(",")
        };
        fs::write(
            path,
            format!(
                "{{\"schema\":1,\"verdict\":\"{}\",\"gnu_sha256\":{},\"gnu_binary\":{},\"selected\":{},\"executed\":{},\"skipped\":{},\"cache_hits\":{},\"oracle_starts\":{},\"cases\":[{}],\"failures\":[{}]}}\n",
                if self.green() { "GREEN" } else { "RED" },
                json_string(&self.gnu_sha256),
                json_string(&self.gnu_binary),
                self.selected,
                self.executed,
                self.skipped,
                self.cache_hits,
                self.oracle_starts,
                strings(&self.cases),
                strings(&self.failures)
            ),
        )
    }
}

/// Cache state is owned by this runner, never by a process-global switch.
pub struct Runner {
    oracle: Oracle,
    cache: PathBuf,
}

impl Runner {
    /// Use `~/.cache/romacs-gnudiff` for the normal cache, or an owned test cache.
    pub fn new(oracle: Oracle, cache: PathBuf) -> io::Result<Self> {
        if oracle.sha256.len() != 64 || !oracle.sha256.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(invalid("GNU pin is not a sha256"));
        }
        if oracle.binary.exists() && sha256(&oracle.binary)? != oracle.sha256 {
            return Err(invalid("GNU executable differs from pin"));
        }
        Ok(Self { oracle, cache })
    }

    /// Recompute GNU outputs explicitly, never as a hidden warm-run switch.
    pub fn refresh(&self, files: &[PathBuf]) -> io::Result<()> {
        if files.is_empty() {
            return Err(invalid("empty-selection"));
        }
        for file in files {
            self.load(file, true)?;
        }
        Ok(())
    }

    /// Compare exactly EXPECTED case identities, each exactly once. On failure,
    /// the receipt is RED and this returns an error; no skip is green.
    pub fn run(
        &self,
        files: &[PathBuf],
        expected: &[&str],
        dispatch: &mut Dispatch<'_>,
        receipt_path: &Path,
    ) -> io::Result<Receipt> {
        let mut receipt = Receipt {
            gnu_sha256: self.oracle.sha256.clone(),
            gnu_binary: self.oracle.binary.to_string_lossy().into_owned(),
            selected: expected.len(),
            ..Receipt::default()
        };
        let result = self.compare(files, expected, dispatch, &mut receipt);
        if let Err(ref error) = result {
            receipt.failures.push(error.to_string());
        }
        receipt.write(receipt_path)?;
        result?;
        if !receipt.green() {
            return Err(invalid("GNU differential mismatch; see receipt"));
        }
        Ok(receipt)
    }

    fn compare(
        &self,
        files: &[PathBuf],
        expected: &[&str],
        dispatch: &mut Dispatch<'_>,
        receipt: &mut Receipt,
    ) -> io::Result<()> {
        if expected.is_empty() || files.is_empty() {
            return Err(invalid("empty-selection"));
        }
        let mut cases = Vec::new();
        for file in files {
            let (mut entries, hit) = self.load(file, false)?;
            receipt.cache_hits += usize::from(hit);
            receipt.oracle_starts += usize::from(!hit);
            cases.append(&mut entries);
        }
        let actual: BTreeSet<_> = cases.iter().map(|c| c.name.as_str()).collect();
        let wanted: BTreeSet<_> = expected.iter().copied().collect();
        receipt.cases = cases.iter().map(|c| c.name.clone()).collect();
        if wanted.len() != expected.len() || actual.len() != cases.len() || actual != wanted {
            return Err(invalid("case inventory changed, missing or duplicated"));
        }
        for case in cases {
            let actual = dispatch(&case.function, &case.args);
            receipt.executed += 1;
            if actual != case.result {
                receipt.failures.push(format!(
                    "{}: GNU {:?}, crate {:?}",
                    case.name, case.result, actual
                ));
            }
        }
        Ok(())
    }

    fn load(&self, file: &Path, refresh: bool) -> io::Result<(Vec<Case>, bool)> {
        let case_sha = sha256(file)?;
        let mut hash = Command::new("/usr/bin/sha256sum")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        hash.stdin
            .take()
            .ok_or_else(|| invalid("driver hash stdin"))?
            .write_all(DRIVER.as_bytes())?;
        let output = hash.wait_with_output()?;
        if !output.status.success() {
            return Err(invalid("driver hash failed"));
        }
        let driver_sha =
            String::from_utf8(output.stdout).map_err(|_| invalid("driver hash output"))?;
        let driver_sha = driver_sha
            .split_whitespace()
            .next()
            .ok_or_else(|| invalid("driver hash empty"))?;
        let directory = self
            .cache
            .join(CODEC_VERSION)
            .join(driver_sha)
            .join(&self.oracle.sha256);
        let cached = directory.join(&case_sha);
        if !refresh && cached.exists() {
            return Ok((decode(&fs::read_to_string(cached)?)?, true));
        }
        // Even on a formerly pinned host, cache misses require the same binary.
        if sha256(&self.oracle.binary)? != self.oracle.sha256 {
            return Err(invalid("GNU executable differs from pin"));
        }
        fs::create_dir_all(&directory)?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid("clock before epoch"))?
            .as_nanos();
        let driver = directory.join(format!(".driver-{}-{nonce}.el", std::process::id()));
        fs::write(&driver, DRIVER)?;
        // Bound oracle errors/hangs and kill its descendants after the deadline.
        let output = Command::new("/usr/bin/timeout")
            .args(["--kill-after=2", "15"])
            .arg(&self.oracle.binary)
            .args(["--batch", "-Q", "-l"])
            .arg(&driver)
            .arg(file.canonicalize()?)
            .output();
        fs::remove_file(&driver)?;
        let output = output?;
        if !output.status.success() {
            return Err(invalid(&format!(
                "GNU failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let text = String::from_utf8(output.stdout).map_err(|_| invalid("non-UTF8 wire output"))?;
        let cases = decode(&text)?;
        if sha256(file)? != case_sha || sha256(&self.oracle.binary)? != self.oracle.sha256 {
            return Err(invalid("GNU/case input changed during oracle run"));
        }
        // Publish only a fully decoded successful result. Concurrent writers
        // have unique temporaries and identical content-addressed output.
        let temporary = directory.join(format!(".cache-{}-{nonce}", std::process::id()));
        fs::write(&temporary, text)?;
        fs::rename(temporary, cached)?;
        Ok((cases, false))
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn sha256(file: &Path) -> io::Result<String> {
    let output = Command::new("/usr/bin/sha256sum")
        .arg("--")
        .arg(file)
        .output()?;
    if !output.status.success() {
        return Err(invalid("sha256 input missing/unreadable"));
    }
    let text = String::from_utf8(output.stdout).map_err(|_| invalid("invalid sha256 output"))?;
    let sha = text
        .split_whitespace()
        .next()
        .ok_or_else(|| invalid("empty sha256"))?;
    if sha.len() != 64 || !sha.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(invalid("invalid sha256"));
    }
    Ok(sha.to_owned())
}

fn json_string(text: &str) -> String {
    let mut encoded = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            c if c.is_control() => encoded.push_str(&format!("\\u{:04x}", c as u32)),
            c => encoded.push(c),
        }
    }
    encoded.push('"');
    encoded
}

#[derive(Debug)]
struct Case {
    name: String,
    function: String,
    args: Vec<Arg>,
    result: Result<Arg, Err>,
}

fn unhex(text: &str) -> io::Result<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return Err(invalid("bad hex"));
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |c: u8| (c as char).to_digit(16).ok_or_else(|| invalid("bad hex"));
            Ok((digit(pair[0])? * 16 + digit(pair[1])?) as u8)
        })
        .collect()
}

fn string(text: &str) -> io::Result<String> {
    String::from_utf8(unhex(text)?).map_err(|_| invalid("bad UTF8 symbol"))
}

struct Tokens<'a> {
    items: std::str::Split<'a, char>,
}
impl<'a> Tokens<'a> {
    fn next(&mut self) -> io::Result<&'a str> {
        self.items
            .next()
            .ok_or_else(|| invalid("truncated wire record"))
    }
    fn count(&mut self) -> io::Result<usize> {
        let n = self.next()?.parse().map_err(|_| invalid("invalid count"))?;
        if n > 1_000_000 {
            return Err(invalid("wire count too large"));
        }
        Ok(n)
    }
    fn arg(&mut self, depth: usize) -> io::Result<Arg> {
        if depth > 128 {
            return Err(invalid("wire nesting too deep"));
        }
        let token = self.next()?;
        if let Some(v) = token.strip_prefix('i') {
            return Ok(Arg::Fixnum(
                v.parse().map_err(|_| invalid("invalid fixnum"))?,
            ));
        }
        if let Some(v) = token.strip_prefix('f') {
            if v.len() != 16 {
                return Err(invalid("invalid float bits"));
            }
            return Ok(Arg::Float(
                u64::from_str_radix(v, 16).map_err(|_| invalid("invalid float bits"))?,
            ));
        }
        if let Some(v) = token.strip_prefix('y') {
            return Ok(Arg::Symbol(string(v)?));
        }
        if let Some(v) = token.strip_prefix("sm") {
            return Ok(Arg::String {
                multibyte: true,
                bytes: unhex(v)?,
            });
        }
        if let Some(v) = token.strip_prefix("su") {
            return Ok(Arg::String {
                multibyte: false,
                bytes: unhex(v)?,
            });
        }
        if token == "c" {
            return Ok(Arg::Cons(
                Box::new(self.arg(depth + 1)?),
                Box::new(self.arg(depth + 1)?),
            ));
        }
        if let Some(v) = token.strip_prefix('v') {
            let n: usize = v.parse().map_err(|_| invalid("invalid vector count"))?;
            if n > 1_000_000 {
                return Err(invalid("vector too large"));
            }
            return Ok(Arg::Vector(
                (0..n)
                    .map(|_| self.arg(depth + 1))
                    .collect::<io::Result<_>>()?,
            ));
        }
        Err(invalid("unknown wire token"))
    }
}

fn decode(text: &str) -> io::Result<Vec<Case>> {
    let mut lines = text.lines();
    if lines.next() != Some("gnudiff-1") {
        return Err(invalid("wire version"));
    }
    let mut cases = Vec::new();
    for line in lines {
        let mut t = Tokens {
            items: line.split('\t'),
        };
        if t.next()? != "case" {
            return Err(invalid("invalid wire record"));
        }
        let name = string(t.next()?)?;
        let function = string(t.next()?)?;
        let n = t.count()?;
        let args = (0..n).map(|_| t.arg(0)).collect::<io::Result<_>>()?;
        let result = match t.next()? {
            "ok" => Ok(t.arg(0)?),
            "error" => {
                let Arg::Symbol(symbol) = t.arg(0)? else {
                    return Err(invalid("error symbol"));
                };
                Err(Err {
                    symbol,
                    data: t.arg(0)?,
                })
            }
            _ => return Err(invalid("invalid outcome")),
        };
        if t.items.next().is_some() {
            return Err(invalid("trailing tokens"));
        }
        cases.push(Case {
            name,
            function,
            args,
            result,
        });
    }
    if cases.is_empty() {
        return Err(invalid("empty-selection"));
    }
    Ok(cases)
}
