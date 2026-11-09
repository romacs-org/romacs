//! Derivation of the introspection index from the artefacts it describes.
//!
//! Compiled twice: by `build.rs`, which runs it over the real tree on every
//! build whose inputs changed, and by `tests/derive.rs`, which runs it over
//! small fixtures to prove each rule. It is deliberately not part of the
//! library: the library only ever reads what this produced.
//!
//! The live registry (`data/registry.json`, written by `romacs-introspect-live`
//! from booted runtimes) is the source of truth for WHAT is registered. Source
//! is parsed here only for WHERE: registration sites, implementing functions,
//! test mentions, Lisp definitions and the divergence ledger. The two are
//! joined by the library, which reports any name one side has and the other
//! lacks instead of hiding it.

#![allow(dead_code)]

use regex::Regex;
use serde_json::{Map, Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

pub const SCHEMA: u64 = 1;

/// Source files the index reads, relative to the workspace root.
pub const CORE_SRC: &str = "crates/romacs-core/src";
pub const VM2_SRC: &str = "crates/romacs-vm/src";

// ------------------------------------------------------------------ files

pub fn rs_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(dir, &mut |p| {
        if p.extension().is_some_and(|e| e == "rs") {
            out.push(p.to_path_buf());
        }
    });
    out.sort();
    out
}

pub fn walk(dir: &Path, f: &mut dyn FnMut(&Path)) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == "target" || name.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            walk(&p, f);
        } else {
            f(&p);
        }
    }
}

/// A test-only source file. Registrations in these are fixtures, not the
/// runtime, so they never count as a registration site.
pub fn is_test_path(rel: &str) -> bool {
    rel.contains("/tests/")
        || rel.ends_with("_test.rs")
        || rel.ends_with("/tests.rs")
        || rel.ends_with("_tests.rs")
}

pub struct SourceFile {
    pub rel: String,
    pub text: String,
    line_starts: Vec<usize>,
}

impl SourceFile {
    pub fn new(rel: String, text: String) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Self {
            rel,
            text,
            line_starts,
        }
    }
    /// Is `offset` inside a `//` line comment (including doc comments)?
    pub fn in_comment(&self, offset: usize) -> bool {
        let start = self.line_starts[self.line_of(offset) - 1];
        self.text[start..offset].contains("//")
    }
    pub fn line_of(&self, offset: usize) -> usize {
        match self.line_starts.binary_search(&offset) {
            Ok(i) => i + 1,
            Err(i) => i,
        }
    }
}

pub fn load_sources(root: &Path, dir: &str) -> Vec<SourceFile> {
    rs_files(&root.join(dir))
        .into_iter()
        .filter_map(|p| {
            let rel = p
                .strip_prefix(root)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            if is_test_path(&rel) {
                return None;
            }
            let text = fs::read_to_string(&p).ok()?;
            Some(SourceFile::new(rel, text))
        })
        .collect()
}

// --------------------------------------------------------- core registrations

/// A Rust string literal holding a Lisp name, captured with its escapes.
const NAME: &str = r#""((?:[^"\\]|\\.)*)""#;
/// A Rust path to a function: `builtin_car`, `super::x`, `crate::a::b`.
const PATH: &str = r"((?:[A-Za-z_][A-Za-z0-9_]*::)*[A-Za-z_][A-Za-z0-9_]*)";

/// Every registration shape the derivation recognises, and what each yields.
/// Order matters only for reporting; a site matched by a specific shape is not
/// re-reported by the generic `SubrSpec::<ctor>` fallback.
pub struct Shape {
    pub id: &'static str,
    pub re: Regex,
}

