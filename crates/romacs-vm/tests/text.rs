use romacs_vm::{Form, Vm};

#[test]
fn scalar_mutation_d001_and_byte_strings_remain_strict() {
    let mut vm = Vm::new();
    // GNU-DIVERGENCES.md D001: widths may change in scalar strings.
    for (code, expected) in [(946, "aβc"), (955, "aλc"), (97, "aac"), (128512, "a😀c")] {
        let v = vm
            .eval_source(&format!(
                "(let ((s (copy-sequence \"aβc\"))) (aset s 1 {code}) s)"
            ))
            .unwrap();
        assert_eq!(v.display().unwrap(), format!("\"{expected}\""));
    }
    for code in [-1, 256, 955, 128512] {
        assert!(
            vm.eval_source(&format!("(aset (unibyte-string 0 1 255) 1 {code})"))
                .is_err()
        );
    }
    for code in [0, 255] {
        let v = vm
            .eval_source(&format!(
                "(let ((s (unibyte-string 0 1 255))) (aset s 1 {code}) (aref s 1))"
            ))
            .unwrap();
        assert_eq!(v.as_integer().unwrap(), code);
    }
    // GNU: (t t 3 4) for characterp/encoded widths of 55296 and 1114112.
    // D001 deliberately allows storing them despite GNU's width restriction.
    for (code, width) in [(55296, 3), (1114112, 4)] {
        let v = vm.eval_source(&format!("(let ((s (copy-sequence \"β\"))) (aset s 0 {code}) (list (aref s 0) (string-bytes s)))")).unwrap();
        assert_eq!(v.display().unwrap(), format!("({code} {width})"));
    }
    for code in [-1, 4194304] {
        assert!(
            vm.eval_source(&format!("(aset (copy-sequence \"β\") 0 {code})"))
                .is_err()
        );
    }
}

#[test]
fn replay_live_gnu_text_answers() {
    let rows: serde_json::Value =
        serde_json::from_str(include_str!("../evidence/text-known-answers.json")).unwrap();
    for row in rows.as_array().unwrap() {
        let mut vm = Vm::new();
        let form = Form::from_json(row["transport"].clone()).unwrap();
        let f = vm.compile(&form).unwrap();
        let value = vm.execute(f).unwrap();
        assert_eq!(
            value.display().unwrap(),
            row["expected"].as_str().unwrap(),
            "{}",
            row["source"]
        );
    }
}
