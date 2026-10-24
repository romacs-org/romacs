use romacs_gnudiff::{Arg, Err, Oracle, Runner};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn destructive_input_wrong_dispatch_is_red() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("gnudiff-mutation-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("mutation.el");
    fs::write(
        &file,
        "(case aset-check aset [1 2] 0 9)\n(case setcar-check setcar '(1 2) 9)\n",
    )
    .unwrap();
    let runner = Runner::new(
        Oracle::pin(std::path::Path::new("/usr/bin/emacs")).unwrap(),
        PathBuf::from(std::env::var_os("HOME").unwrap()).join(".cache/romacs-gnudiff-mutation"),
    )
    .unwrap();
    let names = ["aset-check", "setcar-check"];
    let mut wrong = |_: &str, args: &[Arg]| {
        Ok(match &args[0] {
            Arg::Vector(items) => items[0].clone(),
            Arg::Cons(head, _) => *head.clone(),
            _ => panic!("fixture"),
        })
    };
    assert!(
        runner
            .run(
                std::slice::from_ref(&file),
                &names,
                &mut wrong,
                &dir.join("wrong.json")
            )
            .is_err()
    );
    let mut correct = |function: &str, args: &[Arg]| {
        match &args[0] {
            Arg::Vector(items) => assert_eq!(items[0], Arg::Fixnum(1)),
            Arg::Cons(head, _) => assert_eq!(**head, Arg::Fixnum(1)),
            _ => panic!("fixture"),
        }
        Ok(args[if function == "aset" { 2 } else { 1 }].clone())
    };
    assert!(
        runner
            .run(&[file], &names, &mut correct, &dir.join("correct.json"))
            .unwrap()
            .green()
    );
}

#[test]
fn corrupt_or_omitted_cache_records_are_red() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("gnudiff-corrupt-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("cases.el");
    fs::write(&file, "(case one identity 1)\n(case two identity 2)\n").unwrap();
    let cache =
        PathBuf::from(std::env::var_os("HOME").unwrap()).join(".cache/romacs-gnudiff-corruption");
    let oracle = Oracle::pin(std::path::Path::new("/usr/bin/emacs")).unwrap();
    let runner = Runner::new(oracle.clone(), cache.clone()).unwrap();
    let names = ["one", "two"];
    runner
        .run(
            std::slice::from_ref(&file),
            &names,
            &mut dispatch,
            &dir.join("cold.json"),
        )
        .unwrap();
    fn records(path: &std::path::Path, found: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                records(&path, found);
            } else {
                found.push(path);
            }
        }
    }
    let mut found = Vec::new();
    records(&cache, &mut found);
    assert!(!found.is_empty());
    // Copy the persistent seed: corrupt only owned per-run records. Every
    // namespace is copied, so a codec/driver change needs no test hardcoding.
    let private_cache = dir.join("private-cache");
    let mut private_records = Vec::new();
    for path in found {
        let copy = private_cache.join(path.strip_prefix(&cache).unwrap());
        fs::create_dir_all(copy.parent().unwrap()).unwrap();
        fs::copy(path, &copy).unwrap();
        private_records.push(copy);
    }
    let runner = Runner::new(oracle.clone(), private_cache).unwrap();
    let originals: Vec<_> = private_records
        .iter()
        .map(|p| fs::read_to_string(p).unwrap())
        .collect();
    let cached = &private_records[0];
    let original = fs::read_to_string(cached).unwrap();
    let lines: Vec<_> = original.lines().collect();
    for (label, text) in [
        ("truncated", original[..original.len() - 4].to_owned()),
        ("omitted", format!("{}\n{}\n", lines[0], lines[1])),
        ("shape", "gnudiff-1\ncase\tbad\n".into()),
        ("version", original.replacen("gnudiff-1", "gnudiff-0", 1)),
    ] {
        for path in &private_records {
            fs::write(path, &text).unwrap();
        }
        let receipt = dir.join(format!("{label}.json"));
        assert!(
            runner
                .run(std::slice::from_ref(&file), &names, &mut dispatch, &receipt)
                .is_err()
        );
        let evidence = fs::read_to_string(receipt).unwrap();
        assert!(evidence.contains("\"verdict\":\"RED\""));
        assert!(evidence.contains(&oracle.sha256));
        assert!(evidence.contains("/usr/bin/emacs"));
    }
    for (path, text) in private_records.iter().zip(originals) {
        fs::write(path, text).unwrap();
    }
    let warm = runner
        .run(&[file], &names, &mut dispatch, &dir.join("warm.json"))
        .unwrap();
    assert_eq!(warm.oracle_starts, 0);
}

