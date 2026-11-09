//! Every place a Value can live outside the four heap tables, named. A later
//! collector relies on the rule that a trace of a table entry reads only that
//! entry's own storage, and that a Value anywhere else is a root or shades at
//! its store (design note gc-smooth.md 4.15). Nothing else enumerates these:
//! a field in no root list is invisible to the collector and to `gc_check`.
//!
//! So each `Vm` field and each `Object` variant has a class here, and a new
//! one fails until it has one, as does a name that is gone. The class is a
//! decision read from the code, not something this test can prove; it makes
//! the decision someone's to take when the field is added.
#[path = "support/rust_tokens.rs"]
mod rust_tokens;
use rust_tokens::{Token, tokens};
// The shared lexer's own list, which this test has no use for.
const _: &[&str] = rust_tokens::CONVERTED;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Clone, Copy, Debug)]
enum Class {
    /// The collector reads it as a root at every mark.
    Root,
    /// Holds Values a later collector learns of at their stores: the hook runs.
    Barrier,
    /// Holds Values that shade at their stores once C1 adds the shade. Until
    /// then a store here runs nothing: not `Barrier`.
    Owed,
    /// Read for what a mark reached (an own-obarray symbol, a marked region),
    /// and read again at the finish of an incremental mark.
    Rescanned,
    /// Holds no heap Value.
    Free,
    /// Keeps nothing alive: the collector clears or checks what it holds, or a
    /// stale one is inert by use.
    Cache,
    /// Holds Values only between two points where no collection runs.
    Transient,
}
use Class::{Barrier, Cache, Free, Owed, Rescanned, Root, Transient};

