//! GNU process.c num-processors, with gnulib nproc's queries.
use super::*;

/// gnulib parse_omp_threads: a decimal count, optionally surrounded by
/// whitespace and followed by a comma-separated rest; 0 or anything else is
/// no count.
fn omp_threads(name: &str) -> Option<u64> {
    let text = crate::host::getenv(name)?.into_string().ok()?;
    let text = text.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let digits = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    let rest = text[digits..].trim_start_matches(|c: char| c.is_ascii_whitespace());
    if digits == 0 || !(rest.is_empty() || rest.starts_with(',')) { return None; }
    text[..digits].parse().ok().filter(|&n| n > 0)
}

defsubr! { name = "num-processors", gnu = "process.c:8610", effect = Reads,
fn num_processors(cx; optional query: Value) {
    let count = if query == cx.intern("all") {
        crate::host::processors(crate::host::Processors::All)
    } else if query == cx.intern("current") {
        crate::host::processors(crate::host::Processors::Current)
    } else {
        let limit = omp_threads("OMP_THREAD_LIMIT").unwrap_or(u64::MAX);
        omp_threads("OMP_NUM_THREADS").unwrap_or_else(|| crate::host::processors(crate::host::Processors::Current)).min(limit)
    };
    Value::integer(count as i64).map_err(Fault::Error)
}}
