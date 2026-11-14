use super::*;
use super::runs::Run;

/// The text each span of KIND covers, in order.
fn covered(text: &str, kind: Kind) -> Vec<&str> {
    let mut spans: Vec<Span> = spans(text).spans.into_iter().filter(|s| s.kind == kind).collect();
    spans.sort_by_key(|s| (s.start, s.end));
    spans.iter().map(|s| &text[s.start..s.end]).collect()
}

/// The kinds of the run holding the first byte of NEEDLE in TEXT.
fn kinds_at(text: &str, needle: &str) -> Vec<Kind> {
    let at = text.find(needle).expect("needle");
    runs(&spans(text).spans).into_iter().find(|r| r.start <= at && at < r.end).map_or(vec![], |r| r.kinds)
}

#[test]
fn headings_and_their_markers() {
    let text = "# Title *em*\n\nSub\n---\n";
    assert_eq!(covered(text, Kind::Heading(1)), ["# Title *em*\n"]);
    assert_eq!(covered(text, Kind::Heading(2)), ["Sub\n---\n"]);
    assert_eq!(kinds_at(text, "# "), [Kind::Markup, Kind::Heading(1)]);
    assert_eq!(kinds_at(text, "em"), [Kind::Emphasis, Kind::Heading(1)]);
    assert_eq!(kinds_at(text, "*em"), [Kind::Markup, Kind::Emphasis, Kind::Heading(1)]);
    assert_eq!(kinds_at(text, "---"), [Kind::Markup, Kind::Heading(2)]);
}

#[test]
fn emphasis_strong_strike_and_code() {
    let text = "a **b** _c_ ~~d~~ `e` ``f`g``\n";
    assert_eq!(covered(text, Kind::Strong), ["**b**"]);
    assert_eq!(covered(text, Kind::Emphasis), ["_c_"]);
    assert_eq!(covered(text, Kind::Strike), ["~~d~~"]);
    assert_eq!(covered(text, Kind::Code), ["`e`", "``f`g``"]);
    assert_eq!(covered(text, Kind::Markup), ["**", "**", "_", "_", "~~", "~~", "`", "`", "``", "``"]);
}

#[test]
fn links_images_and_definitions() {
    let text = "[a](http://x) ![i](p.png) <http://y> [r][d] [no]\n\n[d]: /u\n";
    assert_eq!(covered(text, Kind::Link), ["[a](http://x)", "<http://y>", "[r][d]"]);
    assert_eq!(covered(text, Kind::Image), ["![i](p.png)"]);
    assert_eq!(covered(text, Kind::Url), ["](http://x)", "](p.png)", "][d]", "[d]: /u"]);
    // An undefined reference is text.
    assert_eq!(kinds_at(text, "[no]"), Vec::<Kind>::new());
}

#[test]
fn fenced_and_indented_code() {
    let text = "```rust title\nfn x() {}\n```\n\n    indented\n";
    // pulldown's ranges: a fenced block to its closing fence, an indented
    // one from its text.
    assert_eq!(covered(text, Kind::CodeBlock), ["```rust title\nfn x() {}\n```", "indented\n"]);
    assert_eq!(covered(text, Kind::Language), ["rust title"]);
    assert_eq!(kinds_at(text, "fn x"), [Kind::CodeBlock]);
    assert_eq!(kinds_at(text, "```\n\n"), [Kind::Markup, Kind::CodeBlock]);
    // Unclosed, the block runs to the end, and nothing in it is Markdown.
    let open = "```\n# not a heading\n";
    assert_eq!(covered(open, Kind::Heading(1)), Vec::<&str>::new());
    assert_eq!(kinds_at(open, "# not"), [Kind::CodeBlock]);
    // The language is the language's, not markup.
    assert_eq!(kinds_at(text, "rust"), [Kind::Language, Kind::CodeBlock]);
    assert_eq!(kinds_at(text, "```rust"), [Kind::Markup, Kind::CodeBlock]);
}

#[test]
fn lists_tasks_and_quotes() {
    let text = "- a\n- [x] b\n10. c\n\n> q\n> > r\n";
    assert_eq!(covered(text, Kind::ListMarker), ["-", "-", "10."]);
    assert_eq!(covered(text, Kind::Task), ["[x]"]);
    assert_eq!(kinds_at(text, "q"), [Kind::Quote]);
    assert_eq!(kinds_at(text, "> >"), [Kind::Markup, Kind::Quote]);
    assert_eq!(kinds_at(text, "r\n"), [Kind::Quote]);
    // A `>` in quoted code is code, not a marker.
    let code = "> ```\n> > literal\n> ```\n";
    let second = code.find("> > literal").unwrap() + 2;
    let at = runs(&spans(code).spans).into_iter().find(|r| r.start <= second && second < r.end).unwrap();
    assert!(!at.kinds.contains(&Kind::Markup), "{:?}", at.kinds);
    assert_eq!(covered(code, Kind::Markup).iter().filter(|m| **m == ">").count(), 3);
}

