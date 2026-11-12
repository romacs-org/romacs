//! Build-time partial-world producer; input is JSON strings, not an Elisp loader.
use romacs_vm::{ImageBuilder, Fixed};
use sha2::{Digest, Sha256};
use std::path::Path;
fn main() {
    if let Err(e) = run() {
        eprintln!("vm2-image: {e}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() == 3 && matches!(args[0].as_str(), "--code" | "--snapshot-code") {
        // HOST(std::fs): bin adapter that builds no worlds
        let source = std::fs::read(&args[2]).map_err(|e| e.to_string())?;
        let input: serde_json::Value =
            serde_json::from_slice(&source).map_err(|e| e.to_string())?;
        let config: [u8; 32] = Sha256::digest(&source).into();
        let mut vm = romacs_vm::Vm::new();
        let form = if args[0] == "--snapshot-code" {
            let setup = romacs_vm::Form::from_json(
                input.get("setup").ok_or("snapshot requires setup")?.clone(),
            )?;
            let setup = vm.compile(&setup)?;
            vm.execute(setup)?;
            romacs_vm::Form::from_json(
                input.get("entry").ok_or("snapshot requires entry")?.clone(),
            )?
        } else {
            romacs_vm::Form::from_json(input)?
        };
        let entry = vm.compile(&form)?;
        let key = vm.publish_code_image(Path::new(&args[1]), entry, config)?;
        println!(
            "{}",
            serde_json::json!({"key":key,"config":config.iter().map(|b|format!("{b:02x}")).collect::<String>(),"abi":"VM2CODE3"})
        );
        return Ok(());
    }
    if args.len() != 2 {
        return Err("usage: romacs-vm-image STORE INPUT.json".into());
    }
    // HOST(std::fs): bin adapter that builds no worlds
    let source = std::fs::read(&args[1]).map_err(|e| e.to_string())?;
    let strings: Vec<String> = serde_json::from_slice(&source).map_err(|e| e.to_string())?;
    if strings.is_empty() {
        return Err("partial world requires at least one string".into());
    }
    let mut builder = ImageBuilder::default();
    let mut root = Fixed::NIL;
    for text in strings.iter().rev() {
        let text = builder.string(text);
        root = builder.cons(text, root);
    }
    let key = builder.publish(Path::new(&args[0]), Sha256::digest(&source).into())?;
    println!(
        "{}",
        serde_json::json!({"key": key, "root": strings.len()-1, "strings": strings.len(), "abi": "VM2IMG02"})
    );
    Ok(())
}
