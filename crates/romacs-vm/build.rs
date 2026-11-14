use sha2::{Digest, Sha256};
use std::{env, fs, path::Path, process::Command};

fn sources(root: &Path, dir: &Path, hash: &mut Sha256) {
    let mut paths: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            sources(root, &path, hash);
        } else if path.extension().is_some_and(|e| e == "rs") {
            println!("cargo:rerun-if-changed={}", path.display());
            hash.update(
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .as_bytes(),
            );
            let bytes = fs::read(&path).unwrap();
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        }
    }
}
// A small lexical scanner, not a line/substring matcher. Bodies remain Rust's
// responsibility; the declaration grammar here intentionally mirrors defsubr!.
#[derive(Debug)]
struct Token {
    text: String,
    string: Option<String>,
    line: usize,
}

fn primitive_error(path: &Path, line: usize, message: &str) -> ! {
    panic!("{}:{line}: primitive registry: {message}", path.display())
}

fn decode_string(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        out.push(match chars.next()? {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            '0' => '\0',
            '\\' => '\\',
            '\'' => '\'',
            '"' => '"',
            'x' => {
                let a = chars.next()?.to_digit(16)?;
                let b = chars.next()?.to_digit(16)?;
                let value = a * 16 + b;
                if value > 127 {
                    return None;
                }
                char::from_u32(value)?
            }
            'u' => {
                if chars.next()? != '{' {
                    return None;
                }
                let mut digits = String::new();
                loop {
                    let c = chars.next()?;
                    if c == '}' {
                        break;
                    }
                    if c != '_' {
                        digits.push(c);
                    }
                }
                if digits.is_empty() || digits.len() > 6 {
                    return None;
                }
                char::from_u32(u32::from_str_radix(&digits, 16).ok()?)?
            }
            '\n' => {
                while chars.clone().next().is_some_and(char::is_whitespace) {
                    chars.next();
                }
                continue;
            }
            '\r' if chars.next()? == '\n' => {
                while chars.clone().next().is_some_and(char::is_whitespace) {
                    chars.next();
                }
                continue;
            }
            _ => return None,
        });
    }
    Some(out)
}

