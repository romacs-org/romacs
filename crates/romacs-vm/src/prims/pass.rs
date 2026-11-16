//! The password store's primitives (`crate::pass`), for
//! lisp/romacs-pass.el.  TEXT is an entry's decrypted text: each
//! primitive reads it once and returns only what it was asked for, and
//! its own copies of it are wiped when it returns.
use super::{Fault, Value};
use super::orderless::list;
use zeroize::Zeroizing;

// The names of the entries under DIRECTORY, sorted: each `NAME.gpg'
// below it, hidden ones aside, links followed (`crate::pass::store').
defsubr! { name = "romacs-pass--entries", gnu = "romacs:pass", effect = Reads,
fn romacs_pass_entries(cx, directory: Value) {
    let root = cx.string_text(directory)?;
    let names = crate::pass::store::entries(std::path::Path::new(&root))
        .map_err(|e| Fault::Error(format!("Password store is not readable: {root}: {e}")))?;
    let names: Vec<Value> = names.iter().map(|name| cx.string(name)).collect();
    Ok(list(cx, &names))
}}

// TEXT as `auth-source-pass-parse-entry' gives it: ((secret . SECRET)
// (KEY . VALUE)...), SECRET the first line that is not empty, or nil.
defsubr! { name = "romacs-pass--parse", gnu = "romacs:pass", effect = Pure,
fn romacs_pass_parse(cx, text: Value) {
    let text = Zeroizing::new(cx.string_text(text)?);
    let secret = crate::pass::entry::secret(&text).map_or(Value::NIL, |s| cx.string(s));
    let fields: Vec<Value> = crate::pass::entry::fields(&text).into_iter().map(|(key, value)| {
        let (key, value) = (cx.string(key), cx.string(value));
        cx.cons(key, value)
    }).collect();
    let symbol = cx.intern("secret");
    let first = cx.cons(symbol, secret);
    let rest = list(cx, &fields);
    Ok(cx.cons(first, rest))
}}

// The one-time code of TEXT's `otpauth://totp/' URI at UNIX-SECONDS, as
// (CODE . SECONDS-VALID), or nil when TEXT holds no such URI.  A URI that
// cannot give one signals an error saying why.
defsubr! { name = "romacs-pass--otp", gnu = "romacs:pass", effect = Pure,
fn romacs_pass_otp(cx, text: Value, unix_seconds: Value) {
    let now = unix_seconds.integer_value().and_then(|n| u64::try_from(n).ok()).ok_or(Fault::Wrong("natnump", unix_seconds))?;
    let text = Zeroizing::new(cx.string_text(text)?);
    let Some(uri) = crate::pass::entry::otp_uri(&text) else { return Ok(Value::NIL) };
    let totp = crate::pass::otp::parse(uri).map_err(|why| Fault::Error(format!("Bad otpauth URI: {why}")))?;
    let (code, valid) = crate::pass::otp::code(&totp, now);
    let code = Zeroizing::new(code);
    let code = cx.string(&code);
    Ok(cx.cons(code, i64::try_from(valid).ok().and_then(|n| Value::integer(n).ok()).expect("a period's seconds")))
}}

// A new password of LENGTH characters drawn uniformly from the
// characters of ALPHABET with the kernel's entropy.
defsubr! { name = "romacs-pass--generate", gnu = "romacs:pass", effect = Global,
fn romacs_pass_generate(cx, length: Value, alphabet: Value) {
    let count = length.integer_value().and_then(|n| usize::try_from(n).ok()).filter(|&n| n <= 4096)
        .ok_or(Fault::Wrong("natnump", length))?;
    let characters: Vec<char> = cx.string_text(alphabet)?.chars().collect();
    let password = crate::pass::generate::password(count, &characters)
        .map_err(|e| Fault::Error(format!("Cannot generate a password: {e}")))?;
    Ok(cx.string(&password))
}}
