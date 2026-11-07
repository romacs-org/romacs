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
    "call_lisp_at_write", "safe_call", "set_variable_value", "bind_variable", "unbind_one", "unwind", "unbind",
    "unbind_after", "unbind_forwarded", "checked_unbind", "loader_primitive", "eval_primitive",
    "macroexpand_primitive", "macroexpand_value", "read_host_event", "force_lazy", "autoload_then",
    "lambda_list_then", "lambda_list_raise", "lambda_list_closure", "nested_then", "bind_special",
    "reentrant_load", "reentrant_step", "signal_hook", "call_debugger", "call_in_signal", "handler_callback", "finish_collection", "nested_with_room", "load_with_autoload_queue",
    "eval_load_toplevel", "compile", "compile_request", "compile_lazy_request", "cached_source_form",
    "finish_request", "poll_collect", "poll_collect_unless_key", "collect_garbage", "collect_garbage_until_key", "collect_compacting",
    "collect_until_key", "watched_write", "call_suspended",
    "load_source_named", "variable_set", "notify", "prepared_function",
    "prepare_source", "call_prepared_source_raw", "eval_datum_raw", "cached_datum", "compile_guarded",
    "load_file_kind", "load_artifact", "compile_walk", "compile_unit",
];

const CLASSES: &[&str] = &["suspended", "rooted", "clean", "held", "boundary", "collector"];

type Site = (String, String, String);

#[derive(Debug, PartialEq)]
enum Token {
    Ident(String),
    Punct(char),
}

/// Rust tokens with comments, strings and character literals removed:
/// identifiers and single punctuation characters, which is all the scan
/// needs to see calls, declarations and braces.
fn tokens(source: &str) -> Vec<Token> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    let at = |i: usize| chars.get(i).copied().unwrap_or('\0');
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '/' && at(i + 1) == '/' {
            while i < chars.len() && chars[i] != '\n' { i += 1; }
        } else if c == '/' && at(i + 1) == '*' {
            let mut depth = 0;
            loop {
                if at(i) == '/' && at(i + 1) == '*' { depth += 1; i += 2; }
                else if at(i) == '*' && at(i + 1) == '/' { depth -= 1; i += 2; if depth == 0 { break; } }
                else if i >= chars.len() { break; }
                else { i += 1; }
            }
        } else if (c == 'r' || (c == 'b' && at(i + 1) == 'r')) && {
            let j = i + if c == 'b' { 2 } else { 1 };
            let mut k = j;
            while at(k) == '#' { k += 1; }
            at(k) == '"' && (k > j || at(j) == '"')
        } {
            // Raw string: r"..", r#".."#, br#".."#.
            i += if c == 'b' { 2 } else { 1 };
            let mut hashes = 0;
            while at(i) == '#' { hashes += 1; i += 1; }
            i += 1;
            while i < chars.len() && !(chars[i] == '"' && (1..=hashes).all(|h| at(i + h) == '#')) { i += 1; }
            i += 1 + hashes;
        } else if c == '"' || (c == 'b' && at(i + 1) == '"') {
            i += if c == 'b' { 2 } else { 1 };
            while i < chars.len() && chars[i] != '"' {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            i += 1;
        } else if c == '\'' || (c == 'b' && at(i + 1) == '\'') {
            // A character literal closes within a few characters; a lifetime
            // or label does not, and is skipped as a quote and an identifier.
            let j = i + if c == 'b' { 2 } else { 1 };
            let close = if at(j) == '\\' { (j + 2..j + 12).find(|&k| at(k) == '\'') } else { (at(j + 1) == '\'').then_some(j + 1) };
            i = close.map_or(j, |k| k + 1);
        } else if c.is_alphanumeric() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') { i += 1; }
            out.push(Token::Ident(chars[start..i].iter().collect()));
        } else {
            out.push(Token::Punct(c));
            i += 1;
        }
    }
    out
}

