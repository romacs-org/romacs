//! Tier P probes over Rust primitives in master. One resolution-bounded counter per
//! probe; `scripts/probe` builds this binary and runs each probe in its own
//! process. To add a probe: write a `fn() -> Result<u64, String>` below and
//! add one entry to `PROBES`.
//!
//! Counters:
//! - instructions: `instructions:u` of this thread and its new threads around
//!   the measured region, via perf_event_open (no perf binary, no startup).
//! - allocator calls: `alloc` + `alloc_zeroed` + `realloc` through the global
//!   allocator while the region runs.
//! - live objects: romacs-mem's own collection report.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicU64, Ordering};

use romacs_chars::BytePos;
use romacs_text::Text;

struct Probe {
    name: &'static str,
    unit: &'static str,
    /// Known answer, when one exists independently of the implementation.
    answer: Option<u64>,
    doc: &'static str,
    run: fn() -> Result<u64, String>,
}

const PROBES: &[Probe] = &[
    Probe {
        name: "counter-nop-1000",
        unit: "instructions:u",
        answer: Some(1000),
        doc: "1000 inline-asm nops minus an empty region: checks counter sanity within the tier instruction resolution",
        run: counter_nop_1000,
    },
    Probe {
        name: "counter-alloc-3",
        unit: "allocator calls",
        answer: Some(3),
        doc: "a Box, a Vec and a String allocated once each: proves the allocation counter is exact",
        run: counter_alloc_3,
    },
    Probe {
        name: "text-find-newline-instructions",
        unit: "instructions:u",
        answer: None,
        doc: "romacs-text: step 10000 lines with find_newline(pos, end, 1), the C-n kernel path",
        run: text_find_newline_instructions,
    },
    Probe {
        name: "text-chat-stream-allocs",
        unit: "allocator calls",
        answer: None,
        doc: "romacs-text: chat-stream-20k's 22000 inserts at an advancing marker before a prompt",
        run: text_chat_stream_allocs,
    },
    Probe {
        name: "mem-live-conses",
        unit: "live objects",
        answer: Some(1000),
        doc: "romacs-mem (heap being wired, not yet shipping): objects live after a full GC of a rooted 1000-cons list plus 1000 unreachable conses",
        run: mem_live_conses,
    },
];

// ---------------------------------------------------------------- counters

struct Counting;
static ALLOCATOR_CALLS: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATOR_CALLS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATOR_CALLS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATOR_CALLS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Allocator calls made while `f` runs.
fn count_allocations<R>(f: impl FnOnce() -> R) -> (u64, R) {
    let before = ALLOCATOR_CALLS.load(Ordering::SeqCst);
    let result = f();
    (ALLOCATOR_CALLS.load(Ordering::SeqCst) - before, result)
}

/// 128-byte Linux perf_event_attr, including inherit_thread.
#[repr(C)]
#[derive(Default)]
struct PerfEventAttr {
    kind: u32,
    size: u32,
    config: u64,
    sample_period: u64,
    sample_type: u64,
    read_format: u64,
    flags: u64,
    wakeup_events: u32,
    bp_type: u32,
    config1: u64,
    remaining: [u64; 8],
}

const PERF_TYPE_HARDWARE: u32 = 0;
const PERF_COUNT_HW_INSTRUCTIONS: u64 = 1;
const FLAG_DISABLED: u64 = 1 << 0;
const FLAG_INHERIT: u64 = 1 << 1;
const FLAG_INHERIT_THREAD: u64 = 1 << 35;
const READ_ENABLED_RUNNING: u64 = 3;
const FLAG_EXCLUDE_KERNEL: u64 = 1 << 5;
const FLAG_EXCLUDE_HV: u64 = 1 << 6;
const PERF_FLAG_FD_CLOEXEC: libc::c_ulong = 1 << 3;
const IOC_ENABLE: libc::c_ulong = 0x2400;
const IOC_DISABLE: libc::c_ulong = 0x2401;
const IOC_RESET: libc::c_ulong = 0x2403;