pub static CORE_SHAPES: LazyLock<Vec<Shape>> = LazyLock::new(|| {
    let s = |id: &'static str, pat: String| Shape {
        id,
        re: Regex::new(&pat).unwrap(),
    };
    vec![
        // SubrSpec::new("car", NativeFn::Pure(builtin_car), SubrArity::new(1, Some(1)))
        s(
            "SubrSpec::new",
            format!(r"SubrSpec::new\(\s*{NAME}\s*,\s*NativeFn::(\w+)\(\s*(?:{PATH}\s*\)|(\|))"),
        ),
        // SubrSpec::fixed2("looking-at", builtin_looking_at_2, FixedMin2::One)
        s(
            "SubrSpec::fixedN",
            format!(r"SubrSpec::(fixed\d+)\(\s*{NAME}\s*,\s*{PATH}"),
        ),
        // EvaluatorSubr::special("if", SubrArity::new(2, None), SpecialFormHandler::If)
        s(
            "EvaluatorSubr",
            format!(
                r"EvaluatorSubr::(special|callable)\(\s*{NAME}\s*,\s*SubrArity::new\((?:[^()]|\([^()]*\))*\)\s*,\s*((?:SpecialFormHandler|CallableHandler)::\w+)"
            ),
        ),
        // register_pure_subr!(ctx, "take", crate::emacs_core::builtins_extra::builtin_take, 2, Some(2))
        s(
            "register_pure_subr!",
            format!(r"register_pure_subr!\(\s*\w+\s*,\s*{NAME}\s*,\s*{PATH}"),
        ),
        // Any other SubrSpec constructor that takes the name first. Catches a
        // new constructor before anyone teaches this file about it; the site is
        // found even though the implementing function is not.
        s("SubrSpec::<other>", format!(r"SubrSpec::(\w+)\(\s*{NAME}")),
    ]
});

pub fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[derive(Clone, Debug)]
pub struct CoreSite {
    pub name: String,
    pub file: String,
    pub line: usize,
    pub shape: String,
    /// `fn` path, `closure`, a handler variant, or `None` when the shape
    /// carries no parsable implementation.
    pub impl_ref: Option<String>,
    pub impl_kind: &'static str,
}

pub fn core_sites(files: &[SourceFile]) -> Vec<CoreSite> {
    let mut out = Vec::new();
    for f in files {
        let mut taken: BTreeSet<usize> = BTreeSet::new();
        for shape in CORE_SHAPES.iter() {
            for c in shape.re.captures_iter(&f.text) {
                let start = c.get(0).unwrap().start();
                // `/// SubrSpec::fixed2("bad", ...)` in a compile_fail doctest
                // registers nothing.
                if f.in_comment(start) || !taken.insert(start) {
                    continue;
                }
                let (name, impl_ref, impl_kind) = match shape.id {
                    "SubrSpec::new" => {
                        let closure = c.get(4).is_some();
                        (
                            c[1].to_string(),
                            if closure {
                                Some(closure_target(&f.text, c.get(0).unwrap().end()))
                            } else {
                                c.get(3).map(|m| m.as_str().to_string())
                            },
                            if closure { "closure" } else { "fn" },
                        )
                    }
                    "SubrSpec::fixedN" => (c[2].to_string(), Some(c[3].to_string()), "fn"),
                    "EvaluatorSubr" => (
                        c[2].to_string(),
                        Some(c[3].to_string()),
                        "evaluator-handler",
                    ),
                    "register_pure_subr!" => (c[1].to_string(), Some(c[2].to_string()), "fn"),
                    _ => {
                        // the generic fallback; `new`/`fixedN` already matched
                        // above, and `SubrSpec::evaluator(name, ..)` takes a
                        // variable rather than a literal.
                        (c[2].to_string(), None, "unparsed")
                    }
                };
                let shape_name = match shape.id {
                    "SubrSpec::fixedN" => format!("SubrSpec::{}", &c[1]),
                    "SubrSpec::<other>" => format!("SubrSpec::{}", &c[1]),
                    "EvaluatorSubr" => format!("EvaluatorSubr::{}", &c[1]),
                    id => id.to_string(),
                };
                out.push(CoreSite {
                    name: unescape(&name),
                    file: f.rel.clone(),
                    line: f.line_of(start),
                    shape: shape_name,
                    impl_ref: impl_ref.filter(|s| !s.is_empty()),
                    impl_kind,
                });
            }
        }
        // a specific shape and the generic fallback can both match one call
        // at different offsets; keep the specific one.
        let _ = &mut taken;
    }
    dedupe_sites(out)
}

