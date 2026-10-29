//! GNU src/timefns.c timestamp encoding; no floating-point conversion.
use super::*;

// GNU timespec_ticks (timefns.c:720), bounded by VM2's fixnum substrate.
fn ticks_value(nanos: i128) -> std::result::Result<Value, Fault> {
    i64::try_from(nanos)
        .ok()
        .and_then(|n| Value::integer(n).ok())
        .ok_or_else(|| {
            Fault::Error("VM2 current-time requiring bignum ticks is unsupported".into())
        })
}

// GNU make_lisp_time (timefns.c:820), hi_time/lo_time (:382/:389).
fn list_fields(nanos: i128) -> std::result::Result<[i64; 4], Fault> {
    let seconds = nanos.div_euclid(1_000_000_000);
    let ns = nanos.rem_euclid(1_000_000_000) as i64;
    let hi = i64::try_from(seconds.div_euclid(65536))
        .ok()
        .filter(|n| Value::integer(*n).is_ok())
        .ok_or_else(|| {
            Fault::Error("VM2 current-time requiring bignum HIGH is unsupported".into())
        })?;
    Ok([
        hi,
        seconds.rem_euclid(65536) as i64,
        ns / 1000,
        ns % 1000 * 1000,
    ])
}

defsubr! { name = "current-time", gnu = "timefns.c:1817", effect = Global,
fn current_time(cx) {
    use std::time::UNIX_EPOCH;
    // duration_since's error carries the magnitude before the epoch. Preserve
    // every nanosecond, including negative subsecond readings; never use f64.
    let nanos = match crate::host::clock().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos() as i128,
        Err(error) => -(error.duration().as_nanos() as i128),
    };
    lisp_time(cx, nanos)
}}

/// GNU make_lisp_time of NANOS nanoseconds, in `current-time-list''s form.
pub(super) fn lisp_time(cx: &mut Leaf<'_>, nanos: i128) -> Out {
    let format = cx.intern("current-time-list");
    if cx.variable_value(format)?.unwrap_or(Value::NIL) == Value::NIL {
        return Ok(cx.cons(ticks_value(nanos)?, Value::integer(1_000_000_000).unwrap()));
    }
    let mut time = Value::NIL;
    for n in list_fields(nanos)?.into_iter().rev() {
        time = cx.cons(Value::integer(n).map_err(Fault::Error)?, time);
    }
    Ok(time)
}

// Retain numerator/denominator until the caller chooses seconds or nanoseconds.
#[derive(Clone, Copy)]
struct Stamp { ticks: i128, hz: i128 }
fn invalid_time() -> Fault { Fault::Error("Invalid time specification".into()) }
fn time_overflow() -> Fault { Fault::Error("Specified time is not representable".into()) }
fn time_integer(cx: &Leaf<'_>, v: Value) -> std::result::Result<i128, Fault> {
    cx.bigint_value(v).ok_or_else(invalid_time)?.try_into().map_err(|_| time_overflow())
}
fn stamp(cx: &Leaf<'_>, value: Value, seconds_only: bool) -> std::result::Result<Stamp, Fault> {
    if value == Value::NIL {
        let n = match crate::host::clock().duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_nanos() as i128, Err(e) => -(e.duration().as_nanos() as i128),
        };
        return Ok(Stamp { ticks: n, hz: 1_000_000_000 });
    }
    if let Some((high, low)) = cx.pair(value) {
        if let Some((low, tail)) = cx.pair(low) {
            let low = time_integer(cx, low)?;
            let seconds = time_integer(cx, high)?.checked_mul(65536)
                .and_then(|n| n.checked_add(low)).ok_or_else(time_overflow)?;
            if seconds_only { return Ok(Stamp { ticks: seconds, hz: 1 }); }
            let (us, ps, hz) = if let Some((us, tail)) = cx.pair(tail) {
                if let Some((ps, _)) = cx.pair(tail) { (time_integer(cx, us)?, time_integer(cx, ps)?, 1_000_000_000_000) }
                else { (time_integer(cx, us)?, 0, 1_000_000) }
            } else if tail == Value::NIL { (0, 0, 1) }
            else { (time_integer(cx, tail)?, 0, 1_000_000) };
            let sub = if hz == 1_000_000_000_000 { us.checked_mul(1_000_000).and_then(|n| n.checked_add(ps)) }
                else { Some(us) }.ok_or_else(time_overflow)?;
            let ticks = seconds.checked_mul(hz).and_then(|n| n.checked_add(sub)).ok_or_else(time_overflow)?;
            return Ok(Stamp { ticks, hz });
        }
        let hz = time_integer(cx, low)?;
        if hz <= 0 { return Err(invalid_time()); }
        return Ok(Stamp { ticks: time_integer(cx, high)?, hz });
    }
    if cx.bigint_value(value).is_some() { return Ok(Stamp { ticks: time_integer(cx, value)?, hz: 1 }); }
    if let Ok(n) = cx.float_value(value) {
        if n.is_nan() { return Err(invalid_time()); }
        if !n.is_finite() || n < i64::MIN as f64 || n >= i64::MAX as f64 { return Err(time_overflow()); }
        // Divide the exact binary ratio: subtracting floor(n) loses tiny
        // negative fractions when their distance from -1 rounds to 1.0.
        let (ticks, hz) = float_ticks(n);
        let scaled = ticks * num_bigint::BigInt::from(1_000_000_000);
        let quotient = &scaled / &hz;
        let remainder = &scaled % &hz;
        let floor = if remainder < num_bigint::BigInt::from(0) { quotient - 1 } else { quotient };
        return Ok(Stamp { ticks: floor.try_into().map_err(|_| time_overflow())?, hz: 1_000_000_000 });
    }
    Err(invalid_time())
}
impl Stamp {
    fn seconds(self) -> std::result::Result<i64, Fault> {
        self.ticks.div_euclid(self.hz).try_into().map_err(|_| time_overflow())
    }
    fn nanos(self) -> std::result::Result<i64, Fault> {
        self.ticks.rem_euclid(self.hz).checked_mul(1_000_000_000)
            .map(|n| (n / self.hz) as i64).ok_or_else(time_overflow)
    }
}
/// GNU lisp_time_argument (timefns.c): TIME's seconds and nanoseconds,
/// floored as lisp_to_timespec does. nil is the current time.
pub(super) fn timespec(cx: &Leaf<'_>, time: Value) -> std::result::Result<(i64, i64), Fault> {
    let stamp = stamp(cx, time, false)?;
    Ok((stamp.seconds()?, stamp.nanos()?))
}
#[derive(PartialEq, Eq)]
enum FrequencyIdentity {
    Lisp(Value),
    FloatPower(u64),
}

