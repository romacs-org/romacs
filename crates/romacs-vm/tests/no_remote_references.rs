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
use std::collections::{BTreeMap, BTreeSet};
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
    /// Read for what a mark reached (an own-obarray symbol, a marked region).
    /// An incremental mark will have to read it again at its finish; until C
    /// does, nothing reads it twice.
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
    // Keeps the last kernel's arguments until the next edit clears it, past
    // any collection; during one it is `stack`, swapped in. Inert: nothing
    // reads it before that clear (insdel.rs).
    ("kernel_stack", Cache),
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
    // A frozen region's symbols, read once it is marked. Written when it is
    // placed and when its hash tables are put in, both before it is returned
    // and before any heap Value names it, with no collection between
    // (frozen.rs), so no store.
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
    // Their nodes are shared and locked, so a node's store hooks no one entry:
    // their contents are written only through char_table.rs's funnel (`wrote`),
    // which every write method passes and the collector's barrier will hook.
    // Past it only the collector's own visits write (PAST_THE_BARRIER).
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

/// The `Table` methods that write past the barrier, and who may name them:
/// the collector's own rewrites, and the hash table's setters, whose `Stores`
/// handle tells each store but whose `&mut HashTable` would let a caller write
/// around it. Each name is the method's alone, so any use of it counts. The
/// rule covers names only: a wrapper in an allowed file that hands a method out
/// is that file's to answer for.
const PAST_THE_BARRIER: &[(&str, MayCall)] = &[
    ("collector_entries_mut", collector), ("collector_compact", collector), ("collector_prepend", collector),
    ("hash_mut", |file| file == "table.rs" || file == "hash.rs"),
    ("collector_visit_node", char_table_collector), ("collector_visit_values", char_table_collector),
];

/// Whether a file under `src` may call a method.
type MayCall = fn(&str) -> bool;

fn collector(file: &str) -> bool {
    ["table.rs", "gc.rs", "gc_mark.rs", "gc_check.rs"].contains(&file)
}

/// The collector, and the char table and keymap cursor files that give it
/// their visits: those write node contents past char_table.rs's funnel.
fn char_table_collector(file: &str) -> bool {
    collector(file) || file == "char_table.rs" || file == "map_keymap.rs"
}

/// Each file under `src` that names a method of PAST_THE_BARRIER it may not,
/// as `FILE: METHOD`: a call, a path to it, a reference to it or a macro's
/// argument, any use of the identifier. A comment or a string never counts.
fn names_past_the_barrier(sources: &[(String, String)]) -> Vec<String> {
    let mut problems = vec![];
    for (file, text) in sources {
        let toks = tokens(text);
        for (method, allowed) in PAST_THE_BARRIER {
            if !allowed(file) && toks.contains(&Token::Ident((*method).into())) {
                problems.push(format!("{file}: {method}"));
            }
        }
    }
    problems
}

#[test]
fn only_the_collector_and_the_hash_setters_write_past_the_barrier() {
    let problems = names_past_the_barrier(&sources());
    assert!(problems.is_empty(), "past the barrier: {}", problems.join(", "));
}

/// The check can fire: each method, named from a file that may not, by a
/// call, a path, a reference and a macro's argument.
#[test]
fn a_name_past_the_barrier_is_refused() {
    let file = |name: &str, text: &str| (name.to_owned(), text.to_owned());
    let sources = [
        file("window.rs", "fn f(vm: &mut Vm) { vm.objects.hash_mut(1); vm.conses.collector_compact(|_| {}); }"),
        file("prim/gc_leaf.rs", "fn f(vm: &mut Vm) { vm.cells.collector_entries_mut(); Table::collector_prepend(&mut vm.cells, 1, || NIL); }"),
        file("gc.rs", "fn f(vm: &mut Vm) { vm.cells.collector_entries_mut(); let g = Table::hash_mut; g(&mut vm.objects, 2); }"),
        file("frame.rs", "fn f(vm: &mut Vm) { through!(vm.objects, hash_mut, 3); }"),
        file("hash.rs", "fn f(vm: &mut Vm) { vm.objects.hash_mut(1); } // collector_compact in a comment"),
        file("handle.rs", "fn f(vm: &mut Vm) { let collector_compact_count = 0; vm.hash_mut_ish(); let s = \"collector_prepend\"; }"),
        file("gcov.rs", "fn f(vm: &mut Vm) { vm.cells.collector_entries_mut(); }"),
        file("prim/leaf/casetab.rs", "fn f(t: &mut CharTable) { t.collector_visit_values(&mut seen, &mut |v| *v = NIL); char_table::collector_visit_node(t.root(), &mut seen, &mut |_| {}); }"),
        file("map_keymap.rs", "fn f(n: &CharTableNodeHandle) { collector_visit_node(n, &mut seen, &mut |_| {}); }"),
    ];
    assert_eq!(names_past_the_barrier(&sources), [
        "window.rs: collector_compact", "window.rs: hash_mut",
        "prim/gc_leaf.rs: collector_entries_mut", "prim/gc_leaf.rs: collector_prepend",
        "gc.rs: hash_mut",
        "frame.rs: hash_mut",
        "gcov.rs: collector_entries_mut",
        "prim/leaf/casetab.rs: collector_visit_node", "prim/leaf/casetab.rs: collector_visit_values",
    ]);
}

