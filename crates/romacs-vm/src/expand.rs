//! Built-in surface forms lowered entirely in Rust, before IR emission.
//! Fresh temporaries are excluded from both source symbols and lexical bindings.
use crate::{Form, Result};
use std::sync::atomic::{AtomicU64, Ordering};

// A process-wide identity never reuses an ID, including across VMs.
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

fn s(name: &str) -> Form {
    Form::Sym(name.into())
}
fn l(items: Vec<Form>) -> Form {
    Form::List(items)
}
fn call(name: &str, args: Vec<Form>) -> Form {
    let mut items = vec![s(name)];
    items.extend(args);
    l(items)
}
fn progn(forms: &[Form]) -> Form {
    call("progn", forms.to_vec())
}
fn binding(name: Form, value: Form, body: Form) -> Form {
    call("let", vec![l(vec![l(vec![name, value])]), body])
}
struct Fresh;
pub(super) fn generated_id() -> crate::GeneratedName {
    crate::GeneratedName(
        NEXT_TEMP
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("generated symbol identity exhausted"),
    )
}
impl Fresh {
    fn symbol(&mut self) -> Form {
        Form::Generated(generated_id())
    }
}

struct Place {
    bindings: Vec<Form>,
    read: Form,
    store: Form,
    value: Form,
}
fn place(f: &Form, fresh: &mut Fresh) -> Result<Place> {
    let value = fresh.symbol();
    if matches!(f, Form::Sym(_) | Form::Generated(_)) {
        return Ok(Place {
            bindings: vec![],
            read: f.clone(),
            store: call("setq", vec![f.clone(), value.clone()]),
            value,
        });
    }
    let a = f.list()?;
    let head = a.first().ok_or("empty place")?.sym()?;
    if head == "gethash" && (3..=4).contains(&a.len()) {
        // GNU's gv setter: (puthash KEY VALUE TABLE); DEFAULT only reads.
        let mut bindings = vec![];
        let mut temps = vec![];
        for arg in &a[1..] {
            let temp = fresh.symbol();
            bindings.push(l(vec![temp.clone(), arg.clone()]));
            temps.push(temp);
        }
        return Ok(Place {
            bindings,
            read: call(head, temps.clone()),
            store: call(
                "puthash",
                vec![temps[0].clone(), value.clone(), temps[1].clone()],
            ),
            value,
        });
    }
    let setter = match (head, a.len()) {
        ("car", 2) => "setcar",
        ("cdr", 2) => "setcdr",
        ("aref", 3) => "aset",
        _ => return Err("unsupported generalized place".into()),
    };
    let mut bindings = vec![];
    let mut temps = vec![];
    for arg in &a[1..] {
        let temp = fresh.symbol();
        bindings.push(l(vec![temp.clone(), arg.clone()]));
        temps.push(temp);
    }
    let read = call(head, temps.clone());
    temps.push(value.clone());
    Ok(Place {
        bindings,
        read,
        store: call(setter, temps),
        value,
    })
}
fn store(p: Place, value: Form) -> Form {
    let mut bindings = p.bindings;
    bindings.push(l(vec![p.value, value]));
    call("let*", vec![l(bindings), p.store])
}

fn quote(form: Form) -> Form {
    call("quote", vec![form])
}

fn backquote(form: &Form, depth: usize, fresh: &mut Fresh) -> Result<Form> {
    if let Form::List(items) = form {
        if let Some(Form::Sym(head)) = items.first()
            && matches!(head.as_str(), "`" | "," | ",@")
            && items.len() == 2
        {
            if head == "," && depth == 1 {
                return Ok(items[1].clone());
            }
            if head == ",@" && depth == 1 {
                return Err("unquote-splicing outside list or vector".into());
            }
            let nested = if head == "`" { depth + 1 } else { depth - 1 };
            return Ok(call(
                "list",
                vec![quote(s(head)), backquote(&items[1], nested, fresh)?],
            ));
        }
        return backquote_list(items, &s("nil"), depth, fresh);
    }
    match form {
        Form::Dotted(items, tail) => backquote_list(items, tail, depth, fresh),
        Form::Vector(items) => Ok(call(
            "apply",
            vec![
                quote(s("vector")),
                backquote_list(items, &s("nil"), depth, fresh)?,
            ],
        )),
        _ => Ok(quote(form.clone())),
    }
}