fn dedupe_sites(mut sites: Vec<CoreSite>) -> Vec<CoreSite> {
    sites.sort_by(|a, b| {
        (&a.file, a.line, &a.name, a.impl_kind == "unparsed").cmp(&(
            &b.file,
            b.line,
            &b.name,
            b.impl_kind == "unparsed",
        ))
    });
    let mut out: Vec<CoreSite> = Vec::new();
    for s in sites {
        if let Some(last) = out.last()
            && last.file == s.file
            && last.line == s.line
            && last.name == s.name
        {
            continue;
        }
        out.push(s);
    }
    out
}

/// `NativeFn::Pure(|args| builtin_foo(args, x))` -> `builtin_foo`, else `closure`.
fn closure_target(text: &str, from: usize) -> String {
    static CALL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(&format!(r"^[^|]*\|\s*(?:move\s*)?\{{?\s*{PATH}\s*\(")).unwrap()
    });
    let window = &text[from..text.len().min(from + 400)];
    CALL.captures(window)
        .map(|c| c[1].to_string())
        .unwrap_or_default()
}

// ------------------------------------------------------------ fn definitions

pub static FN_DEF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?m)^[ \t]*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+"[^"]*"\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)"#,
    )
    .unwrap()
});

pub fn fn_defs(
    files: &[SourceFile],
    wanted: &BTreeSet<String>,
) -> BTreeMap<String, Vec<(String, usize)>> {
    let mut out: BTreeMap<String, Vec<(String, usize)>> = BTreeMap::new();
    for f in files {
        for c in FN_DEF.captures_iter(&f.text) {
            let id = &c[1];
            if wanted.contains(id) {
                out.entry(id.to_string())
                    .or_default()
                    .push((f.rel.clone(), f.line_of(c.get(1).unwrap().start())));
            }
        }
    }
    out
}

/// (module-path segments matched, directory depth shared with the site, same file)
type Score = (usize, usize, bool);

/// Resolve an implementing path to the `fn` that defines it.
///
/// Returns the span, the method used and every candidate considered. A path
/// that names several functions and no rule separates them is reported as
/// ambiguous with all of them, never silently resolved to the first.
pub fn resolve_fn(
    path: &str,
    site_file: &str,
    defs: &BTreeMap<String, Vec<(String, usize)>>,
) -> Json {
    let ident = path.rsplit("::").next().unwrap_or(path);
    let Some(cands) = defs.get(ident) else {
        return json!({"resolution": "unresolved", "span": null, "candidates": []});
    };
    let cand_json: Vec<Json> = cands
        .iter()
        .map(|(f, l)| json!(format!("{f}:{l}")))
        .collect();
    if cands.len() == 1 {
        let (f, l) = &cands[0];
        return json!({"resolution": "unique", "span": format!("{f}:{l}"), "candidates": cand_json});
    }
    let segs: Vec<&str> = path
        .split("::")
        .filter(|s| !matches!(*s, "crate" | "super" | "self" | "emacs_core"))
        .collect();
    let mods = &segs[..segs.len().saturating_sub(1)];
    let site_dir: Vec<&str> = site_file.split('/').collect();
    let score = |file: &str| -> Score {
        let comps: Vec<&str> = file.trim_end_matches(".rs").split('/').collect();
        let module_hits = mods.iter().filter(|m| comps.contains(m)).count();
        let shared = comps
            .iter()
            .zip(site_dir.iter())
            .take_while(|(a, b)| a == b)
            .count();
        (module_hits, shared, file == site_file)
    };
    let mut scored: Vec<(Score, &(String, usize))> =
        cands.iter().map(|c| (score(&c.0), c)).collect();
    scored.sort_by_key(|s| std::cmp::Reverse(s.0));
    // Same file wins outright when the path is bare: Rust would resolve a bare
    // ident to the local item before any import.
    if mods.is_empty() {
        let local: Vec<_> = cands.iter().filter(|c| c.0 == site_file).collect();
        if local.len() == 1 {
            let (f, l) = local[0];
            return json!({"resolution": "same-file", "span": format!("{f}:{l}"), "candidates": cand_json});
        }
    }
    let best = scored[0].0;
    if best.0 > 0
        && scored
            .iter()
            .filter(|s| s.0.0 == best.0 && s.0.1 == best.1)
            .count()
            == 1
    {
        let (f, l) = scored[0].1;
        return json!({"resolution": "module-path", "span": format!("{f}:{l}"), "candidates": cand_json});
    }
    json!({"resolution": "ambiguous", "span": null, "candidates": cand_json})
}