/// Every `Vm` field.
const FIELDS: &[(&str, Class)] = &[
    ("charsets", Root), ("standard_case_tables", Root), ("command_point", Root),
    ("change_prepared", Free),
    // Keeps the last kernel's arguments until the next edit clears it; during
    // one it is `stack`, swapped in, and nothing reads it in between (insdel.rs).
    ("kernel_stack", Transient),
    ("change_slots", Free), ("case_symbols_slot", Free), ("print_number_index", Free), ("undo_list_slot", Free),
    // `Shown`'s Values are compared only while `collections` still matches, and
    // `previous` and the bar's caches go the same way; faces are hidden slots.
    ("display_state", Cache),
    ("window_state", Free), ("world_version", Free), ("motion_quit", Free),
    ("variable_read_probes", Free), ("speculative_hits", Free), ("inline_hits", Free), ("disable_inline", Free), ("speculative_misses", Free),
    ("regexp", Free),
    // A pattern's syntax table goes at every collection (`clear_regexp_cache`).
    ("regexp_cache", Cache),
    ("mode_line_forms", Cache), ("lambda_lists", Cache),
    ("images", Free),
    // A `Band`: its object constants are `Rooted` handles, and its `rule` keeps
    // a `CharTableView` no root list reads. Inert: those Values reach only
    // `== NIL` and `integer_value()` (width_rule.rs), so a stale one changes
    // nothing.
    ("status_bar_line", Cache),
    ("execution_fatal", Free), ("loader", Free), ("source_cache", Free), ("macros", Free), ("expanding", Free),
    ("compile_depth", Free), ("eval_depth", Free), ("dynamic_scope", Free),
    ("propagated_escape", Root),
    ("site_leaf", Free),
    ("signal_handlers", Root), ("backtrace", Root), ("profiler", Root),
    ("propagated_escape_hooked", Free), ("propagated_eval_depth", Free), ("propagate_next", Free),
    ("host_input", Free), ("layout_host", Free), ("display", Free), ("visual", Free), ("session_host", Free), ("idle_since", Free),
    ("redisplay_since_mark", Free), ("treesit_plain", Free), ("treesit_deferred", Free), ("treesit_parse_owed", Free),
    ("quit", Free), ("force_quit_count", Free), ("quit_slots", Free),
    // Interned symbols, which no collection frees, and indices.
    ("face_keys", Free),
    ("debug_event_slots", Free), ("keymap_generation", Free), ("process_jobs", Free), ("subprocesses", Free),
    ("environment_snapshot", Free), ("trapped", Free), ("retrap", Free), ("doc_file", Free),
    ("terminal_host", Free), ("terminal_bindings", Root),
    ("pending_specials", Free), ("file_specials", Free), ("seed_file_specials", Free), ("file_level_request", Free),
    ("lazy_regime", Free), ("interpreting_source", Free), ("defer_expansion_errors", Free), ("evaluating", Free), ("quit_unwinding", Free),
    // Weak: one whose error a Rust caller discarded goes at the next collection.
    ("thrown_no_catch", Cache),
    ("strict_root", Free), ("reached_head", Free), ("reached_generated", Free), ("loader_order_request", Free),
    ("keep_captures", Free), ("lambda_depth", Free), ("window_change_seen", Free), ("expanded_macros", Free), ("expanded_symbols", Free),
    ("image", Free),
    // A frozen region's symbols, read once it is marked. Written once at
    // placement, before any Value names the region (frozen.rs), so no store.
    ("regions", Rescanned),
    ("region_free", Free), ("region_collect", Free), ("isolates", Free), ("inotify", Free),
    ("buffer_slot", Free), ("multibyte_slot", Free), ("truncate_lines_slot", Free), ("defun_start_epoch", Free),
    // An alias names a symbol slot, a Value in all but type.
    ("variable_redirects", Owed),
    ("forwarded", Root),
    ("fixnum_slots", Free), ("local_binding_slots", Free), ("free_local_binding_slots", Free),
    ("gc_stats", Free), ("gc_times", Free), ("gc_census", Free), ("collection_log", Free), ("gc_retained", Free), ("gc_payload_bytes", Free),
    ("collect_poll", Free), ("gc_retained_slots", Free), ("gc_code", Free), ("gc_floor", Free), ("job_budget", Free), ("gc_retained_payload", Free),
    ("gc_holds", Free), ("running_engines", Free), ("rooted_write", Free), ("gc_stress", Free), ("allocation", Free),
    ("names", Free), ("initial_obarray", Free), ("symbol_names", Free),
    // Read for the symbols marked, which own-obarray ones are only if reached.
    ("obarray_symbol_names", Rescanned),
    ("permanent_symbol_names", Root),
    // Weak references to nodes the case tables own.
    ("native_case_roots", Cache),
    ("obarray_free_slots", Free), ("generated_symbols", Free), ("generated_slots", Free),
    ("slots", Root),
    ("specials", Free), ("special_changes", Free), ("special_cache", Free),
    // Each function's ops hold constants, traced from the code that is live.
    ("functions", Root),
    ("request_code", Root),
    ("register_width", Free), ("spread_reserve", Free),
    // Lowered code, which adds nothing to its function's ops, and registers.
    ("execution", Root), ("spare_programs", Root),
    ("reader_trampolines", Free),
    ("closures", Barrier), ("cells", Barrier), ("conses", Barrier), ("objects", Barrier),
    // A paired cell's trace pushes the pair's Value, and its writes will shade.
    ("cell_pairs", Owed),
    ("random", Free), ("fence", Free), ("pending", Free), ("mktime_offset", Free), ("tz_rule_dstoff", Free),
    ("local_zone", Free), ("recent_keys", Free), ("lossage_limit", Free),
    ("referrers", Cache), ("local_owners", Cache),
    ("completion_walks", Free), ("completion_case_memo", Free),
    ("caches", Cache),
    ("stack", Root), ("external_roots", Root),
    // Not roots: the collector dooms those it does not reach.
    ("finalizers", Cache),
    ("doomed_finalizers", Root),
    ("post_gc_due", Free), ("unwinding", Free), ("aftermath_runs", Free),
    ("suspended_stacks", Root), ("suspended", Root),
    // Set and cleared around one keymap walk, which never collects (keymap.rs).
    ("keymap_filters", Transient), ("keymap_pending_filter", Transient),
    ("dynamic", Root),
    // Custom symbols' entries are read with their symbol, and will shade at their store.
    ("conditions", Owed),
    ("rooted", Root),
];

