//! `romacs-gui-oracle check SPEC CAPTURE` asserts a capture against a scene.
//! `romacs-gui-oracle read SPEC CAPTURE` prints the read-back grid as JSON.
//! `romacs-gui-oracle pair SPEC CAPTURE_A CAPTURE_B` compares two captures.
//! `romacs-gui-oracle equiv SPEC` lists characters the font draws alike.
//!
//! Exit status: 0 pass, 1 assertion failure, 2 usage or input error.

use std::path::Path;
use std::process::ExitCode;

use romacs_gui_oracle::extract::{equivalence_classes, read_grid};
use romacs_gui_oracle::glyphs::{FontModel, alphabet};
use romacs_gui_oracle::{Capture, Spec, check, pair};

fn load(spec: &str) -> Result<Spec, String> {
    let text = std::fs::read_to_string(spec).map_err(|e| format!("{spec}: {e}"))?;
    Spec::from_json(&text)
}

fn run(args: &[String]) -> Result<bool, String> {
    match args {
        [cmd, spec, cap] if cmd == "check" => {
            let spec = load(spec)?;
            let report = check(&spec, &Capture::load_png(Path::new(cap))?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
            );
            Ok(report.passed())
        }
        [cmd, spec, cap] if cmd == "read" => {
            let spec = load(spec)?;
            let model = FontModel::load(&spec.font, &alphabet([]))?;
            let origin = spec.expect.as_ref().map_or([0, 0], |e| e.origin);
            let grid = read_grid(&Capture::load_png(Path::new(cap))?, &model, origin);
            println!(
                "{}",
                serde_json::to_string(&grid).map_err(|e| e.to_string())?
            );
            Ok(true)
        }
        [cmd, spec, a, b] if cmd == "pair" => {
            let spec = load(spec)?;
            let p = pair::pair(
                &spec,
                &Capture::load_png(Path::new(a))?,
                &Capture::load_png(Path::new(b))?,
            )?;
            println!(
                "{}",
                serde_json::to_string_pretty(&p.diffs).map_err(|e| e.to_string())?
            );
            Ok(p.diffs.is_empty())
        }
        [cmd, spec] if cmd == "equiv" => {
            let spec = load(spec)?;
            let alpha = alphabet([]);
            let model = FontModel::load(&spec.font, &alpha)?;
            let classes = equivalence_classes(&model, &alpha);
            println!(
                "{}",
                serde_json::to_string(&classes).map_err(|e| e.to_string())?
            );
            Ok(true)
        }
        _ => Err("usage: romacs-gui-oracle check|read SPEC CAPTURE | pair SPEC A B".into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("romacs-gui-oracle: {e}");
            ExitCode::from(2)
        }
    }
}