/// Where a handler variant such as `SpecialFormHandler::If` is dispatched.
/// Tries the qualified variant first, then the bare one (VM2 matches under
/// `use Primitive::*`). Heuristic and labelled so: the first arm naming a
/// variant may be an arity or classification table rather than its body.
pub fn first_match_arm(files: &[SourceFile], variant: &str) -> Option<String> {
    let bare = variant.rsplit("::").next().unwrap_or(variant);
    for pat in [
        regex::escape(variant),
        format!(r"\b(?:Self::)?{}\b", regex::escape(bare)),
    ] {
        let needle = Regex::new(&format!(r"{pat}\s*(?:=>|\|)")).unwrap();
        for f in files {
            if let Some(m) = needle.find(&f.text) {
                return Some(format!("{}:{}", f.rel, f.line_of(m.start())));
            }
        }
    }
    None
}

// ---------------------------------------------------------------- VM2 table

#[derive(Clone, Debug)]
pub struct Vm2Site {
    pub name: String,
    pub file: String,
    pub line: usize,
    pub variant: String,
    /// `primitive-table`, `defsubr`, `direct-slot`, or `lisp-definition` (a Lisp body
    /// VM2 embeds in Rust and installs on first use).
    pub kind: &'static str,
}

/// Entries of VM2's `const PRIMITIVES` table, plus any `vm.slot("x")` whose
/// function cell is assigned a `Primitive::` directly (that is how `not` is
/// registered).
pub fn vm2_sites(files: &[SourceFile]) -> Vec<Vm2Site> {
    static ENTRY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(&format!(r"\(\s*{NAME}\s*,\s*(Primitive::\w+)\s*,?\s*\)")).unwrap()
    });
    static TABLE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"const PRIMITIVES:\s*&\[\(&str,\s*Primitive\)\]\s*=\s*&\[").unwrap()
    });
    static DIRECT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(&format!(
            r"let\s+(\w+)\s*=\s*\w+\.slot\(\s*{NAME}\s*\)\s*;\s*\w+\.slots\[\s*(\w+)\s*\]\.function\s*=\s*Value::reference\(\s*(Primitive::\w+)"
        ))
        .unwrap()
    });
    // `"kill-region" => serde_json::json!([...])`: a Lisp definition VM2
    // carries as data and installs when the name is first called.
    static LISP_BODY: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(&format!(r"{NAME}\s*=>\s*serde_json::json!\(\s*\[")).unwrap());
    static DEFSUBR: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(&format!(r#"defsubr\s*!\s*[{{(\[]\s*name\s*=\s*{NAME}\s*,\s*gnu\s*=\s*"(?:\\.|[^"\\])*"\s*,\s*effect\s*=\s*\w+\s*,\s*fn\s+(\w+)\s*\("#)).unwrap()
    });
    let mut out = Vec::new();
    for f in files {
        if is_test_path(&f.rel) {
            continue;
        }
        for c in DEFSUBR.captures_iter(&f.text) {
            let at = c.get(0).unwrap().start();
            // Only src/prims is scanned by VM2's generator. Macro examples
            // and adapter fixtures elsewhere (notably prim.rs) are not entries.
            if f.rel.starts_with("crates/romacs-vm/src/prims/") && !f.in_comment(at) {
                out.push(Vm2Site {
                    name: unescape(&c[1]),
                    file: f.rel.clone(),
                    line: f.line_of(at),
                    variant: c[2].to_string(),
                    kind: "defsubr",
                });
            }
        }
        if let Some(t) = TABLE.find(&f.text) {
            let end = f.text[t.end()..]
                .find("\n];")
                .map(|e| t.end() + e)
                .unwrap_or(f.text.len());
            for c in ENTRY.captures_iter(&f.text[t.end()..end]) {
                let at = t.end() + c.get(0).unwrap().start();
                if f.in_comment(at) {
                    continue;
                }
                out.push(Vm2Site {
                    name: unescape(&c[1]),
                    file: f.rel.clone(),
                    line: f.line_of(at),
                    variant: c[2].to_string(),
                    kind: "primitive-table",
                });
            }
        }
        for c in DIRECT.captures_iter(&f.text) {
            if c[1] == c[3] && !f.in_comment(c.get(0).unwrap().start()) {
                out.push(Vm2Site {
                    name: unescape(&c[2]),
                    file: f.rel.clone(),
                    line: f.line_of(c.get(0).unwrap().start()),
                    variant: c[4].to_string(),
                    kind: "direct-slot",
                });
            }
        }
        for c in LISP_BODY.captures_iter(&f.text) {
            if f.in_comment(c.get(0).unwrap().start()) {
                continue;
            }
            out.push(Vm2Site {
                name: unescape(&c[1]),
                file: f.rel.clone(),
                line: f.line_of(c.get(0).unwrap().start()),
                variant: String::new(),
                kind: "lisp-definition",
            });
        }
    }
    out
}