// The two ratchets. A store must reach the barrier, so nothing reachable from
// the VM may hold a Value behind a lock or cell, where a shared borrow writes
// it, and a table entry's Value fields may not be public, where code writes
// them without the entry's own methods. Today's offenders are listed, each with
// what will remove it; a new one fails at once, and a listed one that has gone
// fails until its line is dropped, so the lists only shrink.
//
// The check reads source, not types: a name resolves to the type declared in
// the same file, else to the one a `use` or a path names, else to every type of
// that name, and a type is taken to hold a Value if any of its fields could.
// It sees no `dyn` contents and no Value kept as bare bits.

/// Today's Values behind a lock or cell, reachable from the VM, as
/// `FILE:TYPE.FIELD` (an enum's whole body as `FILE:TYPE`). A line names the
/// one writer such a Value may have, or what removes the cell. A shared node or
/// body stays behind its lock; what matters is that only its owner's write
/// funnel writes it, which privacy, not this check, holds.
const INTERIOR_ALLOWED: &[(&str, &str)] = &[
    ("char_table.rs:CharTable.root", "a char table node, written only through char_table.rs's funnel (`wrote`)"),
    ("char_table.rs:CharTableSlot", "a char table node, written only through char_table.rs's funnel"),
    ("char_table.rs:CharTableView.root", "a char table node, which a view only reads"),
    ("data.rs:Object", "`SubCharTable`'s char table node, written only through char_table.rs's funnel"),
    ("lib.rs:Vm.native_case_roots", "weak references to case tables' char table nodes, written only through char_table.rs's funnel"),
    ("map_keymap.rs:Frame.node", "a keymap cursor's char table node, written only through char_table.rs's funnel"),
    ("properties.rs:KeyReader.run", "a reader borrowed for one call, holding the last run it read"),
    ("text.rs:Text.body", "text shared by indirect buffers: to be written only through `TextBody`'s own methods"),
    ("treesit/mod.rs:Parser.native", "read as holding a Value through a shared name; holds a tree-sitter parser"),
    ("treesit/mod.rs:Task.state", "a tree-sitter query's state between predicate calls: to settle with C1"),
];

/// Today's table entry types with a public Value field, as `FILE:TYPE`, each
/// with what removes it. A `&mut` to an entry comes only from `entry_mut`,
/// which runs the barrier, so these write past no barrier; the list is the
/// funnel each type still lacks.
const PUBLIC_ALLOWED: &[(&str, &str)] = &[
    ("annotations.rs:AnnotatedBuffer", "buffer accessors"),
    ("annotations.rs:Marker", "marker accessors"),
    ("annotations.rs:Overlay", "overlay accessors"),
    ("bytes.rs:ByteBuffer", "buffer accessors"),
    ("data.rs:LispString", "string property accessors"),
    ("frame.rs:Frame", "frame accessors"),
    ("frame.rs:Terminal", "terminal accessors"),
    ("hash.rs:HashTable", "hash table accessors"),
    ("subprocess.rs:Process", "process accessors"),
    ("window.rs:Window", "window accessors"),
];

