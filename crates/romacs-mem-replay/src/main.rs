use romacs_mem_replay::{
    Arm, KIND_NAMES, Verdict, accounted_median, classify, forced_matches, p99, replay, slope,
    windows,
};
use std::{
    fs::File,
    io::{BufWriter, Write},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let arm = match args.first().map(String::as_str) {
        None | Some("normal") => Arm::Natural,
        Some("forced") => Arm::Forced,
        Some("leak") => Arm::Leak,
        _ => return Err("usage: romacs-mem-replay [normal|forced|leak] [records.tsv]".into()),
    };
    if args.len() > 2 {
        return Err("too many arguments".into());
    }
    let report = replay(arm)?;
    let verdict = classify(&report);
    println!("arm={arm:?} verdict={verdict:?} scope=scaled-strong-only logical_seconds=7200");
    println!(
        "accounted_slope_mib_per_logical_hour={:.6} accounted_median_bytes={:.0}",
        slope(&report).ok_or("no slope samples")?,
        accounted_median(&report)
    );
    println!(
        "gc_count={} gc_sum_ns={} gc_max_ns={} gc_p99_ns={} gc_share_percent={:.3} wall_seconds={:.3} overshoot_max_bytes={}",
        report.stats.count,
        report.stats.sum_ns,
        report.stats.max_ns,
        p99(&report).ok_or("no GC records")?,
        report.stats.sum_ns as f64 / report.wall_ns as f64 * 100.0,
        report.wall_ns as f64 / 1e9,
        report.stats.max_overshoot
    );
    println!(
        "accounted_windows={:?} allocator_committed_windows={:?}",
        windows(&report.samples, false),
        windows(&report.samples, true)
    );
    for (name, count) in KIND_NAMES.iter().zip(report.reclaimed) {
        println!("reclaimed_{name}={count}");
    }
    println!(
        "legacy_payload_drops={} released_pages={} planted_bytes={} special_churn=deferred-S5",
        report.payload_drops, report.released_pages, report.planted_bytes
    );
    if let Some(path) = args.get(1) {
        let mut out = BufWriter::new(File::create(path)?);
        writeln!(
            out,
            "logical_seconds\tseq\ttrigger\tgc_elapsed_ns\taccounted_before\taccounted_after\tallocator_committed_bytes\tovershoot"
        )?;
        for s in &report.samples {
            let r = s.record;
            writeln!(
                out,
                "{}\t{}\t{:?}\t{}\t{}\t{}\t{}\t{}",
                s.logical_seconds,
                r.seq,
                r.trigger,
                r.gc_elapsed_ns,
                r.accounted_before,
                r.accounted_after,
                s.committed_bytes,
                r.overshoot
            )?;
        }
        out.flush()?;
    }
    if matches!(arm, Arm::Forced) {
        let normal = replay(Arm::Natural)?;
        let matches = classify(&normal) == Verdict::Pass && forced_matches(&normal, &report);
        println!("forced_natural_median_within_5_percent={matches}");
        if !matches {
            std::process::exit(1);
        }
    }
    if verdict == Verdict::Fail {
        std::process::exit(1);
    }
    Ok(())
}
