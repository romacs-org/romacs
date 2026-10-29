//! GNU editfns.c user identity: uids and accounts from the C library
//! (user_db), names from the launch environment as getenv sees it.
use super::*;

use crate::user_db::{self, Account};

/// getenv: the launch environment, which Lisp's setenv does not change.
fn launch_env(vm: &mut Vm, name: &str) -> Option<Vec<u8>> {
    let slot = vm.slot("initial-environment");
    crate::host::lisp_env(crate::host::EnvList::Initial, name.as_bytes());
    let mut tail = vm.variable_value(slot).ok()?;
    let prefix = format!("{name}=");
    while let Ok((entry, rest)) = vm.cons_pair(tail) {
        if let Some(bytes) = vm.str_view(entry).map(|v| v.bytes().to_vec()) {
            if let Some(value) = bytes.strip_prefix(prefix.as_bytes()) { return Some(value.to_vec()); }
        }
        tail = rest;
    }
    None
}

/// USER_FULL_NAME's gecos field up to the first comma, with `&` replaced by
/// the capitalized login name (AMPERSAND_FULL_NAME).
fn full_name(account: &Account) -> String {
    let gecos = account.gecos.split(',').next().unwrap_or("");
    let mut login = account.name.clone();
    if let Some(first) = login.get_mut(0..1) { first.make_ascii_uppercase(); }
    gecos.replace('&', &login)
}

fn string(vm: &mut Vm, bytes: Vec<u8>) -> Value {
    let multibyte = crate::environment::multibyte(&bytes);
    vm.make_lisp_string(bytes, multibyte)
}

/// init_editfns for user-login-name, user-real-login-name and user-full-name.
pub(crate) fn initial_value(vm: &mut Vm, variable: &str) -> Value {
    let (real, effective) = (user_db::real_uid(), user_db::effective_uid());
    let real_name = user_db::by_uid(real).map_or_else(|| "unknown".into(), |a| a.name);
    let login = launch_env(vm, "LOGNAME").or_else(|| launch_env(vm, "USER"))
        .unwrap_or_else(|| user_db::by_uid(effective).map_or_else(|| "unknown".into(), |a| a.name).into_bytes());
    match variable {
        "user-real-login-name" => string(vm, real_name.into_bytes()),
        "user-login-name" => string(vm, login),
        _ => {
            if let Some(name) = launch_env(vm, "NAME") { return string(vm, name); }
            // The claimed login name selects the account when it matches the
            // real user; otherwise the effective uid does.
            let account = if login == real_name.as_bytes() { std::str::from_utf8(&login).ok().and_then(user_db::by_name) }
                else { user_db::by_uid(effective) };
            let full = account.as_ref().map(full_name).unwrap_or_else(|| "unknown".into());
            string(vm, full.into_bytes())
        }
    }
}

/// data.c cons_to_unsigned into uid_t: an integer, an integral float, or
/// GNU's (HI . LO) / (HI MID . LO) cons forms.
fn uid_argument(cx: &Leaf<'_>, uid: Value) -> std::result::Result<u32, Fault> {
    let nat = |v: Value, limit: i64| v.integer_value().filter(|&n| (0..limit).contains(&n));
    let value: Option<u128> = if cx.kind(uid) == leaf::Kind::Float {
        cx.float_value(uid).ok().filter(|d| *d >= 0.0 && d.fract() == 0.0 && *d < 1.0 + u32::MAX as f64).map(|d| d as u128)
    } else {
        let hi = cx.pair(uid).map_or(uid, |(car, _)| car);
        cx.bigint_value(hi).and_then(|n| u128::try_from(n).ok()).and_then(|top| match cx.pair(uid) {
            None => Some(top),
            Some((_, rest)) => match cx.pair(rest) {
                Some((mid, lo)) if nat(mid, 1 << 24).is_some() && nat(lo, 1 << 16).is_some() =>
                    (top <= u128::from(u32::MAX)).then(|| top << 40 | (nat(mid, 1 << 24).unwrap_or(0) as u128) << 16 | nat(lo, 1 << 16).unwrap_or(0) as u128),
                pair => {
                    let rest = pair.map_or(rest, |(car, _)| car);
                    if top > u128::from(u32::MAX) { return None; }
                    Some(top << 16 | nat(rest, 1 << 16)? as u128)
                }
            },
        })
    };
    value.and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| Fault::Error("Not an in-range integer, integral float, or cons of integers".into()))
}

defsubr! { name = "user-login-name", gnu = "editfns.c:1115", effect = Reads,
fn user_login_name(cx; optional uid: Value) {
    if uid == Value::NIL {
        let symbol = cx.intern("user-login-name");
        return Ok(cx.variable_value(symbol)?.unwrap_or(Value::NIL));
    }
    let uid = uid_argument(cx, uid)?;
    Ok(user_db::by_uid(uid).map_or(Value::NIL, |a| cx.string(&a.name)))
}}

defsubr! { name = "user-real-login-name", gnu = "editfns.c:1144", effect = Reads,
fn user_real_login_name(cx) {
    let symbol = cx.intern("user-real-login-name");
    Ok(cx.variable_value(symbol)?.unwrap_or(Value::NIL))
}}

defsubr! { name = "user-uid", gnu = "editfns.c:1159", effect = Reads,
fn user_uid(cx) { Ok(cx.bigint(user_db::effective_uid().into())) }}

defsubr! { name = "user-real-uid", gnu = "editfns.c:1167", effect = Reads,
fn user_real_uid(cx) { Ok(cx.bigint(user_db::real_uid().into())) }}

defsubr! { name = "user-full-name", gnu = "editfns.c:1209", effect = Reads,
fn user_full_name(cx; optional uid: Value) {
    if uid == Value::NIL {
        let symbol = cx.intern("user-full-name");
        return Ok(cx.variable_value(symbol)?.unwrap_or(Value::NIL));
    }
    let account = if matches!(cx.kind(uid), leaf::Kind::String | leaf::Kind::Bytes) {
        let name = cx.string_text(uid)?;
        user_db::by_name(&name)
    } else if cx.bigint_value(uid).is_some() || cx.kind(uid) == leaf::Kind::Float {
        let uid = uid_argument(cx, uid)?;
        user_db::by_uid(uid)
    } else {
        return Err(Fault::Error("Invalid UID specification".into()));
    };
    Ok(account.map_or(Value::NIL, |a| { let full = full_name(&a); cx.string(&full) }))
}}