/// Every `Object` variant. `Barrier`: a whole-entry store hooks its entry
/// (`Table::entry_mut`), and its trace reads what `visit_values` reads.
const VARIANTS: &[(&str, Class)] = &[
    ("SyntaxTable", Free), ("Float", Free), ("Bignum", Free),
    ("Window", Barrier), ("Frame", Barrier), ("Terminal", Barrier), ("WindowConfiguration", Barrier),
    ("Sqlite", Free), ("Treesit", Barrier), ("Process", Barrier), ("Finalizer", Barrier), ("Thread", Free),
    ("String", Barrier), ("RawString", Barrier), ("Vector", Barrier), ("BoolVector", Free), ("Record", Barrier),
    // Their nodes are shared and locked: a node's store is its own (char_table.rs).
    ("CharTable", Barrier), ("SubCharTable", Barrier), ("MapKeymapCursor", Barrier),
    // Text shared by indirect buffers is traced from the base alone.
    ("Buffer", Barrier),
    ("Marker", Barrier), ("Overlay", Barrier), ("Bytes", Free), ("ByteBuffer", Barrier),
    // Stores go through its setters (`Table::hash_mut`, hash.rs alone), weak ones' too.
    ("HashTable", Barrier),
    ("Obarray", Barrier), ("Free", Free),
];

fn source(name: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name)).unwrap()
}

/// The body's tokens of the item `KEYWORD NAME { ... }`, between its braces.
fn body_of(toks: &[Token], keyword: &str, name: &str) -> Vec<String> {
    let start = toks
        .windows(2)
        .position(|w| w[0] == Token::Ident(keyword.into()) && w[1] == Token::Ident(name.into()))
        .unwrap_or_else(|| panic!("no {keyword} {name}"));
    let open = (start..toks.len()).find(|&i| toks[i] == Token::Punct('{')).unwrap();
    let mut depth = 0;
    let mut out = vec![];
    for t in &toks[open..] {
        match t {
            Token::Punct('{') => depth += 1,
            Token::Punct('}') if depth == 1 => return out,
            Token::Punct('}') => depth -= 1,
            _ => {}
        }
        out.push(match t {
            Token::Ident(name) => name.clone(),
            Token::Punct(c) => c.to_string(),
        });
    }
    panic!("{keyword} {name} never closes");
}

/// The members a body declares: an identifier at depth 0 at the start of a
/// member, past its attributes and visibility, up to `,` at depth 0.
fn members(body: &[String]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let (mut depth, mut start, mut i) = (0i32, true, 1);
    while i < body.len() {
        let t = body[i].as_str();
        if start && t == "#" {
            let mut d = 0;
            i += 1;
            loop {
                match body[i].as_str() {
                    "[" => d += 1,
                    "]" => d -= 1,
                    _ => {}
                }
                i += 1;
                if d == 0 { break; }
            }
            continue;
        }
        if start && t == "pub" {
            i += 1;
            if body[i] == "(" {
                while body[i] != ")" { i += 1; }
                i += 1;
            }
            continue;
        }
        if start {
            out.insert(t.to_owned());
            start = false;
        }
        match t {
            "(" | "[" | "{" | "<" => depth += 1,
            // A `->` or a `>` of `=>` closes nothing; neither appears in these bodies.
            ")" | "]" | "}" | ">" => depth -= 1,
            "," if depth == 0 => start = true,
            _ => {}
        }
        i += 1;
    }
    out
}

/// What NAMED and FOUND differ by, as complaints about WHAT.
fn differences(what: &str, named: &[(&str, Class)], found: &BTreeSet<String>) -> Vec<String> {
    let listed: BTreeSet<String> = named.iter().map(|(name, _)| (*name).to_owned()).collect();
    let mut out: Vec<String> = found.difference(&listed).map(|name| format!("{what} {name} has no class here")).collect();
    out.extend(listed.difference(found).map(|name| format!("{what} {name} is gone: drop its row")));
    if listed.len() != named.len() {
        out.push(format!("a {what} is listed twice"));
    }
    out
}