fn backquote_list(items: &[Form], tail: &Form, depth: usize, fresh: &mut Fresh) -> Result<Form> {
    let Some((first, rest)) = items.split_first() else {
        return backquote(tail, depth, fresh);
    };
    // Cdrs are Lisp data too: (a . ,x) has the same structure as (a , x).
    if matches!(first, Form::Sym(h) if matches!(h.as_str(), "`" | "," | ",@"))
        && items.len() == 2
        && matches!(tail, Form::Sym(h) if h == "nil")
    {
        return backquote(&l(items.to_vec()), depth, fresh);
    }
    let end = backquote_list(rest, tail, depth, fresh)?;
    if depth == 1
        && let Form::List(splice) = first
        && splice.len() == 2
        && matches!(&splice[0], Form::Sym(h) if h == ",@")
    {
        if rest.is_empty() && matches!(tail, Form::Sym(h) if h == "nil") {
            return Ok(splice[1].clone());
        }
        let seq = fresh.symbol();
        let result = fresh.symbol();
        let rev = fresh.symbol();
        return Ok(call(
            "let*",
            vec![
                l(vec![
                    l(vec![seq.clone(), splice[1].clone()]),
                    l(vec![result.clone(), end]),
                    l(vec![rev.clone(), s("nil")]),
                ]),
                call(
                    "while",
                    vec![
                        seq.clone(),
                        call(
                            "setq",
                            vec![
                                rev.clone(),
                                call("cons", vec![call("car", vec![seq.clone()]), rev.clone()]),
                                seq.clone(),
                                call("cdr", vec![seq]),
                            ],
                        ),
                    ],
                ),
                call(
                    "while",
                    vec![
                        rev.clone(),
                        call(
                            "setq",
                            vec![
                                result.clone(),
                                call("cons", vec![call("car", vec![rev.clone()]), result.clone()]),
                                rev.clone(),
                                call("cdr", vec![rev]),
                            ],
                        ),
                    ],
                ),
                result,
            ],
        ));
    }
    Ok(call("cons", vec![backquote(first, depth, fresh)?, end]))
}

