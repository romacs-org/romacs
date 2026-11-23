//! The same probe runs in separate test binaries on the C and Rust revisions.
//! Its records use tree-local preorder indices, so raw addresses and equal
//! spans on different nodes cannot hide changed captures.
use super::{Language, Node, PredicateStep, Query};
use crate::quit::NativeStop;
use romacs_grammars::BUNDLED;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use tree_sitter::{InputEdit, Point};

struct Case {
    language: String,
    text: String,
    queries: Vec<String>,
    states: Vec<State>,
}

type Change = (usize, usize, String);

struct State {
    seed: u64,
    index: u64,
    change: Option<Change>,
    ranges: Vec<[u32; 2]>,
}

impl State {
    fn from_json(row: &Value) -> Self {
        Self {
            seed: row[0].as_u64().expect("state seed"),
            index: row[1].as_u64().expect("state index"),
            change: serde_json::from_value(row[2].clone()).expect("scheduled edit"),
            ranges: serde_json::from_value(row[3].clone()).expect("scheduled ranges"),
        }
    }
}

impl Case {
    fn from_json(value: &Value) -> Self {
        let string = |value: &Value| value.as_str().expect("query corpus string").to_owned();
        Self {
            language: string(&value["language"]),
            text: string(&value["text"]),
            queries: value["queries"]
                .as_array()
                .expect("query corpus array")
                .iter()
                .map(string)
                .collect(),
            states: value["states"]
                .as_array()
                .expect("query states")
                .iter()
                .map(State::from_json)
                .collect(),
        }
    }
}

fn language(name: &str) -> Language {
    let grammar = BUNDLED
        .iter()
        .find(|grammar| grammar.name == name)
        .unwrap_or_else(|| panic!("unknown bundled grammar {name}"));
    Language::from_function(grammar.language)
}

type Identity = (usize, usize);

fn tree_index(root: Node) -> (u64, HashMap<Identity, usize>) {
    let mut hash = DefaultHasher::new();
    let mut ids = HashMap::new();
    let mut cursor = root.ts().walk();
    loop {
        let node = cursor.node();
        (
            node.kind_id(),
            node.byte_range(),
            node.start_position(),
            node.end_position(),
            node.is_named(),
            node.is_missing(),
            node.is_extra(),
            node.is_error(),
            node.child_count(),
            cursor.field_id(),
        )
            .hash(&mut hash);
        assert!(ids.insert(Node::from(node).identity(), ids.len()).is_none());
        if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return (hash.finish(), ids);
            }
        }
    }
}

fn metadata(query: &Query) -> Value {
    let captures: Vec<_> = (0..query.capture_count())
        .map(|id| query.capture_name(id))
        .collect();
    let predicates: Vec<Vec<_>> = (0..query.pattern_count())
        .map(|pattern| {
            let mut out = Vec::new();
            for step in query.predicates(pattern) {
                out.push(match step {
                    PredicateStep::Capture(id) => json!({"capture": id}),
                    PredicateStep::String(bytes) => json!({"string": bytes}),
                    PredicateStep::Done => Value::Null,
                });
            }
            out
        })
        .collect();
    json!({"patterns": query.pattern_count(), "captures": captures, "predicates": predicates})
}

type CaptureFacts = (u32, usize, u16, u8, [usize; 2], [[usize; 2]; 2]);
type MatchFacts = (u32, Vec<CaptureFacts>);

fn capture_facts(node: Node, capture: u32, ids: &HashMap<Identity, usize>) -> CaptureFacts {
    let raw = node.ts();
    let flags = u8::from(raw.is_named())
        | u8::from(raw.is_missing()) << 1
        | u8::from(raw.is_extra()) << 2
        | u8::from(raw.is_error()) << 3;
    let position = |point: Point| [point.row, point.column];
    (
        capture,
        ids[&node.identity()],
        raw.kind_id(),
        flags,
        [raw.start_byte(), raw.end_byte()],
        [position(raw.start_position()), position(raw.end_position())],
    )
}

fn matches(
    query: &Query,
    tree: &tree_sitter::Tree,
    node: Node,
    range: (u32, u32),
    ids: &HashMap<Identity, usize>,
) -> Vec<MatchFacts> {
    query
        .matches_in(tree, node, range, &NativeStop::never())
        .expect("unstopped walk")
        .into_iter()
        .map(|matched| {
            let captures = matched
                .captures
                .into_iter()
                .map(|(node, id)| capture_facts(node, id, ids))
                .collect();
            (matched.pattern, captures)
        })
        .collect()
}

