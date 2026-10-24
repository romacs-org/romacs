use romacs_gnudiff::{Arg, Err, Oracle, Runner};
use romacs_mem::{HeapConfig, spawn_isolate};
use romacs_obj::Obarray;
use std::{
    fs,
    path::{Path, PathBuf},
};
fn boolean(value: bool) -> Arg {
    Arg::Symbol(if value { "t" } else { "nil" }.into())
}
fn dispatch(function: &str, args: &[Arg]) -> Result<Arg, Err> {
    assert_eq!(function, "romacs-obj-probe");
    let Arg::Vector(names) = &args[0] else {
        panic!("name vector")
    };
    let names = names.clone();
    Ok(spawn_isolate(HeapConfig::default(), move |heap| {
        let ob = Obarray::new(&heap).unwrap();
        let mut out = vec![boolean(Obarray::from_value(&heap, ob.value()).is_some())];
        for name in &names {
            let Arg::String { multibyte, bytes } = name else {
                panic!("name string")
            };
            out.push(boolean(ob.intern_soft(bytes, *multibyte).is_some()));
            let s = ob.intern(bytes, *multibyte).unwrap();
            out.push(boolean(s == ob.intern(bytes, *multibyte).unwrap()));
            out.push(boolean(Some(s) == ob.intern_soft(bytes, *multibyte)));
            let name = heap.as_string(heap.as_symbol(s).unwrap().name()).unwrap();
            out.push(Arg::String {
                multibyte: name.multibyte(),
                bytes: name.bytes().to_vec(),
            });
        }
        let mut count = 0;
        ob.for_each(&mut |_| count += 1);
        out.push(Arg::Fixnum(count));
        for name in &names {
            let Arg::String { multibyte, bytes } = name else {
                panic!("name string")
            };
            let other = heap
                .make_symbol(heap.make_string(bytes, *multibyte).unwrap())
                .unwrap();
            out.push(boolean(ob.unintern(bytes, *multibyte, Some(other))));
            out.push(boolean(ob.unintern(bytes, *multibyte, None)));
            out.push(boolean(ob.intern_soft(bytes, *multibyte).is_some()));
        }
        let mut count = 0;
        ob.for_each(&mut |_| count += 1);
        out.push(Arg::Fixnum(count));
        Arg::Vector(out)
    })
    .join()
    .unwrap())
}
#[test]
fn gnu_intern_operations() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/gnudiff/obj");
    let inventory = fs::read_to_string(root.join("cases.txt")).unwrap();
    let names: Vec<_> = inventory.lines().collect();
    let runner = Runner::new(
        Oracle::pin(Path::new("/usr/bin/emacs")).unwrap(),
        PathBuf::from(std::env::var_os("HOME").unwrap()).join(".cache/romacs-gnudiff"),
    )
    .unwrap();
    let evidence = std::env::temp_dir().join(format!("obj-gnudiff-{}", std::process::id()));
    fs::create_dir_all(&evidence).unwrap();
    let files = [root.join("intern.el")];
    let result = runner.run(
        &files,
        &names,
        &mut dispatch,
        &evidence.join("correct.json"),
    );
    assert!(
        result.is_ok(),
        "{}",
        fs::read_to_string(evidence.join("correct.json")).unwrap()
    );
    assert_eq!(result.unwrap().executed, names.len());
    let warm = runner
        .run(&files, &names, &mut dispatch, &evidence.join("warm.json"))
        .unwrap();
    assert_eq!(warm.oracle_starts, 0);
    assert!(
        runner
            .run(
                &files,
                &names,
                &mut |_, _| Ok(boolean(false)),
                &evidence.join("wrong.json")
            )
            .is_err()
    );
    println!(
        "GNU cases: {}; evidence: {}",
        names.len(),
        evidence.display()
    );
}
