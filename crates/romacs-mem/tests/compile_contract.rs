//! Compile-contract gate. Every negative fixture is checked
//! against a freshly built copy of this crate and a positive control first.
use std::{fs, process::Command};
#[test]
fn compile_contracts() {
    let dir = std::env::temp_dir().join(format!("romacs-mem-contract-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let chars = dir.join("libromacs_chars.rlib");
    let result = Command::new("rustc")
        .args([
            "--edition=2024",
            "--crate-name=romacs_chars",
            "--crate-type=rlib",
        ])
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../romacs-chars/src/lib.rs"
        ))
        .arg("-o")
        .arg(&chars)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let lib = dir.join("libromacs_mem.rlib");
    let result = Command::new("rustc")
        .args([
            "--edition=2024",
            "--crate-name=romacs_mem",
            "--crate-type=rlib",
        ])
        .arg("--extern")
        .arg(format!("romacs_chars={}", chars.display()))
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .arg("-o")
        .arg(&lib)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    #[rustfmt::skip]
    let cases = [
        ("positive", "let v=h.fixnum(1).unwrap(); let r=Root::new(&h,v,\"x\"); h.safepoint(); assert_eq!(r.get(&h).as_fixnum(),Some(1));", ""),
        ("payload_positive", "struct P(Vec<u8>); impl Payload for P { fn bytes(&self)->usize {self.0.len()} } let _=h.make_payload(PvecType::UserPtr,&[],Box::new(P(vec![1,2]))).unwrap();", ""),
        ("foreign_positive", "let w=raw::RawValue::from_value(h.nil()); assert_eq!(unsafe {raw::RawValue::from_bits(w.bits())},w);", ""),
        ("raw_send", "let v=raw::RawValue::from_value(h.nil()); std::thread::spawn(move || drop(v));", "cannot be sent"),
        ("payload_raw_value", include_str!("payload_raw_value.rs.inc"), "cannot be sent"),
        ("payload_raw_values", "struct P(Vec<raw::RawValue>); impl Payload for P { fn bytes(&self)->usize {self.0.len()} }", "cannot be sent"),
        ("payload_value", "struct P(Value<'static>); impl Payload for P { fn bytes(&self)->usize {0} }", "cannot be sent"),
        ("payload_root", "struct P(Root); impl Payload for P { fn bytes(&self)->usize {0} }", "cannot be sent"),
        ("payload_stack", "struct P(ValueStack); impl Payload for P { fn bytes(&self)->usize {0} }", "cannot be sent"),
        ("payload_borrow", "struct P<'a>(&'a str); impl Payload for P<'_> { fn bytes(&self)->usize {0} }", "lifetime"),
        ("typed_ref_escape", "fn escape(h:&Heap)->ConsRef<'static> { h.as_cons(h.cons(h.nil(),h.nil()).unwrap()).unwrap() }", "lifetime"),
        ("static_field", "struct Long { v: Value<'static> } let _x=Long { v:h.fixnum(1).unwrap() };", "does not live long enough"),
        ("safepoint", "let v=h.fixnum(1).unwrap(); h.safepoint(); let _=v.kind();", "cannot borrow"),
        ("funcall", "fn funcall(_: &mut Heap) {} let v=h.fixnum(1).unwrap(); funcall(&mut h); let _=v.kind();", "cannot borrow"),
        ("value_send", "let v=h.fixnum(1).unwrap(); std::thread::scope(|s| { s.spawn(move || { let _=v.kind(); }); });", "cannot be sent"),
        ("root_send", "let r=Root::new(&h,sym::T,\"x\"); std::thread::spawn(move || drop(r));", "cannot be sent"),
        ("heap_send", "std::thread::spawn(move || drop(h));", "cannot be sent"),
        ("frame_send", "let mut s=ValueStack::new(&h); s.push(&h,sym::T); let f=s.frame(0,1); std::thread::spawn(move || f.len());", "cannot be sent"),
        ("root_clone", "let r=Root::new(&h,sym::T,\"x\"); let _=r.clone();", "no method named"),
        ("constructor", "let _=Heap::new();", "no associated function or constant"),
        ("value_escape", "fn escape(h:&Heap)->Value<'static> { h.fixnum(1).unwrap() } let _=escape(&h);", "lifetime may not live long enough"),
    ];
    for (name, body, error) in cases {
        let source = dir.join(format!("{name}.rs"));
        fs::write(&source,format!("use romacs_mem::*; fn main() {{ spawn_isolate(HeapConfig::default(),|mut h| {{ {body} }}).join().unwrap(); }}")).unwrap();
        let out = Command::new("rustc")
            .args(["--edition=2024", "--emit=metadata", "--extern"])
            .arg(format!("romacs_mem={}", lib.display()))
            .arg("-L")
            .arg(format!("dependency={}", dir.display()))
            .arg(&source)
            .arg("-o")
            .arg(dir.join(format!("{name}.rmeta")))
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.success(), error.is_empty(), "{name}: {stderr}");
        assert!(stderr.contains(error), "{name}: wrong diagnostic: {stderr}");
        if matches!(name, "payload_raw_value" | "payload_raw_values") {
            for expected in [
                "error[E0277]",
                "*const ()",
                "required by a bound in `romacs_mem::Payload`",
                "Send + 'static",
            ] {
                assert!(
                    stderr.contains(expected),
                    "{name}: wrong Send-bound diagnostic: {stderr}"
                );
            }
        }
    }
    fs::remove_dir_all(dir).unwrap();
}