struct ExactStamp {
    ticks: num_bigint::BigInt,
    hz: num_bigint::BigInt,
    identity: FrequencyIdentity,
}

fn exact_integer(cx: &Leaf<'_>, value: Value) -> std::result::Result<num_bigint::BigInt, Fault> {
    cx.bigint_value(value).ok_or_else(invalid_time)
}

fn exact_stamp(cx: &Leaf<'_>, value: Value) -> std::result::Result<ExactStamp, Fault> {
    use num_bigint::BigInt;
    let immediate = |ticks, hz: i64| ExactStamp {
        ticks,
        hz: BigInt::from(hz),
        identity: FrequencyIdentity::Lisp(Value::integer(hz).unwrap()),
    };
    if value == Value::NIL {
        let nanos = match crate::host::clock().duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => BigInt::from(d.as_nanos()),
            Err(e) => -BigInt::from(e.duration().as_nanos()),
        };
        return Ok(immediate(nanos, 1_000_000_000));
    }
    if let Some((high, low)) = cx.pair(value) {
        if let Some((low, tail)) = cx.pair(low) {
            let seconds = exact_integer(cx, high)? * 65536 + exact_integer(cx, low)?;
            let (us, ps, hz) = if let Some((us, tail)) = cx.pair(tail) {
                if let Some((ps, _)) = cx.pair(tail) {
                    (us, ps, 1_000_000_000_000)
                } else {
                    (us, Value::integer(0).unwrap(), 1_000_000)
                }
            } else if tail == Value::NIL {
                (Value::integer(0).unwrap(), Value::integer(0).unwrap(), 1)
            } else {
                (tail, Value::integer(0).unwrap(), 1_000_000)
            };
            let us = us.integer_value().ok_or_else(invalid_time)?;
            let ps = ps.integer_value().ok_or_else(invalid_time)?;
            let sub = if hz == 1_000_000_000_000 {
                BigInt::from(us) * 1_000_000 + ps
            } else {
                BigInt::from(us)
            };
            return Ok(immediate(seconds * hz + sub, hz));
        }
        let hz = exact_integer(cx, low)?;
        if hz <= BigInt::from(0) {
            return Err(invalid_time());
        }
        return Ok(ExactStamp {
            ticks: exact_integer(cx, high)?,
            hz,
            identity: FrequencyIdentity::Lisp(low),
        });
    }
    if let Some(ticks) = cx.bigint_value(value) {
        return Ok(immediate(ticks, 1));
    }
    if let Ok(n) = cx.float_value(value) {
        if n.is_nan() {
            return Err(invalid_time());
        }
        if !n.is_finite() {
            return Err(time_overflow());
        }
        let (ticks, hz) = float_ticks(n);
        let identity = i64::try_from(&hz)
            .ok()
            .and_then(|n| Value::integer(n).ok())
            .map(FrequencyIdentity::Lisp)
            .unwrap_or_else(|| FrequencyIdentity::FloatPower(hz.bits() - 1));
        return Ok(ExactStamp {
            ticks,
            hz,
            identity,
        });
    }
    Err(invalid_time())
}

fn floor_ratio(n: num_bigint::BigInt, d: &num_bigint::BigInt) -> num_bigint::BigInt {
    let q = &n / d;
    if n % d < num_bigint::BigInt::from(0) {
        q - 1
    } else {
        q
    }
}

fn exact_list(cx: &mut Leaf<'_>, time: &ExactStamp) -> Value {
    use num_bigint::BigInt;
    let trillion = BigInt::from(1_000_000_000_000i64);
    let ps = floor_ratio(&time.ticks * &trillion, &time.hz);
    let seconds = floor_ratio(ps.clone(), &trillion);
    let subseconds = ps - &seconds * &trillion;
    let high = floor_ratio(seconds.clone(), &BigInt::from(65536));
    let low = seconds - &high * 65536;
    let us = &subseconds / 1_000_000;
    let ps = subseconds % 1_000_000;
    let mut result = Value::NIL;
    for n in [ps, us, low, high] {
        let value = cx.bigint(n);
        result = cx.cons(value, result);
    }
    result
}

fn list_time_output(cx: &mut Leaf<'_>) -> std::result::Result<bool, Fault> {
    let symbol = cx.intern("current-time-list");
    Ok(cx.variable_value(symbol)?.unwrap_or(Value::NIL) != Value::NIL)
}

fn exact_pair(cx: &mut Leaf<'_>, time: ExactStamp) -> Value {
    let ticks = cx.bigint(time.ticks);
    let hz = cx.bigint(time.hz);
    cx.cons(ticks, hz)
}

fn invalid_frequency(cx: &mut Leaf<'_>, value: Value) -> Fault {
    let message = cx.string("Invalid time frequency");
    let data = cx.cons(value, Value::NIL);
    let data = cx.cons(message, data);
    Fault::Signal(cx.intern("error"), data)
}

defsubr! { name = "time-convert", gnu = "timefns.c:1778", effect = Global,
fn time_convert(cx, time: Value; optional form: Value) {
    use num_bigint::BigInt;
    let stamp = exact_stamp(cx, time)?;
    let form = if form == Value::NIL {
        if list_time_output(cx)? { cx.intern("list") } else { Value::TRUE }
    } else { form };
    if form == cx.intern("list") {
        return Ok(exact_list(cx, &stamp));
    }
    if form == cx.intern("integer") {
        if cx.bigint_value(time).is_some() { return Ok(time); }
        return Ok(cx.bigint(floor_ratio(stamp.ticks, &stamp.hz)));
    }
    if let Some((_, tail)) = cx.pair(time) {
        if (form == Value::TRUE && cx.pair(tail).is_none()) || form == tail {
            return Ok(time);
        }
    }
    if form == Value::TRUE { return Ok(exact_pair(cx, stamp)); }
    let hz = cx.bigint_value(form).filter(|n| *n > BigInt::from(0))
        .ok_or_else(|| invalid_frequency(cx, form))?;
    let ticks = cx.bigint(floor_ratio(stamp.ticks * &hz, &stamp.hz));
    Ok(cx.cons(ticks, form))
}}