fn captures_only(matches: &[MatchFacts]) -> Vec<(u32, Vec<(u32, usize)>)> {
    matches
        .iter()
        .map(|(pattern, captures)| {
            (
                *pattern,
                captures
                    .iter()
                    .map(|capture| (capture.0, capture.1))
                    .collect(),
            )
        })
        .collect()
}

fn point(text: &[u8], offset: usize) -> Point {
    let mut position = Point { row: 0, column: 0 };
    for &byte in &text[..offset] {
        if byte == b'\n' {
            position.row += 1;
            position.column = 0;
        } else {
            position.column += 1;
        }
    }
    position
}

fn edit(text: &mut Vec<u8>, (start, end, insert): &Change) -> InputEdit {
    let start_position = point(text, *start);
    let old_end_position = point(text, *end);
    text.splice(*start..*end, insert.bytes());
    let new_end_byte = start + insert.len();
    InputEdit {
        start_byte: *start,
        old_end_byte: *end,
        new_end_byte,
        start_position,
        old_end_position,
        new_end_position: point(text, new_end_byte),
    }
}

fn record(output: &mut impl Write, row: &Value) {
    writeln!(output, "{row}").expect("write record");
}

fn write_matches(
    output: &mut impl Write,
    tree: &tree_sitter::Tree,
    queries: &[Option<Query>],
    state: &State,
) {
    let root = Node::from(tree.root_node());
    let (digest, ids) = tree_index(root);
    record(
        output,
        &json!([
            "state",
            state.seed,
            state.index,
            state.change,
            format!("{digest:016x}")
        ]),
    );
    for (index, query) in queries.iter().enumerate() {
        let Some(query) = query else { continue };
        for (slot, &[start, end]) in state.ranges.iter().enumerate() {
            let found = matches(query, tree, root, (start, end), &ids);
            record(output, &json!(["run", index, slot, [start, end], found]));
        }
    }
}

fn run_case(case: &Case, output: &mut impl Write) {
    let language = language(&case.language);
    record(
        output,
        &json!([
            "grammar",
            case.language,
            case.queries.len(),
            language.abi_version()
        ]),
    );
    let queries: Vec<_> = case
        .queries
        .iter()
        .enumerate()
        .map(
            |(index, source)| match Query::new(&language, source.as_bytes()) {
                Ok(query) => {
                    record(output, &json!(["compile", index, ["ok", metadata(&query)]]));
                    Some(query)
                }
                Err((offset, kind)) => {
                    record(output, &json!(["compile", index, ["error", offset, kind]]));
                    None
                }
            },
        )
        .collect();
    run_states(case, output, &language, &queries);
}

fn run_states(
    case: &Case,
    output: &mut impl Write,
    language: &Language,
    queries: &[Option<Query>],
) {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&language.0)
        .expect("compatible grammar");
    let mut text = Vec::new();
    let mut previous: Option<tree_sitter::Tree> = None;
    for state in &case.states {
        if let Some(change) = &state.change {
            previous
                .as_mut()
                .expect("initial state")
                .edit(&edit(&mut text, change));
        } else {
            text = case.text.as_bytes().to_vec();
            previous = None;
        }
        let tree = parser
            .parse(&text, previous.as_ref())
            .expect("parse scheduled state");
        write_matches(output, &tree, queries, state);
        previous = Some(tree);
    }
}

#[test]
fn query_oracle_preserves_match_and_capture_order() {
    let language = language("json");
    let bytes = b"[1, 2]";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&language.0)
        .expect("compatible grammar");
    let tree = parser.parse(bytes, None).expect("JSON tree");
    let root = Node::from(tree.root_node());
    let (_, ids) = tree_index(root);
    let query = Query::new(
        &language,
        b"(number) @item (array (number) @first . (number) @second) @parent",
    )
    .expect("query");
    let found = matches(&query, &tree, root, (0, u32::MAX), &ids);
    assert_eq!(
        captures_only(&found),
        [
            (0, vec![(0, 3)]),
            (1, vec![(3, 1), (1, 3), (2, 5)]),
            (0, vec![(0, 5)])
        ]
    );
}

