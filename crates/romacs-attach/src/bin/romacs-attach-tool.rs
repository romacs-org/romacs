//! romacs-attach-tool prepare SRC --into DIR [--max-edge N] [--max-bytes M]
//!                            [--formats png,jpeg,gif,webp] [--still] [--strip] [--text]
//!                            [--max-image-size W,H] [--memory-limit BYTES]
//! romacs-attach-tool blobs FILE --into STORE      (reduced lines on stdout)
//! romacs-attach-tool sweep ROOT --budget N --max-files C
//!
//! One JSON object on stdout; on failure {"error": MESSAGE} and exit 1.
use romacs_attach::{blobs, prepare, sweep};
use serde_json::json;
use std::io::{BufRead, Write};
use std::path::PathBuf;

/// The value after FLAG in ARGS.
fn value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn number<T: std::str::FromStr>(args: &[String], flag: &str) -> Result<Option<T>, String> {
    value(args, flag).map(|v| v.parse().map_err(|_| format!("{flag}: not a number: {v}"))).transpose()
}

/// Limit this process's data segment to BYTES, so a hostile image fails
/// here rather than in the session.
fn limit_memory(bytes: u64) {
    let limit = libc::rlimit { rlim_cur: bytes, rlim_max: bytes };
    // SAFETY: setrlimit reads the struct it is given.
    unsafe { libc::setrlimit(libc::RLIMIT_DATA, &limit) };
}

fn run(args: &[String]) -> Result<serde_json::Value, String> {
    let into = || value(args, "--into").map(PathBuf::from).ok_or("--into DIR is required");
    match args.first().map(String::as_str) {
        Some("prepare") => {
            limit_memory(number(args, "--memory-limit")?.unwrap_or(2 << 30));
            let src = PathBuf::from(args.get(1).ok_or("prepare SRC")?);
            let mut options = prepare::Options {
                max_edge: number(args, "--max-edge")?,
                max_bytes: number(args, "--max-bytes")?,
                still: args.iter().any(|a| a == "--still"),
                strip: args.iter().any(|a| a == "--strip"),
                text: args.iter().any(|a| a == "--text"),
                ..Default::default()
            };
            if let Some(formats) = value(args, "--formats") {
                options.formats = formats.split(',').map(String::from).collect();
            }
            if let Some(size) = value(args, "--max-image-size") {
                let (w, h) = size.split_once(',').ok_or("--max-image-size W,H")?;
                options.max_image_size = Some((w.parse().map_err(|_| "--max-image-size W,H")?, h.parse().map_err(|_| "--max-image-size W,H")?));
            }
            let p = prepare::prepare(&src, &into()?, &options)?;
            Ok(json!({"path": p.stored.path, "sha256": p.stored.sha256, "mime": p.mime, "bytes": p.stored.bytes,
                      "width": p.size.map(|s| s.0), "height": p.size.map(|s| s.1), "utf8": p.utf8}))
        }
        Some("blobs") => {
            let file = std::fs::File::open(args.get(1).ok_or("blobs FILE")?).map_err(|e| e.to_string())?;
            let store = into()?;
            let mut out = std::io::BufWriter::new(std::io::stdout().lock());
            for line in std::io::BufReader::new(file).lines() {
                let line = line.map_err(|e| e.to_string())?;
                writeln!(out, "{}", blobs::reduce_line(&line, &store).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            }
            out.flush().map_err(|e| e.to_string())?;
            Ok(serde_json::Value::Null)
        }
        Some("sweep") => {
            let root = PathBuf::from(args.get(1).ok_or("sweep ROOT")?);
            let budget = number(args, "--budget")?.ok_or("--budget N")?;
            let files = number(args, "--max-files")?.ok_or("--max-files C")?;
            let totals = sweep::sweep(&root, budget, files).map_err(|e| e.to_string())?;
            Ok(json!({"bytes": totals.bytes, "files": totals.files}))
        }
        _ => Err("usage: romacs-attach-tool prepare|blobs|sweep ...".into()),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(serde_json::Value::Null) => {}
        Ok(result) => println!("{result}"),
        Err(error) => {
            println!("{}", json!({"error": error}));
            std::process::exit(1);
        }
    }
}