fn tokens(path: &Path, source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    let (mut i, mut line) = (0, 1);
    while i < bytes.len() {
        let start = i;
        let token_line = line;
        if bytes[i].is_ascii_whitespace() {
            if bytes[i] == b'\n' {
                line += 1;
            }
            i += 1;
            continue;
        }
        if source[i..].starts_with("//") {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if source[i..].starts_with("/*") {
            i += 2;
            let mut depth = 1;
            while i < bytes.len() && depth != 0 {
                if source[i..].starts_with("/*") {
                    depth += 1;
                    i += 2;
                } else if source[i..].starts_with("*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    if bytes[i] == b'\n' {
                        line += 1;
                    }
                    i += source[i..].chars().next().unwrap().len_utf8();
                }
            }
            if depth != 0 {
                primitive_error(path, token_line, "unterminated comment");
            }
            continue;
        }
        // Raw strings (including byte/C strings) must precede identifier lexing.
        let prefix = if source[i..].starts_with("br") || source[i..].starts_with("cr") {
            2
        } else if bytes[i] == b'r' {
            1
        } else {
            0
        };
        let mut quote = i + prefix;
        if prefix != 0 {
            while bytes.get(quote) == Some(&b'#') {
                quote += 1;
            }
        }
        let mut string = None;
        if prefix != 0 && bytes.get(quote) == Some(&b'"') {
            let hashes = quote - i - prefix;
            let end_marker = format!("\"{}", "#".repeat(hashes));
            let content = quote + 1;
            let end = source[content..]
                .find(&end_marker)
                .map(|n| content + n)
                .unwrap_or_else(|| primitive_error(path, token_line, "unterminated raw string"));
            if prefix == 1 {
                string = Some(source[content..end].to_owned());
            }
            i = end + end_marker.len();
        } else {
            let quote = if (bytes[i] == b'b' || bytes[i] == b'c') && bytes.get(i + 1) == Some(&b'"')
            {
                i + 1
            } else {
                i
            };
            if bytes[quote] == b'"' {
                i = quote + 1;
                let content = i;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    if i < bytes.len() {
                        i += source[i..].chars().next().unwrap().len_utf8();
                    }
                }
                if i == bytes.len() {
                    primitive_error(path, token_line, "unterminated string");
                }
                if quote == start {
                    string = Some(decode_string(&source[content..i]).unwrap_or_else(|| {
                        primitive_error(path, token_line, "invalid string escape")
                    }));
                }
                i += 1;
            } else {
                // Char literals are opaque; lifetimes ('static) are a distinct
                // token so the static-keyword policy doesn't reject references.
                let apostrophe = if bytes[i] == b'b' && bytes.get(i + 1) == Some(&b'\'') {
                    i + 1
                } else {
                    i
                };
                if bytes[apostrophe] == b'\'' {
                    let mut end = apostrophe + 1;
                    if bytes.get(end) == Some(&b'\\') {
                        end += 1;
                        if bytes.get(end) == Some(&b'u') && bytes.get(end + 1) == Some(&b'{') {
                            end += 2;
                            while end < bytes.len() && bytes[end] != b'}' {
                                end += 1;
                            }
                            end += usize::from(end < bytes.len());
                        } else if bytes.get(end) == Some(&b'x') {
                            end += 3;
                        } else if end < bytes.len() {
                            end += source[end..].chars().next().unwrap().len_utf8();
                        }
                    } else if end < bytes.len() {
                        end += source[end..].chars().next().unwrap().len_utf8();
                    }
                    if bytes.get(end) == Some(&b'\'') {
                        i = end + 1;
                    } else {
                        i = apostrophe + 1;
                        while i < bytes.len()
                            && (source[i..].chars().next().unwrap().is_alphanumeric()
                                || bytes[i] == b'_')
                        {
                            i += source[i..].chars().next().unwrap().len_utf8();
                        }
                    }
                } else {
                    let c = source[i..].chars().next().unwrap();
                    i += c.len_utf8();
                    if c.is_alphabetic() || c == '_' {
                        while i < bytes.len()
                            && (source[i..].chars().next().unwrap().is_alphanumeric()
                                || bytes[i] == b'_')
                        {
                            i += source[i..].chars().next().unwrap().len_utf8();
                        }
                    }
                }
            }
        }
        line += source[start..i].bytes().filter(|&b| b == b'\n').count();
        result.push(Token {
            text: source[start..i].to_owned(),
            string,
            line: token_line,
        });
    }
    result
}

struct Header<'a> {
    tokens: &'a [Token],
    pos: usize,
    path: &'a Path,
    line: usize,
}
impl Header<'_> {
    fn fail(&self, message: &str) -> ! {
        primitive_error(self.path, self.line, message)
    }
    fn peek(&self) -> &str {
        self.tokens.get(self.pos).map_or("", |t| t.text.as_str())
    }
    fn expect(&mut self, text: &str) {
        if self.peek() != text {
            self.fail(&format!(
                "malformed declaration: expected {text:?}, found {:?}",
                self.peek()
            ));
        }
        self.pos += 1;
    }
    fn ident(&mut self) -> String {
        let text = self.peek();
        if text.is_empty()
            || !text.chars().enumerate().all(|(i, c)| {
                c == '_'
                    || if i == 0 {
                        c.is_alphabetic()
                    } else {
                        c.is_alphanumeric()
                    }
            })
        {
            self.fail("malformed declaration: expected identifier");
        }
        let value = text.to_owned();
        self.pos += 1;
        value
    }
    fn string(&mut self) -> String {
        let value = self
            .tokens
            .get(self.pos)
            .and_then(|t| t.string.clone())
            .unwrap_or_else(|| self.fail("malformed declaration: expected string literal"));
        self.pos += 1;
        if value.is_empty() {
            self.fail("empty declaration string");
        }
        value
    }
    fn group(&mut self) {
        let close = match self.peek() {
            "(" => ")",
            "[" => "]",
            "{" => "}",
            _ => self.fail("malformed declaration: expected delimited group"),
        };
        self.pos += 1;
        while self.peek() != close {
            match self.peek() {
                "" | ")" | "]" | "}" => self.fail("malformed declaration: unbalanced delimiters"),
                "(" | "[" | "{" => self.group(),
                _ => self.pos += 1,
            }
        }
        self.pos += 1;
    }
}

