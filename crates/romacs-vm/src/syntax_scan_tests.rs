//! The scanner's tests: GNU cases, and the seeded property tests the
//! summaries' tests share (`Plain`, `Rng`, the random tables and texts).
use super::*;

/// ASCII text with a per-character syntax.
pub(crate) struct Plain<F: Fn(u8) -> i32> { pub text: Vec<u8>, pub table: F }
impl<F: Fn(u8) -> i32> Text for Plain<F> {
    fn char_at(&self, at: Pos) -> u32 { self.text.get(at.byte).copied().map_or(0, u32::from) }
    fn inc(&self, at: Pos) -> Pos { Pos { char: at.char + 1, byte: at.byte + 1 } }
    fn dec(&self, at: Pos) -> Pos { Pos { char: at.char - 1, byte: at.byte - 1 } }
    fn pos(&self, charpos: i64) -> Pos { let c = charpos.clamp(1, self.text.len() as i64 + 1); Pos { char: c, byte: c as usize - 1 } }
    fn syntax(&self, at: Pos) -> i32 { self.table_syntax(self.char_at(at)) }
    fn table_syntax(&self, c: u32) -> i32 { (self.table)(c as u8) }
}
fn lisp(c: u8) -> i32 {
    match c {
        b'(' => SOPEN, b')' => SCLOSE, b'"' => SSTRING, b'\\' => SESCAPE, b';' => SCOMMENT,
        b'\n' => SENDCOMMENT, b' ' => SWHITESPACE, b'\'' => SQUOTE | 1 << 20, _ => SWORD,
    }
}
fn scan<T>(text: &str, f: impl FnOnce(&mut Scanner<'_, Plain<fn(u8) -> i32>>) -> T) -> T {
    let plain = Plain { text: text.as_bytes().to_vec(), table: lisp as fn(u8) -> i32 };
    let ppss = BTreeMap::new();
    let options = Options { ignore_comments: true, open_paren_in_column_0_is_defun_start: true, ..Options::default() };
    let mut scanner = Scanner { begv: plain.pos(1), zv: plain.pos(i64::MAX), text: &plain, options, ppss: &ppss, defun_start: None, modiff: 1 };
    f(&mut scanner)
}
/// A scan's result, its stop as the halt alone.
fn halt<T>(result: Result<T, Stopped<Box<Scan>>>) -> R<T> {
    result.map_err(|stopped| stopped.halt)
}

#[test]
fn sexps_skip_strings_comments_and_prefixes() {
    assert_eq!(scan("(a \"b)\" ; c)\n d) e", |s| halt(s.scan_lists(1, 1, 0, true))), Ok(Some(17)));
    assert_eq!(scan("'(a) b", |s| halt(s.scan_lists(1, 1, 0, true))), Ok(Some(5)));
    assert_eq!(scan("(a) b", |s| halt(s.scan_lists(6, -2, 0, true))), Ok(Some(1)));
    assert_eq!(scan("(a b", |s| halt(s.scan_lists(1, 1, 0, true))),
               Err(Halt::ScanError("Unbalanced parentheses", 1, 5)));
    assert_eq!(scan("a) b", |s| halt(s.scan_lists(1, 1, 1, false))), Ok(Some(3)));
    assert_eq!(scan(" ) b", |s| halt(s.scan_lists(1, 1, 0, false))),
               Err(Halt::ScanError("Containing expression ends prematurely", 2, 3)));
}

#[test]
fn comments_move_both_ways() {
    assert_eq!(scan("  ; a\n ;b\nx", |s| halt(s.forward_comment(s.text.pos(1), 5))).map(|(p, f)| (p.char, f)), Ok((11, false)));
    assert_eq!(scan("x ; a\n ;b\n", |s| halt(s.forward_comment(s.text.pos(11), -2))).map(|(p, f)| (p.char, f)), Ok((3, true)));
}

#[test]
fn partial_parse_reports_depth_strings_and_comments() {
    let state = scan("(a (b \"c", |s| {
        let mut state = ParseState::default();
        s.scan_sexps_forward(&mut state, s.text.pos(1), 9, i64::MIN, false, 0).map(|()| state)
    }).unwrap();
    assert_eq!((state.depth, state.instring, state.comstr_start, state.levelstarts.clone(), state.location),
               (2, '"' as i32, 7, vec![Some(1), Some(4)], 9));
}

/// xorshift64*: the seeded randomness of the property tests.
pub(crate) struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    pub fn below(&mut self, n: usize) -> usize { (self.next() % n as u64) as usize }
    pub fn chance(&mut self, percent: u64) -> bool { self.next() % 100 < percent }
}

