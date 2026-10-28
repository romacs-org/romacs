//! The GC roots rule over native reentry (design note A10): no heap Value
//! stays in a Rust local across a call that can run Lisp or collect. Every
//! production call site of such a call is listed, with how its frame keeps
//! that rule, in `fixtures/reentry-sites.txt`. A new site, a site whose call
//! count changed, or a listed site that no longer exists fails here until the
//! fixture classifies it again.
use std::collections::BTreeMap;
use std::path::Path;

/// Calls that can run Lisp (and so collect) or collect directly.
const ENTRIES: &[&str] = &[
    "execute_raw", "execute_rooted", "execute_args", "execute", "reader_call", "eval_form_raw",
    "eval_source_raw", "load_file_raw", "autoload_do_load", "invoke_prepared_raw",
    "call_macro_expander", "expand_macro", "notify_watchers", "notify_watchers_at",
    "call_lisp_watcher", "set_variable_value", "bind_variable", "unbind_one", "unwind", "unbind",
    "unbind_after", "unbind_forwarded", "checked_unbind", "loader_primitive", "eval_primitive",
    "macroexpand_primitive", "macroexpand_value", "read_host_event", "force_lazy", "autoload_then",
    "reentrant_load", "signal_hook", "handler_callback", "load_with_autoload_queue",
    "eval_load_toplevel", "compile", "compile_request", "compile_lazy_request", "cached_source_form",
    "finish_request", "poll_collect", "collect_garbage", "watched_write", "call_suspended",
    "load_source_named", "variable_set", "notify",
];

const CLASSES: &[&str] = &["suspended", "rooted", "clean", "held", "boundary", "collector"];

type Site = (String, String, String);

/// Line ranges of `#[cfg(test)]` items, by brace depth.
fn test_ranges(lines: &[&str]) -> Vec<(usize, usize)> {
    let mut ranges = vec![];
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim() == "#[cfg(test)]" {
            let mut j = i + 1;
            while j < lines.len() && lines[j].trim().starts_with("#[") {
                j += 1;
            }
            if j < lines.len() && lines[j].contains('{') && !lines[j].trim_end().ends_with(';') {
                let mut depth = 0i64;
                let mut k = j;
                while k < lines.len() {
                    depth += lines[k].matches('{').count() as i64 - lines[k].matches('}').count() as i64;
                    if depth <= 0 {
                        break;
                    }
                    k += 1;
                }
                ranges.push((i, k));
                i = k + 1;
                continue;
            }
        }
        i += 1;
    }
    ranges
}

/// The function a line declares, if any.
fn declared_fn(line: &str) -> Option<&str> {
    let mut rest = line.trim_start();
    for prefix in ["pub(crate) ", "pub(super) ", "pub ", "const ", "unsafe ", "extern \"C\" "] {
        rest = rest.strip_prefix(prefix).unwrap_or(rest);
    }
    let rest = rest.strip_prefix("fn ")?;
    let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
    Some(&rest[..end])
}

/// Calls of ENTRIES in CODE: `NAME(` not preceded by an identifier
/// character or `fn `, and `.collect(&` (a collection, not an iterator's).
fn calls(code: &str) -> Vec<&'static str> {
    let mut found = vec![];
    for &name in ENTRIES {
        let pattern = format!("{name}(");
        for (at, _) in code.match_indices(&pattern) {
            let before = &code[..at];
            if before.ends_with(|c: char| c.is_alphanumeric() || c == '_') || before.ends_with("fn ") {
                continue;
            }
            found.push(name);
        }
    }
    found.extend(code.matches(".collect(&").map(|_| "collect"));
    found
}

fn scan(dir: &Path, root: &Path, sites: &mut BTreeMap<Site, usize>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    entries.sort();
    for path in entries {
        let relative = path.strip_prefix(root).unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            if relative != "bin" {
                scan(&path, root, sites);
            }
            continue;
        }
        if !relative.ends_with(".rs") || relative.ends_with("_tests.rs") || relative == "main.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let tests = test_ranges(&lines);
        let mut function = "<top>".to_string();
        for (i, line) in lines.iter().enumerate() {
            if tests.iter().any(|&(a, b)| (a..=b).contains(&i)) {
                continue;
            }
            if let Some(name) = declared_fn(line) {
                function = name.to_string();
                continue;
            }
            let code = line.split("//").next().unwrap();
            for callee in calls(code) {
                *sites.entry((relative.clone(), function.clone(), callee.to_string())).or_default() += 1;
            }
        }
    }
}

#[test]
fn every_reentry_call_site_is_classified() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = manifest.join("src");
    let mut found = BTreeMap::new();
    scan(&src, &src, &mut found);
    let fixture = std::fs::read_to_string(manifest.join("tests/fixtures/reentry-sites.txt")).unwrap();
    let mut listed = BTreeMap::new();
    let mut problems = vec![];
    for line in fixture.lines() {
        let entry = line.split('#').next().unwrap().trim();
        if entry.is_empty() {
            continue;
        }
        let fields: Vec<&str> = entry.split_whitespace().collect();
        let [file, function, callee, count, class] = fields[..] else {
            problems.push(format!("malformed: {line}"));
            continue;
        };
        if !CLASSES.contains(&class) {
            problems.push(format!("unknown class {class}: {line}"));
        }
        if !line.contains('#') {
            problems.push(format!("no reason given: {line}"));
        }
        listed.insert((file.to_string(), function.to_string(), callee.to_string()), count.parse::<usize>().unwrap_or(0));
    }
    for (site, count) in &found {
        match listed.get(site) {
            Some(n) if n == count => {}
            Some(n) => problems.push(format!("{} {} {}: {count} calls, fixture says {n}", site.0, site.1, site.2)),
            None => problems.push(format!("unclassified: {} {} {} {count}", site.0, site.1, site.2)),
        }
    }
    for site in listed.keys().filter(|site| !found.contains_key(*site)) {
        problems.push(format!("no longer present: {} {} {}", site.0, site.1, site.2));
    }
    assert!(found.len() > 100, "the scan found {} sites", found.len());
    assert!(problems.is_empty(), "classify each site in tests/fixtures/reentry-sites.txt:\n{}", problems.join("\n"));
}