const PLAIN: &[&str] = &[
    "Vec", "Box", "Option", "Rc", "Arc", "HashMap", "BTreeMap", "FxHashMap", "HashSet", "BTreeSet", "FxHashSet", "Weak",
    "String", "Result", "VecDeque", "Range", "PhantomData", "NonNull", "Self", "Some", "None",
];
/// What lets a shared borrow write what it holds.
const INTERIOR: &[&str] = &["Cell", "RefCell", "Mutex", "RwLock", "UnsafeCell", "OnceCell", "OnceLock", "LazyCell"];
/// Associated types, which a `type` inside an `impl` declares.
const ASSOCIATED: &[&str] = &["Output", "Target", "Item", "IntoIter", "Error", "Err", "Owned"];

#[derive(Clone, Copy, PartialEq)]
enum Kind { Struct, Enum, Alias }

/// A type's declaration: a struct's fields, an enum's body as one field, an
/// alias's right side as one field.
struct Shape {
    file: String,
    name: String,
    kind: Kind,
    generic: bool,
    fields: Vec<(String, bool, Vec<String>)>,
}

fn is_type_name(word: &str) -> bool {
    word.starts_with(|c: char| c.is_ascii_uppercase()) && word.chars().all(|c| c.is_alphanumeric() || c == '_')
}

fn words(text: &str) -> Vec<String> {
    tokens(text).into_iter().map(|t| match t { Token::Ident(name) => name, Token::Punct(c) => c.to_string() }).collect()
}

/// The index of the bracket closing the one at I; a `->` closes nothing.
fn close(t: &[String], i: usize) -> usize {
    let mut depth = 0;
    let mut k = i;
    while k < t.len() {
        if t[k] == "-" && t.get(k + 1).is_some_and(|w| w == ">") {
            k += 2;
            continue;
        }
        match t[k].as_str() {
            "(" | "[" | "{" | "<" => depth += 1,
            ")" | "]" | "}" | ">" => depth -= 1,
            _ => {}
        }
        if depth == 0 { return k; }
        k += 1;
    }
    t.len() - 1
}

/// T split at its top-level commas.
fn split_top(t: &[String]) -> Vec<Vec<String>> {
    let mut parts = vec![];
    let mut part = vec![];
    let mut depth = 0;
    let mut k = 0;
    while k < t.len() {
        if t[k] == "-" && t.get(k + 1).is_some_and(|w| w == ">") {
            part.extend_from_slice(&t[k..k + 2]);
            k += 2;
            continue;
        }
        match t[k].as_str() {
            "(" | "[" | "{" | "<" => depth += 1,
            ")" | "]" | "}" | ">" => depth -= 1,
            _ => {}
        }
        if t[k] == "," && depth == 0 { parts.push(std::mem::take(&mut part)); } else { part.push(t[k].clone()); }
        k += 1;
    }
    if !part.is_empty() { parts.push(part); }
    parts
}

/// A member past its attributes, and whether it is public.
fn member(mut p: &[String]) -> (bool, &[String]) {
    while p.first().is_some_and(|w| w == "#") {
        let open = if p.get(1).is_some_and(|w| w == "!") { 2 } else { 1 };
        p = &p[close(p, open) + 1..];
    }
    if p.first().is_some_and(|w| w == "pub") {
        p = &p[1..];
        if p.first().is_some_and(|w| w == "(") { p = &p[close(p, 0) + 1..]; }
        return (true, p);
    }
    (false, p)
}

/// The module a type named NAME comes from, by the `use`s of a file's words.
fn imports(t: &[String]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut i = 0;
    while i < t.len() {
        if t[i] != "use" { i += 1; continue; }
        let end = (i..t.len()).find(|&k| t[k] == ";").unwrap_or(t.len());
        let path = &t[i + 1..end];
        for (j, name) in path.iter().enumerate() {
            if !is_type_name(name) { continue; }
            let module = if j >= 3 && path[j - 1] == ":" && path[j - 2] == ":" {
                Some(path[j - 3].clone())
            } else {
                // Inside braces: the segment before the `::{` that opens them.
                let mut depth = 0;
                let mut found = None;
                for q in (0..j).rev() {
                    match path[q].as_str() {
                        "}" => depth += 1,
                        "{" if depth == 0 => {
                            if q >= 3 && path[q - 1] == ":" && path[q - 2] == ":" { found = Some(path[q - 3].clone()); }
                            break;
                        }
                        "{" => depth -= 1,
                        _ => {}
                    }
                }
                found
            };
            if let Some(module) = module { out.insert(name.clone(), module); }
        }
        i = end;
    }
    out
}

