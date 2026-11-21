//! The required-literal rejection (`engine/required.rs`): what it derives,
//! and that a search with it answers as the search without it.

use crate::engine::compile::regex_compile;
use crate::engine::pattern::CompiledPattern;
use crate::engine::required::derive_required_literals;
use crate::engine::search::{re_search, with_fastmap_disabled};
use crate::engine::syntax::DefaultSyntaxLookup;

type Found = Option<(usize, Vec<i64>, Vec<i64>)>;

fn compile(source: &str, fold: bool) -> CompiledPattern {
    regex_compile(source, false, fold).unwrap_or_else(|error| panic!("{source:?}: {error:?}"))
}

fn found(pattern: &CompiledPattern, text: &[u8], start: usize, range: isize, point: usize) -> Found {
    re_search(pattern, text, start, range, &DefaultSyntaxLookup, point)
        .map(|(at, regs)| (at, regs.start.to_vec(), regs.end.to_vec()))
}

/// The search twice on one compiled pattern (the second derives the
/// literals), against the search with every skip disabled.
fn agrees(source: &str, fold: bool, text: &[u8], start: usize, range: isize, point: usize, multibyte: bool) {
    let mut pattern = compile(source, fold);
    pattern.set_target_multibyte(multibyte);
    let oracle = with_fastmap_disabled(|| found(&pattern, text, start, range, point));
    for round in 0..2 {
        assert_eq!(
            found(&pattern, text, start, range, point),
            oracle,
            "{source:?} fold={fold} on {text:?} start={start} range={range} point={point} round {round}"
        );
    }
}

/// The file-name handlers' patterns: each has what every match needs.
#[test]
fn the_handler_patterns_require_their_literals() {
    let tramp = compile("\\`/\\(?:-\\|[^/:|]\\{2,\\}\\):", false);
    let epa = compile("\\.gpg\\(~\\|\\.~[0-9]+~\\)?\\'", false);
    let jka = compile(
        "\\(?:\\.tzst\\|\\.zst\\|\\.dz\\|\\.txz\\|\\.xz\\|\\.lzma\\|\\.lz\\|\\.g?z\\|\\.\\(?:tgz\\|svgz\\|sifz\\)\\|\\.tbz2?\\|\\.bz2\\|\\.Z\\)\\(?:~\\|\\.~[-[:alnum:]:#@^._]+\\(?:~[[:digit:]]+\\)?~\\)?\\'",
        false,
    );
    let non_special = compile("\\`/:", false);
    let name = b"/home/user/.cache/scratch/ffc-none";
    for (label, pattern) in [("tramp", &tramp), ("epa", &epa), ("jka", &jka), ("non-special", &non_special)] {
        let required = derive_required_literals(pattern).unwrap_or_else(|| panic!("{label}: nothing derived"));
        assert!(!required.admits(name), "{label} admits an ordinary name");
    }
    for (pattern, name) in [
        (&tramp, &b"/ssh:host:/etc"[..]),
        (&epa, b"/tmp/a.gpg"),
        (&jka, b"/tmp/a.tar.gz"),
        (&non_special, b"/:/tmp"),
    ] {
        assert!(derive_required_literals(pattern).unwrap().admits(name), "{name:?}");
        assert!(found(pattern, name, 0, name.len().cast_signed(), 0).is_some(), "{name:?}");
    }
}

/// Nothing is derived where nothing is proven: a nullable pattern, a
/// case-folded one, one with no literal; and a literal on an optional path
/// is not required.
#[test]
fn nothing_is_required_without_proof() {
    for (source, fold) in [("a*", false), ("x?:?", false), ("ab:", true), ("[a-c]+", false)] {
        let derived = derive_required_literals(&compile(source, fold));
        assert!(derived.is_none(), "{source:?} fold={fold}: {derived:?}");
    }
    let optional = derive_required_literals(&compile("x\\(?::\\)?y", false));
    assert!(optional.is_none_or(|required| required.admits(b"xy")));
}

