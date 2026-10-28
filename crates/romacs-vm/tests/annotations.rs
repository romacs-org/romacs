use romacs_vm::{Form, Fixed, Vm};

fn execute(vm: &mut Vm, form: serde_json::Value) -> romacs_vm::Local<'_> {
    let entry = vm.compile(&Form::from_json(form).unwrap()).unwrap();
    vm.execute(entry).unwrap()
}

#[test]
fn replay_live_gnu_annotation_m1_answers() {
    let rows: Vec<serde_json::Value> = include_str!("fixtures/annotation_m1.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 301);
    for (index, row) in rows.iter().enumerate() {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let result = execute(&mut vm, row["form"].clone()).root();
        vm.collect_garbage();
        assert_eq!(
            result.get(&vm).unwrap().display().unwrap(),
            row["answer"].as_str().unwrap(),
            "GNU case {index}: {}",
            row["form"]
        );
    }
}

#[test]
fn replay_live_gnu_presence_and_kill_default_answers() {
    let rows: Vec<serde_json::Value> = include_str!("fixtures/annotation_presence.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 814);
    for (index, row) in rows.iter().enumerate() {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let result = execute(&mut vm, row["form"].clone()).root();
        vm.collect_garbage();
        assert_eq!(
            result.get(&vm).unwrap().display().unwrap(),
            row["answer"].as_str().unwrap(),
            "presence GNU case {index}: {}",
            row["form"]
        );
    }
}

#[test]
fn yank_callback_gc_preserves_property_identity_and_live_parameters() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let value = execute(
        &mut vm,
        serde_json::json!(["let", [["kill-ring","nil"],["kill-ring-yank-pointer","nil"]],
      ["with-temp-buffer",["insert",{"string":"λ🙂"}],
        ["put-text-property",1,3,["quote","p"],["vector",41]],
        ["put-text-property",1,3,["quote","yank-handler"],
          ["cons",["function",["lambda",["s"],
            ["dotimes",["i",30000],["vector","i"]], ["insert","s"]]],"nil"]],
        ["kill-region",1,3],["yank"],
        ["cdr",["assq",["get-text-property",1,["quote","p"]],
          ["cons",["cons",["get-text-property",0,["quote","p"],["car","kill-ring"]],1],"nil"]]]]]),
    );
    assert_eq!(
        value,
        Fixed::integer(1).unwrap(),
        "property Value identity must survive callback compaction"
    );
    assert!(
        vm.heap_stats().collections > 0,
        "callback must actually collect"
    );
}

#[test]
fn default_yank_handlers_materialize_font_lock_face_and_category() {
    let mut vm = Vm::new();
    let value = execute(
        &mut vm,
        serde_json::json!(["let",[["kill-ring","nil"]],
      ["put",["quote","cat"],["quote","face"],["quote","bold"]],
      ["vector",
        ["with-temp-buffer",["insert",{"string":"x"}],["put-text-property",1,2,["quote","font-lock-face"],["quote","italic"]],["kill-region",1,2],["yank"],["get-text-property",1,["quote","face"]]],
        ["with-temp-buffer",["insert",{"string":"x"}],["put-text-property",1,2,["quote","category"],["quote","cat"]],["kill-region",1,2],["yank"],["get-text-property",1,["quote","face"]]]]]),
    );
    assert_eq!(value.display().unwrap(), "[italic bold]");
}

#[test]
fn handled_property_callbacks_survive_compaction_before_exclusion() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let value = execute(
        &mut vm,
        serde_json::json!(["let", [["kill-ring","nil"],
      ["yank-excluded-properties",["quote",["p"]]],
      ["yank-handled-properties",["cons",["cons",["quote","p"],["function",["lambda",["value","start","end"],
        ["dotimes",["i",30000],["vector","i"]], ["put-text-property","start","end",["quote","handled"],"value"]]]],"nil"]]],
      ["with-temp-buffer",["insert",{"string":"x"}],["put-text-property",1,2,["quote","p"],["vector",42]],
        ["kill-region",1,2],["yank"],
        ["cdr",["assq",["get-text-property",1,["quote","handled"]],
          ["cons",["cons",["get-text-property",0,["quote","p"],["car","kill-ring"]],1],"nil"]]]]]),
    );
    assert_eq!(value, Fixed::integer(1).unwrap());
    assert!(vm.heap_stats().collections > 0);
}