#[test]
fn every_vm_field_and_object_variant_has_a_class() {
    let vm = members(&body_of(&tokens(&source("lib.rs")), "struct", "Vm"));
    let object = members(&body_of(&tokens(&source("data.rs")), "enum", "Object"));
    assert!(vm.len() > 100 && object.len() > 20, "the parse found too little: {} fields, {} variants", vm.len(), object.len());
    let mut problems = differences("Vm field", FIELDS, &vm);
    problems.extend(differences("Object variant", VARIANTS, &object));
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The check can fire, both ways.
#[test]
fn a_new_name_and_a_stale_name_are_both_refused() {
    let found: BTreeSet<String> = ["kept", "added"].map(str::to_owned).into();
    let problems = differences("Vm field", &[("kept", Root), ("gone", Free)], &found);
    assert_eq!(problems, ["Vm field added has no class here", "Vm field gone is gone: drop its row"]);
    let parsed = members(&body_of(
        &tokens("struct Vm { #[cfg(test)] a: Vec<(u8, u8)>, pub(crate) b: [u8; 2], /// c\n c: std::collections::HashMap<u8, u8>, }"),
        "struct",
        "Vm",
    ));
    assert_eq!(parsed, ["a", "b", "c"].map(str::to_owned).into());
    let parsed = members(&body_of(&tokens("enum Object { A(Box<u8>), B, /// c\n C(Vec<u8>, u8), }"), "enum", "Object"));
    assert_eq!(parsed, ["A", "B", "C"].map(str::to_owned).into());
}

/// The `Table` methods that write past the barrier, and who may call them:
/// the collector's own rewrites, and the hash table's setters, whose `Stores`
/// handle tells each store but whose `&mut HashTable` would let a caller write
/// around it.
const PAST_THE_BARRIER: &[(&str, MayCall)] = &[
    ("collector_entries_mut", collector), ("compact", collector), ("prepend", collector),
    ("hash_mut", |file| file == "table.rs" || file == "hash.rs"),
];

/// Whether a file under `src` may call a method.
type MayCall = fn(&str) -> bool;

fn collector(file: &str) -> bool {
    file == "table.rs" || (file.starts_with("gc") && !file.contains('/'))
}

/// Each file under `src` that calls a method of PAST_THE_BARRIER it may not, as
/// `FILE: METHOD`. A call is `.METHOD(`, so a field of that name, a comment
/// or a string never counts.
fn calls_past_the_barrier(sources: &[(String, String)]) -> Vec<String> {
    let mut problems = vec![];
    for (file, text) in sources {
        let toks = tokens(text);
        for (method, allowed) in PAST_THE_BARRIER {
            let call = [Token::Punct('.'), Token::Ident((*method).into()), Token::Punct('(')];
            if !allowed(file) && toks.windows(3).any(|w| w == call) {
                problems.push(format!("{file}: {method}"));
            }
        }
    }
    problems
}

#[test]
fn only_the_collector_and_the_hash_setters_write_past_the_barrier() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![src.clone()];
    let mut sources = vec![];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let relative = path.strip_prefix(&src).unwrap().to_string_lossy().into_owned();
                sources.push((relative, std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    let problems = calls_past_the_barrier(&sources);
    assert!(problems.is_empty(), "past the barrier: {}", problems.join(", "));
}

/// The check can fire: each method, called from a file that may not.
#[test]
fn a_call_past_the_barrier_is_refused() {
    let file = |name: &str, text: &str| (name.to_owned(), text.to_owned());
    let sources = [
        file("window.rs", "fn f(vm: &mut Vm) { vm.objects.hash_mut(1); vm.conses.compact(|_| {}); }"),
        file("prim/gc_leaf.rs", "fn f(vm: &mut Vm) { vm.cells.collector_entries_mut(); vm.cells.prepend(1, || NIL); }"),
        file("gc.rs", "fn f(vm: &mut Vm) { vm.cells.collector_entries_mut(); vm.objects.hash_mut(2); }"),
        file("hash.rs", "fn f(vm: &mut Vm) { vm.objects.hash_mut(1); } // .compact( in a comment"),
        file("handle.rs", "fn f(vm: &mut Vm) { vm.allocation.compact = true; }"),
    ];
    assert_eq!(calls_past_the_barrier(&sources), [
        "window.rs: compact", "window.rs: hash_mut",
        "prim/gc_leaf.rs: collector_entries_mut", "prim/gc_leaf.rs: prepend",
        "gc.rs: hash_mut",
    ]);
}
