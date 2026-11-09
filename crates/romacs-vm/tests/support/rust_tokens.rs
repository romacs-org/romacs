//! Rust source as tokens, for the source checks of the rooting rule
//! (`reentry_sites.rs` for unconverted files, `rooting_brand_rules.rs` for
//! converted ones).

/// Modules converted to rooting as a type (`romacs_brand`): the brand's own
/// source checks cover what the types leave.
pub const CONVERTED: &[&str] = &["command.rs", "threaded.rs", "threaded_signal.rs", "threaded_lend.rs", "traversal.rs"];

#[derive(Debug, PartialEq)]
pub enum Token {
    Ident(String),
    Punct(char),
}

/// Rust tokens with comments, strings and character literals removed:
/// identifiers and single punctuation characters, which is all the scan
/// needs to see calls, declarations and braces.
pub fn tokens(source: &str) -> Vec<Token> {
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