#[test]
fn tables() {
    // GFM splits cells at every unescaped pipe, code spans included.
    let text = "| a | `b\\|c` |\n|---|:-:|\n| d | e |\n";
    assert_eq!(covered(text, Kind::TableHeader).len(), 1);
    assert_eq!(kinds_at(text, "a |"), [Kind::TableHeader, Kind::Table]);
    assert_eq!(kinds_at(text, "|---"), [Kind::Markup, Kind::Table]);
    assert_eq!(kinds_at(text, "| d"), [Kind::Markup, Kind::Table]);
    assert_eq!(kinds_at(text, "d |"), [Kind::Table]);
    // An escaped pipe is the cell's.
    let pipe = text.find("b\\|c").unwrap() + 2;
    let at_pipe = runs(&spans(text).spans).into_iter().find(|r| r.start <= pipe && pipe < r.end).unwrap();
    assert!(!at_pipe.kinds.contains(&Kind::Markup), "{:?}", at_pipe.kinds);
}

#[test]
fn runs_are_ascending_disjoint_and_joined() {
    let text = "# A **b _c_** d\n\n- `x` [y](z)\n";
    let runs = runs(&spans(text).spans);
    for pair in runs.windows(2) {
        assert!(pair[0].end <= pair[1].start, "{pair:?}");
        assert!(pair[0].end < pair[1].start || pair[0].kinds != pair[1].kinds, "{pair:?}");
    }
    assert!(runs.iter().all(|r| r.start < r.end && !r.kinds.is_empty()));
}

#[test]
fn restart_steps_back_a_block() {
    let parsed = spans("a\n\nb\n\nc\n");
    assert_eq!(parsed.tops, [0, 3, 6]);
    assert_eq!(restart(&parsed.tops, 7), 3);
    assert_eq!(restart(&parsed.tops, 4), 0);
    assert_eq!(restart(&parsed.tops, 0), 0);
    assert_eq!(restart(&[], 5), 0);
}

/// What streaming relies on: text arriving at the end never changes the
/// runs before the restart point, so a reply refaced from there after each
/// chunk ends as if faced whole (link definitions aside, which the end of a
/// reply refaces whole for).
#[test]
fn appended_text_leaves_runs_before_restart() {
    let samples = [
        "# Plan\n\nSome *text* with `code`.\n\n1. one\n2. two\n\n10. ten\n\n```rust\nfn x() {}\n```\n\nTail **bold**\n",
        "| a | b |\n|---|---|\n| c | d |\n\nPara\n===\n\n> quote\ncontinued\n\n- item\n\n  more\n- next\n",
        "Text\n\n---\n\n<div>\nhtml\n</div>\n\n- [ ] task\n  - nested `x`\n\n~~~\nopen fence",
    ];
    for text in samples {
        let cuts: Vec<usize> = (0..=text.len()).filter(|&i| text.is_char_boundary(i)).collect();
        for (i, &cut) in cuts.iter().enumerate() {
            let old = spans(&text[..cut]);
            let old_runs = runs(&old.spans);
            // Every later end: a chunk of any length arriving at once.
            for &next in &cuts[i + 1..] {
                let new = spans(&text[..next]);
                let from = restart(&new.tops, cut);
                let clip = |runs: &[Run]| -> Vec<Run> {
                    runs.iter().filter(|r| r.start < from).map(|r| Run { end: r.end.min(from), ..r.clone() }).collect()
                };
                assert_eq!(clip(&old_runs), clip(&runs(&new.spans)), "appending {:?} after {:?}", &text[cut..next], &text[..cut]);
            }
        }
    }
}

