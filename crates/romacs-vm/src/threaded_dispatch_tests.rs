//! Every handler's dispatch is a jump. A dispatch made by `call` keeps a
//! native frame per instruction it runs, so a long loop through it runs out
//! of native stack; LLVM makes the sibling call only when no stack slot's
//! address has escaped in the function, and with 256 codegen units it cannot
//! see that an out-of-line callee does not keep one.
//!
//! The guarantee is the run-time one: the constant-stack tests
//! (`register_tests::named_calls_dispatch_in_constant_native_stack` and
//! its neighbours) loop over each kind of call and fail when the native
//! stack grows. This test is the lint beside them: it reads the machine
//! code of this very test binary (whatever profile built it) and names the
//! function that broke, including paths no loop exercises. It reads x86-64
//! text, not a model of it, so it fails closed where it cannot follow:
//!
//! - a dispatching function (one that jumps or calls through an
//!   instruction's handler slot, or jumps to one that does) takes no stack
//!   slot's address (`lea ...(%rsp)`), keeps no frame pointer, and calls
//!   through no handler slot (followed through registers and stack slots);
//! - no engine function calls a dispatching one, save the driver and a lend's
//!   nested run (`DRIVER`), which start a run rather than continue one.
//!
//! The exceptions are named below with the breaks each may show and why;
//! a break may hold in one profile only (`gate:`, `release:`), and one no
//! longer shown in its profile must leave the list. GNU binutils' `nm` and
//! `objdump`, on the gate's PATH, read the binary; the test fails, it does
//! not skip, when they cannot.
use super::Inst;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::process::Command;

/// What starts a run, by name below `romacs_vm::threaded::`: the engine's
/// driver and a lend's nested run. A call to one runs Lisp below the caller,
/// as `eval` inside `eval` does; it does not continue the caller's code.
const DRIVER: &[&str] = &["execute_args_inner", "execute_args::{{closure}}", "lend::Nested::run", "lend::Nested::run::{{closure}}"];

/// The functions allowed some breaks, by name below `romacs_vm::threaded::`,
/// which (`stack`, `slot-call`, `frame-pointer`, `call NAME`, each
/// for one profile alone when marked `gate:` or `release:`), and why. Each
/// break lies on a path entered once per run, lend or debugger stop, not
/// once per instruction.
const ALLOWED: &[(&str, &[&str], &str)] = &[
    ("execute_args_inner", &["gate:stack", "gate:slot-call"], "the engine's driver: it runs the first dispatch and each landing, and returns when the run does"),
    ("execute_args::{{closure}}", &["release:stack", "release:slot-call"], "the engine's driver as a release build inlines it into its caller"),
    ("resolve_entry", &["stack"], "a call site's callee lowered or forced, then entered; its loop is a constant-stack case (alternating closures)"),
    ("retry_call", &["stack"], "resumes a call an autoload or a lambda list retried; its loop is a constant-stack case (lambda list by name)"),
    ("t2_enter_closure", &["stack"], "a T2 continuation's closure entry forces its callee first; its loop is a constant-stack case (T2 closure callback)"),
];

/// The profile that built this binary, when it is one the list marks.
fn profile() -> Option<&'static str> {
    let out = env!("OUT_DIR");
    ["gate", "release"].into_iter().find(|p| out.contains(&format!("/{p}/build/")))
}

/// The breaks ENTRY allows in PROFILE (all of them when it is unknown).
fn allowed_in<'a>(entry: &'a [&'a str], profile: Option<&str>) -> impl Iterator<Item = &'a str> + 'a {
    let profile = profile.map(str::to_owned);
    entry.iter().filter_map(move |b| match b.split_once(':') {
        Some((p, b)) if ["gate", "release"].contains(&p) => (profile.is_none() || profile.as_deref() == Some(p)).then_some(b),
        _ => Some(*b),
    })
}

/// A function's machine code: each instruction's address and text, as
/// objdump prints it.
type Code = Vec<(u64, String)>;

/// A function's address and code.
struct Function {
    addr: u64,
    code: Code,
}