fn time_gcd(mut a: num_bigint::BigInt, mut b: num_bigint::BigInt) -> num_bigint::BigInt {
    if a < num_bigint::BigInt::from(0) {
        a = -a;
    }
    while b != num_bigint::BigInt::from(0) {
        let remainder = a % &b;
        a = b;
        b = remainder;
    }
    a
}

fn time_arithmetic(cx: &mut Leaf<'_>, a: Value, b: Value, subtract: bool) -> Out {
    use num_bigint::BigInt;
    let a_stamp = exact_stamp(cx, a)?;
    let b_stamp = exact_stamp(cx, b)?;
    let (ticks, hz) = if a_stamp.identity == b_stamp.identity {
        let ticks = if subtract {
            a_stamp.ticks - b_stamp.ticks
        } else {
            a_stamp.ticks + b_stamp.ticks
        };
        (ticks, a_stamp.hz)
    } else {
        let gcd = time_gcd(a_stamp.hz.clone(), b_stamp.hz.clone());
        let a_factor = &a_stamp.hz / &gcd;
        let b_factor = &b_stamp.hz / gcd;
        let a_ticks = a_stamp.ticks * b_factor;
        let b_ticks = b_stamp.ticks * &a_factor;
        let ticks = if subtract {
            a_ticks - b_ticks
        } else {
            a_ticks + b_ticks
        };
        let hz = a_factor * &b_stamp.hz;
        let minimum = a_stamp.hz.min(b_stamp.hz);
        let gcd = time_gcd(ticks.clone(), hz.clone());
        let mut ticks = ticks / &gcd;
        let mut hz = hz / gcd;
        if hz < minimum {
            let scale = (&minimum + &hz - 1) / &hz;
            ticks *= &scale;
            hz *= scale;
        }
        (ticks, hz)
    };
    if hz == BigInt::from(1) {
        return Ok(cx.bigint(ticks));
    }
    let pair_input = |v| cx.pair(v).is_some_and(|(_, tail)| cx.pair(tail).is_none());
    let needs_pair = pair_input(a)
        || pair_input(b)
        || BigInt::from(1_000_000_000_000i64) % &hz != BigInt::from(0);
    let result = ExactStamp {
        ticks,
        hz,
        identity: FrequencyIdentity::Lisp(Value::NIL),
    };
    if !needs_pair && list_time_output(cx)? {
        Ok(exact_list(cx, &result))
    } else {
        Ok(exact_pair(cx, result))
    }
}

defsubr! { name = "time-add", gnu = "timefns.c:1220", effect = Global,
fn time_add(cx, a: Value, b: Value) { time_arithmetic(cx, a, b, false) }}

defsubr! { name = "time-subtract", gnu = "timefns.c:1229", effect = Global,
fn time_subtract(cx, a: Value, b: Value) {
    if a == b {
        let zero = ExactStamp { ticks: 0.into(), hz: 1_000_000_000.into(),
            identity: FrequencyIdentity::Lisp(Value::NIL) };
        return Ok(if list_time_output(cx)? { exact_list(cx, &zero) } else { exact_pair(cx, zero) });
    }
    time_arithmetic(cx, a, b, true)
}}

fn time_compare(
    cx: &Leaf<'_>,
    a: Value,
    b: Value,
) -> std::result::Result<std::cmp::Ordering, Fault> {
    if a == b {
        return Ok(std::cmp::Ordering::Equal);
    }
    let (x, y) = match (cx.pair(a), cx.pair(b)) {
        (Some((x, tail_a)), Some((y, tail_b))) if tail_a == tail_b => (x, y),
        _ => (a, b),
    };
    if let (Some(x), Some(y)) = (x.integer_value(), y.integer_value()) {
        return Ok(x.cmp(&y));
    }
    let a = exact_stamp(cx, a)?;
    let b = exact_stamp(cx, b)?;
    Ok((a.ticks * b.hz).cmp(&(b.ticks * a.hz)))
}