/// A random syntax table over a small alphabet: every class a scan
/// knows, with random comment flags, styles, nesting and prefix.
pub(crate) fn random_table(rng: &mut Rng) -> [i32; 128] {
    const CLASSES: [i32; 14] = [SWHITESPACE, 1, SWORD, SSYMBOL, SOPEN, SCLOSE, SQUOTE, SSTRING, SMATH, SESCAPE,
                                SCHARQUOTE, SCOMMENT, SENDCOMMENT, SCOMMENT_FENCE];
    let mut table = [SWORD; 128];
    for c in ALPHABET.iter().copied() {
        let class = if rng.chance(8) { SSTRING_FENCE } else { CLASSES[rng.below(CLASSES.len())] };
        let mut flags = 0;
        for bit in 16..24 {
            if rng.chance(if bit == 20 { 10 } else { 18 }) { flags |= 1 << bit; }
        }
        table[usize::from(c)] = class | flags;
    }
    table
}
pub(crate) const ALPHABET: &[u8] = b"abcdefghij \n";
/// The character position of offset AT.
pub(crate) fn position(at: usize) -> i64 {
    i64::try_from(at).expect("a test text") + 1
}
pub(crate) fn random_text(rng: &mut Rng, len: usize) -> Vec<u8> {
    (0..len).map(|_| ALPHABET[rng.below(ALPHABET.len())]).collect()
}

/// ASCII text whose syntax a propagation changes as a parse goes, read
/// live or, on a rerun, replayed in order as `View` replays it.
struct Mutable<'a> {
    text: Vec<u8>,
    table: &'a [i32; 128],
    overrides: std::cell::RefCell<BTreeMap<usize, i32>>,
    record: std::cell::Cell<bool>,
    replay: std::cell::RefCell<Vec<i32>>,
    read: std::cell::RefCell<Vec<i32>>,
}
impl Text for Mutable<'_> {
    fn char_at(&self, at: Pos) -> u32 { self.text.get(at.byte).copied().map_or(0, u32::from) }
    fn inc(&self, at: Pos) -> Pos { Pos { char: at.char + 1, byte: at.byte + 1 } }
    fn dec(&self, at: Pos) -> Pos { Pos { char: at.char - 1, byte: at.byte - 1 } }
    fn pos(&self, charpos: i64) -> Pos {
        let c = charpos.clamp(1, position(self.text.len()));
        Pos { char: c, byte: usize::try_from(c - 1).expect("a position") }
    }
    fn syntax(&self, at: Pos) -> i32 {
        let live = || self.overrides.borrow().get(&at.byte).copied().unwrap_or_else(|| self.table_syntax(self.char_at(at)));
        if !self.record.get() { return live(); }
        let mut read = self.read.borrow_mut();
        let syntax = self.replay.borrow().get(read.len()).copied().unwrap_or_else(live);
        read.push(syntax);
        syntax
    }
    fn table_syntax(&self, c: u32) -> i32 { self.table[usize::try_from(c).expect("ASCII")] }
}

/// One parse's inputs: its text and options, FROM, END, the OLDSTATE,
/// TARGETDEPTH, STOPBEFORE, COMMENTSTOP, where propertization is done,
/// and the propagation's seed.
struct Case<'a> {
    text: &'a Mutable<'a>,
    options: Options,
    from: i64,
    end: i64,
    state: ParseState,
    args: (i64, bool, i32),
    done: i64,
    seed: u64,
}
/// The propagation a stop at CHARPOS runs: change the syntax of a few
/// positions, before, at and after it, and move `done` past it.
fn propagate(rng: &mut Rng, text: &Mutable<'_>, charpos: i64) -> i64 {
    let len = text.text.len();
    for _ in 0..rng.below(3) {
        let c = ALPHABET[rng.below(ALPHABET.len())];
        text.overrides.borrow_mut().insert(rng.below(len.max(1)), text.table[usize::from(c)]);
    }
    charpos + 1 + i64::try_from(rng.below(6)).expect("small")
}
/// The propagation calls of a parse, as (argument, stop position).
type Calls = Vec<(i64, i64)>;
/// Where each stop of a parse went on: none for a stop before any read.
type Resumed = Vec<Option<(At, Option<ForwAt>)>>;