/// The instructions of objdump's output TEXT.
fn instructions(text: &str) -> Code {
    text.lines()
        .filter_map(|l| {
            let (addr, ins) = l.split_once(":\t")?;
            Some((u64::from_str_radix(addr.trim(), 16).ok()?, ins.trim().to_owned()))
        })
        .collect()
}

fn tool(name: &str, args: &[String]) -> String {
    let out = Command::new(name)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{name} must be on PATH to read the engine's machine code: {e}"));
    assert!(out.status.success(), "{name} {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).expect("tool output is UTF-8")
}

/// The engine's functions in BINARY: name, address, size.
fn engine_symbols(binary: &str) -> Vec<(String, u64, u64)> {
    tool("nm", &["-S".into(), "--defined-only".into(), "-C".into(), binary.into()])
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(4, ' ');
            let (addr, size, kind, name) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
            let name = name.strip_prefix("romacs_vm::threaded::")?;
            // The engine's own functions, not its tests'.
            if kind != "t" && kind != "T" || name.contains("tests::") {
                return None;
            }
            // The legacy mangling's hash, which differs per build.
            let name = match name.rsplit_once("::h") {
                Some((n, h)) if h.len() == 16 => n,
                _ => name,
            };
            Some((name.to_owned(), u64::from_str_radix(addr, 16).ok()?, u64::from_str_radix(size, 16).ok()?))
        })
        .collect()
}

fn disassemble(binary: &str, symbols: &[(String, u64, u64)]) -> Vec<Function> {
    let chunks: Vec<_> = symbols.chunks(symbols.len().div_ceil(16).max(1)).collect();
    std::thread::scope(|scope| {
        let workers: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|(_, addr, size)| {
                            let text = tool("objdump", &[
                                "-d".into(), "--no-show-raw-insn".into(),
                                format!("--start-address={addr:#x}"), format!("--stop-address={:#x}", addr + size),
                                binary.into(),
                            ]);
                            Function { addr: *addr, code: instructions(&text) }
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().expect("a disassembly worker")).collect()
    })
}

/// What one function does: dispatch through a slot by jump or call, its
/// direct jumps out and calls by target address, and what makes its
/// dispatches unreadable or blocks their jumps.
#[derive(Default)]
#[expect(clippy::struct_excessive_bools, reason = "one flag per thing the reading sees, each read on its own")]
struct Shape {
    jumps_through_slot: bool,
    calls_through_slot: bool,
    jumps: BTreeSet<u64>,
    calls: BTreeSet<u64>,
    stack_address: bool,
    /// A frame pointer: locals addressed off `%rbp` escape the reading.
    frame_pointer: bool,
}

/// The displacement of a `disp(%reg)` or `(%reg)` operand, unless the base
/// is the stack or instruction pointer or the operand is indexed.
fn slot_displacement(operand: &str) -> Option<u64> {
    let (disp, base) = operand.split_once("(%")?;
    let base = base.strip_suffix(')')?;
    if base.starts_with("rsp") || base.starts_with("rip") || base.contains(',') {
        return None;
    }
    if disp.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(disp.strip_prefix("0x")?, 16).ok()
}

/// An instruction's mnemonic and operands, without the prefixes that do not
/// change what it does (`notrack jmp *%rax` is a jump).
fn parts(line: &str) -> (&str, &str) {
    let mut line = line;
    while let Some(rest) = ["notrack ", "bnd ", "ds ", "repz "].iter().find_map(|p| line.strip_prefix(p)) {
        line = rest.trim_start();
    }
    line.split_once(char::is_whitespace).map_or((line, ""), |(o, a)| (o, a.trim()))
}

/// The 64-bit register a register operand names (`%eax` is `%rax`, `%r8d`
/// is `%r8`); other operands as they are.
fn register(operand: &str) -> String {
    let Some(name) = operand.strip_prefix('%') else { return operand.to_owned() };
    let name = name.trim_end_matches(['d', 'w', 'b']);
    match name.strip_prefix('e') {
        Some(rest) if !name.starts_with('r') => format!("%r{rest}"),
        _ if name.starts_with('r') => format!("%{name}"),
        _ => format!("%r{name}"),
    }
}

