//! POSIX TZ strings as glibc's tzset.c reads them (`__tzset_parse_tz'):
//! a standard name and offset, optionally a DST name, offset and the two
//! change rules. glibc falls back to this when TZ names no readable zone
//! file, and TZif files use it for times after their last transition.

/// One zone of a rule: its abbreviation, seconds east of UTC, DST flag.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Local { pub name: String, pub offset: i64, pub dst: bool }

#[derive(Clone, Copy, Debug)]
enum Date { Julian1(u16), Julian0(u16), Month { m: u16, n: u16, d: u16 } }

#[derive(Clone, Debug)]
struct Change { date: Date, secs: i64 }

#[derive(Clone, Debug)]
pub(crate) struct PosixTz {
    standard: Local,
    /// The DST zone and its start and end changes.
    daylight: Option<(Local, Change, Change)>,
    /// A DST zone was named without a rule: glibc first tries the
    /// `posixrules' file's transitions (`__tzfile_default').
    pub(crate) ruleless: bool,
}

/// parse_tzname: 3+ letters, or a 3+ character `<...>' of letters, digits,
/// `+' and `-'.
fn tzname(s: &str) -> Option<(String, &str)> {
    let letters = s.bytes().take_while(u8::is_ascii_alphabetic).count();
    if letters >= 3 { return Some((s[..letters].to_string(), &s[letters..])); }
    let inner = s.strip_prefix('<')?;
    let len = inner.bytes().take_while(|&b| b.is_ascii_alphanumeric() || b == b'+' || b == b'-').count();
    let rest = inner[len..].strip_prefix('>')?;
    (len >= 3).then(|| (inner[..len].to_string(), rest))
}

/// One `%hu': optional white space and sign, then digits, as an unsigned
/// short (a minus sign wraps).
fn hu(s: &str) -> Option<(u32, &str)> {
    let s = s.trim_start_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r']);
    let (negative, s) = match s.as_bytes().first() {
        Some(b'+') => (false, &s[1..]),
        Some(b'-') => (true, &s[1..]),
        _ => (false, s),
    };
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 { return None; }
    let n = s[..digits].parse::<u64>().unwrap_or(u64::MAX) as u32;
    Some(((if negative { n.wrapping_neg() } else { n }) & 0xffff, &s[digits..]))
}

/// sscanf "%hu%n:%hu%n:%hu%n": up to three colon-separated numbers; the
/// rest starts after the last one read.
fn hms(s: &str) -> (Option<[u32; 3]>, &str) {
    let mut fields = [0u32; 3];
    let mut rest = s;
    let mut parsed = 0;
    for (i, field) in fields.iter_mut().enumerate() {
        let body = if i == 0 { rest } else { match rest.strip_prefix(':') { Some(b) => b, None => break } };
        let Some((n, after)) = hu(body) else { break };
        *field = n;
        rest = after;
        parsed += 1;
    }
    ((parsed > 0).then_some(fields), rest)
}

/// parse_offset: [+-]hh[:mm[:ss]], west positive; returned east positive
/// with compute_offset's clamping. A standard offset must start with a
/// sign or digit.
fn offset(s: &str, standard: bool) -> Option<(i64, &str)> {
    if standard && !s.starts_with(|c: char| c == '+' || c == '-' || c.is_ascii_digit()) { return None; }
    let (east, s) = match s.as_bytes().first() {
        Some(b'+') => (false, &s[1..]),
        Some(b'-') => (true, &s[1..]),
        _ => (false, s),
    };
    let (fields, rest) = hms(s);
    let [h, m, sec] = fields?;
    let seconds = i64::from(sec.min(59)) + i64::from(m.min(59)) * 60 + i64::from(h.min(24)) * 3600;
    Some((if east { seconds } else { -seconds }, rest))
}