/// Every string literal in VM2's source that could be a Lisp name. This is the
/// universe the live VM2 probe tries; see `romacs-introspect-live`.
pub fn vm2_literal_universe(files: &[SourceFile]) -> BTreeSet<String> {
    static LIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(NAME).unwrap());
    let mut out = BTreeSet::new();
    for f in files {
        for c in LIT.captures_iter(&f.text) {
            let s = unescape(&c[1]);
            if !s.is_empty()
                && s.len() <= 80
                && !s
                    .chars()
                    .any(|ch| ch.is_whitespace() || "()\"';`,".contains(ch))
            {
                out.insert(s);
            }
        }
    }
    out
}

/// First occurrence of `"name"` in the given sources: a lead offered when a
/// registered name has no recognised registration site.
pub fn literal_lead(files: &[SourceFile], name: &str) -> Option<String> {
    let needle = format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""));
    for f in files {
        if let Some(i) = f.text.find(&needle) {
            return Some(format!("{}:{}", f.rel, f.line_of(i)));
        }
    }
    None
}

// ------------------------------------------------------------- test mentions

/// Files that are tests, wherever they live. Retired oracle archives are inert.
pub fn test_files(root: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut add = |p: &Path| {
        let rel = p
            .strip_prefix(root)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/");
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        let is_test = match ext {
            "rs" => is_test_path(&rel),
            "el" => rel.starts_with("test/"),
            _ => false,
        };
        if is_test {
            out.push((rel, p.to_path_buf()));
        }
    };
    walk(&root.join("crates"), &mut add);
    walk(&root.join("test"), &mut add);
    out.sort();
    out
}