/// A pattern searched once derives nothing; the second short search does,
/// and a long search never does.
#[test]
fn the_literals_are_derived_by_the_second_short_search() {
    let pattern = compile("\\.gpg\\'", false);
    let name = b"/tmp/notes.txt";
    assert_eq!(found(&pattern, name, 0, name.len().cast_signed(), 0), None);
    assert!(pattern.required.get().is_none());
    assert_eq!(found(&pattern, name, 0, name.len().cast_signed(), 0), None);
    assert!(pattern.required.get().is_some_and(Option::is_some));

    let long = compile("\\.gpg\\'", false);
    let text = vec![b'x'; 1 << 20];
    for _ in 0..3 {
        assert_eq!(found(&long, &text, 0, text.len().cast_signed(), 0), None);
    }
    assert!(long.required.get().is_none(), "a long search must not derive");
}

/// A match ends by the stop (forward, the range's last candidate; backward,
/// `start`): a literal past the stop rejects, and one that ends exactly at the
/// stop is kept.
#[test]
fn the_span_is_the_lowest_candidate_to_the_stop() {
    let text = b"ab:cd";
    // Forward, stop 2: the `:` at 2 lies past it.
    agrees("b:", false, text, 0, 2, 0, true);
    // Forward, stop 3: `b:` ends exactly at it.
    agrees("b:", false, text, 0, 3, 0, true);
    agrees("b:", false, text, 1, 2, 0, true);
    // Backward from 3, and from 4: a match must end by `start`.
    agrees("b:", false, text, 3, -3, 0, true);
    agrees(":c", false, text, 3, -3, 0, true);
    agrees(":c", false, text, 4, -4, 0, true);
    assert!(found(&compile("b:", false), text, 0, 3, 0).is_some());
    assert!(found(&compile(":c", false), text, 4, -4, 0).is_some());
}

/// Case folding, `\=`, raw bytes, unibyte and multibyte targets.
#[test]
fn folding_point_and_representations_agree() {
    agrees("ab:", true, b"AB:", 0, 3, 0, true);
    agrees("a\\=:", false, b"a:", 0, 2, 1, true);
    agrees("a\\=:", false, b"a:", 0, 2, 0, true);
    let multibyte = "é:x".as_bytes();
    agrees("é:", false, multibyte, 0, multibyte.len().cast_signed(), 0, true);
    agrees("\u{e9}:", false, b"\xe9:x", 0, 3, 0, false);
    agrees(":\u{e9}", false, b"x:\xe9", 0, 3, 0, false);
}

/// Generated patterns over a small alphabet, searched forward and backward
/// at every start, against the search without skips.
#[test]
fn generated_searches_agree_with_the_unskipped_search() {
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = move |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        usize::try_from(seed % n as u64).unwrap()
    };
    let atoms = [
        "a", "b", ":", ".gz", "x", "[ab]", "[^:]", ".", "\\(?:a\\|:\\)", "\\(?:xa\\|xb\\)", "a*", ":?", "b+",
        "\\(:\\)", "\\b", "^", "$", "\\`", "\\'", "\\=", "x\\{2,\\}", "\\(?:ab\\)*", "\u{e9}",
    ];
    let alphabet: [&[u8]; 8] = [b"a", b"b", b":", b".", b"g", b"z", b"x", "\u{e9}".as_bytes()];
    for _ in 0..600 {
        let source: String = (0..=next(4)).map(|_| atoms[next(atoms.len())]).collect();
        let fold = next(5) == 0;
        let text: Vec<u8> = (0..next(9)).flat_map(|_| alphabet[next(alphabet.len())].iter().copied()).collect();
        let len = text.len();
        for start in 0..=len {
            let point = next(len + 1);
            agrees(&source, fold, &text, start, (len - start).cast_signed(), point, true);
            agrees(&source, fold, &text, start, -start.cast_signed(), point, true);
        }
    }
}