/// parse_rule: `,DATE[/TIME]'; an empty string is the US default rule.
fn rule(s: &str, start: bool) -> Option<(Change, &str)> {
    let s = s.strip_prefix(',').unwrap_or(s);
    let (date, rest) = if let Some(body) = s.strip_prefix('J') {
        let digits = body.bytes().take_while(u8::is_ascii_digit).count();
        let d: u32 = body[..digits].parse().ok()?;
        if d == 0 || d > 365 { return None; }
        (Date::Julian1(d as u16), &body[digits..])
    } else if s.starts_with(|c: char| c.is_ascii_digit()) {
        let digits = s.bytes().take_while(u8::is_ascii_digit).count();
        let d: u32 = s[..digits].parse().ok()?;
        if d > 365 { return None; }
        (Date::Julian0(d as u16), &s[digits..])
    } else if let Some(body) = s.strip_prefix('M') {
        // sscanf "M%hu.%hu.%hu%n".
        let mut parts = [0u16; 3];
        let mut rest = body;
        for (i, part) in parts.iter_mut().enumerate() {
            if i > 0 { rest = rest.strip_prefix('.')?; }
            let (n, after) = hu(rest)?;
            *part = n as u16;
            rest = after;
        }
        let [m, n, d] = parts;
        if !(1..=12).contains(&m) || !(1..=5).contains(&n) || d > 6 { return None; }
        (Date::Month { m, n, d }, rest)
    } else if s.is_empty() {
        (if start { Date::Month { m: 3, n: 2, d: 0 } } else { Date::Month { m: 11, n: 1, d: 0 } }, s)
    } else {
        return None;
    };
    if let Some(time) = rest.strip_prefix('/') {
        if time.is_empty() { return None; }
        let (negative, time) = match time.strip_prefix('-') { Some(t) => (true, t), None => (false, time) };
        let (fields, after) = hms(time);
        let [h, m, sec] = fields.unwrap_or([2, 0, 0]);
        let secs = i64::from(h) * 3600 + i64::from(m) * 60 + i64::from(sec);
        Some((Change { date, secs: if negative { -secs } else { secs } }, after))
    } else if rest.is_empty() || rest.starts_with(',') {
        Some((Change { date, secs: 2 * 3600 }, rest))
    } else {
        None
    }
}

impl PosixTz {
    /// __tzset_parse_tz: what glibc makes of TZ. A string it cannot read is
    /// unnamed UTC; a standard name without an offset keeps the name.
    pub(crate) fn parse(tz: &str) -> PosixTz {
        let utc = |name: String| PosixTz { standard: Local { name, offset: 0, dst: false }, daylight: None, ruleless: false };
        let Some((name, rest)) = tzname(tz) else { return utc(String::new()) };
        let Some((std_offset, rest)) = offset(rest, true) else { return utc(name) };
        let standard = Local { name, offset: std_offset, dst: false };
        if rest.is_empty() { return PosixTz { standard, daylight: None, ruleless: false }; }
        let Some((dst_name, rest)) = tzname(rest) else { return PosixTz { standard, daylight: None, ruleless: false } };
        let (dst_offset, rest) = offset(rest, false).unwrap_or((std_offset + 3600, rest));
        let daylight = Local { name: dst_name, offset: dst_offset, dst: true };
        let ruleless = rest.is_empty() || rest == ",";
        // A rule that does not parse leaves glibc's zeroed J0 day 0 at 00:00.
        let zero = || Change { date: Date::Julian0(0), secs: 0 };
        let (start, end) = match rule(rest, true) {
            Some((start, rest)) => (start, rule(rest, false).map_or_else(zero, |(end, _)| end)),
            None => (zero(), zero()),
        };
        PosixTz { standard, daylight: Some((daylight, start, end)), ruleless }
    }

    /// The standard and DST zones `__tzfile_default' lends a rule file to.
    pub(crate) fn zones(&self) -> Option<(&Local, &Local)> {
        self.daylight.as_ref().map(|(dst, _, _)| (&self.standard, dst))
    }