struct PrimitiveRow {
    name: String,
    module: String,
    ident: String,
    gnu: String,
    effect: String,
    source: String,
    line: usize,
    test_only: bool,
    arity: usize,
    max: Option<usize>,
}

fn declaration(path: &Path, ts: &[Token], pos: usize, module: &str, source: &str) -> PrimitiveRow {
    let mut h = Header {
        tokens: ts,
        pos: pos + 2,
        path,
        line: ts[pos].line,
    };
    let close = match h.peek() {
        "{" => "}",
        "(" => ")",
        "[" => "]",
        _ => h.fail("malformed declaration: expected macro delimiter"),
    };
    h.pos += 1;
    h.expect("name");
    h.expect("=");
    let name = h.string();
    h.expect(",");
    h.expect("gnu");
    h.expect("=");
    let gnu = h.string();
    h.expect(",");
    h.expect("effect");
    h.expect("=");
    let effect = h.ident();
    h.expect(",");
    h.expect("fn");
    let ident = h.ident();
    h.expect("(");
    let mut names = std::collections::BTreeSet::new();
    names.insert(h.ident()); // untyped context argument
    let mut arity = 0;
    let mut optional = 0;
    let mut in_optional = false;
    let mut many = false;
    while h.peek() != ")" {
        if h.peek() == ";" {
            h.expect(";");
            match h.ident().as_str() {
                "optional" if !in_optional => in_optional = true,
                "rest" => {
                    if !names.insert(h.ident()) {
                        h.fail("duplicate argument identifier");
                    }
                    many = true;
                    if h.peek() == "," {
                        h.expect(",");
                    }
                    break;
                }
                _ => h.fail("expected optional arguments followed by rest"),
            }
        } else {
            h.expect(",");
            if h.peek() == ")" {
                break;
            }
        }
        if !names.insert(h.ident()) {
            h.fail("duplicate argument identifier");
        }
        h.expect(":");
        let ty = h.ident();
        if !matches!(ty.as_str(), "Value" | "Integer" | "List") {
            h.fail("unsupported primitive argument type (expected Value, Integer or List)");
        }
        if in_optional {
            optional += 1;
        } else {
            arity += 1;
        }
    }
    h.expect(")");
    // A future Step return marker can change the body without changing header
    // discovery. Rust still checks the actual macro/body grammar.
    if h.peek() == "-" {
        h.expect("-");
        h.expect(">");
        h.ident();
    }
    if h.peek() != "{" {
        h.fail("malformed declaration: expected function body");
    }
    h.group();
    h.expect(close);
    PrimitiveRow {
        name,
        module: module.to_owned(),
        ident,
        gnu,
        effect,
        source: source.to_owned(),
        line: h.line,
        test_only: is_test_source(source),
        arity,
        max: (!many).then_some(arity + optional),
    }
}

fn rust_files(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
}

// Rust Debug string escaping isn't JSON escaping (e.g. \u{7f}).
fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c <= '\u{1f}' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn is_test_source(source: &str) -> bool {
    source.contains("/tests/") || source.ends_with("/tests.rs")
        || source.ends_with("_test.rs") || source.ends_with("_tests.rs")
}

