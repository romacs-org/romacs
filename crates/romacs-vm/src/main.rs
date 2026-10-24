use romacs_vm::{Fixed, Form, Rooted, Vm};
use std::io::{BufRead, Read, Write};

fn main() {
    if let Err(e) = main_result() {
        eprintln!("vm2: {e}");
        std::process::exit(1);
    }
}
fn main_result() -> Result<(), String> {
    let mut args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--code-image") {
        if args.len() != 4 {
            return Err("--code-image STORE HASH CONFIG_HASH requires three arguments".into());
        }
        let text = &args[3];
        if text.len() != 64
            || !text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("config hash must be 64 lowercase hexadecimal digits".into());
        }
        let mut config = [0u8; 32];
        for (i, b) in config.iter_mut().enumerate() {
            *b = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).map_err(|e| e.to_string())?;
        }
        // SAFETY: same immutable-inode store contract as --image. No source fallback.
        let image =
            unsafe { romacs_vm::ImageRegion::open(std::path::Path::new(&args[1]), &args[2]) }?;
        let (mut vm, entry) = Vm::from_code_image(image.clone(), config)?;
        let before = image.verified_pages();
        let result = vm.execute(entry)?;
        println!("{}", result.display()?);
        eprintln!(
            "{}",
            serde_json::json!({"mapped_image":args[2],"scope":"vm2-executable-world","startup_source_loads":0,"startup_compilations":0,"mapped_bytes":image.mapped_bytes(),"metadata_pages":before,"verified_pages":image.verified_pages(),"materialized_functions":vm.image_functions_materialized()})
        );
        return Ok(());
    }
    let mut vm = Vm::new();
    if args.first().is_some_and(|arg| arg == "--image") {
        if args.len() < 4 {
            return Err("--image STORE HASH ROOT_INDEX requires three arguments".into());
        }
        let root: usize = args[3].parse().map_err(|_| "invalid image root index")?;
        // SAFETY: the content store is an immutable-inode deployment contract.
        // Producer never overwrites a published inode. External writers are forbidden.
        let image =
            unsafe { romacs_vm::ImageRegion::open(std::path::Path::new(&args[1]), &args[2]) }?;
        let world = image.reference(root)?;
        let bytes = image.mapped_bytes();
        vm.attach_image(image)?;
        vm.declare_special("world", world);
        let startup = vm.heap_stats();
        eprintln!(
            "{}",
            serde_json::json!({"mapped_image": args[2], "mapped_bytes": bytes,
            "startup_collections": startup.collections, "startup_private_objects": startup.live_objects,
            "startup_live_bytes": startup.live_bytes, "startup_source_loads": 0,
            "scope": "vm2-partial-data-world"})
        );
        args.drain(..4);
    }
    let session_stats = args.iter().any(|arg| arg == "--session-stats");
    args.retain(|arg| arg != "--session-stats");
    let stats = args.iter().any(|arg| arg == "--heap-stats");
    args.retain(|arg| arg != "--heap-stats");
    let dump = args.iter().any(|arg| arg == "--dump-ir");
    let stream = args.iter().any(|arg| arg == "--eval-stream");
    let source = args.iter().any(|arg| arg == "--source");
    args.retain(|arg| arg != "--dump-ir" && arg != "--eval-stream" && arg != "--source");
    if source && stream {
        return Err(
            "--source and --eval-stream are exclusive; stream clients send {source: TEXT}".into(),
        );
    }
    if stream && dump {
        return Err("--eval-stream and --dump-ir are exclusive".into());
    }
    for pair in args.chunks(2) {
        if pair.len() == 2 && pair[0] == "--cache-store" {
            vm.set_source_cache_store(std::path::PathBuf::from(&pair[1]))?;
            continue;
        }
        if pair.len() == 2 && pair[0] == "--gc-floor" {
            vm.set_collection_floor(pair[1].parse().map_err(|_| "invalid GC floor")?);
            continue;
        }
        if pair.len() != 2 || pair[0] != "--special" {
            return Err(
                "usage: romacs-vm [--source | --eval-stream | --dump-ir] [--heap-stats] [--session-stats] [--gc-floor OBJECTS] [--special NAME=INTEGER]... < already-read-form.json".into(),
            );
        }
        let (name, value) = pair[1]
            .split_once('=')
            .ok_or("special requires NAME=INTEGER")?;
        vm.declare_special(
            name,
            Fixed::integer(value.parse().map_err(|_| "invalid special integer")?)?,
        );
    }
    if stream {
        let mut output = std::io::BufWriter::new(std::io::stdout().lock());
        for input in std::io::stdin().lock().lines() {
            let input = input.map_err(|e| e.to_string())?;
            let result = serde_json::from_str::<serde_json::Value>(&input)
                .map_err(|e| e.to_string())
                .and_then(|request| {
                    if request.is_object()
                        && request.get("string").is_none()
                        && request.get("vector").is_none()
                    {
                        vm.submit(&request)
                    } else {
                        evaluate(&mut vm, &input, false, stats)
                            .map(|value| serde_json::json!({"value":value}))
                    }
                });
            let response = match result {
                Ok(value) => value,
                Err(error) => serde_json::json!({"error": error}),
            };
            writeln!(output, "{response}").map_err(|e| e.to_string())?;
            // A control client must receive this reply before sending another
            // request; no EOF-dependent buffering or batch-only fast path.
            output.flush().map_err(|e| e.to_string())?;
        }
        if session_stats {
            let s = vm.heap_stats();
            eprintln!(
                "{}",
                serde_json::json!({"collections":s.collections,"allocations":s.allocations,"reclaimed":s.reclaimed,"live_objects":s.live_objects})
            );
        }
        return Ok(());
    }
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| e.to_string())?;
    let output = if source && dump {
        let function = vm.compile_source(&input)?;
        vm.dump_ir(function)?
    } else if source {
        let value = vm.eval_source(&input)?.root();
        finish(&mut vm, value, stats)?
    } else {
        evaluate(&mut vm, &input, dump, stats)?
    };
    if dump {
        print!("{output}");
    } else {
        println!("{output}");
    }
    Ok(())
}
fn evaluate(vm: &mut Vm, input: &str, dump: bool, stats: bool) -> Result<String, String> {
    let request: serde_json::Value = serde_json::from_str(input).map_err(|e| e.to_string())?;
    if let Some(source) = request.as_object().and_then(|o| o.get("source")) {
        let source = source.as_str().ok_or("source must be a string")?;
        if dump {
            let function = vm.compile_source(source)?;
            return vm.dump_ir(function);
        }
        let value = vm.eval_source(source)?.root();
        return finish(vm, value, stats);
    }
    let form = Form::from_json(request)?;
    if dump {
        let function = vm.compile(&form)?;
        return vm.dump_ir(function);
    }
    let value = vm.eval_form(&form)?.root();
    finish(vm, value, stats)
}
fn finish(vm: &mut Vm, value: Rooted, stats: bool) -> Result<String, String> {
    let output = value.get(vm)?.display()?;
    if stats {
        let allocated = vm.heap_stats();
        // The rooted result stays live through this collection, as before.
        vm.collect_garbage();
        let live = vm.heap_stats();
        eprintln!(
            "{{\"allocations\":{},\"collections_during_run\":{},\"reclaimed\":{},\"live_objects\":{},\"live_bytes\":{},\"capacity_bytes\":{}}}",
            allocated.allocations,
            allocated.collections,
            live.reclaimed,
            live.live_objects,
            live.live_bytes,
            live.capacity_bytes
        );
    }
    Ok(output)
}