/// User-mode instructions retired by this thread and its new threads while `f` runs.
fn count_instructions<R>(f: impl FnOnce() -> R) -> Result<(u64, R), String> {
    let attr = PerfEventAttr {
        kind: PERF_TYPE_HARDWARE,
        size: std::mem::size_of::<PerfEventAttr>() as u32,
        config: PERF_COUNT_HW_INSTRUCTIONS,
        read_format: READ_ENABLED_RUNNING,
        flags: FLAG_DISABLED
            | FLAG_INHERIT
            | FLAG_INHERIT_THREAD
            | FLAG_EXCLUDE_KERNEL
            | FLAG_EXCLUDE_HV,
        ..Default::default()
    };
    let fd = unsafe {
        libc::syscall(
            libc::SYS_perf_event_open,
            &attr as *const PerfEventAttr,
            0 as libc::pid_t,
            -1 as libc::c_int,
            -1 as libc::c_int,
            PERF_FLAG_FD_CLOEXEC,
        )
    } as libc::c_int;
    if fd < 0 {
        return Err(format!(
            "perf_event_open(instructions:u) failed: {} (check /proc/sys/kernel/perf_event_paranoid <= 2)",
            std::io::Error::last_os_error()
        ));
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    counter_ioctl(&fd, IOC_RESET)?;
    counter_ioctl(&fd, IOC_ENABLE)?;
    let result = f();
    counter_ioctl(&fd, IOC_DISABLE)?;
    let mut reading = [0u64; 3];
    let size = std::mem::size_of_val(&reading);
    let read = unsafe { libc::read(fd.as_raw_fd(), reading.as_mut_ptr().cast(), size) };
    if read != size as isize {
        return Err(format!(
            "reading the instruction counter returned {read}, expected {size}"
        ));
    }
    validate_instructions(reading).map(|count| (count, result))
}

fn counter_ioctl(fd: &OwnedFd, request: libc::c_ulong) -> Result<(), String> {
    if unsafe { libc::ioctl(fd.as_raw_fd(), request, 0) } != 0 {
        return Err(format!(
            "instruction counter ioctl {request:#x} failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn validate_instructions([count, enabled, running]: [u64; 3]) -> Result<u64, String> {
    if count == 0 || enabled == 0 || running != enabled {
        return Err(format!("missing/multiplexed instruction counter: count={count}, enabled={enabled}, running={running}"));
    }
    Ok(count)
}

/// One guarded region count minus one guarded empty-region count; no selection or retries.
fn region_instructions<R: Default>(mut region: impl FnMut() -> R) -> Result<(u64, R), String> {
    let measure = |f: &mut dyn FnMut() -> R| count_instructions(f);
    let empty = measure(&mut || black_box(R::default()))?.0;
    let (full, result) = measure(&mut region)?;
    let count = full
        .checked_sub(empty)
        .ok_or_else(|| format!("empty region ({empty}) counted more than the probe ({full})"))?;
    Ok((count, result))
}

// ------------------------------------------------------------------ probes

fn counter_nop_1000() -> Result<u64, String> {
    region_instructions(|| unsafe {
        std::arch::asm!(
            ".rept 1000",
            "nop",
            ".endr",
            options(nomem, nostack, preserves_flags)
        )
    })
    .map(|(count, ())| count)
}

fn counter_alloc_3() -> Result<u64, String> {
    let (calls, kept) = count_allocations(|| {
        (
            black_box(Box::new(7u64)),
            black_box(Vec::<u8>::with_capacity(64)),
            black_box(String::with_capacity(8)),
        )
    });
    drop(black_box(kept));
    Ok(calls)
}

const LINES: usize = 10_000;

fn text_find_newline_instructions() -> Result<u64, String> {
    let mut text = Text::new(true);
    let mut contents = Vec::new();
    for line in 0..LINES {
        contents.extend_from_slice(
            format!("line {line} of the probe buffer, plus some words\n").as_bytes(),
        );
    }
    text.insert(BytePos(0), &contents, false);
    let end = BytePos(text.len_bytes());
    let (instructions, lines) = region_instructions(|| {
        let mut position = BytePos(0);
        let mut lines = 0usize;
        loop {
            let (next, found) = text.find_newline(black_box(position), end, 1);
            if found == 0 {
                break;
            }
            position = next;
            lines += 1;
        }
        black_box(lines)
    })?;
    if lines != LINES {
        return Err(format!("stepped {lines} lines, expected {LINES}"));
    }
    Ok(instructions)
}

const MESSAGES: usize = 2000;
const DELTAS: usize = 10;

fn text_chat_stream_allocs() -> Result<u64, String> {
    let mut pieces = Vec::with_capacity(MESSAGES * (DELTAS + 1));
    for message in 0..MESSAGES {
        pieces.push(format!("\n* assistant {message}\n").into_bytes());
        for delta in 0..DELTAS {
            pieces.push(
                format!("delta {delta} of message {message} with `code` and words. ").into_bytes(),
            );
        }
    }
    let expected: usize = 3 + pieces.iter().map(Vec::len).sum::<usize>();
    let mut text = Text::new(true);
    text.insert(BytePos(0), b"\n> ", false);
    // GNU (copy-marker 1 t): before the prompt, advancing past each insert.
    let marker = text.marker_new(BytePos(0), true);
    let (allocations, ()) = count_allocations(|| {
        for piece in &pieces {
            let at = text.marker_pos(marker).expect("live marker").byte;
            text.insert(at, black_box(piece), false);
        }
    });
    if text.len_bytes() != expected {
        return Err(format!(
            "buffer has {} bytes, expected {expected}",
            text.len_bytes()
        ));
    }
    let mut tail = Vec::new();
    text.copy_to(BytePos(expected - 3), BytePos(expected), &mut tail);
    if tail != b"\n> " {
        return Err("the prompt did not stay last".into());
    }
    Ok(allocations)
}

const CONSES: usize = 1000;

fn mem_live_conses() -> Result<u64, String> {
    romacs_mem::spawn_isolate(romacs_mem::HeapConfig::default(), |mut heap| {
        let root = {
            let mut list = heap.nil();
            for _ in 0..CONSES {
                list = heap.cons(heap.nil(), list).map_err(|e| e.to_string())?;
                heap.cons(heap.nil(), heap.nil())
                    .map_err(|e| e.to_string())?;
            }
            romacs_mem::Root::new(&heap, list, "probe")
        };
        let garbage = heap.collect(&mut ()).freed_objects;
        if garbage != CONSES {
            return Err(format!(
                "first collection freed {garbage}, expected the {CONSES} unreachable conses"
            ));
        }
        drop(root);
        // Everything still live was reachable only from the root.
        Ok(heap.collect(&mut ()).freed_objects as u64)
    })
    .join()
    .map_err(|_| "isolate panicked".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduling_guard_requires_nonzero_exactly_fully_scheduled_counts() {
        for reading in [
            [0, 1000, 1000],
            [1, 0, 0],
            [1, 1000, 0],
            [1, 1000, 998],
            [1, 1000, 999],
            [1, 1_000_000, 999_999],
            [1, 1000, 1001],
        ] {
            assert!(validate_instructions(reading).is_err(), "{reading:?}");
        }
        for reading in [[1000, 1000, 1000], [u64::MAX, u64::MAX, u64::MAX]] {
            assert_eq!(validate_instructions(reading).unwrap(), reading[0]);
        }
    }

    #[test]
    fn every_control_ioctl_reports_kernel_errors() {
        let file = std::fs::File::open("/dev/null").unwrap();
        let fd = OwnedFd::from(file);
        for request in [IOC_RESET, IOC_ENABLE, IOC_DISABLE] {
            assert!(counter_ioctl(&fd, request).unwrap_err().contains("ioctl"));
        }
    }

    #[test]
    fn region_runs_once_and_returns_its_result() {
        let mut calls = 0;
        let (_, result) = region_instructions(|| {
            calls += 1;
            black_box(42usize)
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(result, 42);
    }

    #[test]
    fn newly_spawned_worker_instructions_are_inherited() {
        let (count, ()) = count_instructions(|| {
            std::thread::spawn(|| unsafe {
                std::arch::asm!(
                    ".rept 100000",
                    "nop",
                    ".endr",
                    options(nomem, nostack, preserves_flags)
                );
            })
            .join()
            .unwrap();
        })
        .unwrap();
        assert!(count >= 100000, "worker instructions missing: {count}");
    }
}

// --------------------------------------------------------------------- cli

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--list" => {
            for p in PROBES {
                let answer = p.answer.map_or(String::new(), |a| a.to_string());
                println!("{}\t{}\t{}\t{}", p.name, p.unit, answer, p.doc);
            }
        }
        [name] => match PROBES.iter().find(|p| p.name == name) {
            Some(p) => match (p.run)() {
                Ok(value) => println!("{value}"),
                Err(error) => {
                    eprintln!("{}: {error}", p.name);
                    std::process::exit(2);
                }
            },
            None => {
                eprintln!("unknown probe: {name}");
                std::process::exit(2);
            }
        },
        _ => {
            eprintln!("usage: romacs-probe --list | romacs-probe NAME");
            std::process::exit(2);
        }
    }
}