/// The manifest supplies one or more cases for every bundled grammar.
/// Run this test by name in each executable and compare the output files.
#[test]
#[ignore = "requires ROMACS_QUERY_CORPUS and ROMACS_QUERY_RECORDS"]
fn query_oracle_record() {
    let path = |name| PathBuf::from(std::env::var_os(name).expect(name));
    let manifest = std::fs::read(path("ROMACS_QUERY_CORPUS")).expect("query corpus");
    let manifest: Value = serde_json::from_slice(&manifest).expect("corpus JSON");
    let cases: Vec<_> = manifest["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .map(Case::from_json)
        .collect();
    let languages: std::collections::BTreeSet<_> =
        cases.iter().map(|case| case.language.as_str()).collect();
    assert_eq!(
        languages,
        BUNDLED.iter().map(|grammar| grammar.name).collect()
    );
    let mut output =
        BufWriter::new(std::fs::File::create(path("ROMACS_QUERY_RECORDS")).expect("records"));
    for case in &cases {
        run_case(case, &mut output);
    }
    write_probes(&mut output);
    output.flush().expect("flush records");
}

fn json_tree(source: &str) -> (Language, tree_sitter::Tree) {
    let language = language("json");
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language.0).expect("JSON grammar");
    let tree = parser.parse(source, None).expect("JSON tree");
    (language, tree)
}

fn probe(
    name: &str,
    source: &str,
    runs: &[(&str, Option<u32>, (u32, u32))],
    output: &mut impl Write,
) {
    let (language, tree) = json_tree(source);
    let root = tree.root_node();
    let (_, ids) = tree_index(Node::from(root));
    for (index, &(query, child, range)) in runs.iter().enumerate() {
        let node = child.map_or(root, |index| root.named_child(index).expect("probe child"));
        let query = Query::new(&language, query.as_bytes()).expect("probe query");
        let found = matches(&query, &tree, Node::from(node), range, &ids);
        record(output, &json!(["probe", name, index, found]));
    }
}

fn write_probes(output: &mut impl Write) {
    probe(
        "subtree-and-bounds",
        "[1, 2]\n",
        &[
            ("(number) @n", Some(0), (0, 0)),
            ("(number) @n", Some(0), (3, 0)),
            ("(number) @n", Some(0), (5, 1)),
            ("(number) @n", Some(0), (1, 3)),
        ],
        output,
    );
    probe(
        "wildcard-negated-field",
        "{\"k\": 1}",
        &[
            ("(pair !key (number) @number) @parent", None, (0, 0)),
            ("(_ !key (number) @number) @parent", None, (0, 0)),
        ],
        output,
    );
    probe(
        "fourth-capture",
        "{\"k\": 1}",
        &[("(number) @a @b @c @d", None, (0, 0))],
        output,
    );
    let query = "(number) @n\n".repeat(32_768) + "(true) @end";
    probe("wide-step-index", "true", &[(&query, None, (0, 0))], output);
}

#[test]
fn query_oracle_keeps_empty_matches_for_predicate_only_patterns() {
    let (language, tree) = json_tree("1");
    let root = Node::from(tree.root_node());
    let (_, ids) = tree_index(root);
    let query = Query::new(&language, br#"(number) @n (#eq? @n "1")"#).expect("probe query");
    let found = matches(&query, &tree, root, (0, 0), &ids);
    assert_eq!(
        captures_only(&found),
        [(1, vec![]), (0, vec![(0, 1)]), (1, vec![])]
    );
}

#[test]
fn query_oracle_stops_no_match_scan() {
    let (language, tree) = json_tree(&format!("[{}0]", "1,".repeat(4096)));
    let root = Node::from(tree.root_node());
    let query = Query::new(&language, b"(string) @s").expect("probe query");
    let stop = NativeStop::with_test_signal(std::sync::Arc::new(crate::quit::QuitSignal::new()));
    let walk = || query.matches_in(&tree, root, (0, 0), &stop);
    let (complete, full_polls) = crate::quit::query_polls::count(None, walk);
    assert!(complete.expect("unstopped scan").is_empty());

    // Cancel on the second real poll. A final check alone cannot reach it.
    let (stopped, early_polls) = crate::quit::query_polls::count(Some(2), walk);
    assert!(stopped.is_none());
    assert!((2..=3).contains(&early_polls), "{early_polls}");
    assert!(
        full_polls > early_polls,
        "full walk only polled {full_polls} times"
    );
}