/// Calls of ENTRIES in SOURCE as (enclosing function, callee): a name
/// followed by `(` that is not being declared, and `.collect(&`, a
/// collection rather than an iterator's. Items under `#[cfg(test)]` are
/// skipped; a nested function's calls are its own.
fn calls(source: &str) -> Vec<(String, String)> {
    use Token::{Ident, Punct};
    let toks = tokens(source);
    let ident = |i: usize| match toks.get(i) { Some(Ident(name)) => Some(name.as_str()), _ => None };
    let punct = |i: usize, c: char| toks.get(i) == Some(&Punct(c));
    // Past `::<..>' explicit generic arguments at I, balanced; `->' inside
    // them does not close one.
    let after_generics = |i: usize| {
        if !(punct(i, ':') && punct(i + 1, ':') && punct(i + 2, '<')) {
            return i;
        }
        let mut j = i + 2;
        let mut angles = 0usize;
        while j < toks.len() {
            if punct(j, '<') { angles += 1; }
            if punct(j, '>') && !punct(j - 1, '-') {
                angles -= 1;
                if angles == 0 { return j + 1; }
            }
            j += 1;
        }
        j
    };
    let mut found = vec![];
    let mut functions: Vec<(String, usize)> = vec![];
    let mut pending: Option<String> = None;
    let mut depth = 0usize;
    let mut i = 0;
    while i < toks.len() {
        // #[cfg(test)] ITEM: skip through its body, or to the `;' or `,'
        // ending it; a field or arm may instead end at its enclosing `}'.
        if punct(i, '#') && punct(i + 1, '[') && ident(i + 2) == Some("cfg") && punct(i + 3, '(') && ident(i + 4) == Some("test") && punct(i + 5, ')') && punct(i + 6, ']') {
            let mut j = i + 7;
            let mut inner = 0usize;
            while j < toks.len() {
                if punct(j, '}') && inner == 0 { j -= 1; break; }
                if punct(j, '{') { inner += 1; }
                if punct(j, '}') { inner -= 1; if inner == 0 { break; } }
                if (punct(j, ';') || punct(j, ',')) && inner == 0 { break; }
                j += 1;
            }
            i = j + 1;
            continue;
        }
        match &toks[i] {
            Ident(word) if word == "fn" => {
                pending = ident(i + 1).map(str::to_owned);
                // Past the parameter list, whose `;' (defsubr! syntax) does
                // not end a body-less declaration.
                let mut j = i + 2;
                while j < toks.len() && !punct(j, '(') && !punct(j, '{') && !punct(j, ';') { j += 1; }
                let mut parens = 0usize;
                while j < toks.len() && (punct(j, '(') || parens > 0) {
                    if punct(j, '(') { parens += 1; }
                    if punct(j, ')') { parens -= 1; }
                    j += 1;
                }
                i = j;
                continue;
            }
            Punct('{') => {
                depth += 1;
                if let Some(name) = pending.take() { functions.push((name, depth)); }
            }
            Punct('}') => {
                if functions.last().is_some_and(|&(_, d)| d == depth) { functions.pop(); }
                depth = depth.saturating_sub(1);
            }
            Punct(';') => { pending = None; }
            Ident(name) if punct(after_generics(i + 1), '(') => {
                let callee = if name == "collect" && i > 0 && punct(i - 1, '.') && punct(i + 2, '&') { Some("collect") }
                    else { ENTRIES.iter().copied().find(|entry| entry == name) };
                if let Some(callee) = callee {
                    let function = functions.last().map_or("<top>", |(name, _)| name.as_str());
                    found.push((function.to_owned(), callee.to_owned()));
                }
            }
            _ => {}
        }
        i += 1;
    }
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
        for (function, callee) in calls(&std::fs::read_to_string(&path).unwrap()) {
            *sites.entry((relative.clone(), function, callee)).or_default() += 1;
        }
    }
}

/// The scanner sees what a line matcher missed, and nothing in comments,
/// strings or test items.
#[test]
fn the_scanner_finds_calls_a_line_matcher_missed() {
    let found = calls(r##"
        fn helper(vm: &mut Vm) { vm.reader_call(f, &[]) }
        fn spaced(vm: &mut Vm) { vm.reader_call (f, &[]); vm.execute_raw
            (id); }
        fn outer(vm: &mut Vm) {
            fn inner(vm: &mut Vm) { vm.eval_form_raw(&form); }
            vm.load_file_raw(path);
        }
        fn quiet(vm: &mut Vm) -> &'static str {
            // vm.reader_call(f, &[])
            /* vm.reader_call(f, /* nested */ &[]) */
            let _c = '{';
            let _s = "vm.reader_call(f) }";
            let _r = r#"vm.execute_raw(id) {"#;
            "done"
        }
        fn generic(e: &mut Engine) { call_suspended::<()>(e, r, |_vm| ()); vm.execute_args::<Vec<(u8, fn() -> u8)>>(x); }
        fn reader_call(&mut self) {}
        defsubr! { name = "x", fn subr(cx, a: Value; optional b: Value) { cx.variable_set(a, b)?; } }
        fn collects(vm: &mut Vm) { let _: Vec<_> = it.collect(); vm.collect(&mut []); }
        #[cfg(test)]
        mod tests { fn t(vm: &mut Vm) { vm.reader_call(f, &[]); } }
        fn after(vm: &mut Vm) { vm.finish_request(o); }
    "##);
    let expect = [("helper", "reader_call"), ("spaced", "reader_call"), ("spaced", "execute_raw"),
        ("inner", "eval_form_raw"), ("outer", "load_file_raw"), ("generic", "call_suspended"), ("generic", "execute_args"), ("subr", "variable_set"), ("collects", "collect"), ("after", "finish_request")];
    let expect: Vec<_> = expect.iter().map(|(f, c)| (f.to_string(), c.to_string())).collect();
    assert_eq!(found, expect);
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