    /// __tz_compute: the zone in effect at UTC time T.
    pub(crate) fn at(&self, t: i64) -> Local {
        let Some((daylight, start, end)) = &self.daylight else { return self.standard.clone() };
        let year = utc_year(t);
        let begins = change(start, year, self.standard.offset);
        let ends = change(end, year, daylight.offset);
        let dst = if begins > ends { t < ends || t >= begins } else { t >= begins && t < ends };
        if dst { daylight.clone() } else { self.standard.clone() }
    }
}

fn leap(year: i64) -> bool { year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) }

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = month + if month > 2 { -3 } else { 9 };
    era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + (153 * mp + 2) / 5 + day - 1 - 719468
}

fn utc_year(t: i64) -> i64 {
    let days = t.div_euclid(86400);
    let (mut lo, mut hi) = (days / 366 + 1970 - 1, days / 365 + 1970 + 1);
    if lo > hi { std::mem::swap(&mut lo, &mut hi); }
    let mut year = lo;
    while days_from_civil(year + 1, 1, 1) <= days { year += 1; }
    while days_from_civil(year, 1, 1) > days { year -= 1; }
    year
}

/// compute_change: the UTC time of RULE's change in YEAR for a zone whose
/// offset (east positive) is OFFSET. glibc counts from 1970 only.
fn change(rule: &Change, year: i64, offset: i64) -> i64 {
    const DAY: i64 = 86400;
    const MON_YDAY: [[i64; 13]; 2] = [
        [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334, 365],
        [0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335, 366],
    ];
    let mut t = if year > 1970 { days_from_civil(year, 1, 1) * DAY } else { 0 };
    match rule.date {
        Date::Julian1(d) => {
            t += (i64::from(d) - 1) * DAY;
            if d >= 60 && leap(year) { t += DAY; }
        }
        Date::Julian0(d) => t += i64::from(d) * DAY,
        Date::Month { m, n, d } => {
            let table = &MON_YDAY[usize::from(leap(year))];
            let m = usize::from(m);
            t += table[m - 1] * DAY;
            // Zeller's congruence: the weekday of the month's first day.
            let m1 = (m as i64 + 9) % 12 + 1;
            let yy0 = if m <= 2 { year - 1 } else { year };
            let (yy1, yy2) = (yy0 / 100, yy0 % 100);
            let mut dow = ((26 * m1 - 2) / 10 + 1 + yy2 + yy2 / 4 + yy1 / 4 - 2 * yy1) % 7;
            if dow < 0 { dow += 7; }
            let mut day = i64::from(d) - dow;
            if day < 0 { day += 7; }
            for _ in 1..n {
                if day + 7 >= table[m] - table[m - 1] { break; }
                day += 7;
            }
            t += day * DAY;
        }
    }
    t - offset + rule.secs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn us_rule_switches_on_the_second_sunday_of_march() {
        let tz = PosixTz::parse("EST5EDT,M3.2.0,M11.1.0");
        // 2024-03-10 06:59:59 UTC is 01:59:59 EST; a second later is EDT.
        assert_eq!(tz.at(1710053999).name, "EST");
        assert_eq!(tz.at(1710054000), Local { name: "EDT".into(), offset: -4 * 3600, dst: true });
        // 2024-11-03 05:59:59 UTC is 01:59:59 EDT; a second later is EST.
        assert!(tz.at(1730613599).dst);
        assert!(!tz.at(1730613600).dst);
    }

    #[test]
    fn names_and_fallbacks_follow_glibc() {
        assert_eq!(PosixTz::parse("ABC").at(0), Local { name: "ABC".into(), offset: 0, dst: false });
        assert_eq!(PosixTz::parse("No/Such_Zone").at(0).name, "");
        assert_eq!(PosixTz::parse("<X>-1").at(0), Local { name: String::new(), offset: 0, dst: false });
        assert_eq!(PosixTz::parse("<+0330>-3:30").at(0).offset, 3 * 3600 + 1800);
        // Southern hemisphere: DST spans the new year.
        let tz = PosixTz::parse("AEST-10AEDT,M10.1.0,M4.1.0/3");
        assert!(tz.at(1704067200).dst);
        assert!(!tz.at(1719792000).dst);
    }
}
