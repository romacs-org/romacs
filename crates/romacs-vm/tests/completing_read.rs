//! The C entrypoint delegates; GNU Lisp owns the completion engine.
use romacs_vm::Vm;
use sha2::{Digest, Sha256};

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn completing_read_pinned_gnu_delegation() {
    let record: serde_json::Value = serde_json::from_str(include_str!("../evidence/completing-read/gnu.json")).unwrap();
    assert_eq!(record["oracle"]["revision"], "45b0f7699df36b3849edc3bdef37d4cb9e75b9ea");
    assert_eq!(record["spec_sha256"], digest(include_bytes!("../evidence/completing-read/spec.json")));
    assert_eq!(record["recorder_sha256"], digest(include_bytes!("../evidence/completing-read/oracle.py")));
    let spec: serde_json::Value = serde_json::from_slice(include_bytes!("../evidence/completing-read/spec.json")).unwrap();
    let rows = record["cases"].as_array().unwrap();
    assert_eq!(rows.len(), spec.as_array().unwrap().len());
    for (row, input) in rows.iter().zip(spec.as_array().unwrap()) {
        assert_eq!(row["form"], input["form"]);
        assert_eq!(row["id"], input["id"]);
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let got = vm.eval_source(row["form"].as_str().unwrap()).and_then(|v| v.display());
        assert_eq!(got, Ok(row["gnu"].as_str().unwrap().to_owned()),
                   "COMPLETING_READ_GNU_DELEGATION case={}", row["id"]);
    }
}