fn generate_primitives(root: &Path, out: &Path) {
    let dir = root.join("src/prims");
    let mut files = Vec::new();
    rust_files(&dir, &mut files);
    files.sort();
    let mut mods = std::collections::BTreeMap::new();
    let mut rows = Vec::new();
    for path in &files {
        let relative = path.strip_prefix(&dir).unwrap();
        // Nested domain files are independent generated modules; the root
        // mod.rs is already included by prim.rs and needs no module declaration.
        let module = if relative == Path::new("mod.rs") {
            String::new()
        } else {
            relative
                .with_extension("")
                .components()
                .map(|c| c.as_os_str().to_str().unwrap().to_owned())
                .collect::<Vec<_>>()
                .join("__")
        };
        let source = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if !module.is_empty() {
            assert!(
                module.chars().enumerate().all(|(i, c)| c == '_'
                    || if i == 0 {
                        c.is_alphabetic()
                    } else {
                        c.is_alphanumeric()
                    }),
                "invalid primitive module name: {}",
                path.display()
            );
            assert!(
                mods.insert(module.clone(), (path, is_test_source(&source)))
                    .is_none(),
                "duplicate primitive module {module}"
            );
        }
        let ts = tokens(path, &fs::read_to_string(path).unwrap());
        for (i, token) in ts.iter().enumerate() {
            if token.text == "static"
                || (token.text == "thread_local" && ts.get(i + 1).is_some_and(|t| t.text == "!"))
            {
                primitive_error(
                    path,
                    token.line,
                    "static and thread_local! are forbidden under src/prims",
                );
            }
            if token.text == "defsubr" && ts.get(i + 1).is_some_and(|t| t.text == "!") {
                rows.push(declaration(path, &ts, i, &module, &source));
            }
        }
    }
    rows.sort_by(|a, b| a.test_only.cmp(&b.test_only).then_with(|| a.name.cmp(&b.name)));
    let mut rust_names = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    for row in &rows {
        assert!(
            names.insert(row.name.clone()),
            "primitive {} declared twice",
            row.name
        );
        assert!(
            rust_names.insert((&row.module, &row.ident)),
            "primitive Rust identifier {} declared twice in {}",
            row.ident,
            row.source
        );
    }
    // Read the actual legacy table, rather than maintaining another name list.
    let lib = root.join("src/lib.rs");
    let legacy = tokens(&lib, &fs::read_to_string(&lib).unwrap());
    let starts: Vec<_> = legacy
        .windows(2)
        .enumerate()
        .filter(|(_, w)| w[0].text == "PRIMITIVES" && w[1].text == ":")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        starts.len(),
        1,
        "primitive registry: expected one legacy PRIMITIVES declaration"
    );
    let mut h = Header {
        tokens: &legacy,
        pos: starts[0],
        path: &lib,
        line: legacy[starts[0]].line,
    };
    while h.peek() != "=" && !h.peek().is_empty() {
        h.pos += 1;
    }
    h.expect("=");
    h.expect("&");
    h.expect("[");
    while h.peek() != "]" {
        h.expect("(");
        let name = h.string();
        assert!(
            names.insert(name.clone()),
            "primitive {name} declared twice (legacy PRIMITIVES collision)"
        );
        h.expect(",");
        // The tuple's implementation expression belongs to Rust, not this scanner.
        while h.peek() != ")" {
            match h.peek() {
                "" | "]" | "}" => h.fail("malformed legacy primitive tuple"),
                "(" | "[" | "{" => h.group(),
                _ => h.pos += 1,
            }
        }
        h.expect(")");
        if h.peek() != "]" {
            h.expect(",");
        }
    }
    let modules: String = mods
        .iter()
        .map(|(module, (path, test_only))| {
            format!(
                "{}#[path = {:?}] pub(crate) mod {module};\n",
                if *test_only { "#[cfg(test)] " } else { "" },
                path.display().to_string()
            )
        })
        .collect();
    let table: String = rows
        .iter()
        .map(|r| {
            let module = if r.module.is_empty() {
                String::new()
            } else {
                format!("{}::", r.module)
            };
            format!("    {}prims::{module}{}::DESC,\n", if r.test_only { "#[cfg(test)] " } else { "" }, r.ident)
        })
        .collect();
    let manifest: Vec<_> = rows.iter().filter(|r| !r.test_only).map(|r| format!(
        "{{\"name\":{},\"rust\":{},\"ident\":{},\"gnu\":{},\"effect\":{},\"min\":{},\"max\":{}}}",
        json_string(&r.name), json_string(&format!("{}:{}", r.source, r.line)), json_string(&r.ident), json_string(&r.gnu), json_string(&r.effect), r.arity, r.max.map_or_else(|| json_string("many"), |n| n.to_string()))).collect();
    fs::write(out.join("prims_mods.rs"), modules).unwrap();
    fs::write(
        out.join("generated_prims.rs"),
        format!("pub(crate) static GENERATED: &[PrimDesc] = &[\n{table}];\n"),
    )
    .unwrap();
    fs::write(
        out.join("primitives.json"),
        format!("[{}]\n", manifest.join(",\n")),
    )
    .unwrap();
}