/// CASE parsed as before: each stop reruns from FROM, replaying what was read.
fn by_replay(case: &Case<'_>) -> (ParseState, Calls) {
    let (text, ppss) = (case.text, BTreeMap::new());
    text.record.set(true);
    text.overrides.borrow_mut().clear();
    text.read.borrow_mut().clear();
    text.replay.borrow_mut().clear();
    let (mut rng, mut done, mut log) = (Rng(case.seed), case.done, vec![]);
    loop {
        let options = Options { propertize_done: Some(done), ..case.options };
        let mut scanner = Scanner { begv: text.pos(1), zv: text.pos(i64::MAX), text, options, ppss: &ppss, defun_start: None, modiff: 1 };
        let mut state = case.state.clone();
        let (target, stopbefore, commentstop) = case.args;
        match scanner.scan_sexps_forward(&mut state, text.pos(case.from), case.end, target, stopbefore, commentstop) {
            Ok(()) => return (state, log),
            Err(Halt::Propertize(arg, charpos)) => {
                log.push((arg, charpos));
                done = propagate(&mut rng, case.text, charpos);
                let read = text.read.take();
                *text.replay.borrow_mut() = read;
            }
            Err(other) => panic!("{other:?}"),
        }
    }
}
/// CASE parsed by resuming each stop; also where each stop went on.
fn by_resume(case: &Case<'_>) -> (ParseState, Calls, Resumed) {
    let (text, ppss) = (case.text, BTreeMap::new());
    text.record.set(false);
    text.overrides.borrow_mut().clear();
    let (mut rng, mut done, mut log, mut labels) = (Rng(case.seed), case.done, vec![], vec![]);
    let mut suspended: Option<Suspended> = None;
    loop {
        let options = Options { propertize_done: Some(done), ..case.options };
        let mut scanner = Scanner { begv: text.pos(1), zv: text.pos(i64::MAX), text, options, ppss: &ppss, defun_start: None, modiff: 1 };
        let (target, stopbefore, commentstop) = case.args;
        let result = if let Some(stopped) = suspended.take() {
            scanner.resume(stopped, case.end, target, stopbefore, commentstop)
        } else {
            let mut state = case.state.clone();
            scanner.parse(&mut state, text.pos(case.from), case.end, target, stopbefore, commentstop).map(|()| state)
        };
        match result {
            Ok(state) => return (state, log, labels),
            Err(Stopped { halt: Halt::Propertize(arg, charpos), suspended: stopped }) => {
                log.push((arg, charpos));
                labels.push(stopped.as_ref().map(|s| (s.at, s.forw.map(|f| f.at))));
                done = propagate(&mut rng, case.text, charpos);
                suspended = stopped.map(|s| *s);
            }
            Err(Stopped { halt, .. }) => panic!("{halt:?}"),
        }
    }
}
/// The state a parse of TEXT from 1 to MID leaves, its exact continuation
/// with it (the summaries' resume), or without (a Lisp OLDSTATE).
fn oldstate(text: &Mutable<'_>, options: Options, mid: i64, exact: bool) -> ParseState {
    let ppss = BTreeMap::new();
    text.record.set(false);
    text.overrides.borrow_mut().clear();
    let mut scanner = Scanner { begv: text.pos(1), zv: text.pos(i64::MAX), text, options, ppss: &ppss, defun_start: None, modiff: 1 };
    let mut state = ParseState::default();
    scanner.scan_sexps_forward(&mut state, text.pos(1), mid, i64::MIN, false, 0).expect("no propertization");
    if !exact { state.continuation = None; }
    state
}
fn mutable<'a>(text: &[u8], table: &'a [i32; 128]) -> Mutable<'a> {
    Mutable { text: text.to_vec(), table, overrides: std::cell::RefCell::default(), record: std::cell::Cell::default(), replay: std::cell::RefCell::default(), read: std::cell::RefCell::default() }
}

