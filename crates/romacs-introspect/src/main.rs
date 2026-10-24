//! `romacs-introspect`: ask the index from a shell, one round trip, JSON out.
//!
//! ```text
//! romacs-introspect describe maphash re-search-forward
//! romacs-introspect set vm2-only            # also: core-only both divergent
//!                                           # observable untested no-span
//!                                           # special-forms all; join with ','
//! romacs-introspect rust builtin_maphash    # or Primitive::Maphash, file.rs:LINE
//! romacs-introspect coverage | provenance
//! ```
//!
//! JSON by default; `--text` renders the same JSON for a human. Exit status:
//! 0 answered, 3 not found by the index (which is not "absent"), 2 usage.

use romacs_introspect::{Implementation, Index, Lookup, Reverse, SetQuery, render_text};
use serde_json::{Value as Json, json};
use std::process::ExitCode;

const USAGE: &str = "usage: romacs-introspect [--text] describe NAME... | set Q[,Q...] | rust FN|VARIANT|FILE:LINE | coverage | provenance
  Q: all vm2-only core-only both divergent observable untested no-span special-forms";

fn parse_query(q: &str) -> Option<SetQuery> {
    Some(match q {
        "all" => SetQuery::All,
        "vm2-only" => SetQuery::Implementation(Implementation::Vm2Only),
        "core-only" => SetQuery::Implementation(Implementation::CoreOnly),
        "both" => SetQuery::Implementation(Implementation::Both),
        "divergent" => SetQuery::Divergent {
            observable_only: false,
        },
        "observable" => SetQuery::Divergent {
            observable_only: true,
        },
        "untested" => SetQuery::NoTestMentions,
        "no-span" => SetQuery::NoSourceSpan,
        "special-forms" => SetQuery::SpecialForms,
        _ => return None,
    })
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let text = if let Some(i) = args.iter().position(|a| a == "--text") {
        args.remove(i);
        true
    } else {
        false
    };
    let ix = Index::embedded();
    let (answer, found): (Json, bool) = match args.first().map(String::as_str) {
        Some("describe") if args.len() > 1 => {
            let answers: Vec<Lookup> = args[1..].iter().map(|n| ix.describe(n)).collect();
            let all = answers.iter().all(|a| matches!(a, Lookup::Found(_)));
            if text {
                for a in &answers {
                    println!("{a}");
                }
                return if all {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(3)
                };
            }
            let js: Vec<Json> = answers.iter().map(Lookup::to_json).collect();
            (
                if js.len() == 1 {
                    js.into_iter().next().unwrap()
                } else {
                    json!(js)
                },
                all,
            )
        }
        Some("set") if args.len() == 2 => {
            let Some(qs) = args[1]
                .split(',')
                .map(parse_query)
                .collect::<Option<Vec<_>>>()
            else {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            };
            (ix.query(&qs).to_json(), true)
        }
        Some("rust") if args.len() == 2 => {
            let r = ix.implements(&args[1]);
            let found = matches!(r, Reverse::Found { .. });
            (r.to_json(), found)
        }
        Some("coverage") => (
            json!({"coverage": ix.coverage().to_json(), "provenance": ix.provenance().to_json()}),
            true,
        ),
        Some("provenance") => (ix.provenance().to_json(), true),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    if text {
        print!("{}", render_text(&answer));
    } else {
        println!("{}", serde_json::to_string_pretty(&answer).unwrap());
    }
    if found {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(3)
    }
}