/// The registers a call may leave changed (System V).
const CALL_CLOBBERED: &[&str] = &["%rax", "%rcx", "%rdx", "%rsi", "%rdi", "%r8", "%r9", "%r10", "%r11"];

/// Whether an instruction OP leaves its last operand unwritten.
fn reads_only(op: &str) -> bool {
    op.starts_with("cmp") || op.starts_with("test") || op.starts_with("bt") || op.starts_with("push") || op.starts_with("ucomi")
}

/// A direct branch's target address.
fn branch_target(args: &str) -> Option<u64> {
    args.split_whitespace().next().and_then(|a| u64::from_str_radix(a, 16).ok())
}

/// Where control goes after instruction AT of CODE (by index), within it.
fn successors(code: &Code, index: &BTreeMap<u64, usize>, at: usize) -> Vec<usize> {
    let (op, args) = parts(&code[at].1);
    let local = || branch_target(args).and_then(|t| index.get(&t).copied());
    let next = (at + 1 < code.len()).then_some(at + 1);
    match op {
        _ if op.starts_with("ret") || op == "ud2" || op == "hlt" => vec![],
        "jmp" => local().into_iter().collect(),
        _ if op.starts_with('j') => next.into_iter().chain(local()).collect(),
        _ => next.into_iter().collect(),
    }
}

/// What reaches an instruction: its edges' state ENTRY and the table
/// jumps' TABLE, when either does.
fn joined(entry: Option<&BTreeSet<String>>, table: Option<&BTreeSet<String>>) -> Option<BTreeSet<String>> {
    match (entry, table) {
        (None, None) => None,
        (e, t) => Some(e.into_iter().chain(t).flatten().cloned().collect()),
    }
}

/// Join HELD into INTO; whether INTO changed, reached for the first time
/// included.
fn join(into: &mut Option<BTreeSet<String>>, held: &BTreeSet<String>) -> bool {
    let first = into.is_none();
    let into = into.get_or_insert_with(BTreeSet::new);
    let before = into.len();
    into.extend(held.iter().cloned());
    first || into.len() != before
}

fn shape(code: &Code, slots: &BTreeSet<u64>) -> Shape {
    let mut s = Shape::default();
    let index: BTreeMap<u64, usize> = code.iter().enumerate().map(|(i, (a, _))| (*a, i)).collect();
    let slot = |operand: &str| slot_displacement(operand).is_some_and(|d| slots.contains(&d));
    // The registers and stack slots (an operand is held as written) that
    // may hold a value loaded from a handler slot on
    // entry to each instruction, over every path to it. A jump table's
    // targets are not known, so what any table jump carried joins every
    // instruction.
    let mut entry: Vec<Option<BTreeSet<String>>> = vec![None; code.len()];
    let mut table: Option<BTreeSet<String>> = None;
    if !code.is_empty() {
        entry[0] = Some(BTreeSet::new());
    }
    loop {
        let mut changed = false;
        for at in 0..code.len() {
            let Some(mut held) = joined(entry[at].as_ref(), table.as_ref()) else { continue };
            let (op, args) = parts(&code[at].1);
            if op == "call" {
                for r in CALL_CLOBBERED {
                    held.remove(*r);
                }
            } else if op == "jmp" && args.starts_with('*') && !held.contains(&register(&args[1..])) && !slot(&args[1..]) {
                changed |= join(&mut table, &held);
            } else if let Some((src, dst)) = args.rsplit_once(',') {
                if op == "mov" && (held.contains(&register(src)) || slot(src)) {
                    held.insert(register(dst));
                } else if !reads_only(op) {
                    held.remove(&register(dst));
                }
            }
            for next in successors(code, &index, at) {
                changed |= join(&mut entry[next], &held);
            }
        }
        if !changed {
            break;
        }
    }
    for (at, (_, line)) in code.iter().enumerate() {
        let (op, args) = parts(line);
        let held = joined(entry[at].as_ref(), table.as_ref()).unwrap_or_default();
        if op.starts_with("lea") && args.contains("(%rsp") {
            s.stack_address = true;
        }
        if op == "mov" && args == "%rsp,%rbp" {
            s.frame_pointer = true;
        }
        if let Some(target) = args.strip_prefix('*') {
            let through = held.contains(&register(target)) || slot(target);
            match op {
                "jmp" if through => s.jumps_through_slot = true,
                "call" if through => s.calls_through_slot = true,
                _ => {}
            }
            continue;
        }
        // A call to the function's own entry is a call like any other.
        let own = code.first().map(|(a, _)| *a);
        let Some(addr) = branch_target(args).filter(|t| !index.contains_key(t) || (op == "call" && Some(*t) == own)) else { continue };
        if op == "call" {
            s.calls.insert(addr);
        } else if op.starts_with('j') {
            // A jump out of the function, conditional or not, is a tail call.
            s.jumps.insert(addr);
        }
    }
    s
}