/// A parse that stops for propagation and goes on where it stopped
/// reads, ends and calls the propagation as one that reruns from FROM
/// replaying what it read, which is GNU's one scan. Over fixed texts with
/// every construct, every stop position, start and argument set, every
/// place a stop goes on is reached; random tables and texts follow.
#[test]
fn a_resumed_parse_is_a_replayed_one() {
    const P: i32 = 1;
    let flags = |bits: &[u32]| bits.iter().fold(0, |f, &b| f | 1 << b);
    let mut table = [SWORD; 128];
    for (c, code) in [
        (b' ', SWHITESPACE), (b'\n', SENDCOMMENT), (b'(', SOPEN), (b')', SCLOSE), (b'"', SSTRING),
        (b'\\', SESCAPE), (b'|', SSTRING_FENCE), (b'!', SCOMMENT_FENCE), (b';', SCOMMENT), (b'%', SSYMBOL),
        (b'\'', SQUOTE | flags(&[20])), (b'/', P | flags(&[16, 19])), (b'*', P | flags(&[17, 18])),
        (b'<', P | flags(&[16, 19, 22])), (b'#', P | flags(&[17, 18, 22])),
        // A comment starter inside a symbol.
        (b'{', SWORD | flags(&[16])), (b'}', SSYMBOL | flags(&[17])),
    ] {
        table[usize::from(c)] = code;
    }
    let fixtures: [&[u8]; 4] = [
        b"(a \"b\\\"c\" d) ; x\\\n y ab/* c\\*/ */ |f\\|g| e\\(f 'h %i ! j !",
        b"<# a <# b #< c \\#< #< (k) x\\y/*z*/ \"s\\",
        b"; e\\\nq ab\\ /*\\*/*/ \"\\\"\" | \\| | (((a)) b",
        b"ab/*c*/ <#<##<#< \\ \"x\" xy{}z\n w ;\\",
    ];
    let mut reached = Vec::new();
    let mut seed = 1;
    for fixture in fixtures {
        let text = mutable(fixture, &table);
        let len = position(fixture.len());
        for escaped in [false, true] {
            let options = Options { comment_end_can_be_escaped: escaped, ..Options::default() };
            for mid in 1..=len {
                for exact in [true, false] {
                    let state = oldstate(&text, options, mid, exact);
                    for args in [(i64::MIN, false, 0), (i64::MIN, true, 0), (i64::MIN, false, 1), (i64::MIN, false, -1), (state.depth + 1, false, 0), (state.depth - 1, false, 0)] {
                        for done in mid..=len + 1 {
                            seed += 1;
                            let case = Case { text: &text, options, from: mid, end: len, state: state.clone(), args, done, seed };
                            let (replayed, replay_log) = by_replay(&case);
                            let (resumed, resume_log, labels) = by_resume(&case);
                            assert_eq!((&resumed, &resume_log), (&replayed, &replay_log), "{:?} from {mid} done {done} {args:?} exact {exact} escaped {escaped}", std::str::from_utf8(fixture));
                            for label in labels { if !reached.contains(&label) { reached.push(label); } }
                        }
                    }
                }
            }
        }
    }
    let wanted = [
        Some((At::CommentStarted, None)), Some((At::CommentEscaped, None)), Some((At::TopDispatch, None)),
        Some((At::Symbol, None)), Some((At::SymbolToComment, None)), Some((At::SymbolQuoted, None)),
        Some((At::InString, None)), Some((At::QuotedInString, None)), Some((At::StringEnded, None)),
        Some((At::CommentEnded, None)), Some((At::InComment, Some(ForwAt::Read))),
        Some((At::InComment, Some(ForwAt::EscapeSkipped))), Some((At::InComment, Some(ForwAt::Incomment))),
        Some((At::InComment, Some(ForwAt::NestedChecks))), None,
    ];
    let missed: Vec<_> = wanted.iter().filter(|label| !reached.contains(label)).collect();
    assert!(missed.is_empty(), "never stopped at {missed:?}");
    let mut rng = Rng(0x5eed_0f57);
    for _ in 0..3000 {
        let table = random_table(&mut rng);
        let size = rng.below(70);
        let fixture = random_text(&mut rng, size);
        let text = mutable(&fixture, &table);
        let len = position(fixture.len());
        let options = Options { comment_end_can_be_escaped: rng.chance(50), ..Options::default() };
        let mid = position(rng.below(fixture.len() + 1));
        let state = oldstate(&text, options, mid, rng.chance(50));
        let args = (if rng.chance(30) { state.depth + i64::try_from(rng.below(3)).expect("small") - 1 } else { i64::MIN },
                    rng.chance(20), [0, 0, 1, -1][rng.below(4)]);
        let done = mid + i64::try_from(rng.below(fixture.len() + 2)).expect("small");
        let case = Case { text: &text, options, from: mid, end: len, state, args, done, seed: rng.next() };
        let (replayed, replay_log) = by_replay(&case);
        let (resumed, resume_log, _) = by_resume(&case);
        assert_eq!((resumed, resume_log), (replayed, replay_log), "{fixture:?} from {mid} done {done} {args:?}");
    }
}

