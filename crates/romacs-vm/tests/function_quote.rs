use romacs_vm::Vm;
use sha2::{Digest, Sha256};
#[test]
fn function_quotes_non_lambda_objects() {
    let record: serde_json::Value = serde_json::from_str(include_str!("../evidence/function-quote/gnu.json")).unwrap();
    assert_eq!(record["oracle"]["revision"], "45b0f7699df36b3849edc3bdef37d4cb9e75b9ea");
    assert_eq!(record["recorder_sha256"], Sha256::digest(include_bytes!("../evidence/function-quote/oracle.py")).iter().map(|b| format!("{b:02x}")).collect::<String>());
    let rows = record["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    for (index, row) in rows.iter().enumerate() {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let got = vm.eval_source(row["form"].as_str().unwrap()).and_then(|v| v.display());
        assert_eq!(got, Ok(row["gnu"].as_str().unwrap().to_owned()), "FUNCTION_QUOTE_GNU_OBJECT_IDENTITY case={index}");
    }
}