/// CODE given as objdump prints it, one `address: instruction` a line.
fn code(lines: &[&str]) -> Code {
    lines.iter().map(|l| {
        let (addr, ins) = l.split_once(": ").expect("address: instruction");
        (u64::from_str_radix(addr, 16).expect("a hex address"), ins.trim().to_owned())
    }).collect()
}

#[test]
fn the_reading_follows_a_slot_through_copies_and_zero_displacement() {
    let slots: BTreeSet<u64> = [0, 0x30].into_iter().collect();
    let read = |lines: &[&str]| shape(&code(lines), &slots);
    assert!(read(&["0: mov    0x30(%rdx),%rax", "4: mov    %rax,%r11", "7: call   *%r11"]).calls_through_slot);
    assert!(read(&["0: mov    (%rdx),%rax", "3: test   %rax,%rax", "6: jmp    *%rax"]).jumps_through_slot);
    assert!(read(&["0: call   *(%rdx)"]).calls_through_slot);
    assert!(read(&["0: notrack jmp *0x30(%rdx)"]).jumps_through_slot);
    let overwritten = read(&["0: mov    0x30(%rdx),%rax", "4: mov    %rax,%r11", "7: mov    0x8(%rdx),%r11d", "b: call   *%r11"]);
    assert!(!overwritten.calls_through_slot, "a register written over no longer holds the slot");
    assert!(!read(&["0: mov    0x30(%rsp),%rax", "5: call   *%rax"]).calls_through_slot, "a stack slot is not a handler slot");
}

/// A slot's value reaches a dispatch along a branch that skips a write in
/// between in the listing.
#[test]
fn the_reading_follows_a_slot_along_branches() {
    let slots: BTreeSet<u64> = [0x30].into_iter().collect();
    let branchy = shape(&code(&[
        "0: mov    0x30(%rdx),%rax",
        "4: jne    c <f+0xc>",
        "6: mov    %rbx,%rax",
        "9: ret",
        "c: call   *%rax",
    ]), &slots);
    assert!(branchy.calls_through_slot);
}

/// A table jump's state joins a target that also has a direct edge, and a
/// table jump carrying nothing still reaches code before it.
#[test]
fn the_reading_joins_table_jumps() {
    let slots: BTreeSet<u64> = [0x30].into_iter().collect();
    let shared = shape(&code(&[
        "0: mov    0x30(%rdx),%rax",
        "4: test   %rdi,%rdi",
        "7: jne    10 <f+0x10>",
        "9: xor    %eax,%eax",
        "b: jmp    14 <f+0x14>",
        "10: jmp    *(%rcx,%rsi,8)",
        "14: call   *%rax",
    ]), &slots);
    assert!(shared.calls_through_slot, "the direct edge clears %rax, the table edge holds the slot");
    let backward = shape(&code(&[
        "0: jmp    8 <f+0x8>",
        "2: mov    0x30(%rdx),%rax",
        "6: call   *%rax",
        "8: jmp    *(%rcx,%rsi,8)",
    ]), &slots);
    assert!(backward.calls_through_slot, "a table jump with nothing held makes the code before it reachable");
}