/// The types a file declares.
fn declarations(file: &str, t: &[String]) -> Vec<Shape> {
    let mut out = vec![];
    let mut i = 0;
    while i + 2 < t.len() {
        let kind = match t[i].as_str() {
            "struct" | "union" => Some(Kind::Struct),
            "enum" => Some(Kind::Enum),
            "type" if !ASSOCIATED.contains(&t[i + 1].as_str()) => Some(Kind::Alias),
            _ => None,
        };
        let Some(kind) = kind.filter(|_| is_type_name(&t[i + 1])) else { i += 1; continue };
        let mut shape = Shape { file: file.to_owned(), name: t[i + 1].clone(), kind, generic: false, fields: vec![] };
        let mut j = i + 2;
        if t[j] == "<" {
            let end = close(t, j);
            // A lifetime reads as a lower-case word, a type parameter does not.
            shape.generic = t[j + 1..end].iter().any(|w| is_type_name(w));
            j = end + 1;
        }
        if kind == Kind::Alias {
            if t.get(j).is_some_and(|w| w == "=") {
                let end = (j..t.len()).find(|&k| t[k] == ";").unwrap_or(t.len());
                shape.fields.push(("=".to_owned(), false, t[j + 1..end].to_vec()));
                out.push(shape);
                i = end;
            } else {
                i += 1;
            }
            continue;
        }
        while j < t.len() && !["{", "(", ";"].contains(&t[j].as_str()) { j += 1; }
        if j < t.len() && t[j] != ";" {
            let end = close(t, j);
            let body = &t[j + 1..end];
            if kind == Kind::Enum {
                shape.fields.push((String::new(), false, body.to_vec()));
            } else if t[j] == "{" {
                for part in split_top(body) {
                    let (public, p) = member(&part);
                    if p.len() >= 2 && p[1] == ":" { shape.fields.push((p[0].clone(), public, p[2..].to_vec())); }
                }
            } else {
                for (n, part) in split_top(body).iter().enumerate() {
                    let (public, p) = member(part);
                    shape.fields.push((n.to_string(), public, p.to_vec()));
                }
            }
            j = end;
        }
        out.push(shape);
        i = j + 1;
    }
    out
}

fn module_of(file: &str) -> &str {
    let stem = file.trim_end_matches(".rs");
    let mut parts = stem.rsplit('/');
    let last = parts.next().unwrap_or(stem);
    if last == "mod" { parts.next().unwrap_or(last) } else { last }
}

/// Every type under `src`, and what each holds.
struct Types {
    shapes: Vec<Shape>,
    by_name: BTreeMap<String, Vec<usize>>,
    imports: BTreeMap<String, BTreeMap<String, String>>,
    holds: Vec<bool>,
    wrappers: BTreeSet<String>,
}