/// TOKENS without the items `#[cfg(test)]' or `#[test]' marks: test code
/// declares variables production never does.
fn production_tokens(tokens: &[Token]) -> Vec<&Token> {
    let text = |i: usize| tokens.get(i).map_or("", |t| t.text.as_str());
    let (mut kept, mut i) = (Vec::new(), 0);
    while i < tokens.len() {
        let test_attribute = text(i) == "#" && text(i + 1) == "["
            && ((text(i + 2) == "cfg" && text(i + 3) == "(" && text(i + 4) == "test" && text(i + 5) == ")" && text(i + 6) == "]")
                || (text(i + 2) == "test" && text(i + 3) == "]"));
        if !test_attribute { kept.push(&tokens[i]); i += 1; continue; }
        // The item: to its `;' or through its balanced `{...}', at depth 0.
        let mut depth = 0i32;
        while i < tokens.len() {
            match text(i) {
                "(" | "[" => depth += 1,
                ")" | "]" => depth -= 1,
                ";" if depth == 0 => { i += 1; break; }
                "{" if depth == 0 => {
                    let mut braces = 0;
                    while i < tokens.len() {
                        match text(i) { "{" => braces += 1, "}" => { braces -= 1; if braces == 0 { break; } } _ => {} }
                        i += 1;
                    }
                    i += 1;
                    break;
                }
                _ => {}
            }
            i += 1;
        }
    }
    kept
}

/// Where Rust declares each variable GNU declares with DEFVAR, outside
/// test code: a C_VARIABLES row (`CVariable { name: "NAME"'), or a
/// `declare_special_raw' call, whose name is a string or a `for' loop's
/// variable over a list of names (or of tuples headed by one).  (NAME,
/// FILE relative to the source tree, LINE of its name's literal, that
/// literal), the first for each name, for `romacs--rust-definition'.
fn generate_variable_sites(root: &Path, out: &Path) {
    fn walk(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
        let mut paths: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
        paths.sort();
        for path in paths {
            if path.is_dir() { walk(&path, files); }
            else if path.extension().is_some_and(|e| e == "rs") && !path.file_name().unwrap().to_string_lossy().contains("test") { files.push(path); }
        }
    }
    let mut files = vec![];
    walk(&root.join("src"), &mut files);
    let (mut seen, mut rows) = (std::collections::HashSet::new(), String::new());
    for path in files {
        let relative = format!("crates/romacs-vm/{}", path.strip_prefix(root).unwrap().display());
        let all = tokens(&path, &fs::read_to_string(&path).unwrap());
        let toks = production_tokens(&all);
        let text = |i: usize| toks.get(i).map_or("", |t| t.text.as_str());
        let string = |i: usize| toks.get(i).and_then(|t| t.string.clone());
        let mut add = |name: String, line: usize| {
            if seen.insert(name.clone()) { rows.push_str(&format!("    ({name:?}, {relative:?}, {line}, {:?}),\n", format!("{name:?}"))); }
        };
        for i in 0..toks.len() {
            if text(i) == "CVariable" && text(i + 1) == "{" && text(i + 2) == "name" && text(i + 3) == ":" {
                if let Some(name) = string(i + 4) { add(name, toks[i + 4].line); }
            }
            if text(i) != "declare_special_raw" || text(i + 1) != "(" { continue; }
            if let Some(name) = string(i + 2) { add(name, toks[i + 2].line); continue; }
            // `for NAME in [...]' or `for (NAME, ..) in [...]' before it.
            let variable = text(i + 2);
            let Some(head) = (0..i).rev().find(|&j| text(j) == "for"
                && (text(j + 1) == variable || (text(j + 1) == "(" && text(j + 2) == variable))) else { continue };
            let Some(after) = (head..i).find(|&j| text(j) == "in").map(|j| j + 1) else { continue };
            // An inline list, or a named one: the `const' or `static' this
            // file declares by that name, from its value's `['.
            let open = if text(after) == "[" { after } else if text(after).chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                let name = text(after);
                let Some(open) = (0..toks.len()).find(|&k| matches!(text(k), "const" | "static") && text(k + 1) == name)
                    .and_then(|k| (k..toks.len()).find(|&j| text(j) == "=")).and_then(|k| (k..toks.len()).find(|&j| text(j) == "[")) else { continue };
                open
            } else { continue };
            let tuples = text(head + 1) == "(";
            let mut depth = 0;
            for j in open..toks.len() {
                match text(j) {
                    "[" | "(" => depth += 1,
                    "]" | ")" => { depth -= 1; if depth == 0 { break; } }
                    _ => {}
                }
                let first = if tuples { depth == 2 && text(j - 1) == "(" } else { depth == 1 };
                if first { if let Some(name) = string(j) { add(name, toks[j].line); } }
            }
        }
    }
    // A legacy primitive's row in lib.rs's PRIMITIVES: ("NAME", Primitive::X).
    let mut legacy = String::new();
    for (n, line) in fs::read_to_string(root.join("src/lib.rs")).unwrap().lines().enumerate() {
        let Some(rest) = line.trim_start().strip_prefix("(\"") else { continue };
        let Some(end) = rest.find("\", Primitive::") else { continue };
        legacy.push_str(&format!("    ({:?}, {}),\n", &rest[..end], n + 1));
    }
    fs::write(out.join("variable_sites.rs"), format!("pub(crate) static VARIABLE_SITES: &[(&str, &str, u32, &str)] = &[\n{rows}];\n\
        pub(crate) static LEGACY_SITES: &[(&str, u32)] = &[\n{legacy}];\n")).unwrap();
}

