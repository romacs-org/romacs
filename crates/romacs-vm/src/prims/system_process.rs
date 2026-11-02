//! GNU process.c list-system-processes and process-attributes, with
//! sysdep.c's /proc reading (the GNU/Linux branch). Reading is pure over the
//! files' bytes; the host boundary supplies them.
use super::*;

defsubr! { name = "list-system-processes", gnu = "process.c:8524", effect = Global,
fn list_system_processes(cx) -> Step {
    let directory = super::fileio::var(cx, "default-directory")?;
    if let Some(call) = super::fileio::handler(cx, directory, "list-system-processes", vec![])? { return call; }
    let entries = match crate::host::read_dir("/proc") {
        Ok(entries) => entries,
        Err(error) => {
            let proc = cx.string("/proc");
            let data = cx.cons(proc, Value::NIL);
            return Err(cx.file_errno_data("Opening directory", &error, data));
        }
    };
    // directory_files_internal's "[0-9]+" matches, in the order read,
    // each through string-to-number.
    let pids: Vec<i64> = entries.filter_map(|e| e.ok()).map(|e| e.file_name())
        .filter_map(|name| name.to_str().filter(|n| n.bytes().any(|c| c.is_ascii_digit())).map(leading_number))
        .collect();
    let mut list = Value::NIL;
    for pid in pids.into_iter().rev() { list = cx.cons(Value::integer(pid).map_err(Fault::Error)?, list); }
    Ok(Step::Done(list))
}}

/// string-to-number of a /proc name: its leading decimal digits, else 0.
fn leading_number(name: &str) -> i64 {
    let digits = name.bytes().take_while(u8::is_ascii_digit).count();
    name[..digits].parse().unwrap_or(0)
}

defsubr! { name = "process-attributes", gnu = "process.c:8542", effect = Global,
fn process_attributes(cx, pid: Value) -> Step {
    let directory = super::fileio::var(cx, "default-directory")?;
    if let Some(call) = super::fileio::handler(cx, directory, "process-attributes", vec![pid])? { return call; }
    let pid = process_id(cx, pid)?;
    Ok(Step::Done(attributes(cx, pid)?))
}}

/// CHECK_NUMBER, then CONS_TO_INTEGER (pid, pid_t, ...): an integer, an
/// integral float or a (HIGH . LOW) cons within pid_t.
fn process_id(cx: &Leaf<'_>, pid: Value) -> std::result::Result<i64, Fault> {
    let out_of_range = || Fault::Error("Not an in-range integer, integral float, or cons of integers".into());
    let (min, max) = (i64::from(i32::MIN), i64::from(i32::MAX));
    if let Some(n) = pid.integer_value() { return Some(n).filter(|n| (min..=max).contains(n)).ok_or_else(out_of_range); }
    if cx.kind(pid) == Kind::Float {
        let d = cx.float_value(pid)?;
        return (d >= min as f64 && d < 1.0 + max as f64 && d.fract() == 0.0).then_some(d as i64).ok_or_else(out_of_range);
    }
    if cx.bigint_value(pid).is_some() { return Err(out_of_range()); }
    Err(Fault::Wrong("numberp", pid))
}

/// The fields of /proc/PID/stat after the command name that GNU reads.
#[derive(Debug, PartialEq)]
struct Stat {
    state: u8,
    ppid: i64, pgrp: i64, sess: i64, tty: i32, tpgid: i64,
    minflt: u64, cminflt: u64, majflt: u64, cmajflt: u64,
    utime: u64, stime: u64, cutime: u64, cstime: u64,
    priority: i64, nice: i64, thcount: i64, start: u64, vsize: u64, rss: i64,
}

/// /proc/PID/stat's first 1024 bytes: the command name between the first
/// `(' and the last `)' ("???" without both), and the fields after it when
/// all twenty parse (sysdep.c's sscanf).
fn parse_stat(bytes: &[u8]) -> (Vec<u8>, Option<Stat>) {
    let bytes = &bytes[..bytes.len().min(1024)];
    let bytes = &bytes[..bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len())];
    let Some(open) = bytes.iter().position(|&c| c == b'(') else { return (b"???".to_vec(), None) };
    let Some(close) = bytes[open + 1..].iter().rposition(|&c| c == b')').map(|i| open + 1 + i) else { return (b"???".to_vec(), None) };
    let comm = bytes[open + 1..close].to_vec();
    let rest = bytes.get(close + 2..).unwrap_or_default();
    (comm, stat_fields(rest))
}