/// `(name` and `#'name` occurrences: the lexical mentions of a Lisp function.
pub fn mentions(text: &str, into: &mut BTreeSet<String>) {
    static CALL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?:\(|#')([^\s()\[\]"';`,#?][^\s()\[\]"';`,]*)"#).unwrap());
    for c in CALL.captures_iter(text) {
        let s = c[1].trim_end_matches('\\');
        into.insert(s.to_string());
    }
}

// ----------------------------------------------------------- Lisp definitions

/// name -> (index into the file list, line, defining form)
pub type LispDefs = BTreeMap<String, (usize, usize, String)>;

/// `(defun NAME`, `(defmacro NAME`, `(defsubst NAME`, `(defalias 'NAME`,
/// `(define-inline NAME`, `(cl-defun NAME`, `(cl-defgeneric NAME`.
pub fn lisp_defs(root: &Path) -> (Vec<String>, LispDefs) {
    static DEF: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?m)^\s*\((defun|defmacro|defsubst|define-inline|cl-defun|cl-defmacro|cl-defgeneric|defalias|define-obsolete-function-alias)\s+'?([^\s()]+)").unwrap()
    });
    let mut files = Vec::new();
    let mut defs: LispDefs = BTreeMap::new();
    let mut paths = Vec::new();
    walk(&root.join("gnu-lisp"), &mut |p| {
        if p.extension().is_some_and(|e| e == "el") {
            paths.push(p.to_path_buf());
        }
    });
    paths.sort();
    for p in paths {
        let Ok(text) = fs::read_to_string(&p) else {
            continue;
        };
        let rel = p
            .strip_prefix(root)
            .unwrap_or(&p)
            .to_string_lossy()
            .replace('\\', "/");
        let sf = SourceFile::new(rel.clone(), text);
        let idx = files.len();
        let mut any = false;
        for c in DEF.captures_iter(&sf.text) {
            let name = c[2].trim_start_matches("#'").to_string();
            if name.is_empty() {
                continue;
            }
            any = true;
            defs.entry(name).or_insert((
                idx,
                sf.line_of(c.get(1).unwrap().start()),
                c[1].to_string(),
            ));
        }
        // `idx` was taken as files.len() above, so pushing now gives this
        // file exactly that index; files with no definition are never pushed
        // and never referenced.
        if any {
            files.push(rel);
        }
    }
    (files, defs)
}

// ------------------------------------------------------------- divergences

/// Port of `tools/lib/divergence_index.py`: rows of the prose ledgers, keyed by the
/// symbols they affect. The prose stays canonical; this has no content of its
/// own.
pub static NOT_A_SYMBOL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)^(?: .*\.(?:c|rs|el|md|json)(?::\d+(?:[-:]\d+)?)?
              | romacs(?:-[a-z0-9]+)*
              | [A-Z][A-Za-z0-9_]*
              | :[a-z-]+
              | \{.*\}
              | [0-9^*>=\ ]+
              | .*::.*
              | .*\(\)
              | [-%].*
              | .*[A-Z].*
              )$",
    )
    .unwrap()
});

pub fn symbols_in(cells: &[&str], override_: Option<&str>) -> Vec<String> {
    static TICK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`]+)`").unwrap());
    static REQ: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\(require '([a-z0-9-]+)\)$").unwrap());
    static FQ: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#'([a-z0-9-]+)$").unwrap());
    if let Some(o) = override_ {
        return o
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
    }
    let joined = cells.join(" ");
    let mut out: Vec<String> = Vec::new();
    for c in TICK.captures_iter(&joined) {
        let mut tok = c[1].trim().to_string();
        if let Some(m) = REQ.captures(&tok).or_else(|| FQ.captures(&tok)) {
            tok = m[1].to_string();
        }
        if NOT_A_SYMBOL.is_match(&tok) || tok.contains(' ') {
            continue;
        }
        if !out.contains(&tok) {
            out.push(tok);
        }
    }
    out
}

pub fn parse_ledger(text: &str, source: &str) -> Vec<Json> {
    static HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^##+\s+(.*)").unwrap());
    static SEP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\|[\s:|-]+\|$").unwrap());
    static NOTDIV: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"<!--\s*not-a-divergence\s*-->").unwrap());
    static SYMS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"<!--\s*symbols:\s*([^>]*?)\s*-->").unwrap());
    let mut rows = Vec::new();
    let mut section: Option<String> = None;
    for (i, raw) in text.split('\n').enumerate() {
        if let Some(h) = HEAD.captures(raw) {
            section = Some(h[1].trim().to_string());
            continue;
        }
        if !raw.starts_with("| ") || SEP.is_match(raw) {
            continue;
        }
        if NOTDIV.is_match(raw) {
            continue;
        }
        let mut line = raw.to_string();
        let mut override_ = None;
        if let Some(m) = SYMS.captures(raw) {
            override_ = Some(m[1].to_string());
            line = raw[..m.get(0).unwrap().start()].to_string();
        }
        let trimmed = line.trim().trim_matches('|');
        let cells: Vec<&str> = trimmed.split('|').map(str::trim).collect();
        if cells.len() < 4 {
            continue;
        }
        let ident = cells[0];
        if matches!(
            ident.to_lowercase().as_str(),
            "#" | "crate" | "mechanism" | "what" | "lane"
        ) {
            continue;
        }
        let sec = section.clone();
        let observable = sec
            .as_deref()
            .is_some_and(|s| s.contains("Observable by a package"));
        let planned = sec.as_deref().is_some_and(|s| s.contains("Planned"));
        rows.push(json!({
            "id": if matches!(ident, "—" | "-" | "") { Json::Null } else { json!(ident) },
            "source": source,
            "line": i + 1,
            "section": sec,
            "observable": observable,
            "planned": planned,
            "mechanism": cells[1],
            "gnu": cells[2],
            "romacs": cells[3],
            "why": cells.get(4).copied().unwrap_or(""),
            "symbols": symbols_in(&cells[1..4], override_.as_deref()),
            "symbols_derived": override_.is_none(),
        }));
    }
    rows
}