pub(crate) fn expand(f: &Form) -> Result<Option<Form>> {
    let Form::List(a) = f else { return Ok(None) };
    let Some(Form::Sym(head)) = a.first() else {
        return Ok(None);
    };
    let args = &a[1..];
    let mut fresh = Fresh;
    let expanded = match head.as_str() {
        "defface" if args.len() >= 3 => call(
            "#:vm2-defface",
            vec![quote(args[0].clone()), args[1].clone()],
        ),
        "with-current-buffer" if !args.is_empty() => binding(
            s("#:vm2-current-buffer"),
            call("current-buffer", vec![]),
            call("progn", {
                let mut body = vec![call("set-buffer", vec![args[0].clone()])];
                body.extend_from_slice(&args[1..]);
                body
            }),
        ),
        // GNU editfns.c:843 `save-current-buffer' (a special form): run
        // BODY, then make the buffer that was current, current again. VM2's
        // current buffer is the special slot `with-current-buffer' binds
        // above, so this is the same binding without the `set-buffer'.
        // Reached by GNU window.el (`with-current-buffer' in split-window-below).
        "save-current-buffer" => binding(
            s("#:vm2-current-buffer"),
            call("current-buffer", vec![]),
            progn(args),
        ),
        "declare" => s("nil"),
        "`" if args.len() == 1 => backquote(&args[0], 1, &mut fresh)?,
        "with-temp-buffer" => {
            let temp = fresh.symbol();
            binding(temp.clone(), call("#:vm2-temp-buffer", vec![]),
                call("unwind-protect", vec![
                    binding(s("#:vm2-current-buffer"), temp.clone(),
                        binding(s("font-lock-defaults"), s("nil"), progn(args))),
                    call("#:vm2-temp-buffer", vec![temp]),
                ]))
        }
        "when" | "unless" => {
            if args.is_empty() {
                return Err(format!("{head} needs a condition"));
            }
            let body = progn(&args[1..]);
            if head == "when" {
                call("if", vec![args[0].clone(), body])
            } else {
                call("if", vec![args[0].clone(), s("nil"), body])
            }
        }
        "and" | "or" => {
            if args.is_empty() {
                s(if head == "and" { "t" } else { "nil" })
            } else if args.len() == 1 {
                args[0].clone()
            } else {
                let tail = call(head, args[1..].to_vec());
                if head == "and" {
                    call("if", vec![args[0].clone(), tail])
                } else {
                    let tmp = fresh.symbol();
                    binding(
                        tmp.clone(),
                        args[0].clone(),
                        call("if", vec![tmp.clone(), tmp, tail]),
                    )
                }
            }
        }
        "cond" => {
            let mut tail = s("nil");
            for clause in args.iter().rev() {
                let items = clause.list()?;
                if items.is_empty() {
                    continue;
                }
                tail = if items.len() == 1 {
                    let tmp = fresh.symbol();
                    binding(
                        tmp.clone(),
                        items[0].clone(),
                        call("if", vec![tmp.clone(), tmp, tail]),
                    )
                } else {
                    call("if", vec![items[0].clone(), progn(&items[1..]), tail])
                };
            }
            tail
        }
        "prog1" | "prog2" => {
            let index = usize::from(head == "prog2");
            if args.len() <= index {
                return Err(format!("{head} has too few arguments"));
            }
            let tmp = fresh.symbol();
            let mut body = args[index + 1..].to_vec();
            body.push(tmp.clone());
            let saved = binding(tmp, args[index].clone(), progn(&body));
            if index == 0 {
                saved
            } else {
                call("progn", vec![args[0].clone(), saved])
            }
        }
        "dotimes" | "dolist" => {
            if args.is_empty() {
                return Err(format!("{head} needs a binding"));
            }
            let spec = args[0].list()?;
            if !(2..=3).contains(&spec.len()) {
                return Err(format!("bad {head} binding"));
            }
            spec[0].name()?;
            let var = spec[0].clone();
            let tmp = fresh.symbol();
            let mut body = args[1..].to_vec();
            let result = spec.get(2).cloned().unwrap_or_else(|| s("nil"));
            if head == "dotimes" {
                let counter = fresh.symbol();
                let iteration = binding(var.clone(), counter.clone(), progn(&body));
                let advance = call(
                    "setq",
                    vec![counter.clone(), call("1+", vec![counter.clone()])],
                );
                let loop_form = call(
                    "while",
                    vec![
                        call("<", vec![counter.clone(), tmp.clone()]),
                        iteration,
                        advance,
                    ],
                );
                let finish = binding(var, counter.clone(), result);
                binding(
                    tmp,
                    spec[1].clone(),
                    binding(
                        counter,
                        Form::Int(0),
                        call("progn", vec![loop_form, finish]),
                    ),
                )
            } else {
                body.push(call(
                    "setq",
                    vec![tmp.clone(), call("cdr", vec![tmp.clone()])],
                ));
                let iteration = binding(var, call("car", vec![tmp.clone()]), progn(&body));
                let loop_form = call("while", vec![tmp.clone(), iteration]);
                // GNU 32.0.50's dolist result is outside the iteration binding.
                binding(tmp, spec[1].clone(), call("progn", vec![loop_form, result]))
            }
        }
        "funcall"
            if args.len() == 3
                && matches!(&args[0], Form::List(q) if q.len() == 2
                    && matches!(&q[0], Form::Sym(h) if h == "function" || h == "quote")
                    && matches!(&q[1], Form::Sym(n) if n == "maphash")) =>
        {
            // GNU's byte compiler turns (funcall #'maphash F TABLE) into a
            // direct call; lower it to the same slot walk.
            call("maphash", args[1..].to_vec())
        }
        "maphash" if args.len() == 2 => {
            // A primitive cannot call Lisp, so the slot walk of GNU's
            // DOHASH_SAFE is compiled inline. Slot indices are integers and
            // survive collection inside FUNCTION; primitive calls never
            // collect, so each key/value pair is read atomically.
            let (function, table, at, slot) = (
                fresh.symbol(),
                fresh.symbol(),
                fresh.symbol(),
                fresh.symbol(),
            );
            let step = call(
                "while",
                vec![
                    call(
                        "setq",
                        vec![
                            slot.clone(),
                            call("#:vm2-hash-next", vec![table.clone(), at.clone()]),
                        ],
                    ),
                    call("setq", vec![at.clone(), call("1+", vec![slot.clone()])]),
                    call(
                        "funcall",
                        vec![
                            function.clone(),
                            call("#:vm2-hash-key", vec![table.clone(), slot.clone()]),
                            call("#:vm2-hash-value", vec![table.clone(), slot.clone()]),
                        ],
                    ),
                ],
            );
            binding(
                function,
                args[0].clone(),
                binding(
                    table,
                    args[1].clone(),
                    binding(
                        at,
                        Form::Int(0),
                        binding(slot, s("nil"), call("progn", vec![step, s("nil")])),
                    ),
                ),
            )
        }
        "push" => {
            if args.len() != 2 {
                return Err("push needs value and place".into());
            }
            if !matches!(args[1], Form::Sym(_) | Form::Generated(_)) {
                let value = fresh.symbol();
                let p = place(&args[1], &mut fresh)?;
                let update = call("cons", vec![value.clone(), p.read.clone()]);
                return Ok(Some(binding(value, args[0].clone(), store(p, update))));
            }
            call(
                "setq",
                vec![
                    args[1].clone(),
                    call("cons", vec![args[0].clone(), args[1].clone()]),
                ],
            )
        }
        "pop" => {
            if args.len() != 1 {
                return Err("pop needs a place".into());
            }
            if !matches!(args[0], Form::Sym(_) | Form::Generated(_)) {
                let saved = fresh.symbol();
                let p = place(&args[0], &mut fresh)?;
                let mut bindings = p.bindings;
                bindings.push(l(vec![saved.clone(), p.read]));
                bindings.push(l(vec![p.value, call("cdr", vec![saved.clone()])]));
                return Ok(Some(call(
                    "let*",
                    vec![l(bindings), p.store, call("car", vec![saved])],
                )));
            }
            call(
                "prog1",
                vec![
                    call("car", vec![args[0].clone()]),
                    call(
                        "setq",
                        vec![args[0].clone(), call("cdr", vec![args[0].clone()])],
                    ),
                ],
            )
        }
        "setf" => {
            if !args.len().is_multiple_of(2) {
                return Err("setf needs place/value pairs".into());
            }
            let mut forms = vec![];
            for pair in args.chunks_exact(2) {
                let p = place(&pair[0], &mut fresh)?;
                forms.push(store(p, pair[1].clone()));
            }
            progn(&forms)
        }
        "cl-incf" | "cl-decf" => {
            if !(1..=2).contains(&args.len()) {
                return Err(format!("bad {head} arguments"));
            }
            let p = place(&args[0], &mut fresh)?;
            let delta = args.get(1).cloned().unwrap_or(Form::Int(1));
            let value = call(
                if head == "cl-incf" { "+" } else { "-" },
                vec![p.read.clone(), delta],
            );
            store(p, value)
        }
        _ => return Ok(None),
    };
    Ok(Some(expanded))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_names_are_unique_across_expansions() {
        let form = call("or", vec![Form::Int(1), Form::Int(2)]);
        let first = expand(&form).unwrap().unwrap();
        let second = expand(&form).unwrap().unwrap();
        assert_ne!(
            format!("{first:?}"),
            format!("{second:?}"),
            "separate expansions must not reuse generated identities"
        );
    }

    #[test]
    fn generated_namespace_is_disjoint_from_all_source_strings() {
        use crate::{Name, Vm};
        let Form::Generated(id) = Fresh.symbol() else {
            panic!("typed identity required")
        };
        for text in [
            format!("#:vm2-temp-{id}"),
            format!("Generated({id})"),
            format!("\0vm2-temp-{id}"),
            format!("g{id}"),
        ] {
            let source = Form::from_json(serde_json::json!(text)).unwrap();
            assert_ne!(source.name().unwrap(), Name::Generated(id));
            // Escaping makes every character (including # and NUL) reader-producible.
            let escaped = text.chars().map(|c| format!("\\{c}")).collect::<String>();
            let mut vm = Vm::new();
            let (value, _) = vm.read_source_raw(&escaped).unwrap().unwrap();
            let source = vm.source_form(value, 0).unwrap();
            assert_ne!(source.name().unwrap(), Name::Generated(id));
        }
    }

    #[test]
    fn generated_bindings_survive_user_macro_roundtrips() {
        let mut vm = crate::Vm::new();
        vm.eval_source_raw("(defmacro + (a b) (list '- a b))").unwrap();
        let value = vm
            .eval_source_raw("(let ((x (cons 5 nil))) (cl-incf (car x) 2))")
            .unwrap();
        assert_eq!(value.as_integer().unwrap(), 3);
    }

    #[test]
    fn generated_literals_remain_uninterned_across_macro_data_and_images() {
        use crate::{ImageRegion, Name, Value, Vm};
        let id = generated_id();
        let mut vm = Vm::new();
        let generated = vm.literal(&Form::Generated(id)).unwrap();
        let spelling = vm.display(generated).unwrap();
        assert_ne!(generated, vm.intern(&spelling));
        assert_eq!(
            vm.source_form(generated, 0).unwrap().name().unwrap(),
            Name::Generated(id)
        );
        let slot = vm.slot("saved-temporary");
        vm.slots[slot].value = generated;
        let entry = vm.compile_source("saved-temporary").unwrap();
        let store = std::env::temp_dir().join(format!("vm2-generated-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&store).unwrap();
        let config = [17; 32];
        let key = vm.publish_code_image(&store, entry, config).unwrap();
        let image = unsafe { ImageRegion::open(&store, &key) }.unwrap();
        let (mut restored, entry) = Vm::from_code_image(image, config).unwrap();
        let value = restored.execute_raw(entry).unwrap();
        assert_ne!(value, Value::NIL);
        assert_eq!(restored.display(value).unwrap(), spelling);
        assert_ne!(value, restored.intern(&spelling));
        let Name::Generated(new_id) = restored.source_form(value, 0).unwrap().name().unwrap()
        else {
            panic!("identity must survive image restore")
        };
        assert_ne!(
            new_id, id,
            "restoration must acquire a fresh process identity"
        );
        assert_eq!(restored.literal(&Form::Generated(new_id)).unwrap(), value);
        std::fs::remove_dir_all(store).unwrap();
    }

    #[test]
    fn nested_expansions_preserve_bindings_and_closure_captures() {
        let mut vm = crate::Vm::new();
        let value = vm.eval_source_raw("(let ((vm2-temp-0 19)) (+ (or nil (or nil vm2-temp-0)) (prog1 23 (funcall (lambda () vm2-temp-0)))))").unwrap();
        assert_eq!(value.as_integer().unwrap(), 42);
        // JSON symbols need not obey reader syntax; hostile old-prefix names remain ordinary.
        let form = Form::from_json(serde_json::json!([
            "let",
            [["#:vm2-temp-0", 42]],
            ["or", "nil", "#:vm2-temp-0"]
        ]))
        .unwrap();
        let f = vm.compile(&form).unwrap();
        assert_eq!(vm.execute_raw(f).unwrap().as_integer().unwrap(), 42);
    }
}
