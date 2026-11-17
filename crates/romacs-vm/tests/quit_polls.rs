//! Holds each long native loop to the quit rule (AGENTS.md: a Rust loop that
//! quit cannot reach is a stall): it polls (`Vm::native_poll`, or the probe
//! `NativeStop::due` a loop without the VM asks), or says why it need not,
//! with `// NOPOLL: BOUND` in the comment lines directly above its loop.
//!
//! Which loops are long is decided by measurement, not by reading: a loop
//! is long once the safepoint-gap probe (feature `safepoint-gap-probe`,
//! `crates/romacs-vm/src/gap_probe.rs`) charged a span over 50 ms to its
//! primitive on the corpus workloads with 10-100 MB inputs, or once it was
//! known to be (`LONG`'s comments say which). Each is named here by its file
//! and the text of the function holding it, and that function's body must
//! poll or carry a `// NOPOLL:`. A new long loop is found by running the
//! probe again; this test keeps the known ones from losing their poll.
//!
//! Every `// NOPOLL:` in the crate's sources names a bound and stands above
//! a loop, so an annotation cannot outlive its loop.

use std::path::{Path, PathBuf};

/// The long loops: a file under `crates/romacs-vm/src` and the text that
/// opens the function holding the loop.
const LONG: &[(&str, &str)] = &[
    // Named in the design (l1/DESIGN.md §4) and measured by the probe.
    ("syntax_scan.rs", "fn run(&mut self, state: &mut ParseState"),
    ("syntax_scan.rs", "fn forw_run(&self"),
    ("syntax_scan.rs", "fn back_comment(&mut self"),
    ("syntax_scan.rs", "fn find_defun_start(&mut self"),
    ("syntax_scan/lists.rs", "fn comments_forward(&mut self"),
    ("syntax_scan/lists.rs", "fn comments_backward(&mut self"),
    ("syntax_scan/lists.rs", "pub(crate) fn backward_prefix_chars(&mut self"),
    ("syntax_scan/lists.rs", "pub(crate) fn lists(&mut self"),
    ("prim/leaf/json.rs", "fn parse(&mut self) -> Parsed<Value>"),
    ("prim/leaf/json.rs", "fn string(&mut self) -> Parsed<Vec<u8>>"),
    ("prim/leaf/json.rs", "fn json_out(&mut self"),
    ("digest.rs", "pub(crate) fn digest_until("),
    ("digest.rs", "fn sha3("),
    ("prims/leaf_sequences.rs", "fn sort_native("),
    ("sqlite.rs", "pub(crate) fn stoppable<T>("),
    ("prim/leaf/sqlite.rs", "pub(crate) fn sqlite_execute("),
    ("prim/leaf/sqlite.rs", "pub(crate) fn sqlite_select("),
    ("treesit/native.rs", "pub(crate) fn parse_until("),
    ("treesit/native.rs", "pub(crate) fn matches("),
    ("prim/leaf/process.rs", "fn call_process_wait("),
    // Measured by the probe (a3/GAPS.md).
    ("prim/leaf/coding.rs", "fn coding_convert(&mut self"),
    ("prim/leaf/casing.rs", "pub(crate) fn case_mappings("),
    ("prim/leaf/casing.rs", "pub(crate) fn casify_region_edit("),
    ("prim/leaf/subst_char.rs", "pub fn subst_char_hits("),
    ("prim/leaf/subst_char.rs", "pub fn subst_char_write("),
    ("prim/leaf/width.rs", "pub(crate) fn string_width("),
    ("prim/leaf/skip_chars.rs", "pub fn skip_characters("),
    ("prims/fns_distance.rs", "fn string_distance(cx"),
    ("prims/compare_buffer.rs", "fn compare_buffer_substrings(cx"),
    ("annotation_primitives.rs", "fn property_single_change("),
];

/// What a poll looks like in a loop's function.
const POLLS: &[&str] = &["native_poll", ".due()", "stop_due()", "self.poll()", "pass.", "stop()"];

const MARK: &str = "// NOPOLL:";

fn src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// The body of the function SOURCE opens at OPENER: from its first `{` to
/// the brace that closes it, comments and strings counted as code (the
/// files named here keep their braces balanced in both).
fn body<'a>(source: &'a str, opener: &str) -> Option<&'a str> {
    let at = source.find(opener)?;
    let open = at + source[at..].find('{')?;
    let mut depth = 0usize;
    for (i, c) in source[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 { return Some(&source[at..=open + i]); }
            }
            _ => {}
        }
    }
    None
}

#[test]
fn every_long_loop_polls_or_names_its_bound() {
    let mut wrong = Vec::new();
    for &(file, opener) in LONG {
        let Ok(source) = std::fs::read_to_string(src().join(file)) else {
            wrong.push(format!("{file}: no such file"));
            continue;
        };
        if source.matches(opener).count() != 1 {
            wrong.push(format!("{file}: `{opener}` opens {} functions, not one", source.matches(opener).count()));
            continue;
        }
        let Some(body) = body(&source, opener) else {
            wrong.push(format!("{file}: `{opener}` has no body"));
            continue;
        };
        if !POLLS.iter().any(|poll| body.contains(poll)) && !body.contains(MARK) {
            wrong.push(format!("{file}: `{opener}` neither polls nor says `{MARK} BOUND`"));
        }
    }
    assert!(wrong.is_empty(), "long loops off the quit rule:\n{}", wrong.join("\n"));
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("a source directory").flatten() {
        let path = entry.path();
        if path.is_dir() { rust_files(&path, out); } else if path.extension().is_some_and(|e| e == "rs") { out.push(path); }
    }
}

#[test]
fn every_nopoll_names_a_bound_above_a_loop() {
    let mut files = Vec::new();
    rust_files(&src(), &mut files);
    let mut wrong = Vec::new();
    for path in files {
        let source = std::fs::read_to_string(&path).expect("a source file");
        let lines: Vec<&str> = source.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let Some(bound) = line.trim_start().strip_prefix(MARK) else { continue };
            let place = format!("{}: `{}`", path.strip_prefix(src()).unwrap_or(&path).display(), line.trim());
            if bound.trim().is_empty() { wrong.push(format!("{place}: no bound")); }
            // The first line past the comment run is the loop it stands over.
            let next = lines[i + 1..].iter().map(|l| l.trim_start()).find(|l| !l.starts_with("//"));
            let looped = next.is_some_and(|l| ["loop", "while ", "for "].iter().any(|k| l.starts_with(k)));
            if !looped { wrong.push(format!("{place}: stands above no loop")); }
        }
    }
    assert!(wrong.is_empty(), "stale or empty NOPOLL annotations:\n{}", wrong.join("\n"));
}
