//! treesit-fuzz FILE EDITS SEED: FILE parsed with the grammar Romacs ships for
//! its extension, then EDITS seeded typing-like edits, each followed by a
//! reparse from the edited tree. One line per state: the edit's number, a
//! digest of the whole tree (each node's kind, extent, error and missing
//! marks, child count and field), and the reparse's microseconds. Built
//! against two runtimes, the digest columns must be equal line for line.
use std::hash::{Hash, Hasher};
use tree_sitter::{InputEdit, Parser, Point, Tree};

/// The shipped grammar for PATH's extension.
fn language(path: &str) -> tree_sitter::Language {
    let extension = path.rsplit('.').next().unwrap_or_default();
    let name = match extension {
        "c" | "h" => "c",
        "cc" | "cpp" | "hpp" => "cpp",
        "py" => "python",
        "rs" => "rust",
        "go" => "go",
        "js" => "javascript",
        "ts" => "typescript",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "sh" | "bash" => "bash",
        "css" => "css",
        "html" => "html",
        other => panic!("no shipped grammar for .{other}"),
    };
    let grammar = romacs_grammars::BUNDLED.iter().find(|g| g.name == name).expect("a shipped grammar");
    grammar.language.into()
}

fn point(text: &[u8], byte: usize) -> Point {
    let before = &text[..byte];
    let row = before.iter().filter(|&&b| b == b'\n').count();
    let column = byte - before.iter().rposition(|&b| b == b'\n').map_or(0, |p| p + 1);
    Point { row, column }
}

/// xorshift64*, seeded; its state is never zero.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// The next edit: (start, old end, new text). Mostly where the last one
/// ended, sometimes a jump; code characters, newlines, deletions and a pasted
/// line, as a person types.
fn next_edit(rng: &mut Rng, text: &[u8], at: &mut usize) -> (usize, usize, Vec<u8>) {
    if rng.below(20) == 0 || *at > text.len() {
        *at = rng.below(text.len() as u64 + 1) as usize;
    }
    let pos = (*at).min(text.len());
    let pieces: [&[u8]; 14] = [b"x", b"x", b"a", b"_", b"1", b" ", b";", b"{", b"}", b"(", b")", b"\n", b"\"", b"/*"];
    let edit = match rng.below(10) {
        0..=5 => (pos, pos, pieces[rng.below(pieces.len() as u64) as usize].to_vec()),
        6 | 7 if pos > 0 => (pos - 1, pos, vec![]),
        8 => (pos, pos + (rng.below(40) as usize).min(text.len() - pos), vec![]),
        _ => (pos, pos, b"int z = f(a, b);\n".to_vec()),
    };
    *at = edit.0 + edit.2.len();
    edit
}

fn digest(tree: &Tree) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        (node.kind_id(), node.byte_range(), node.is_missing(), node.is_error(), node.child_count(),
         cursor.field_id().map(std::num::NonZero::get), node.start_position(), node.end_position()).hash(&mut hash);
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return hash.finish();
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, path, edits, seed] = &args[..] else { panic!("usage: treesit-fuzz FILE EDITS SEED") };
    let (edits, seed): (usize, u64) = (edits.parse().expect("EDITS"), seed.parse().expect("SEED"));
    let mut text = std::fs::read(path).expect("FILE");
    let mut parser = Parser::new();
    parser.set_language(&language(path)).expect("a compatible grammar");
    let mut tree = parser.parse(&text, None).expect("a tree");
    println!("0 {:016x} 0", digest(&tree));
    let mut rng = Rng(seed.max(1));
    let mut at = text.len() / 2;
    for i in 1..=edits {
        let (start, old_end, new) = next_edit(&mut rng, &text, &mut at);
        let (start_position, old_end_position) = (point(&text, start), point(&text, old_end));
        text.splice(start..old_end, new.iter().copied());
        let new_end = start + new.len();
        tree.edit(&InputEdit { start_byte: start, old_end_byte: old_end, new_end_byte: new_end, start_position,
                               old_end_position, new_end_position: point(&text, new_end) });
        let started = std::time::Instant::now();
        tree = parser.parse(&text, Some(&tree)).expect("a tree");
        println!("{i} {:016x} {}", digest(&tree), started.elapsed().as_micros());
    }
}