impl Types {
    fn new(sources: &[(String, String)]) -> Types {
        let mut shapes = vec![];
        let mut imported = BTreeMap::new();
        for (file, text) in sources {
            let t = words(text);
            imported.insert(file.clone(), imports(&t));
            shapes.extend(declarations(file, &t));
        }
        let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, shape) in shapes.iter().enumerate() { by_name.entry(shape.name.clone()).or_default().push(i); }
        let n = shapes.len();
        let mut types = Types { shapes, by_name, imports: imported, holds: vec![false; n], wrappers: INTERIOR.iter().map(|w| (*w).to_owned()).collect() };
        // A generic struct that keeps its parameter behind a cell is a cell.
        loop {
            let found: Vec<String> = types.shapes.iter()
                .filter(|s| s.kind == Kind::Struct && s.generic && !types.wrappers.contains(&s.name))
                .filter(|s| s.fields.iter().any(|(_, _, ty)| ty.iter().any(|w| types.wrappers.contains(w))))
                .map(|s| s.name.clone()).collect();
            if found.is_empty() { break; }
            types.wrappers.extend(found);
        }
        loop {
            let found: Vec<usize> = (0..n).filter(|&i| !types.holds[i]).filter(|&i| {
                let s = &types.shapes[i];
                s.fields.iter().any(|(_, _, ty)| types.holds_value_except(&s.file, ty, Some(i)))
            }).collect();
            if found.is_empty() { break; }
            for i in found { types.holds[i] = true; }
        }
        types
    }

    /// The types NAME may be in FILE: declared there, else in the module a
    /// path or `use` names (none, if that module is another crate's), else
    /// every type of that name.
    fn resolve(&self, file: &str, name: &str, hint: Option<&str>) -> Vec<usize> {
        let Some(all) = self.by_name.get(name) else { return vec![] };
        let here: Vec<usize> = all.iter().copied().filter(|&i| self.shapes[i].file == file).collect();
        if !here.is_empty() { return here; }
        let module = hint.filter(|m| !["crate", "super", "self"].contains(m))
            .or_else(|| self.imports.get(file).and_then(|u| u.get(name)).map(String::as_str));
        match module {
            Some(m) => all.iter().copied().filter(|&i| {
                let f = &self.shapes[i].file;
                module_of(f) == m || f.starts_with(&format!("{m}/")) || f.contains(&format!("/{m}/"))
            }).collect(),
            None => all.clone(),
        }
    }

    /// Each type name in TY, with the module a path names it by.
    fn refs<'t>(&self, ty: &'t [String]) -> Vec<(&'t str, Option<&'t str>)> {
        ty.iter().enumerate()
            .filter(|(_, w)| is_type_name(w) && !PLAIN.contains(&w.as_str()) && !self.wrappers.contains(*w))
            .map(|(k, w)| (w.as_str(), (k >= 3 && ty[k - 1] == ":" && ty[k - 2] == ":").then(|| ty[k - 3].as_str())))
            .collect()
    }

    fn holds_value_except(&self, file: &str, ty: &[String], skip: Option<usize>) -> bool {
        ty.iter().any(|w| w == "Value")
            || self.refs(ty).iter().any(|(name, hint)| self.resolve(file, name, *hint).into_iter().any(|i| Some(i) != skip && self.holds[i]))
    }

    fn holds_value(&self, file: &str, ty: &[String]) -> bool {
        self.holds_value_except(file, ty, None)
    }

    /// TY with its aliases written out.
    fn expand(&self, file: &str, ty: &[String], depth: usize) -> Vec<String> {
        let mut out = vec![];
        for w in ty {
            let alias = if is_type_name(w) && depth < 8 {
                self.resolve(file, w, None).into_iter().find(|&i| self.shapes[i].kind == Kind::Alias)
            } else { None };
            match alias {
                Some(i) => out.extend(self.expand(&self.shapes[i].file, &self.shapes[i].fields[0].2, depth + 1)),
                None => out.push(w.clone()),
            }
        }
        out
    }

    /// Whether TY keeps a Value behind a cell or lock.
    fn wraps_value(&self, file: &str, ty: &[String]) -> bool {
        let t = self.expand(file, ty, 0);
        (0..t.len()).any(|k| {
            self.wrappers.contains(&t[k]) && t.get(k + 1).is_some_and(|w| w == "<") && self.holds_value(file, &t[k + 2..close(&t, k + 1)])
        })
    }

    fn named(&self, file: &str, name: &str, kind: Kind) -> Vec<usize> {
        self.by_name.get(name).into_iter().flatten().copied()
            .filter(|&i| self.shapes[i].file == file && self.shapes[i].kind == kind).collect()
    }

    /// The structs the heap tables hold: each `Object` variant's payload, and closures.
    fn entries(&self) -> BTreeSet<usize> {
        let mut out = BTreeSet::new();
        for i in self.named("data.rs", "Object", Kind::Enum) {
            for part in split_top(&self.shapes[i].fields[0].2) {
                let (_, p) = member(&part);
                if p.len() > 2 && p[1] == "(" {
                    let payload = self.expand("data.rs", &p[2..close(p, 1)], 0);
                    let found = self.refs(&payload).into_iter().rev().filter(|(name, _)| *name != "Value").find_map(|(name, hint)| {
                        let structs: Vec<usize> = self.resolve("data.rs", name, hint).into_iter().filter(|&k| self.shapes[k].kind == Kind::Struct).collect();
                        (!structs.is_empty()).then_some(structs)
                    });
                    out.extend(found.into_iter().flatten());
                }
            }
        }
        out.extend(self.named("lib.rs", "Closure", Kind::Struct));
        out
    }

    /// Every type reachable from the VM and the tables' entries.
    fn reachable(&self) -> BTreeSet<usize> {
        let mut seen = BTreeSet::new();
        let mut stack: Vec<usize> = self.named("lib.rs", "Vm", Kind::Struct);
        stack.extend(self.entries());
        while let Some(i) = stack.pop() {
            if !seen.insert(i) { continue; }
            let s = &self.shapes[i];
            for (_, _, ty) in &s.fields {
                let t = self.expand(&s.file, ty, 0);
                for (name, hint) in self.refs(&t) { stack.extend(self.resolve(&s.file, name, hint)); }
            }
        }
        seen
    }

    fn interior_holders(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for i in self.reachable() {
            let s = &self.shapes[i];
            for (field, _, ty) in &s.fields {
                if self.wraps_value(&s.file, ty) {
                    out.insert(if field.is_empty() || field == "=" { format!("{}:{}", s.file, s.name) } else { format!("{}:{}.{field}", s.file, s.name) });
                }
            }
        }
        out
    }

    fn public_holders(&self) -> BTreeSet<String> {
        self.entries().into_iter().map(|i| &self.shapes[i])
            .filter(|s| s.fields.iter().any(|(_, public, ty)| *public && self.holds_value(&s.file, &self.expand(&s.file, ty, 0))))
            .map(|s| format!("{}:{}", s.file, s.name)).collect()
    }
}