/// A handler stored to a stack slot is followed back out of it; a frame
/// pointer, which the reading does not follow, is itself a break.
#[test]
fn the_reading_follows_stack_slots_and_fails_closed_on_frame_pointers() {
    let slots: BTreeSet<u64> = [0x30].into_iter().collect();
    let spilled = shape(&code(&[
        "0: mov    0x30(%rdx),%rax",
        "4: mov    %rax,0x18(%rsp)",
        "9: mov    0x18(%rsp),%rax",
        "e: call   *%rax",
    ]), &slots);
    assert!(spilled.calls_through_slot);
    assert!(shape(&code(&["0: mov    0x30(%rdx),%rax", "4: mov    %rax,0x8(%rsp)", "9: call   *0x8(%rsp)"]), &slots).calls_through_slot);
    assert!(shape(&code(&["0: push   %rbp", "1: mov    %rsp,%rbp", "4: jmp    *0x30(%rdx)"]), &slots).frame_pointer);
}

/// A call to a dispatcher is a break wherever it is, the caller dispatching
/// or not and whatever it does after: a wrapper whose sibling call was
/// lost, even with cleanup after the call. A dispatcher calling itself is
/// one too.
#[test]
fn a_call_to_a_dispatcher_is_reported() {
    let slots: BTreeSet<u64> = [0x30].into_iter().collect();
    let functions = [
        ("handler".to_owned(), 0x100, code(&["100: jmp    200 <wrapper>"])),
        ("wrapper".to_owned(), 0x200, code(&[
            "200: push   %rbx",
            "201: call   300 <next>",
            "206: mov    %rax,%rbx",
            "209: call   400 <drop>",
            "20e: mov    %rbx,%rax",
            "211: pop    %rbx",
            "212: ret",
        ])),
        ("next".to_owned(), 0x300, code(&["300: jmp    *0x30(%rdx)"])),
        ("drop".to_owned(), 0x400, code(&["400: ret"])),
        ("again".to_owned(), 0x500, code(&[
            "500: test   %rdi,%rdi",
            "503: je     50a <again+0xa>",
            "505: call   500 <again>",
            "50a: jmp    *0x30(%rdx)",
        ])),
    ];
    let broken = read_dispatch(&functions, &slots).broken;
    let breaks = |name: &str| broken.get(name).map(|b| b.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(breaks("wrapper"), Some(vec!["call next"]));
    assert_eq!(breaks("again"), Some(vec!["call again"]));
    assert!(!broken.contains_key("handler"), "a jump to the wrapper is still a jump: {broken:?}");
}

/// The dispatching functions of a binary, by name, and the breaks each
/// function shows.
struct Reading {
    dispatching: BTreeSet<String>,
    broken: BTreeMap<String, BTreeSet<String>>,
}

/// FUNCTIONS (name, address, code) read for their dispatches.
fn read_dispatch(functions: &[(String, u64, Code)], slots: &BTreeSet<u64>) -> Reading {
    let by_addr: BTreeMap<u64, &str> = functions.iter().map(|(n, a, _)| (*a, n.as_str())).collect();
    // By address: a generic handler's instances share a name.
    let shapes: BTreeMap<u64, Shape> = functions.iter().map(|(_, a, code)| (*a, shape(code, slots))).collect();
    let mut dispatching: BTreeSet<u64> =
        shapes.iter().filter(|(_, s)| s.jumps_through_slot || s.calls_through_slot).map(|(a, _)| *a).collect();
    loop {
        let more: Vec<u64> = shapes
            .iter()
            .filter(|(a, s)| !dispatching.contains(a) && s.jumps.iter().any(|t| dispatching.contains(t)))
            .map(|(a, _)| *a)
            .collect();
        if more.is_empty() {
            break;
        }
        dispatching.extend(more);
    }
    let mut broken: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (addr, s) in &shapes {
        let mut why: BTreeSet<String> = s
            .calls
            .iter()
            .filter(|t| dispatching.contains(t) && !DRIVER.contains(&by_addr[t]))
            .map(|t| format!("call {}", by_addr[t]))
            .collect();
        if dispatching.contains(addr) {
            let flags = [(s.calls_through_slot, "slot-call"), (s.stack_address, "stack"), (s.frame_pointer, "frame-pointer")];
            why.extend(flags.into_iter().filter(|(on, _)| *on).map(|(_, b)| b.to_owned()));
        }
        if !why.is_empty() {
            broken.entry(by_addr[addr].to_owned()).or_default().extend(why);
        }
    }
    Reading { dispatching: dispatching.iter().map(|a| by_addr[a].to_owned()).collect(), broken }
}

#[test]
#[cfg_attr(debug_assertions, ignore = "a debug build dispatches through its trampoline, not by jumps")]
fn every_dispatch_is_a_jump() {
    // ROMACS_DISPATCH_BINARY reads another build's test binary instead (a
    // release one from a quick build); ROMACS_DISPATCH_PROFILE then names
    // its profile, or none is assumed.
    let other = std::env::var("ROMACS_DISPATCH_BINARY").ok();
    let binary = other.clone().unwrap_or_else(|| std::env::current_exe().expect("this test's binary").display().to_string());
    let size = std::mem::size_of::<Inst>() as u64;
    let slots: BTreeSet<u64> = [std::mem::offset_of!(Inst, handler), std::mem::offset_of!(Inst, observed_handler), std::mem::offset_of!(Inst, kernel)]
        .into_iter()
        .flat_map(|o| [o as u64, o as u64 + size])
        .collect();
    let symbols = engine_symbols(&binary);
    let functions: Vec<(String, u64, Code)> = disassemble(&binary, &symbols)
        .into_iter()
        .zip(&symbols)
        .map(|(f, (name, addr, _))| {
            assert_eq!(f.addr, *addr);
            (name.clone(), f.addr, f.code)
        })
        .collect();
    let Reading { dispatching, broken } = read_dispatch(&functions, &slots);
    for required in ["ret", "global", "cons", "audited::t_mov", "traversal::car"] {
        assert!(dispatching.contains(required), "{required} was not found dispatching: the reading of the binary is broken");
    }
    let named = std::env::var("ROMACS_DISPATCH_PROFILE").ok().filter(|p| ["gate", "release"].contains(&p.as_str()));
    let profile = if other.is_some() { named.as_deref() } else { profile() };
    let report: String = broken.iter().fold(String::new(), |mut out, (n, why)| {
        let _ = writeln!(out, "  {n}: {}", why.iter().cloned().collect::<Vec<_>>().join("; "));
        out
    });
    println!("profile {profile:?}; every break in this binary:\n{report}");
    let allowed: BTreeMap<&str, BTreeSet<&str>> = ALLOWED.iter().map(|(n, b, _)| (*n, allowed_in(b, profile).collect())).collect();
    let new: Vec<String> = broken
        .iter()
        .filter_map(|(n, why)| {
            let left: Vec<&str> =
                why.iter().map(String::as_str).filter(|b| !allowed.get(n.as_str()).is_some_and(|a| a.contains(b))).collect();
            (!left.is_empty()).then(|| format!("{n}: {}", left.join("; ")))
        })
        .collect();
    assert!(
        new.is_empty(),
        "a dispatch is not a jump in {} function(s); keep frame addresses out of the handler (an Option or Result returned \
         through a pointer, a `&[x]` temporary, a closure handed out of line), call nothing that dispatches with more than \
         six arguments (a seventh goes on the stack), or name the break in ALLOWED with why:\n{}\nevery break in this binary:\n{report}",
        new.len(),
        new.join("\n"),
    );
    // Stale, in a profile the list marks: a break allowed here this binary
    // does not show.
    if profile.is_some() {
        let stale: Vec<String> = allowed
            .iter()
            .flat_map(|(n, breaks)| {
                breaks.iter().filter(|b| !broken.get(*n).is_some_and(|w| w.contains(**b))).map(move |b| format!("{n}: {b}"))
            })
            .collect();
        assert!(stale.is_empty(), "these breaks are gone in a {profile:?} build; take them out of ALLOWED (or mark them for the other profile): {stale:?}\nevery break in this binary:\n{report}");
    }
}