defsubr! { name = "time-less-p", gnu = "timefns.c:1287", effect = Global,
fn time_less_p(cx, a: Value, b: Value) {
    Ok(if time_compare(cx, a, b)?.is_lt() { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "time-equal-p", gnu = "timefns.c:1296", effect = Reads,
fn time_equal_p(cx, a: Value, b: Value) {
    if (a == Value::NIL) != (b == Value::NIL) { return Ok(Value::NIL); }
    Ok(if time_compare(cx, a, b)?.is_eq() { Value::TRUE } else { Value::NIL })
}}

#[derive(Clone)]
struct Zone { offset: i64, name: String, dst: bool }
fn zone_fault(cx: &mut Leaf<'_>, v: Value) -> Fault {
    let message = cx.string("Invalid time zone specification");
    let data = cx.cons(v, Value::NIL);
    let data = cx.cons(message, data);
    let error = cx.intern("error");
    Fault::Signal(error, data)
}
fn numeric_zone(offset: i64) -> Zone {
    let n = offset.unsigned_abs();
    let sign = if offset < 0 { '-' } else { '+' };
    let name = if offset == 0 { "GMT".into() }
        else if n % 3600 == 0 { format!("{sign}{:02}", n/3600) }
        else if n % 60 == 0 { format!("{sign}{:02}{:02}", n/3600,n/60%60) }
        else { format!("{sign}{:02}{:02}{:02}", n/3600,n/60%60,n%60) };
    Zone { offset, name, dst: false }
}
fn zone(cx: &mut Leaf<'_>, value: Value, seconds: i64) -> std::result::Result<Zone, Fault> {
    zone_rule(cx, value)?.at(seconds)
}
/// tzlookup: a zone fixed at one offset, or a TZif file's transitions.
enum ZoneRule { Fixed(Zone), Posix(crate::posix_tz::PosixTz), Tzif(crate::tzfile::TzFile) }
impl ZoneRule {
    fn at(&self, seconds: i64) -> std::result::Result<Zone, Fault> {
        match self {
            ZoneRule::Fixed(zone) => Ok(zone.clone()),
            ZoneRule::Posix(tz) => Ok(posix_zone(tz.at(seconds))),
            ZoneRule::Tzif(file) => file.at(seconds).map(posix_zone).ok_or_else(|| Fault::Error("VM2 unsupported TZif rule or timestamp".into())),
        }
    }
}
fn zone_rule(cx: &mut Leaf<'_>, value: Value) -> std::result::Result<ZoneRule, Fault> {
    if value == Value::TRUE || (value == Value::NIL && matches!(cx.local_zone(), crate::LocalZone::Utc)) {
        return Ok(ZoneRule::Fixed(numeric_zone(0)));
    }
    if let Some(n) = value.integer_value() { return Ok(ZoneRule::Fixed(numeric_zone(n))); }
    if let Some((offset, tail)) = cx.pair(value) {
        if let (Some(n), Some((name, _))) = (offset.integer_value(), cx.pair(tail)) {
            let name = cx.string_text(name).map_err(|_| Fault::Wrong("sequencep", name))?;
            // tzlookup hands glibc "<NAME>[-]H:MM:SS".
            let a = n.unsigned_abs();
            let rule = format!("<{name}>{}{}:{:02}:{:02}", if n < 0 { "" } else { "-" }, a / 3600, a / 60 % 60, a % 60);
            return Ok(ZoneRule::Posix(crate::posix_tz::PosixTz::parse(&rule)));
        }
        return Err(zone_fault(cx, value));
    }
    let wall = cx.symbol_name(value).ok().as_deref() == Some("wall");
    let rule = if value == Value::NIL {
        match cx.local_zone() {
            crate::LocalZone::Process => crate::host::getenv("TZ").and_then(|v| v.into_string().ok()),
            crate::LocalZone::Rule(rule) => Some(rule),
            crate::LocalZone::Wall | crate::LocalZone::Utc => None,
        }
    }
        else if wall { None } else { Some(cx.string_text(value).map_err(|_| zone_fault(cx, value))?) };
    // tzset_internal: the empty string is UTC; a leading colon is ignored;
    // a readable zone file wins; with no file, nothing or the default file
    // is UTC, and any other string is a POSIX rule.
    let utc = || ZoneRule::Fixed(Zone { offset: 0, name: "UTC".into(), dst: false });
    let rule = rule.as_deref().map(|r| r.strip_prefix(':').unwrap_or(r));
    if rule == Some("") { return Ok(utc()); }
    let path = match rule {
        None => std::path::PathBuf::from("/etc/localtime"),
        Some(r) if r.starts_with('/') => std::path::PathBuf::from(r),
        Some(r) => std::path::PathBuf::from(ZONEINFO).join(r),
    };
    match crate::host::zone_file(&path) {
        Ok(bytes) if bytes.starts_with(b"TZif") => {
            let file = crate::tzfile::TzFile::parse(&bytes)
                .ok_or_else(|| Fault::Error("VM2 unsupported TZif rule or timestamp".into()))?;
            let retained = cx.tz_rule_dstoff();
            cx.set_tz_rule_dstoff(file.rule_dstoff(retained));
            Ok(ZoneRule::Tzif(file))
        }
        _ => {
            let Some(rule) = rule else { return Ok(utc()) };
            let posix = crate::posix_tz::PosixTz::parse(rule);
            if posix.ruleless && let Some((std, dst)) = posix.zones()
                && let Some(rules) = crate::host::zone_file(std::path::Path::new(ZONEINFO).join("posixrules")).ok()
                    .and_then(|bytes| crate::tzfile::TzFile::parse(&bytes)) {
                let retained = cx.tz_rule_dstoff();
                cx.set_tz_rule_dstoff(rules.rule_dstoff(retained));
                if let Some(file) = rules.with_default(std, dst, retained) {
                    cx.set_tz_rule_dstoff(dst.offset);
                    return Ok(ZoneRule::Tzif(file));
                }
            }
            Ok(ZoneRule::Posix(posix))
        }
    }
}
const ZONEINFO: &str = "/usr/share/zoneinfo";
fn posix_zone(local: crate::posix_tz::Local) -> Zone { Zone { offset: local.offset, name: local.name, dst: local.dst } }
struct Calendar { year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64, weekday: i64, yearday: i64, days: i64 }
fn civil_days(year: i64, month: i64, day: i64) -> i64 {
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y-era*400;
    let mp = month + if month > 2 { -3 } else { 9 };
    era*146097 + yoe*365 + yoe/4 - yoe/100 + (153*mp+2)/5 + day-1 - 719468
}
fn calendar(seconds: i64, offset: i64) -> std::result::Result<Calendar, Fault> {
    let local = seconds.checked_add(offset).ok_or_else(time_overflow)?;
    let days = local.div_euclid(86400);
    let z = days+719468;
    let era = z.div_euclid(146097);
    let doe = z-era*146097;
    let yoe = (doe-doe/1460+doe/36524-doe/146096)/365;
    let mut year = yoe+era*400;
    let doy = doe-(365*yoe+yoe/4-yoe/100);
    let mp = (5*doy+2)/153;
    let day = doy-(153*mp+2)/5+1;
    let month = mp+if mp<10 {3} else {-9};
    year += i64::from(month<=2);
    // GNU's struct tm stores tm_year in a signed C int.
    if i32::try_from(year-1900).is_err() { return Err(time_overflow()); }
    Ok(Calendar { year, month, day, hour: local.rem_euclid(86400)/3600,
        minute: local.rem_euclid(3600)/60, second: local.rem_euclid(60),
        weekday: (days+4).rem_euclid(7), yearday: days-civil_days(year,1,1), days })
}
const DAYS: [&str;7] = ["Sunday","Monday","Tuesday","Wednesday","Thursday","Friday","Saturday"];
const MONTHS: [&str;12] = ["January","February","March","April","May","June","July","August","September","October","November","December"];
fn format_calendar(format: &str, c: &Calendar, z: &Zone, seconds: i64, nanos: i64) -> std::result::Result<String, Fault> {
    let mut out = String::new();
    let chars: Vec<char> = format.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '%' { out.push(chars[i]); i+=1; continue; }
        let begin = i; i+=1;
        let mut pad = None; let mut upper = false; let mut swap = false; let mut plus = false;
        while i < chars.len() && "-_0+^#".contains(chars[i]) {
            match chars[i] { '-' => pad=Some('\0'), '_' => pad=Some(' '), '0' => pad=Some('0'), '+' => {pad=Some('0'); plus=true;}, '^' => upper=true, '#' => swap=true, _=>() }; i+=1;
        }
        let mut width = 0usize;
        while i < chars.len() && chars[i].is_ascii_digit() {
            width = width.saturating_mul(10).saturating_add(chars[i] as usize-'0' as usize); i+=1;
        }
        if width > 1_000_000 { return Err(time_overflow()); }
        if i < chars.len() && (chars[i]=='E' || chars[i]=='O') { i+=1; }
        let mut colons=0;
        while i < chars.len() && chars[i]==':' { colons+=1; i+=1; }
        if i == chars.len() { out.extend(chars[begin..i].iter()); break; }
        let spec=chars[i]; i+=1;
        let iso = if matches!(spec, 'V'|'G'|'g') {
            let thursday=c.days+3-(c.weekday+6)%7;
            let year=calendar(thursday*86400,0)?.year;
            Some((year, (thursday-civil_days(year,1,1))/7+1))
        } else { None };
        let numeric = match spec {
            'Y'=>Some((c.year,4,'0')), 'y'=>Some((c.year.abs()%100,2,'0')), 'C'=>Some((c.year/100,2,'0')),
            'V'=>Some((iso.unwrap().1,2,'0')), 'G'=>Some((iso.unwrap().0,4,'0')), 'g'=>Some((iso.unwrap().0.abs()%100,2,'0')),
            'm'=>Some((c.month,2,'0')), 'd'=>Some((c.day,2,'0')), 'e'=>Some((c.day,2,' ')),
            'H'=>Some((c.hour,2,'0')), 'k'=>Some((c.hour,2,' ')),
            'I'=>Some(((c.hour+11)%12+1,2,'0')), 'l'=>Some(((c.hour+11)%12+1,2,' ')),
            'M'=>Some((c.minute,2,'0')), 'S'=>Some((c.second,2,'0')), 'j'=>Some((c.yearday+1,3,'0')),
            'w'=>Some((c.weekday,1,'0')), 'u'=>Some(((c.weekday+6)%7+1,1,'0')), 'q'=>Some(((c.month+2)/3,1,'0')),
            's'=>Some((seconds,1,'0')), 'U'=>Some(((c.yearday+7-c.weekday)/7,2,'0')),
            'W'=>Some(((c.yearday+7-(c.weekday+6)%7)/7,2,'0')), _=>None,
        };
        let mut text = if let Some((n, default_width, default_pad)) = numeric {
            let p=pad.unwrap_or(default_pad); let w=if width==0 {default_width} else {width};
            // GNU strftime keeps the century's sign even when truncation yields
            // zero, and includes an explicit year sign within the field width.
            let negative=n<0 || (spec=='C' && c.year<0);
            let signed_year=plus && p=='0' && matches!(spec,'Y'|'y'|'G'|'g'|'C')
                && (n.unsigned_abs() >= if spec=='C' {100} else {10000} || width>default_width);
            let sign=if negative {"-"} else if signed_year {"+"} else {""};
            let digits=n.unsigned_abs().to_string();
            let padding=if p=='\0' {0} else {w.saturating_sub(sign.len()+digits.len())};
            if p=='0' { format!("{sign}{}{digits}","0".repeat(padding)) }
            else { format!("{}{sign}{digits}"," ".repeat(padding)) }
        } else { match spec {
            '%' => "%".into(), 'n'=>"\n".into(), 't'=>"\t".into(),
            'a'=>DAYS[c.weekday as usize][..3].into(), 'A'=>DAYS[c.weekday as usize].into(),
            'b'|'h'=>MONTHS[c.month as usize-1][..3].into(), 'B'=>MONTHS[c.month as usize-1].into(),
            'p'=>if c.hour<12 {"AM"} else {"PM"}.into(), 'P'=>if c.hour<12 {"am"} else {"pm"}.into(),
            'Z'=>z.name.clone(),
            'z'=>{
                let n=z.offset.unsigned_abs(); let sign=if z.offset<0 {'-'} else {'+'};
                match colons { 0=>format!("{sign}{:02}{:02}",n/3600,n/60%60), 1=>format!("{sign}{:02}:{:02}",n/3600,n/60%60),
                    2=>format!("{sign}{:02}:{:02}:{:02}",n/3600,n/60%60,n%60),
                    3 if n%3600==0=>format!("{sign}{:02}",n/3600),
                    3 if n%60==0=>format!("{sign}{:02}:{:02}",n/3600,n/60%60),
                    3=>format!("{sign}{:02}:{:02}:{:02}",n/3600,n/60%60,n%60), _=>chars[begin..i].iter().collect() }
            },
            'N'=> { let w=if width==0 {9} else {width}; let mut s=format!("{nanos:09}"); s.truncate(w); while s.len()<w { s.push('0'); } s },
            'c'|'x'|'X'|'r'|'D'|'F'|'R'|'T'=>{
                let f=match spec {'c'=>"%a %b %e %T %Y", 'x'|'D'=>"%m/%d/%y", 'X'|'T'=>"%H:%M:%S", 'r'=>"%I:%M:%S %p", 'F'=>"%+4Y-%m-%d", 'R'=>"%H:%M", _=>unreachable!()};
                format_calendar(f,c,z,seconds,nanos)?
            },
            _=>chars[begin..i].iter().collect(),
        }};
        if upper {text=text.to_uppercase();}
        if swap { text=if matches!(spec,'p'|'P'|'Z') {text.to_lowercase()} else {text.to_uppercase()}; }
        if numeric.is_none() && spec!='N' && pad!=Some('\0') && text.len()<width {
            text=format!("{}{}",pad.unwrap_or(' ').to_string().repeat(width-text.len()),text);
        }
        out.push_str(&text);
    }
    Ok(out)
}
defsubr! { name = "format-time-string", gnu = "timefns.c:1415", effect = Global,
fn format_time_string(cx, format: Value; optional time: Value, tz: Value) {
    let stamp=stamp(cx,time,false)?;
    let seconds=stamp.seconds()?;
    let format=cx.string_text(format)?;
    let zone=zone(cx,tz,seconds)?;
    let c=calendar(seconds,zone.offset)?;
    let result=format_calendar(&format,&c,&zone,seconds,stamp.nanos()?)?;
    // GNU converts the formatted locale bytes back through locale-coding-system.
    // In the supported C-locale regime that is no-conversion: retain raw bytes.
    Ok(cx.byte_string(result.as_bytes()))
}}
defsubr! { name = "current-time-string", gnu = "timefns.c:1844", effect = Global,
fn current_time_string(cx; optional time: Value, tz: Value) {
    let seconds=stamp(cx,time,true)?.seconds()?;
    let zone=zone(cx,tz,seconds)?;
    let c=calendar(seconds,zone.offset)?;
    let result=format!("{} {}",format_calendar("%a %b %e %T",&c,&zone,seconds,0)?,c.year);
    Ok(cx.string(&result))
}}
defsubr! { name = "current-time-zone", gnu = "timefns.c:1893", effect = Global,
fn current_time_zone(cx; optional time: Value, tz: Value) {
    let seconds=stamp(cx,time,true)?.seconds()?;
    let z=zone(cx,tz,seconds)?;
    calendar(seconds,z.offset)?;
    // With no abbreviation, the name is the numeric offset: +HH[MM[SS]].
    let name = if z.name.is_empty() {
        let n = z.offset.unsigned_abs();
        let mut name = format!("{}{:02}", if z.offset < 0 { '-' } else { '+' }, n / 3600);
        if n % 3600 != 0 { name += &format!("{:02}", n / 60 % 60); if n % 60 != 0 { name += &format!("{:02}", n % 60); } }
        name
    } else { z.name.clone() };
    let name=cx.string(&name);
    let tail=cx.cons(name,Value::NIL);
    let offset=cx.bigint(z.offset.into());
    Ok(cx.cons(offset,tail))
}}
// GNU decode_float_time keeps the binary precision, including trailing zeros
// in the significand (timefns.c:623). Subnormals need a bignum denominator.
fn float_ticks(n: f64) -> (num_bigint::BigInt, num_bigint::BigInt) {
    use num_bigint::BigInt;
    if n == 0.0 { return (BigInt::from(0), BigInt::from(1)); }
    let bits = n.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let mantissa = (bits & ((1u64 << 52) - 1)) | if exponent == 0 { 0 } else { 1u64 << 52 };
    let scale = if exponent == 0 { 1074 } else { 1075 - exponent };
    let mut ticks = BigInt::from(mantissa);
    let hz = if scale >= 0 { BigInt::from(1) << scale as usize }
        else { ticks <<= -scale as usize; BigInt::from(1) };
    if n.is_sign_negative() { ticks = -ticks; }
    (ticks, hz)
}
defsubr! { name = "decode-time", gnu = "timefns.c:1503", effect = Global,
fn decode_time(cx; optional time: Value, tz: Value, form: Value) {
    let precise=form==Value::TRUE;
    let float = if precise { cx.float_value(time).ok() } else { None };
    let stamp=if let Some(n) = float {
        if n.is_nan() { return Err(invalid_time()); }
        if !n.is_finite() || n < i64::MIN as f64 || n >= i64::MAX as f64 { return Err(time_overflow()); }
        Stamp { ticks: n.floor() as i128, hz: 1 }
    } else { stamp(cx,time,!precise)? };
    let seconds=stamp.seconds()?;
    let z=zone(cx,tz,seconds)?;
    let c=calendar(seconds,z.offset)?;
    let sec=if let Some(n) = float.filter(|n| *n != 0.0) {
        let (ticks, hz) = float_ticks(n);
        let ticks = ticks - num_bigint::BigInt::from(seconds as i128 - c.second as i128) * &hz;
        if hz == num_bigint::BigInt::from(1) { cx.bigint(ticks) }
        else { let ticks=cx.bigint(ticks); let hz=cx.bigint(hz); cx.cons(ticks,hz) }
    } else if precise && stamp.hz!=1 {
        let ticks=(c.second as i128).checked_mul(stamp.hz).and_then(|n|n.checked_add(stamp.ticks.rem_euclid(stamp.hz))).ok_or_else(time_overflow)?;
        let ticks=cx.bigint(ticks.into()); let hz=cx.bigint(stamp.hz.into()); cx.cons(ticks,hz)
    } else {cx.bigint(c.second.into())};
    let offset=cx.bigint(z.offset.into());
    let mut result=cx.cons(offset,Value::NIL);
    result=cx.cons(if z.dst {Value::TRUE} else {Value::NIL},result);
    for n in [c.weekday,c.year,c.month,c.day,c.hour,c.minute] {
        let v=cx.bigint(n.into());
        result=cx.cons(v,result);
    }
    Ok(cx.cons(sec,result))
}}
// GNU frac_to_double rounds the exact ratio, not two rounded integer casts.
fn stamp_float(s: Stamp) -> f64 {
    use num_bigint::BigUint;
    if s.ticks == 0 { return 0.0; }
    let n = BigUint::from(s.ticks.unsigned_abs());
    let d = BigUint::from(s.hz as u128);
    let mut exponent = n.bits() as i32 - d.bits() as i32;
    let below = if exponent >= 0 { n < (&d << exponent as usize) }
        else { (&n << (-exponent) as usize) < d };
    if below { exponent -= 1; }
    let scale = 52 - exponent;
    let (n, d) = if scale >= 0 { (n << scale as usize, d) }
        else { (n, d << (-scale) as usize) };
    let q = &n / &d;
    let remainder = n % &d;
    let mut significand: u64 = q.try_into().expect("53-bit time significand");
    let twice = remainder << 1usize;
    if twice > d || (twice == d && significand & 1 != 0) { significand += 1; }
    let result = significand as f64 * 2.0_f64.powi(-scale);
    if s.ticks < 0 { -result } else { result }
}

defsubr! { name = "float-time", gnu = "timefns.c:1307", effect = Global,
fn float_time(cx; optional time: Value) {
    if cx.float_value(time).is_ok() {return Ok(time);}
    let s=stamp(cx,time,false)?;
    Ok(cx.float(stamp_float(s)))
}}

/// check_tm_member: a fixnum, less OFFSET, as a C int.
fn tm_member(value: Value, offset: i64) -> std::result::Result<i64, Fault> {
    let n = value.integer_value().ok_or(Fault::Wrong("fixnump", value))?;
    let member = n.checked_sub(offset).ok_or_else(time_overflow)?;
    i32::try_from(member).map(i64::from).map_err(|_| time_overflow())
}

/// __mktime_internal (gnulib/glibc mktime.c) for a zone: the time whose
/// local time in RULE is LOCAL (seconds since the epoch as if in UTC).
/// It starts from OFFSET, the previous result's difference, which it
/// updates as glibc's per-process localtime_offset; a spring-forward gap
/// prefers the time whose DST flag differs from ISDST (-1: the DST one), and
/// a mismatched ISDST borrows the offset of a time within a year that has
/// it, or else moves the time by an hour.
fn mktime(rule: &ZoneRule, local: i64, isdst: i64, offset: &mut i64) -> std::result::Result<i64, Fault> {
    let t0 = local.checked_add(*offset).ok_or_else(time_overflow)?;
    let (mut t, mut t1, mut t2, mut dst2) = (t0, t0, t0, false);
    let mut zone = rule.at(t)?;
    let mut probes = 6;
    loop {
        let dt = local - t.checked_add(zone.offset).ok_or_else(time_overflow)?;
        if dt == 0 { break; }
        let gap = if isdst < 0 { dst2 <= zone.dst } else { (isdst != 0) != zone.dst };
        if t == t1 && t != t2 && gap {
            *offset = t - local;
            return Ok(t);
        }
        probes -= 1;
        if probes == 0 { return Err(time_overflow()); }
        (t1, t2, t, dst2) = (t2, t, t.checked_add(dt).ok_or_else(time_overflow)?, zone.dst);
        zone = rule.at(t)?;
    }
    if isdst >= 0 && (isdst != 0) != zone.dst {
        const STRIDE: i64 = 601_200;
        const BOUND: i64 = 366 * 24 * 60 * 60 + 1 + STRIDE;
        let found = 'probe: {
            for delta in (STRIDE..BOUND).step_by(STRIDE as usize) {
                for direction in [-1, 1] {
                    let other = rule.at(t + delta * direction)?;
                    if other.dst == (isdst != 0) { break 'probe Some(local - other.offset); }
                }
            }
            None
        };
        // With no such time nearby, glibc (2.44 on the reference host)
        // takes daylight saving as one hour ahead of standard time.
        t = found.unwrap_or_else(|| t + 3600 * (i64::from(zone.dst) - i64::from(isdst != 0)));
    }
    *offset = t - local;
    Ok(t)
}

defsubr! { name = "encode-time", gnu = "timefns.c:1657", effect = Global,
fn encode_time(cx, time: Value; rest args) {
    let nargs = 1 + args.len();
    let (mut zone, mut isdst) = (Value::NIL, -1i64);
    let fields = if nargs == 1 {
        let mut fields = [Value::NIL; 6];
        let mut tail = time;
        for field in &mut fields {
            let (car, cdr) = cx.pair(tail).ok_or(Fault::Wrong("consp", tail))?;
            *field = car;
            tail = cdr;
        }
        if tail != Value::NIL {
            let (_, rest) = cx.pair(tail).ok_or(Fault::Wrong("consp", tail))?;
            let (dst, rest2) = cx.pair(rest).ok_or(Fault::Wrong("consp", rest))?;
            let (z, _) = cx.pair(rest2).ok_or(Fault::Wrong("consp", rest2))?;
            zone = z;
            if cx.kind(dst) == Kind::Symbol && z.integer_value().is_none() && cx.pair(z).is_none() {
                isdst = i64::from(dst != Value::NIL);
            }
        }
        fields
    } else if nargs < 6 {
        let name = cx.intern("encode-time");
        return Err(Fault::WrongNumberOfArguments(name, Value::integer(nargs as i64).unwrap()));
    } else {
        if nargs > 6 { zone = args[args.len() - 1]; }
        [time, args[0], args[1], args[2], args[3], args[4]]
    };
    // SEC is floor (TICKS / HZ); the remainder rides along in the result.
    let stamp = exact_stamp(cx, fields[0])?;
    let seconds = floor_ratio(stamp.ticks.clone(), &stamp.hz);
    let subseconds = &stamp.ticks - &seconds * &stamp.hz;
    let second = match i64::try_from(&seconds).ok().and_then(|n| Value::integer(n).ok()) {
        Some(value) => tm_member(value, 0)?,
        None => { let value = cx.bigint(seconds); return Err(Fault::Wrong("fixnump", value)); }
    };
    let (minute, hour, day) = (tm_member(fields[1], 0)?, tm_member(fields[2], 0)?, tm_member(fields[3], 0)?);
    let (month, year) = (tm_member(fields[4], 1)?, tm_member(fields[5], 1900)?);
    // Out-of-range members are carried by date arithmetic, as mktime does;
    // seconds outside 0..59 are searched clamped and added back after.
    let year = year + 1900 + month.div_euclid(12);
    let days = civil_days(year, month.rem_euclid(12) + 1, 1) + day - 1;
    let clamped = second.clamp(0, 59);
    let local = days.checked_mul(86400).and_then(|s| s.checked_add(hour * 3600 + minute * 60 + clamped)).ok_or_else(time_overflow)?;
    // GNU's utc_tz: t, 0, or the local zone set-time-zone-rule made UTC.
    let utc = zone == Value::TRUE || zone.integer_value() == Some(0)
        || (zone == Value::NIL && matches!(cx.local_zone(), crate::LocalZone::Utc));
    let rule = zone_rule(cx, zone)?;
    // mktime_z runs timegm for UTC, which keeps its own offset guess.
    let mut offset = if utc { 0 } else { cx.mktime_offset() };
    let value = mktime(&rule, local, if utc { -1 } else { isdst }, &mut offset)?;
    if !utc { cx.set_mktime_offset(offset); }
    let value = value.checked_add(second - clamped).ok_or_else(time_overflow)?;
    calendar(value, rule.at(value)?.offset)?;
    if stamp.hz == num_bigint::BigInt::from(1) {
        if list_time_output(cx)? {
            let high = cx.bigint(value.div_euclid(65536).into());
            let low = Value::integer(value.rem_euclid(65536)).unwrap();
            let tail = cx.cons(low, Value::NIL);
            return Ok(cx.cons(high, tail));
        }
        return Ok(cx.bigint(value.into()));
    }
    let ticks = cx.bigint(num_bigint::BigInt::from(value) * &stamp.hz + subseconds);
    let hz = match stamp.identity { FrequencyIdentity::Lisp(hz) => hz, FrequencyIdentity::FloatPower(_) => cx.bigint(stamp.hz) };
    Ok(cx.cons(ticks, hz))
}}

#[cfg(test)]
mod tests {
    use super::*;

    fn replay_time(name: &str, expected_count: usize) {
        if std::env::var_os("ROMACS_TIME_ORACLE_CHILD").is_none() {
            let test = std::thread::current().name().expect("named test thread").to_owned();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([&test, "--exact", "--nocapture"])
                .env("ROMACS_TIME_ORACLE_CHILD", "1")
                .env("TZ", "UTC0").env("LC_ALL", "C")
                .output().expect("time oracle child");
            assert!(output.status.success(), "TIME_ORACLE child failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            return;
        }
        use sha2::{Digest, Sha256};
        let oracle: serde_json::Value = serde_json::from_str(include_str!("../../evidence/time/oracle.json")).unwrap();
        let rows: Vec<_> = oracle["rows"].as_array().unwrap().iter().filter(|r| r["primitive"] == name).collect();
        assert_eq!(rows.len(), expected_count, "TIME_ORACLE_INVENTORY {name}");
        for row in rows {
            let source = row["source"].as_str().unwrap();
            // The child fixes TZ and locale without changing its parent's environment.
            for path in ["/etc/localtime", "/usr/share/zoneinfo/America/New_York"] {
                if source.contains("wall") || source.contains("America/New_York") {
                    let bytes=std::fs::read(path).expect("TIME_ORACLE_TZDATA");
                    assert_eq!(Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(),oracle["environment_files"][path].as_str().unwrap(),"TIME_ORACLE_TZDATA {path}");
                }
            }
            let mut vm=crate::Vm::new();
            vm.set_collection_floor(1);
            let value=vm.eval_source_raw(source).unwrap_or_else(|e|panic!("TIME_ORACLE {}: {e}",row["id"]));
            let actual=Leaf::new(&mut vm).string_text(value).unwrap_or_else(|_| panic!("expected string"));
            assert_eq!(actual,row["expected"].as_str().unwrap(),"TIME_ORACLE {}",row["id"]);
        }
    }
    #[test] fn format_time_oracle() { replay_time("format-time-string",99); }
    #[test] fn current_time_string_oracle() { replay_time("current-time-string",27); }
    #[test] fn current_time_zone_oracle() { replay_time("current-time-zone",16); }
    #[test] fn decode_time_oracle() { replay_time("decode-time",34); }
    #[test] fn float_time_oracle() { replay_time("float-time",21); }

    #[test]
    fn float_time_live_clock_bracket() {
        let now=||std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        let mut vm=crate::Vm::new();
        let before=now();
        let value=vm.eval_source_raw("(float-time)").unwrap();
        let after=now();
        let actual=Leaf::new(&mut vm).float_value(value).unwrap_or_else(|_| panic!("expected float"));
        assert!(before<=actual && actual<=after,"TIME_LIVE_CLOCK_BRACKET {before} {actual} {after}; assumes no realtime clock step");
    }

    #[test]
    fn current_time_tick_fixnum_boundaries_refuse_without_rounding() {
        for ticks in [
            Value::MIN_FIXNUM as i128,
            -1,
            0,
            1,
            Value::MAX_FIXNUM as i128,
        ] {
            assert_eq!(
                ticks_value(ticks).ok().and_then(Value::integer_value),
                Some(ticks as i64),
                "CURRENT_TIME_EXACT_FIXNUM_BOUNDARY {ticks}"
            );
        }
        for ticks in [
            Value::MIN_FIXNUM as i128 - 1,
            Value::MAX_FIXNUM as i128 + 1,
            i64::MIN as i128 - 1,
            i64::MAX as i128 + 1,
        ] {
            assert!(
                matches!(ticks_value(ticks), Err(Fault::Error(message)) if message == "VM2 current-time requiring bignum ticks is unsupported"),
                "CURRENT_TIME_EXACT_BIGNUM_REFUSAL {ticks}"
            );
        }
    }

    #[test]
    fn current_time_list_preserves_negative_subseconds_and_large_seconds() {
        for (nanos, expected) in [
            (-1, [-1, 65535, 999999, 999000]),
            (0, [0, 0, 0, 0]),
            (65538000003004, [1, 2, 3, 4000]),
            (2621440000000000000, [40000, 0, 0, 0]),
            (i64::MIN as i128 * 1000000000, [-140737488355328, 0, 0, 0]),
            (
                i64::MAX as i128 * 1000000000 + 999999999,
                [140737488355327, 65535, 999999, 999000],
            ),
        ] {
            assert_eq!(
                list_fields(nanos).ok(),
                Some(expected),
                "CURRENT_TIME_EXACT_LIST_COMPONENTS {nanos}"
            );
        }
    }
}

// Fset_time_zone_rule, tzlookup (ZONE, true): the rule local time uses
// from now on. GNU sets the process's TZ, which getenv (reading
// process-environment) does not see; VM2 keeps the rule in the VM.
defsubr! { name = "set-time-zone-rule", gnu = "timefns.c:1962", effect = Global,
fn set_time_zone_rule(cx, tz: Value) {
    let wall = tz == Value::NIL || cx.symbol_name(tz).ok().as_deref() == Some("wall");
    let zone = if wall {
        crate::LocalZone::Wall
    } else if tz == Value::TRUE || tz.integer_value() == Some(0) {
        crate::LocalZone::Utc
    } else if let Ok(text) = cx.string_text(tz) {
        crate::LocalZone::Rule(text)
    } else {
        // tzbuf_format: "<NAME>[-]H:MM:SS", NAME the numeric offset of an
        // integer or the abbreviation of an (OFFSET ABBR) list.
        let (offset, name) = match (tz.integer_value(), cx.pair(tz)) {
            (Some(n), _) => (n, None),
            (None, Some((offset, tail))) => match (offset.integer_value(), cx.pair(tail)) {
                (Some(n), Some((abbr, _))) => (n, Some(abbr)),
                _ => return Err(zone_fault(cx, tz)),
            },
            _ => return Err(zone_fault(cx, tz)),
        };
        let name = match name {
            None => numeric_zone(offset).name,
            Some(abbr) => cx.string_text(abbr).map_err(|_| zone_fault(cx, tz))?,
        };
        let a = offset.unsigned_abs();
        crate::LocalZone::Rule(format!("<{name}>{}{}:{:02}:{:02}", if offset < 0 { "" } else { "-" }, a / 3600, a / 60 % 60, a % 60))
    };
    // tzlookup validates the rule before installing it.
    if let crate::LocalZone::Rule(rule) = &zone {
        let probe = cx.string(rule);
        zone_rule(cx, probe)?;
    }
    cx.set_local_zone(zone);
    Ok(Value::NIL)
}}