fn stat_fields(rest: &[u8]) -> Option<Stat> {
    let text = std::str::from_utf8(rest).ok()?;
    let state = *rest.first()?;
    let mut words = text.get(1..)?.split_ascii_whitespace();
    let mut next = || words.next();
    fn num<T: std::str::FromStr>(w: Option<&str>) -> Option<T> { w?.parse().ok() }
    let (ppid, pgrp, sess, tty, tpgid) = (num(next())?, num(next())?, num(next())?, num(next())?, num(next())?);
    num::<u64>(next())?;
    let (minflt, cminflt, majflt, cmajflt) = (num(next())?, num(next())?, num(next())?, num(next())?);
    let (utime, stime, cutime, cstime) = (num(next())?, num(next())?, num(next())?, num(next())?);
    let (priority, nice, thcount) = (num(next())?, num(next())?, num(next())?);
    num::<i64>(next())?;
    let (start, vsize, rss) = (num(next())?, num(next())?, num(next())?);
    Some(Stat { state, ppid, pgrp, sess, tty, tpgid, minflt, cminflt, majflt, cmajflt,
                utime, stime, cutime, cstime, priority, nice, thcount, start, vsize, rss })
}

/// sysdep.c procfs_ttyname: the /proc/tty/drivers device whose major is
/// RDEV's and whose minor range holds its minor, with the minor appended;
/// "" when none does.
fn tty_name(drivers: &[u8], rdev: i32) -> Vec<u8> {
    let rdev = rdev as u32;
    let (major, minor) = ((rdev >> 8) & 0xfff, u64::from((rdev & 0xff) | ((rdev & 0xfff0_0000) >> 12)));
    let words: Vec<&[u8]> = drivers.split(u8::is_ascii_whitespace).filter(|w| !w.is_empty()).collect();
    for line in words.chunks(5) {
        let [_, name, line_major, range, ..] = line else { break };
        if std::str::from_utf8(line_major).ok().and_then(|m| m.parse::<u32>().ok()) != Some(major) { continue; }
        let (low, rest) = strtoul(range);
        let high = match rest.first() {
            None => low,
            Some(b'-') => strtoul(&rest[1..]).0,
            Some(_) => continue,
        };
        if (low..=high).contains(&minor) {
            let mut name = name.to_vec();
            name.extend_from_slice(minor.to_string().as_bytes());
            return name;
        }
    }
    Vec::new()
}

/// strtoul with base 0: hexadecimal after 0x, octal after 0, else
/// decimal; the value and what follows it.
fn strtoul(s: &[u8]) -> (u64, &[u8]) {
    let (radix, digits) = match s {
        [b'0', b'x' | b'X', rest @ ..] if rest.first().is_some_and(u8::is_ascii_hexdigit) => (16, rest),
        [b'0', ..] => (8, s),
        _ => (10, s),
    };
    let n = digits.iter().take_while(|c| (**c as char).is_digit(radix)).count();
    let value = std::str::from_utf8(&digits[..n]).ok().and_then(|d| u64::from_str_radix(d, radix).ok()).unwrap_or(0);
    (value, &digits[n..])
}

/// sysdep.c procfs_get_total_memory: MemTotal in KiB, else 2 GiB.
fn total_memory(meminfo: &[u8]) -> u64 {
    meminfo.split(|&c| c == b'\n')
        .find_map(|line| line.strip_prefix(b"MemTotal:"))
        .and_then(|rest| std::str::from_utf8(rest).ok()?.split_ascii_whitespace().next()?.parse().ok())
        .unwrap_or(2 * 1024 * 1024)
}

/// /proc/PID/cmdline as GNU shows it: trailing NULs dropped, NULs as
/// spaces, whitespace and backslashes escaped with a backslash; when empty,
/// "[COMM]".
fn command_line(cmdline: &[u8], comm: &[u8]) -> Vec<u8> {
    let end = cmdline.iter().rposition(|&c| c != 0).map_or(0, |i| i + 1);
    let mut out = Vec::with_capacity(end);
    for &c in &cmdline[..end] {
        if matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r' | b'\\') { out.push(b'\\'); }
        out.push(if c == 0 { b' ' } else { c });
    }
    if out.is_empty() { [b"[", comm, b"]"].concat() } else { out }
}

