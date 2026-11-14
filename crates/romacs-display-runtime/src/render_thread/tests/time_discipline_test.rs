//! Guards the injected-time discipline.
//!
//! The render thread used to call `Instant::now()` in 98 places. Anything that
//! samples a visual now takes a `FrameSample`, anything that records an
//! observation takes an `EventTime`, and anything that schedules takes a
//! `FutureDeadline` — but nothing in the type system stops the next change from
//! reaching for the wall clock again, and a clock read is easy to add and hard
//! to notice in review.
//!
//! So every remaining raw clock read says why, where it is: a `// CLOCK:
//! reason` in the comment lines directly above it. Adding a read fails until
//! it has one, which is the point: the cost of a new clock read should be
//! writing down, beside it, why it is not a frame sample. A reason that
//! outlives its read fails too.
//!
//! `.elapsed()` is checked alongside `Instant::now()` because it is the same
//! read wearing a different name, and it is the form that hid most of the
//! original sites.

use std::path::{Path, PathBuf};

/// Directories whose production sources must not read the wall clock.
const GUARDED_DIRS: [&str; 2] = ["src/render_thread", "src/core"];

/// What a raw clock read carries in the comment lines directly above it,
/// followed by what the read measures and why a frame sample would be wrong
/// for it.
const MARK: &str = "// CLOCK:";

fn crate_root() -> PathBuf {
    romacs_infra::crate_root!()
}

/// Production `.rs` files under the guarded directories, relative to the crate
/// root, with `*_test.rs` and `tests.rs` excluded.
fn guarded_files() -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let root = crate_root();
    let mut found = Vec::new();
    for dir in GUARDED_DIRS {
        walk(&root.join(dir), &mut found);
    }
    let mut rel: Vec<String> = found
        .iter()
        .filter(|p| {
            let name = p.file_name().unwrap_or_default().to_string_lossy();
            !name.ends_with("_test.rs") && name != "tests.rs"
        })
        .map(|p| {
            p.strip_prefix(&root)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    rel.sort();
    rel
}

fn is_comment(line: &str) -> bool {
    line.trim_start().starts_with("//")
}

fn reads_a_clock(line: &str) -> bool {
    !is_comment(line) && (line.contains("Instant::now()") || line.contains(".elapsed()"))
}

/// What is wrong with FILE's clock reads, given its SOURCE: a read with no
/// `// CLOCK:` in the comment lines directly above it, a `// CLOCK:` with no
/// reason, and one with no read directly below its comment.
fn problems(file: &str, source: &str) -> Vec<String> {
    let lines: Vec<&str> = source.lines().collect();
    let mut found = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if reads_a_clock(line) {
            let justified = lines[..index]
                .iter()
                .rev()
                .take_while(|above| is_comment(above))
                .any(|above| above.trim_start().starts_with(MARK));
            if !justified {
                found.push(format!(
                    "  unjustified {file}:{}  {}",
                    index + 1,
                    line.trim()
                ));
            }
        }
        if let Some(reason) = line.trim_start().strip_prefix(MARK) {
            let below = lines[index + 1..].iter().find(|below| !is_comment(below));
            if reason.trim().is_empty() {
                found.push(format!("  no reason {file}:{}", index + 1));
            } else if !below.is_some_and(|below| reads_a_clock(below)) {
                found.push(format!(
                    "  covers nothing {file}:{}  {}",
                    index + 1,
                    line.trim()
                ));
            }
        }
    }
    found
}

#[test]
fn every_raw_clock_read_in_the_render_thread_is_justified() {
    let root = crate_root();
    let found: Vec<String> = guarded_files()
        .iter()
        .filter_map(|file| {
            Some(problems(
                file,
                &std::fs::read_to_string(root.join(file)).ok()?,
            ))
        })
        .flatten()
        .collect();
    assert!(
        found.is_empty(),
        "Temporal code should take a FrameSample (visual sampling), an EventTime \
         (an observation) or a FutureDeadline (a wakeup). If a read genuinely \
         cannot be one of those, write `{MARK} reason` in the comment lines \
         directly above it: what it measures, and why a frame sample would be \
         wrong. Delete a `{MARK}` whose read is gone.\n{}",
        found.join("\n")
    );
}

#[test]
fn each_rule_of_the_discipline_fires() {
    let source = "\
fn f() {
    let a = std::time::Instant::now();
    // CLOCK: a stopwatch over CPU work.
    // It spans the frame.
    let b = a.elapsed();
    // CLOCK:
    let c = a.elapsed();
    // CLOCK: nothing reads a clock below.
    let d = 1;
    // Instant::now() in a comment is no read.
}
";
    assert_eq!(
        problems("src/x.rs", source),
        [
            "  unjustified src/x.rs:2  let a = std::time::Instant::now();",
            "  no reason src/x.rs:6",
            "  covers nothing src/x.rs:8  // CLOCK: nothing reads a clock below.",
        ]
    );
}

#[test]
fn the_guarded_directories_are_actually_being_scanned() {
    // Guards against the scan silently matching nothing — a path typo would
    // otherwise make both tests above pass vacuously forever.
    let files = guarded_files();
    assert!(
        files.len() > 20,
        "expected to scan the whole render thread, found only {}: {files:?}",
        files.len()
    );
    assert!(
        files.iter().any(|f| f.ends_with("frame_sched.rs")),
        "the scheduler should be in scope"
    );
    assert!(
        files.iter().all(|f| !f.ends_with("_test.rs")),
        "test files must be excluded"
    );
}