#[test]
fn failing_yank_handler_unwinds_and_vm_remains_usable() {
    let mut vm = Vm::new();
    let value = execute(
        &mut vm,
        serde_json::json!(["let", [["kill-ring","nil"]],
      ["with-temp-buffer",["insert",{"string":"x"}],
        ["put-text-property",1,2,["quote","yank-handler"],
          ["cons",["function",["lambda",["s"],["signal",["quote","error"],"nil"]]],"nil"]],
        ["kill-region",1,2], ["condition-case","err",["yank"],["error",7]]]]),
    );
    assert_eq!(value, Fixed::integer(7).unwrap());
    let value = execute(
        &mut vm,
        serde_json::json!(["with-temp-buffer",["insert",{"string":"ok"}],["buffer-size"]]),
    );
    assert_eq!(value, Fixed::integer(2).unwrap());
}

#[test]
fn evaporation_preserves_overlay_order_across_processes() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    for _ in 0..40 {
        let mut child = Command::new(env!("CARGO_BIN_EXE_romacs-vm"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(include_bytes!("../evidence/overlay-expiry-order.json"))
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, b"(1 0 3 7 6 5 4 2)\n");
    }
}
#[test]
fn replay_live_gnu_32_marker_overlay_answers() {
    let rows: serde_json::Value =
        serde_json::from_str(include_str!("../evidence/annotations-known-answers.json")).unwrap();
    for row in rows.as_array().unwrap() {
        let mut vm = Vm::new();
        let form = Form::from_json(row["transport"].clone()).unwrap();
        let f = vm.compile(&form).unwrap();
        let v = vm
            .execute(f)
            .unwrap_or_else(|e| panic!("{}: {e}", row["source"]));
        assert_eq!(
            v.display().unwrap(),
            row["expected"].as_str().unwrap(),
            "{}",
            row["source"]
        );
    }
}

#[test]
fn overlays_compose_with_text_resolution_and_marker_insertion() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let value = execute(
        &mut vm,
        serde_json::json!([
        "let", [["default-text-properties", ["quote", ["fallback", 99]]],
                 ["char-property-alias-alist", ["quote", [["alias", "face"]]]]],
        ["put", ["quote", "cat"], ["quote", "face"], 17],
        ["with-temp-buffer", ["insert", {"string":"abc"}],
          ["put-text-property", 1, 4, ["quote", "category"], ["quote", "cat"]],
          ["let", [["o", ["make-overlay", 1, 4]], ["m", ["copy-marker", 2]],
                    ["s", ["copy-sequence", {"string":"λ🙂"}]]],
            ["overlay-put", "o", ["quote", "face"], 42],
            ["put-text-property", 0, 2, ["quote", "face"], 23, "s"],
            ["goto-char", 2], ["insert-before-markers", "s"],
            ["vector", ["get-char-property", 1, ["quote", "face"]],
              ["get-text-property", 1, ["quote", "face"]],
              ["get-char-property", 1, ["quote", "alias"]],
              ["get-char-property", 1, ["quote", "fallback"]],
              ["get-char-property", 0, ["quote", "face"], "s"],
              ["get-text-property", 2, ["quote", "face"]],
              ["marker-position", "m"]]]]]),
    );
    // Pinned GNU 32.0.50: the overlay's `face' answers the `alias' query,
    // because Foverlay_get follows char-property-alias-alist too.
    assert_eq!(value.display().unwrap(), "[42 17 42 99 23 23 4]");
}

#[test]
fn byte_buffer_character_properties_keep_implicit_and_explicit_targets() {
    let mut vm = Vm::new();
    let value = execute(
        &mut vm,
        serde_json::json!([
            "with-temp-buffer", ["set-buffer-multibyte", "nil"],
            ["insert", {"string":"abc"}],
            ["put-text-property", 1, 3, ["quote", "face"], 17],
            ["vector", ["get-char-property", 1, ["quote", "face"]],
                ["get-char-property", 1, ["quote", "face"], "nil"],
                ["get-char-property", 1, ["quote", "face"], ["current-buffer"]]]
        ]),
    );
    assert_eq!(value.display().unwrap(), "[17 17 17]");
}