#[cfg(test)]
#[path = "build/attribution_tests.rs"]
mod attribution_tests;

/// Add this checkout to the list, in the Cargo profile directory it is building into
/// (`target/PROFILE/romacs-build-trees`), of the trees built there. An executable in that
/// directory assumes the tree compiled into it only when the list names it (host.rs,
/// `in_build_target`); one carried into another build's directory finds another list.
fn name_build_tree(root: &Path, out: &Path) {
    let (Some(profile), Some(tree)) = (out.ancestors().nth(3), root.parent().and_then(Path::parent)) else { return };
    let (list, line) = (profile.join("romacs-build-trees"), tree.display().to_string());
    let known = fs::read_to_string(&list).unwrap_or_default();
    if !known.lines().any(|known| known == line) {
        fs::write(&list, format!("{known}{line}\n")).unwrap_or_else(|error| panic!("cannot write {}: {error}", list.display()));
    }
}

fn main() {
    let root = env::current_dir().unwrap();
    let out = std::path::PathBuf::from(env::var_os("OUT_DIR").unwrap());
    name_build_tree(&root, &out);
    generate_primitives(&root, &out);
    generate_variable_sites(&root, &out);
    let mut hash = Sha256::new();
    println!("cargo:rerun-if-changed=src");
    sources(&root, &root.join("src"), &mut hash);
    for path in [
        root.join("Cargo.toml"),
        root.join("build.rs"),
        root.join("../../Cargo.lock"),
    ] {
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "cannot read compiler identity input {}: {error}",
                path.display()
            )
        });
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    let mut features: Vec<_> = env::vars()
        .filter(|(name, _)| {
            name.starts_with("CARGO_FEATURE_")
                || matches!(
                    name.as_str(),
                    "TARGET" | "CARGO_CFG_TARGET_ENDIAN" | "CARGO_CFG_TARGET_POINTER_WIDTH"
                )
        })
        .collect();
    features.sort();
    for (name, value) in features {
        hash.update(name.as_bytes());
        hash.update([0]);
        hash.update(value.as_bytes());
        hash.update([0]);
    }
    let rustc = Command::new(env::var_os("RUSTC").unwrap())
        .arg("-vV")
        .output()
        .unwrap();
    assert!(rustc.status.success());
    hash.update(rustc.stdout);
    println!(
        "cargo:rustc-env=VM2_COMPILER_ID={}",
        hash.finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
}