/// code_convert_string_norecord (BYTES, Vlocale_coding_system, 0): decoded
/// without recording the coding system; as given when there is none.
fn locale_decoded(cx: &mut Leaf<'_>, bytes: Vec<u8>) -> Out {
    let coding = super::fileio::var(cx, "locale-coding-system")?;
    if coding == Value::NIL { return Ok(cx.make_string(bytes, false)); }
    let last = cx.intern("last-coding-system-used");
    let saved = cx.variable_value(last)?.unwrap_or(Value::NIL);
    let text = cx.decode_output(bytes, coding);
    cx.variable_set(last, saved)?;
    text
}

/// An account name as build_string makes it.
fn account_name(cx: &mut Leaf<'_>, name: Vec<u8>) -> Value {
    let multibyte = !name.is_ascii() && std::str::from_utf8(&name).is_ok();
    cx.make_string(name, multibyte)
}

/// sysdep.c system_process_attributes (GNU/Linux): nil when /proc/PID is
/// missing, else the alist in GNU's order.
fn attributes(cx: &mut Leaf<'_>, pid: i64) -> Out {
    use std::os::unix::fs::MetadataExt;
    let dir = format!("/proc/{pid}");
    let Ok(meta) = crate::host::metadata(&dir) else { return Ok(Value::NIL) };
    // The process may exit while it is read: read its files together first.
    let stat_bytes = crate::host::read_file(format!("{dir}/stat"));
    let cmdline = crate::host::read_file(format!("{dir}/cmdline"));
    let mut attrs: Vec<(&str, Value)> = Vec::new();
    let int = |n: i128| -> std::result::Result<Value, Fault> { Value::integer(n as i64).map_err(Fault::Error) };
    attrs.push(("euid", int(meta.uid().into())?));
    if let Some(name) = crate::host::user_by_uid(meta.uid(), 16384).ok().and_then(|p| p.name) { attrs.push(("user", account_name(cx, name))); }
    attrs.push(("egid", int(meta.gid().into())?));
    if let Ok(name) = crate::host::group_by_gid(meta.gid(), 16384) { attrs.push(("group", account_name(cx, name))); }
    let (comm, stat) = match stat_bytes {
        Ok(bytes) if !bytes.is_empty() => {
            let (comm, stat) = parse_stat(&bytes);
            let decoded = locale_decoded(cx, comm.clone())?;
            attrs.push(("comm", decoded));
            (comm, stat)
        }
        _ => (b"???".to_vec(), None),
    };
    if let Some(stat) = stat { stat_attributes(cx, &stat, &mut attrs)?; }
    if let Ok(cmdline) = cmdline {
        let args = locale_decoded(cx, command_line(&cmdline, &comm))?;
        attrs.push(("args", args));
    }
    let mut list = Value::NIL;
    for (key, value) in attrs {
        let key = cx.intern(key);
        let pair = cx.cons(key, value);
        list = cx.cons(pair, list);
    }
    Ok(list)
}