pub fn ledgers(root: &Path) -> Vec<(String, PathBuf)> {
    let mut out = vec![("DIVERGENCES.md".to_string(), root.join("DIVERGENCES.md"))];
    let mut crates: Vec<_> = fs::read_dir(root.join("crates"))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path().join("GNU-DIVERGENCES.md"))
                .filter(|p| p.is_file())
                .collect()
        })
        .unwrap_or_default();
    crates.sort();
    for p in crates {
        let rel = p
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        out.push((rel, p));
    }
    out
}

// ---------------------------------------------------------------- assembly

/// Names the live registry knows, from `data/registry.json`.
pub fn registry_names(registry: &Json) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for e in registry["core"]["entries"].as_array().into_iter().flatten() {
        if let Some(n) = e["name"].as_str() {
            out.insert(n.to_string());
        }
    }
    for e in registry["vm2"]["entries"].as_array().into_iter().flatten() {
        if let Some(n) = e.as_str() {
            out.insert(n.to_string());
        }
    }
    out
}

/// Derive everything except provenance. `registry` is the live snapshot.
pub fn derive(root: &Path, registry: &Json) -> Json {
    let core = load_sources(root, CORE_SRC);
    let vm2 = load_sources(root, VM2_SRC);

    // --- core registration sites and their implementing functions
    let sites = core_sites(&core);
    let wanted: BTreeSet<String> = sites
        .iter()
        .filter(|s| s.impl_kind == "fn" || s.impl_kind == "closure")
        .filter_map(|s| s.impl_ref.as_ref())
        .map(|p| p.rsplit("::").next().unwrap_or(p).to_string())
        .collect();
    let defs = fn_defs(&core, &wanted);
    let mut core_sites_json: BTreeMap<String, Vec<Json>> = BTreeMap::new();
    for s in &sites {
        let implementation = match (s.impl_kind, &s.impl_ref) {
            ("fn", Some(p)) | ("closure", Some(p)) => {
                let mut r = resolve_fn(p, &s.file, &defs);
                r["path"] = json!(p);
                r["kind"] = json!(s.impl_kind);
                r
            }
            ("evaluator-handler", Some(v)) => json!({
                "path": v, "kind": "evaluator-handler",
                "resolution": if first_match_arm(&core, v).is_some() { "first-match-arm" } else { "unresolved" },
                "span": first_match_arm(&core, v), "candidates": []
            }),
            (kind, _) => {
                json!({"path": null, "kind": kind, "resolution": "unparsed", "span": null, "candidates": []})
            }
        };
        core_sites_json
            .entry(s.name.clone())
            .or_default()
            .push(json!({
                "site": format!("{}:{}", s.file, s.line),
                "shape": s.shape,
                "implementation": implementation,
            }));
    }

    // --- VM2
    let vsites = vm2_sites(&vm2);
    let mut vm2_json: BTreeMap<String, Vec<Json>> = BTreeMap::new();
    for s in &vsites {
        let arm = if s.variant.is_empty() {
            None
        } else {
            first_match_arm(&vm2, &s.variant)
        };
        vm2_json.entry(s.name.clone()).or_default().push(json!({
            "site": format!("{}:{}", s.file, s.line),
            "variant": s.variant,
            "kind": s.kind,
            "handler": arm,
            "handler_resolution": if s.variant.is_empty() { "the site is the definition" } else { "first-match-arm" },
        }));
    }

    // --- leads for registered names the shapes above did not find
    let live = registry_names(registry);
    let mut leads = Map::new();
    for n in &live {
        if !core_sites_json.contains_key(n) && !vm2_json.contains_key(n) {
            let lead = literal_lead(&core, n).or_else(|| literal_lead(&vm2, n));
            leads.insert(n.clone(), json!(lead));
        }
    }

    // --- tests: which registered names any test file mentions
    let tests = test_files(root);
    let interesting: BTreeSet<String> = live
        .iter()
        .cloned()
        .chain(core_sites_json.keys().cloned())
        .chain(vm2_json.keys().cloned())
        .collect();
    let mut test_hits: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();
    let mut scanned = 0usize;
    for (rel, p) in &tests {
        let Ok(text) = fs::read(p) else { continue };
        scanned += 1;
        let text = String::from_utf8_lossy(&text);
        let mut seen = BTreeSet::new();
        mentions(&text, &mut seen);
        for n in seen.intersection(&interesting) {
            let e = test_hits.entry(n.clone()).or_default();
            e.0 += 1;
            if e.1.len() < 3 {
                e.1.push(rel.clone());
            }
        }
    }
    let test_json: Map<String, Json> = test_hits
        .into_iter()
        .map(|(n, (count, ex))| (n, json!({"files": count, "examples": ex})))
        .collect();

    // --- Lisp definitions, for honest negatives on names that are not primitives
    let (lisp_files, lisp) = lisp_defs(root);
    let lisp_json: Map<String, Json> = lisp
        .into_iter()
        .map(|(n, (f, l, kind))| (n, json!([f, l, kind])))
        .collect();

    // --- divergences
    let mut rows = Vec::new();
    let mut ledger_sources = Vec::new();
    for (rel, p) in ledgers(root) {
        if let Ok(text) = fs::read_to_string(&p) {
            rows.extend(parse_ledger(&text, &rel));
            ledger_sources.push(rel);
        }
    }
    let mut by_symbol: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, r) in rows.iter().enumerate() {
        for s in r["symbols"].as_array().into_iter().flatten() {
            by_symbol
                .entry(s.as_str().unwrap().to_string())
                .or_default()
                .push(i);
        }
    }

    let source_core: BTreeSet<&String> = sites.iter().map(|s| &s.name).collect();
    let source_vm2: BTreeSet<&String> = vsites.iter().map(|s| &s.name).collect();
    json!({
        "schema": SCHEMA,
        "core_sites": core_sites_json,
        "vm2_sites": vm2_json,
        "source_registered": {"core": source_core, "vm2": source_vm2},
        "shapes": CORE_SHAPES.iter().map(|s| s.id).collect::<Vec<_>>(),
        "leads": leads,
        "tests": {
            "universe": {"files": scanned, "rule": "Rust files under a tests/ directory or named *_test.rs, and test/**/*.el"},
            "mentions": test_json,
        },
        "lisp": {"files": lisp_files, "defs": lisp_json},
        "divergences": {"sources": ledger_sources, "rows": rows, "by_symbol": by_symbol},
        "source_counts": {
            "core_files": core.len(),
            "vm2_files": vm2.len(),
            "core_sites": sites.len(),
            "vm2_sites": vsites.len(),
        },
    })
}
