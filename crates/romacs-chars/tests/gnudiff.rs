use romacs_chars::{str_as_multibyte, str_as_unibyte, str_to_multibyte, str_to_unibyte};
use romacs_gnudiff::{Arg, Err, Oracle, Runner};
use std::{fs, path::PathBuf};

fn list(values: Vec<Arg>) -> Arg {
    values
        .into_iter()
        .rev()
        .fold(Arg::Symbol("nil".into()), |tail, head| {
            Arg::Cons(Box::new(head), Box::new(tail))
        })
}

fn dispatch(function: &str, args: &[Arg]) -> Result<Arg, Err> {
    if function == "chars-normalization-report" {
        let Arg::Vector(inputs) = &args[0] else {
            panic!("normalization inputs must be a vector");
        };
        return Ok(Arg::Vector(
            inputs
                .iter()
                .map(|input| {
                    let Arg::String {
                        multibyte: false,
                        bytes,
                    } = input
                    else {
                        panic!("normalization input must be unibyte");
                    };
                    let (text, count) = str_as_multibyte(bytes);
                    let decoded: Vec<_> = romacs_chars::chars(&text)
                        .map(|c| Arg::Fixnum(c as i64))
                        .collect();
                    assert_eq!(romacs_chars::multibyte_chars_in_text(&text), count);
                    assert_eq!(decoded.len(), count);
                    Arg::Vector(vec![
                        Arg::Fixnum(count as i64),
                        Arg::Fixnum(text.len() as i64),
                        Arg::Vector(decoded),
                        Arg::String {
                            multibyte: false,
                            bytes: str_as_unibyte(&text),
                        },
                    ])
                })
                .collect(),
        ));
    }
    let Arg::String { multibyte, bytes } = &args[0] else {
        return Err(Err {
            symbol: "wrong-type-argument".into(),
            data: list(vec![Arg::Symbol("stringp".into()), args[0].clone()]),
        });
    };
    let string = |multibyte, bytes| Ok(Arg::String { multibyte, bytes });
    match function {
        "string-bytes" => Ok(Arg::Fixnum(bytes.len() as i64)),
        "string-to-multibyte" if !multibyte => {
            let out = str_to_multibyte(bytes);
            string(true, out)
        }
        "string-as-multibyte" if !multibyte => string(true, str_as_multibyte(bytes).0),
        "string-as-unibyte" if *multibyte => string(false, str_as_unibyte(bytes)),
        "string-to-unibyte" if *multibyte => match str_to_unibyte(bytes) {
            Ok(out) => string(false, out),
            Err(index) => Err(Err {
                symbol: "error".into(),
                data: list(vec![Arg::String {
                    multibyte: false,
                    bytes: format!("Cannot convert character at index {index} to unibyte")
                        .into_bytes(),
                }]),
            }),
        },
        "string-to-multibyte"
        | "string-as-multibyte"
        | "string-as-unibyte"
        | "string-to-unibyte" => Ok(args[0].clone()),
        _ => panic!("unknown case function: {function}"),
    }
}

#[test]
fn gnu_string_conversions() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/gnudiff/chars");
    let mut files: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "el"))
        .collect();
    files.sort();
    let inventory = fs::read_to_string(root.join("cases.txt")).unwrap();
    let names: Vec<_> = inventory.lines().filter(|l| !l.is_empty()).collect();
    let cache = PathBuf::from(std::env::var_os("HOME").unwrap()).join(".cache/romacs-gnudiff");
    let runner = Runner::new(
        Oracle::pin(std::path::Path::new("/usr/bin/emacs")).unwrap(),
        cache,
    )
    .unwrap();
    let evidence = std::env::temp_dir().join(format!("chars-gnudiff-{}", std::process::id()));
    fs::create_dir_all(&evidence).unwrap();
    let result = runner.run(
        &files,
        &names,
        &mut dispatch,
        &evidence.join("correct.json"),
    );
    if result.is_err() {
        panic!(
            "{}",
            fs::read_to_string(evidence.join("correct.json")).unwrap()
        );
    }
    let cold = result.unwrap();
    assert_eq!(cold.executed, names.len());
    let warm = runner
        .run(&files, &names, &mut dispatch, &evidence.join("warm.json"))
        .unwrap();
    assert_eq!(warm.oracle_starts, 0);
    let mut wrong = |_: &str, _: &[Arg]| Ok(Arg::Fixnum(0));
    assert!(
        runner
            .run(&files, &names, &mut wrong, &evidence.join("wrong.json"))
            .is_err()
    );
    println!(
        "GNU cases: {}; evidence: {}",
        names.len(),
        evidence.display()
    );
}
