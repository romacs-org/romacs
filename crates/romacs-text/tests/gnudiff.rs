use romacs_chars::{BytePos, CharPos, str_to_multibyte};
use romacs_gnudiff::{Arg, Err, Oracle, Runner};
use romacs_text::Text;
use std::{
    fs,
    path::{Path, PathBuf},
};
fn num(a: &Arg) -> usize {
    let Arg::Fixnum(n) = a else {
        panic!("integer required")
    };
    *n as usize
}
fn integer(n: usize) -> Arg {
    Arg::Fixnum(n as i64)
}
fn truth(a: &Arg) -> bool {
    a != &Arg::Symbol("nil".into())
}
fn list(mut a: &Arg) -> Vec<&Arg> {
    let mut v = vec![];
    while let Arg::Cons(h, t) = a {
        v.push(h.as_ref());
        a = t;
    }
    assert_eq!(*a, Arg::Symbol("nil".into()));
    v
}
fn bytes(a: &Arg, mb: bool) -> Vec<u8> {
    let Arg::String { multibyte, bytes } = a else {
        panic!("string required")
    };
    if mb && !*multibyte {
        str_to_multibyte(bytes)
    } else {
        bytes.clone()
    }
}
fn dispatch(name: &str, args: &[Arg]) -> Result<Arg, Err> {
    assert_eq!(name, "gnudiff-text-run");
    let mb = truth(&args[0]);
    let mut t = Text::from_bytes(&bytes(&args[1], mb), mb);
    let tick = t.modiff();
    let ctick = t.chars_modiff();
    let mut markers = vec![];
    let mut results = vec![];
    for op in list(&args[2]) {
        let a = list(op);
        let Arg::Symbol(name) = a[0] else {
            panic!("op")
        };
        let n = num(a[1]);
        let pos = |t: &Text, n: usize| t.byte_of_char(CharPos(n - 1));
        match name.as_str() {
            "ins" | "ins-bm" => {
                t.insert(pos(&t, n), &bytes(a[2], mb), name == "ins-bm");
            }
            "del" => {
                t.delete(pos(&t, n), pos(&t, num(a[2])));
            }
            "rep" => {
                t.replace(pos(&t, n), pos(&t, num(a[2])), &bytes(a[3], mb));
            }
            "mk" => markers.push(t.marker_new(pos(&t, n), truth(a[2]))),
            "mset" => {
                t.marker_set(markers[n], pos(&t, num(a[2])));
            }
            "mtype" => {
                t.marker_set_type(markers[n], truth(a[2]));
            }
            "b2p" => results.push(integer(
                t.char_of_byte(t.floor_char_boundary(BytePos(n - 1))).0 + 1,
            )),
            "fl" => {
                let Arg::Fixnum(count) = a[2] else {
                    panic!("count")
                };
                let from = pos(&t, n);
                let bol = t.find_newline(from, BytePos(0), -1);
                let start = if bol.1 > 0 { bol.0 + 1 } else { BytePos(0) };
                let (end, found) = if *count >= 0 {
                    t.find_newline(start, BytePos(t.len_bytes()), *count as isize)
                } else {
                    let available = t.newlines_before(start);
                    let found = available.min(count.unsigned_abs() as usize);
                    let target = available - found;
                    (
                        if target == 0 {
                            BytePos(0)
                        } else {
                            t.find_newline(BytePos(0), start, target as isize).0
                        },
                        found,
                    )
                };
                let p = end;
                let mut shortage = count.unsigned_abs() as usize - found;
                if *count > 0
                    && shortage > 0
                    && end != from
                    && t.byte_at(BytePos(t.len_bytes() - 1)) != b'\n'
                {
                    shortage -= 1;
                }
                results.push(Arg::Vector(vec![
                    Arg::Fixnum(if *count < 0 {
                        -(shortage as i64)
                    } else {
                        shortage as i64
                    }),
                    integer(t.char_of_byte(p).0 + 1),
                ]));
            }
            _ => panic!("unknown op {name}"),
        }
        t.check_invariants();
    }
    let mut out = vec![];
    t.copy_to(BytePos(0), BytePos(t.len_bytes()), &mut out);
    Ok(Arg::Vector(vec![
        Arg::String {
            multibyte: false,
            bytes: out,
        },
        Arg::Vector(
            markers
                .iter()
                .map(|&m| integer(t.marker_pos(m).unwrap().char.0 + 1))
                .collect(),
        ),
        Arg::Vector(
            (0..=t.len_chars())
                .map(|c| integer(t.byte_of_char(CharPos(c)).0 + 1))
                .collect(),
        ),
        integer((t.modiff() - tick) as usize),
        integer((t.chars_modiff() - ctick) as usize),
        Arg::Vector(
            (0..=t.len_chars())
                .map(|c| integer(1 + t.newlines_before(t.byte_of_char(CharPos(c)))))
                .collect(),
        ),
        Arg::Vector(results),
    ]))
}
#[test]
fn gnu_text_scenarios() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/gnudiff/text");
    let inventory = fs::read_to_string(root.join("cases.txt")).unwrap();
    let names: Vec<_> = inventory.lines().collect();
    assert_eq!(names.len(), 13);
    let cache = PathBuf::from(std::env::var_os("HOME").unwrap()).join(".cache/romacs-gnudiff");
    let runner = Runner::new(Oracle::pin(Path::new("/usr/bin/emacs")).unwrap(), cache).unwrap();
    let evidence = std::env::temp_dir().join(format!("text-gnudiff-{}", std::process::id()));
    fs::create_dir_all(&evidence).unwrap();
    let files = [root.join("text.el")];
    let report = evidence.join("correct.json");
    let r = runner.run(&files, &names, &mut dispatch, &report);
    assert!(r.is_ok(), "{}", fs::read_to_string(&report).unwrap());
    assert_eq!(r.unwrap().executed, names.len());
    assert_eq!(
        runner
            .run(&files, &names, &mut dispatch, &evidence.join("warm.json"))
            .unwrap()
            .oracle_starts,
        0
    );
    assert!(
        runner
            .run(
                &files,
                &names,
                &mut |_, _| Ok(integer(0)),
                &evidence.join("wrong.json")
            )
            .is_err()
    );
    println!("evidence={}", evidence.display());
}