/// A list or comment scan: `scan_lists`'s FROM, COUNT, DEPTH and SEXPFLAG,
/// or `forward_comment`'s FROM and COUNT.
#[derive(Clone, Copy, Debug)]
enum Call { Lists(i64, i64, i64, bool), Comments(i64, i64) }
/// What a scan ends in: a position and `forward_comment`'s flag, or its
/// scan error.
type Ended = Result<(Option<i64>, bool), (&'static str, i64, i64)>;
/// Where each stop of a scan went on: none for a stop before any read.
type Went = Vec<Option<(ScanAt, Option<ForwAt>)>>;

/// CALL over TEXT, propertized to DONE, SEED's propagation run at each stop
/// for Lisp; a `syntax-ppss` stop (logged with argument -1) is answered
/// with a defun start at or before the position asked. RESUMED, each stop
/// goes on where it stopped, with that answer alone; else the scan reruns
/// from its start, replaying what it read, with every answer so far.
fn scanned(text: &Mutable<'_>, options: Options, call: Call, done: i64, seed: u64, resumed: bool) -> (Ended, Calls, Went) {
    text.record.set(!resumed);
    text.overrides.borrow_mut().clear();
    text.read.borrow_mut().clear();
    text.replay.borrow_mut().clear();
    let (mut rng, mut done, mut log, mut went) = (Rng(seed), done, vec![], vec![]);
    let mut answers = BTreeMap::new();
    let mut kept: Option<Scan> = None;
    loop {
        let options = Options { propertize_done: Some(done), ..options };
        let mut scanner = Scanner { begv: text.pos(1), zv: text.pos(i64::MAX), text, options, ppss: &answers, defun_start: None, modiff: 1 };
        let result = match (call, kept.take()) {
            (Call::Lists(_, _, _, sexpflag), Some(scan)) => scanner.lists(scan, sexpflag).map(|at| (at, false)),
            (Call::Lists(from, count, depth, sexpflag), None) => scanner.scan_lists(from, count, depth, sexpflag).map(|at| (at, false)),
            (Call::Comments(..), Some(scan)) => scanner.comments(scan).map(|(at, found)| (Some(at.char), found)),
            (Call::Comments(from, count), None) => scanner.forward_comment(text.pos(from), count).map(|(at, found)| (Some(at.char), found)),
        };
        let stopped = match result {
            Ok(value) => return (Ok(value), log, went),
            Err(Stopped { halt: Halt::ScanError(message, a, b), .. }) => return (Err((message, a, b)), log, went),
            Err(stopped) => stopped,
        };
        match stopped.halt {
            Halt::Propertize(arg, charpos) => {
                log.push((arg, charpos));
                done = propagate(&mut rng, text, charpos);
            }
            Halt::Ppss(pos) => {
                log.push((-1, pos));
                propagate(&mut rng, text, pos);
                if resumed { answers.clear(); }
                let back = i64::try_from(rng.below(3) * rng.below(9)).expect("small");
                answers.insert(pos, (pos - back).max(1));
            }
            Halt::ScanError(..) => unreachable!("returned above"),
        }
        if resumed {
            went.push(stopped.suspended.as_ref().map(|scan| (scan.at, scan.forw.map(|forw| forw.at))));
            kept = stopped.suspended.map(|scan| *scan);
        } else {
            let read = text.read.take();
            *text.replay.borrow_mut() = read;
        }
    }
}

/// `scan_lists` and `forward_comment` stopped for Lisp and gone on where
/// they stopped end, and call Lisp, as ones rerun from their start
/// replaying what they read, which is GNU's one scan: over random tables
/// and texts whose syntax each stop changes, forward through
/// propertization and backward through `syntax-ppss`, every place a stop
/// goes on is reached.
#[test]
fn a_resumed_scan_is_a_replayed_one() {
    let mut rng = Rng(0x5ca9_11f7);
    let mut reached: Went = Vec::new();
    for case in 0..40_000 {
        let table = random_table(&mut rng);
        let size = rng.below(70);
        let fixture = random_text(&mut rng, size);
        let text = mutable(&fixture, &table);
        let options = Options {
            ignore_comments: rng.chance(80), comment_end_can_be_escaped: rng.chance(40),
            open_paren_in_column_0_is_defun_start: rng.chance(50), comment_use_syntax_ppss: rng.chance(70),
            ..Options::default()
        };
        let from = position(rng.below(fixture.len() + 1));
        let count = i64::try_from(rng.below(4)).expect("small") + 1;
        let count = if rng.chance(50) { count } else { -count };
        let call = if rng.chance(70) {
            Call::Lists(from, count, i64::try_from(rng.below(3)).expect("small") - 1, rng.chance(60))
        } else {
            Call::Comments(from, count)
        };
        // Backward, propertization stops a scan at its start only.
        let done = if count > 0 { from + i64::try_from(rng.below(8)).expect("small") } else { position(rng.below(fixture.len() + 2)) };
        let seed = rng.next();
        let (replayed, replay_log, _) = scanned(&text, options, call, done, seed, false);
        let (resumed, resume_log, went) = scanned(&text, options, call, done, seed, true);
        assert_eq!((&resumed, &resume_log), (&replayed, &replay_log),
                   "case {case}: {call:?} over {:?} done {done} {options:?}", String::from_utf8_lossy(&fixture));
        if let Call::Lists(..) = call { for at in went { if !reached.contains(&at) { reached.push(at); } } }
    }
    let wanted = [
        Some((ScanAt::Top, None)), Some((ScanAt::Stepped, None)), Some((ScanAt::Dispatch, None)),
        Some((ScanAt::Symbol, None)), Some((ScanAt::String, None)), Some((ScanAt::Decode, None)),
        Some((ScanAt::Comment, Some(ForwAt::Read))), Some((ScanAt::Comment, Some(ForwAt::EscapeSkipped))),
        Some((ScanAt::Comment, Some(ForwAt::Incomment))), Some((ScanAt::Comment, Some(ForwAt::NestedChecks))), None,
    ];
    let missed: Vec<_> = wanted.iter().filter(|at| !reached.contains(at)).collect();
    assert!(missed.is_empty(), "never stopped at {missed:?}");
}

/// A scan stopped anywhere and resumed from its own state, with its
/// continuation, ends where one scan from the start does.
#[test]
fn a_scan_resumed_from_its_continuation_is_one_scan() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for case in 0..4000 {
        let table = random_table(&mut rng);
        let len = if case % 50 == 0 { 300 } else { rng.below(60) };
        let mut text = random_text(&mut rng, len);
        if case % 50 == 0 {
            // Deep enough for the 100-level cap.
            let open = ALPHABET.iter().copied().find(|&c| table[usize::from(c)] & 0xff == SOPEN);
            if let Some(open) = open { for byte in text.iter_mut().take(150) { *byte = open; } }
        }
        let plain = Plain { text, table: |c: u8| table[usize::from(c)] };
        let ppss = BTreeMap::new();
        let options = Options { comment_end_can_be_escaped: rng.chance(50), ..Options::default() };
        let mut scanner = Scanner { begv: plain.pos(1), zv: plain.pos(i64::MAX), text: &plain, options, ppss: &ppss, defun_start: None, modiff: 1 };
        let end = position(len);
        let from_at = rng.below(len + 1);
        let from = position(from_at);
        let mut mid = position(from_at + rng.below(len - from_at + 1));
        // Not inside a two-character comment start (Continuation).
        while mid > from && mid < end && comstart_first(plain.syntax(plain.pos(mid - 1))) && comstart_second(plain.syntax(plain.pos(mid))) { mid -= 1; }
        let mut whole = ParseState::default();
        scanner.scan_sexps_forward(&mut whole, plain.pos(from), end, i64::MIN, false, 0).unwrap();
        let mut first = ParseState::default();
        scanner.scan_sexps_forward(&mut first, plain.pos(from), mid, i64::MIN, false, 0).unwrap();
        if first.continuation.is_some_and(|c| c.spent) { continue; }
        let mut second = first.clone();
        scanner.scan_sexps_forward(&mut second, plain.pos(mid), end, i64::MIN, false, 0).unwrap();
        second.mindepth = second.mindepth.min(first.mindepth);
        second.maxdepth = second.maxdepth.max(first.maxdepth);
        assert_eq!(second, whole, "case {case}: {:?} split at {mid} of {from}..{end}, escaped ends {}",
                   String::from_utf8_lossy(&plain.text), options.comment_end_can_be_escaped);
    }
}