fn list(values: Vec<Arg>) -> Arg {
    values
        .into_iter()
        .rev()
        .fold(Arg::Symbol("nil".into()), |tail, head| {
            Arg::Cons(Box::new(head), Box::new(tail))
        })
}

fn dispatch(function: &str, args: &[Arg]) -> Result<Arg, Err> {
    match function {
        "+" => {
            let mut sum = 0i64;
            let mut float = 0.0;
            let mut has_float = false;
            for arg in args {
                match arg {
                    Arg::Fixnum(n) => {
                        sum += n;
                        float += *n as f64;
                    }
                    Arg::Float(bits) => {
                        has_float = true;
                        float += f64::from_bits(*bits);
                    }
                    other => {
                        return Err(Err {
                            symbol: "wrong-type-argument".into(),
                            data: list(vec![
                                Arg::Symbol("number-or-marker-p".into()),
                                other.clone(),
                            ]),
                        });
                    }
                }
            }
            Ok(if has_float {
                Arg::Float(float.to_bits())
            } else {
                Arg::Fixnum(sum)
            })
        }
        "identity" => Ok(args[0].clone()),
        "string-bytes" => {
            let Arg::String { bytes, .. } = &args[0] else {
                panic!("fixture contract")
            };
            Ok(Arg::Fixnum(bytes.len() as i64))
        }
        "concat" => {
            let mut multibyte = false;
            let mut bytes = Vec::new();
            for arg in args {
                let Arg::String {
                    multibyte: m,
                    bytes: b,
                } = arg
                else {
                    panic!("fixture contract")
                };
                multibyte |= *m;
                bytes.extend(b);
            }
            Ok(Arg::String { multibyte, bytes })
        }
        _ => panic!("unknown selftest primitive"),
    }
}

#[test]
fn primitive_oracle_and_negative_controls() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let evidence = std::env::temp_dir().join(format!("gnudiff-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&evidence).unwrap();
    let area = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/gnudiff/selftest");
    let files: Vec<_> = fs::read_dir(&area)
        .unwrap()
        .map(|f| f.unwrap().path())
        .filter(|f| f.extension().is_some_and(|x| x == "el"))
        .collect();
    // Adding case data and its inventory never recompiles this test binary.
    let inventory = fs::read_to_string(area.join("cases.txt")).unwrap();
    let names: Vec<_> = inventory.lines().collect();
    let cache = PathBuf::from(std::env::var_os("HOME").unwrap()).join(".cache/romacs-gnudiff");
    let oracle = Oracle::pin(std::path::Path::new("/usr/bin/emacs")).unwrap();
    let runner = Runner::new(oracle.clone(), cache.clone()).unwrap();
    let cold = runner
        .run(&files, &names, &mut dispatch, &evidence.join("cold.json"))
        .unwrap();
    assert_eq!((cold.executed, cold.skipped), (names.len(), 0));
    assert_eq!(cold.cache_hits + cold.oracle_starts, files.len());
    let warm = runner
        .run(&files, &names, &mut dispatch, &evidence.join("warm.json"))
        .unwrap();
    assert_eq!((warm.cache_hits, warm.oracle_starts), (files.len(), 0));
    // These negative controls start no GNU even on warm tier-zero runs.
    let missing = Oracle {
        binary: evidence.join("no-emacs"),
        ..oracle
    };
    let cached_only = Runner::new(missing.clone(), cache.clone()).unwrap();
    assert!(
        cached_only
            .run(
                &files,
                &names,
                &mut dispatch,
                &evidence.join("cache-only.json")
            )
            .unwrap()
            .green()
    );
    let uncached = Runner::new(missing, evidence.join("empty-cache")).unwrap();
    assert!(
        uncached
            .run(
                &files,
                &names,
                &mut dispatch,
                &evidence.join("missing.json")
            )
            .is_err()
    );
    let wrong_sha = Oracle {
        binary: evidence.join("no-emacs"),
        sha256: "0".repeat(64),
    };
    let changed = Runner::new(wrong_sha, cache).unwrap();
    assert!(
        changed
            .run(
                &files,
                &names,
                &mut dispatch,
                &evidence.join("different-sha.json")
            )
            .is_err()
    );
    let mut wrong = |_: &str, _: &[Arg]| Ok(Arg::Fixnum(-999));
    assert!(
        runner
            .run(
                &files,
                &names,
                &mut wrong,
                &evidence.join("wrong-result.json")
            )
            .is_err()
    );
    assert!(
        runner
            .run(
                &files[..files.len() - 1],
                &names,
                &mut dispatch,
                &evidence.join("deleted-file.json")
            )
            .is_err()
    );
    assert!(
        runner
            .run(&[], &[], &mut dispatch, &evidence.join("empty.json"))
            .is_err()
    );
    println!("GNU differential evidence: {}", evidence.display());
}