#[test]
fn blocks_name_structure() {
    let text = "# T\n\n```py\nx = 1\n```\n\n1. a\n   - b\n\n| h |\n|---|\n\n> q\n\n***\n";
    let blocks = blocks(text);
    let kinds: Vec<_> = blocks.iter().map(|b| (b.kind.clone(), &text[b.range.clone()], b.depth)).collect();
    assert_eq!(kinds[0], (BlockKind::Heading(1), "# T\n", 0));
    match &blocks[1].kind {
        BlockKind::Code { language, body } => {
            assert_eq!(language.as_deref(), Some("py"));
            assert_eq!(&text[body.clone()], "x = 1\n");
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(blocks[2].kind, BlockKind::Item { ordered: true }));
    assert_eq!((blocks[3].kind.clone(), blocks[3].depth), (BlockKind::Item { ordered: false }, 1));
    assert_eq!(blocks[4].kind, BlockKind::Table);
    assert_eq!(blocks[5].kind, BlockKind::Quote);
    assert_eq!(blocks[6].kind, BlockKind::Rule);
}

#[test]
fn kind_names_and_indexes_agree() {
    for (i, kind) in Kind::ALL.iter().enumerate() {
        assert_eq!(kind.index(), i);
    }
    let names: std::collections::HashSet<_> = Kind::ALL.iter().map(|k| k.name()).collect();
    assert_eq!(names.len(), Kind::ALL.len());
}

/// Every character one column.
fn narrow(_: char) -> usize {
    1
}

#[test]
fn grid_draws_a_table() {
    let drawn = grid("| a | bb |\n|---|---|\n| ccc | d |\n", 40, &narrow).expect("a table");
    assert_eq!(drawn.text, "+-----+----+\n| a   | bb |\n+-----+----+\n| ccc | d  |\n+-----+----+\n");
    let kinds_at = |needle: &str| {
        let at = drawn.text.find(needle).unwrap();
        drawn.runs.iter().find(|r| r.start <= at && at < r.end).map(|r| r.kinds.clone()).unwrap_or_default()
    };
    assert_eq!(kinds_at("+-"), [Kind::Markup, Kind::Table]);
    assert_eq!(kinds_at("a "), [Kind::TableHeader, Kind::Table]);
    assert_eq!(kinds_at("ccc"), [Kind::Table]);
    assert!(grid("no table", 40, &narrow).is_none());
}

#[test]
fn grid_wraps_cells_and_keeps_their_kinds() {
    let drawn = grid("| k | v |\n|---|---|\n| x | one `two` three |\n", 16, &narrow).expect("a table");
    for line in drawn.text.lines() {
        assert!(line.chars().count() <= 16, "{line:?} in\n{}", drawn.text);
    }
    assert!(drawn.text.contains("| one "), "{}", drawn.text);
    let at = drawn.text.find("two").unwrap();
    let run = drawn.runs.iter().find(|r| r.start <= at && at < r.end).unwrap();
    assert_eq!(run.kinds, [Kind::Code, Kind::Table]);
}

#[test]
fn grid_splits_wide_tables_into_panels() {
    let source = "| a | b | c | d | e | f |\n|---|---|---|---|---|---|\n| 1 | 2 | 3 | 4 | 5 | 6 |\n";
    let drawn = grid(source, 20, &narrow).expect("a table");
    // Each panel repeats the header row of its columns.
    assert!(drawn.text.contains("| a  |") && drawn.text.contains("| d  |"), "{}", drawn.text);
    assert!(drawn.text.lines().all(|l| l.chars().count() <= 20), "{}", drawn.text);
    assert!(drawn.text.contains("\n\n"), "{}", drawn.text);
}

#[test]
fn grid_wraps_wide_characters_within_the_width() {
    let wide = |c: char| if c == '中' { 2 } else { 1 };
    let drawn = grid("| a | b | c |\n|---|---|---|\n| x | y | abcdef abcde中 |\n", 20, &wide).expect("a table");
    for line in drawn.text.lines() {
        assert!(line.chars().map(wide).sum::<usize>() <= 20, "{line:?} in\n{}", drawn.text);
    }
}

#[test]
fn grid_fills_ragged_rows() {
    let drawn = grid("| a | b |\n|---|---|\n| only |\n", 40, &narrow).expect("a table");
    assert!(drawn.text.contains("| only |    |"), "{}", drawn.text);
}

/// TEXT's characters START..END replaced by INSERT.
fn replace_chars(text: &mut String, start: usize, end: usize, insert: &str) {
    let byte = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(i, _)| i);
    let (a, b) = (byte(start), byte(end));
    text.replace_range(a..b, insert);
}

/// An index followed through edits keeps the block starts a fresh read of
/// the text finds, edits that join, split, open and close blocks included.
#[test]
fn index_follows_edits() {
    let mut text = String::from(
        "# Título\n\nSome *text* — `code`.\n\n1. one\n2. two\n\n```rust\nfn x() {}\n```\n\n| a | b |\n|---|---|\n| c | d |\n\n> quote\n> more\n\nTail ü\n",
    );
    let snippets = ["\n", "\n\n", "```", "```\n", "- ", "1. ", "> ", "# ", "===\n", "---\n", "    ", "*", "`", "|", "x", "é", "[r]: /u\n"];
    let mut index = Index::default();
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = |m: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        usize::try_from(seed % (m.max(1) as u64)).unwrap_or(0)
    };
    for i in 0..1500 {
        let len = text.chars().count();
        let current = text.clone();
        index.sync(len, &mut |r| current.chars().skip(r.start).take(r.end - r.start).collect());
        let start = next(len + 1);
        let (end, insert) = if next(3) == 0 { ((start + 1 + next(12)).min(len), "") } else { (start, snippets[next(snippets.len())]) };
        replace_chars(&mut text, start, end, insert);
        index.edit(start, end, insert.chars().count());
        let len = text.chars().count();
        let current = text.clone();
        index.sync(len, &mut |r| current.chars().skip(r.start).take(r.end - r.start).collect());
        let mut fresh = Index::default();
        fresh.sync(len, &mut |r| current.chars().skip(r.start).take(r.end - r.start).collect());
        assert_eq!(index.starts(), fresh.starts(), "edit {i}: {start}..{end} -> {insert:?} in\n{text}");
    }
}

#[test]
fn index_blocks_hold_a_region() {
    let text = "a\n\nb\n\nc\n";
    let mut index = Index::default();
    index.sync(text.len(), &mut |r| text[r].to_owned());
    assert_eq!(index.blocks(4, 4), 3..6);
    assert_eq!(index.blocks(0, 4), 0..6);
    assert_eq!(index.blocks(6, 8), 6..8);
}
