//! Compiler lowering for variable-defining macros. All Values use existing
//! traced slots/plists; compiling these forms allocates nothing in Vm::new.
use super::*;
fn sym(s: &str) -> Form { Form::Sym(s.into()) }
fn quote(f: Form) -> Form { Form::List(vec![sym("quote"), f]) }
fn call(s: &str, args: Vec<Form>) -> Form { Form::List(std::iter::once(sym(s)).chain(args).collect()) }

fn custom_locality(name: Form, expression: Form) -> Form {
    let value = Form::Generated(expand::generated_id());
    let automatic = call("memq", vec![value.clone(), quote(Form::List(vec![sym("t"), sym("permanent")]))]);
    let permanent = call("memq", vec![value.clone(), quote(Form::List(vec![sym("permanent"), sym("permanent-only")]))]);
    call("let", vec![
        Form::List(vec![Form::List(vec![value, expression])]),
        call("if", vec![automatic, call("make-variable-buffer-local", vec![quote(name.clone())])]),
        call("if", vec![permanent, call("put", vec![quote(name), quote(sym("permanent-local")), sym("t")])]),
    ])
}

impl Vm {
    pub(super) fn variable_definition(&mut self, b: &mut Builder, head: &str, rest: &[Form]) -> Result<usize> {
        if rest.is_empty() { return Err("variable definition requires a symbol".into()); }
        rest[0].sym()?;
        let name = rest[0].clone();
        let lowered = match head {
            "defvar-local" if (2..=3).contains(&rest.len()) => {
                call("progn", vec![call("defvar", rest.to_vec()), call("make-variable-buffer-local", vec![quote(name)])])
            }
            "defgroup" if rest.len() >= 3 && (rest.len()-3).is_multiple_of(2) => {
                let mut forms = vec![call("put", vec![quote(name.clone()), quote(sym("group-documentation")), rest[2].clone()])];
                forms.push(call("put", vec![quote(name.clone()), quote(sym("custom-group-members")), rest[1].clone()]));
                for pair in rest[3..].chunks_exact(2) {
                    let key = match pair[0].sym()? {
                        ":group" => "custom-group".to_owned(),
                        ":prefix" => "custom-prefix".to_owned(),
                        ":version" => "custom-version".to_owned(),
                        key if key.starts_with(':') => key[1..].to_owned(),
                        _ => return Err("defgroup keyword required".into()),
                    };
                    forms.push(call("put", vec![quote(name.clone()), quote(sym(&key)), pair[1].clone()]));
                }
                forms.push(quote(name));
                call("progn", forms)
            }
            "defcustom" if rest.len() >= 3 && (rest.len()-3).is_multiple_of(2) => {
                let delayed = rest[3..].chunks_exact(2).any(|pair| {
                    matches!(&pair[0], Form::Sym(key) if key == ":initialize")
                        && matches!(&pair[1], Form::List(value) if value.len() == 2
                            && matches!(&value[0], Form::Sym(head) if head == "function" || head == "quote")
                            && matches!(&value[1], Form::Sym(init) if init == "custom-initialize-delay"))
                });
                let mut forms = vec![call("put", vec![quote(name.clone()), quote(sym("standard-value")), quote(Form::List(vec![rest[1].clone()]))])];
                if delayed {
                    forms.push(call("defvar", vec![sym("custom-delayed-init-variables"), sym("nil")]));
                    forms.push(call("if", vec![
                        call("listp", vec![sym("custom-delayed-init-variables")]),
                        call("progn", vec![
                            call("internal--define-uninitialized-variable", vec![quote(name.clone()), rest[2].clone()]),
                            call("setq", vec![sym("custom-delayed-init-variables"), call("cons", vec![quote(name.clone()), sym("custom-delayed-init-variables")])]),
                        ]),
                        call("defvar", rest[..3].to_vec()),
                    ]));
                } else {
                    forms.push(call("defvar", rest[..3].to_vec()));
                }
                for pair in rest[3..].chunks_exact(2) {
                    if pair[0].sym()? == ":local" {
                        forms.push(custom_locality(name.clone(), pair[1].clone()));
                        continue;
                    }
                    let key = match pair[0].sym()? {
                        ":type" => "custom-type", ":set" => "custom-set", ":get" => "custom-get",
                        ":group" => "custom-group", ":initialize" => "custom-initialize",
                        ":require" => "custom-require", ":version" => "custom-version",
                        ":options" => "custom-options", ":safe" => "safe-local-variable",
                        key if key.starts_with(':') => &key[1..],
                        _ => return Err("defcustom keyword required".into()),
                    };
                    forms.push(call("put", vec![quote(name.clone()), quote(sym(key)), pair[1].clone()]));
                }
                forms.push(quote(name));
                call("progn", forms)
            }
            "defvar-keymap" => {
                let mut at = 1;
                let mut doc = None;
                let mut prompt = sym("nil");
                let mut repeat = false;
                while at < rest.len() && rest[at].sym().is_ok_and(|s| s.starts_with(':')) {
                    if at+1 == rest.len() { return Err("uneven keymap options".into()); }
                    match rest[at].sym()? {
                        ":doc" => doc = Some(rest[at+1].clone()),
                        ":name" => prompt = rest[at+1].clone(),
                        ":repeat" if matches!(&rest[at+1], Form::Sym(s) if s == "t" || s == "nil") => repeat = matches!(&rest[at+1], Form::Sym(s) if s == "t"),
                        _ => return Err("unsupported defvar-keymap option".into()),
                    }
                    at += 2;
                }
                if !(rest.len()-at).is_multiple_of(2) { return Err("uneven keymap definitions".into()); }
                let map = Form::Generated(expand::generated_id());
                let mut body = vec![];
                let mut props = vec![];
                let mut keys = NameSet::default();
                for pair in rest[at..].chunks_exact(2) {
                    if !keys.insert(format!("{:?}", pair[0])) { return Err("duplicate key definition".into()); }
                    body.push(call("define-key", vec![map.clone(), call("kbd", vec![pair[0].clone()]), pair[1].clone()]));
                    if repeat && matches!(&pair[1], Form::List(v) if v.len()==2 && matches!(v[0].sym(), Ok("quote" | "function"))) {
                        let Form::List(def) = &pair[1] else { unreachable!() };
                        props.push(call("put", vec![quote(def[1].clone()), quote(sym("repeat-map")), quote(name.clone())]));
                    }
                }
                body.push(map.clone());
                let init = call("let", std::iter::once(Form::List(vec![Form::List(vec![map, call("make-sparse-keymap", vec![prompt])])])).chain(body).collect());
                let mut args = vec![name.clone(), init];
                if let Some(doc) = doc { args.push(doc); }
                props.insert(0, call("defvar", args));
                // GNU returns the final repeat property value when present.
                call("progn", props)
            }
            _ => return Err(format!("invalid {head}")),
        };
        self.expr(b, &lowered)
    }
}