/// What FOUND and an ALLOWED list differ by: a new offender, or a listed one gone.
fn ratchet(what: &str, found: &BTreeSet<String>, allowed: &[(&str, &str)]) -> Vec<String> {
    let listed: BTreeSet<String> = allowed.iter().map(|(name, _)| (*name).to_owned()).collect();
    let mut out: Vec<String> = found.difference(&listed).map(|name| format!("{name}: {what}")).collect();
    out.extend(listed.difference(found).map(|name| format!("{name}: no longer {what}; drop its line, the list only shrinks")));
    out
}

fn sources() -> Vec<(String, String)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![src.clone()];
    let mut out = vec![];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push((path.strip_prefix(&src).unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    out
}

#[test]
fn values_behind_a_cell_or_a_public_field_only_shrink() {
    let types = Types::new(&sources());
    let mut problems = ratchet("holds a Value behind a cell or lock, reachable from the VM", &types.interior_holders(), INTERIOR_ALLOWED);
    problems.extend(ratchet("a table entry with a public Value field", &types.public_holders(), PUBLIC_ALLOWED));
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The check can fire: a Value behind a cell, a custom lock and an alias, a
/// public Value field in an entry, and a listed name that has gone; a cell of
/// plain data and a private field do not count.
#[test]
fn a_new_value_behind_a_cell_or_a_public_field_is_refused() {
    let file = |name: &str, text: &str| (name.to_owned(), text.to_owned());
    let types = Types::new(&[
        file("lib.rs", "pub(crate) struct Vm { things: Thing } pub(crate) struct Closure { captures: Vec<Value> } pub(crate) struct Value(u64);"),
        file("data.rs", "pub(super) enum Object { A(Box<entry::Entry>), B(Value), C(Vec<Value>) }"),
        file("entry.rs", "pub(crate) struct Entry { pub v: Value, w: std::cell::RefCell<Value>, x: Shared<Inner>, quiet: Value }
                          pub(crate) struct Inner { v: Value }"),
        file("indirect.rs", "pub(crate) struct Shared<T>(Rc<UnsafeCell<T>>);"),
        file("thing.rs", "use crate::entry::Inner; type Handle = Arc<Mutex<Inner>>; pub(crate) struct Thing { h: Handle, plain: Cell<usize> }"),
    ]);
    let interior: Vec<String> = types.interior_holders().into_iter().collect();
    assert_eq!(interior, ["entry.rs:Entry.w", "entry.rs:Entry.x", "thing.rs:Thing.h"]);
    let public: Vec<String> = types.public_holders().into_iter().collect();
    assert_eq!(public, ["entry.rs:Entry"]);
    let found: BTreeSet<String> = ["new.rs:A".to_owned()].into();
    assert_eq!(ratchet("held", &found, &[("gone.rs:B", "why")]), [
        "new.rs:A: held".to_owned(),
        "gone.rs:B: no longer held; drop its line, the list only shrinks".to_owned(),
    ]);
}