fn stat_attributes(cx: &mut Leaf<'_>, stat: &Stat, attrs: &mut Vec<(&'static str, Value)>) -> std::result::Result<(), Fault> {
    let int = |n: i128| cx_integer(n);
    let state = cx.make_string(vec![stat.state], false);
    attrs.push(("state", state));
    attrs.push(("ppid", int(stat.ppid.into())?));
    attrs.push(("pgrp", int(stat.pgrp.into())?));
    attrs.push(("sess", int(stat.sess.into())?));
    let tty = tty_name(&crate::host::read_file("/proc/tty/drivers").unwrap_or_default(), stat.tty);
    attrs.push(("ttname", cx.make_string(tty, false)));
    attrs.push(("tpgid", int(stat.tpgid.into())?));
    attrs.push(("minflt", int(stat.minflt.into())?));
    attrs.push(("majflt", int(stat.majflt.into())?));
    attrs.push(("cminflt", int(stat.cminflt.into())?));
    attrs.push(("cmajflt", int(stat.cmajflt.into())?));
    let hz = crate::host::clock_ticks_per_second() as i64;
    let mut jiffies = |name, ticks: u64| -> std::result::Result<(), Fault> {
        let time = super::time::ticks_time(cx, ticks.into(), hz)?;
        attrs.push((name, time));
        Ok(())
    };
    jiffies("utime", stat.utime)?;
    jiffies("stime", stat.stime)?;
    jiffies("time", stat.stime + stat.utime)?;
    jiffies("cutime", stat.cutime)?;
    jiffies("cstime", stat.cstime)?;
    jiffies("ctime", stat.cstime + stat.cutime)?;
    if let Some(uptime) = crate::host::boot_clock() {
        // Each in ticks of HZ, as GNU converts them before its arithmetic.
        let at_hz = |nanos: i128| (nanos * i128::from(hz)).div_euclid(1_000_000_000);
        let now = match crate::host::clock().duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_nanos() as i128,
            Err(e) => -(e.duration().as_nanos() as i128),
        };
        let uptime = at_hz(uptime.as_nanos() as i128);
        let boot = at_hz(now) - uptime;
        let start = i128::from(stat.start);
        let lstart = super::time::ticks_time(cx, boot + start, hz)?;
        attrs.push(("start", lstart));
        let etime = uptime - start;
        let etime_value = super::time::ticks_time(cx, etime, hz)?;
        attrs.push(("etime", etime_value));
        let pcpu = 100.0 * (stat.stime + stat.utime) as f64 / (hz as f64 * (etime as f64 / hz as f64));
        attrs.push(("pcpu", cx.float(pcpu)));
    }
    attrs.push(("pri", int(stat.priority.into())?));
    attrs.push(("nice", int(stat.nice.into())?));
    attrs.push(("thcount", int(stat.thcount.into())?));
    attrs.push(("vsize", int((stat.vsize / 1024).into())?));
    let rss = (stat.rss as u64).wrapping_mul(crate::host::page_size() >> 10);
    attrs.push(("rss", int(rss.into())?));
    let total = total_memory(&crate::host::read_file("/proc/meminfo").unwrap_or_default());
    attrs.push(("pmem", cx.float((100.0 * rss as f64 / total as f64).min(100.0))));
    Ok(())
}

fn cx_integer(n: i128) -> std::result::Result<Value, Fault> {
    i64::try_from(n).ok().and_then(|n| Value::integer(n).ok())
        .ok_or_else(|| Fault::Error("VM2 process attribute requiring a bignum is unsupported".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_line_fields() {
        let line = b"123 (a (b) c) S 1 123 123 0 -1 4194560 10 20 3 4 5 6 7 8 20 0 1 0 999 4096 50 18446744073709551615";
        let (comm, stat) = parse_stat(line);
        assert_eq!(comm, b"a (b) c");
        let stat = stat.unwrap();
        assert_eq!((stat.state, stat.ppid, stat.tty, stat.minflt, stat.cminflt, stat.utime, stat.start, stat.vsize, stat.rss),
                   (b'S', 1, 0, 10, 20, 5, 999, 4096, 50));
        assert_eq!(parse_stat(b"1 no-paren"), (b"???".to_vec(), None));
    }

    #[test]
    fn tty_names_and_command_lines() {
        let drivers = b"/dev/tty /dev/tty 5 0 system:/dev/tty\npty_slave /dev/pts 136 0-1048575 pty:slave\nserial /dev/ttyS 4 064-0111 serial\n";
        assert_eq!(tty_name(drivers, (136 << 8) | 3), b"/dev/pts3");
        assert_eq!(tty_name(drivers, (4 << 8) | 0o70), b"/dev/ttyS56");
        assert_eq!(tty_name(drivers, 0), b"");
        assert_eq!(command_line(b"emacs\0-Q\0a b\\c\0\0", b"emacs"), b"emacs -Q a\\ b\\\\c");
        assert_eq!(command_line(b"", b"kworker"), b"[kworker]");
        assert_eq!(total_memory(b"MemTotal:       16000 kB\nMemFree: 1 kB\n"), 16000);
        assert_eq!(total_memory(b""), 2 * 1024 * 1024);
    }
}
