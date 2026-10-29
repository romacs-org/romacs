//! Malformed special forms (eval.c Flet, FletX, Fsetq, Fcondition_case,
//! Fdefvar, Fdefconst). GNU checks a special form's shape as it evaluates
//! it, so a bad one signals only when it runs, after whatever GNU
//! evaluates first. VM2 compiles ahead of time; a malformed form compiles
//! to exactly those evaluations followed by GNU's signal, so a closure
//! holding it can still be made. The signal builds its data afresh each
//! time, as GNU's xsignal does, so a handler that mutates it changes no
//! later error.
use crate::Form;

fn nil() -> Form { Form::Sym("nil".into()) }
fn is_nil(form: &Form) -> bool { matches!(form, Form::Sym(s) if s == "nil") || matches!(form, Form::List(v) if v.is_empty()) }
fn quote(form: Form) -> Form { Form::List(vec![Form::Sym("quote".into()), form]) }
/// `(signal 'CONDITION (list 'DATUM...))'.
pub(crate) fn signal(condition: &str, data: Vec<Form>) -> Form {
    let data = [vec![Form::Sym("list".into())], data.into_iter().map(quote).collect()].concat();
    Form::List(vec![Form::Sym("signal".into()), quote(Form::Sym(condition.into())), Form::List(data)])
}
fn listp(object: &Form) -> Form { signal("wrong-type-argument", vec![Form::Sym("listp".into()), object.clone()]) }
fn progn(forms: Vec<Form>) -> Form { Form::List([vec![Form::Sym("progn".into())], forms].concat()) }

/// Fcar of a list form, else GNU's (wrong-type-argument listp OBJECT).
fn car(form: &Form) -> Result<Form, Form> {
    match form {
        Form::List(v) => Ok(v.first().cloned().unwrap_or_else(nil)),
        Form::Dotted(v, _) => Ok(v[0].clone()),
        _ if is_nil(form) => Ok(nil()),
        _ => Err(listp(form)),
    }
}
/// Fcdr of a list form, else GNU's (wrong-type-argument listp OBJECT).
fn cdr(form: &Form) -> Result<Form, Form> {
    match form {
        Form::List(v) if v.len() > 1 => Ok(Form::List(v[1..].to_vec())),
        Form::List(_) => Ok(nil()),
        Form::Dotted(v, tail) if v.len() > 1 => Ok(Form::Dotted(v[1..].to_vec(), tail.clone())),
        Form::Dotted(_, tail) => Ok((**tail).clone()),
        _ if is_nil(form) => Ok(nil()),
        _ => Err(listp(form)),
    }
}
fn symbol(form: &Form) -> bool { matches!(form, Form::Sym(_) | Form::Generated(_)) }

/// An element's value form, or the signal its shape raises first:
/// `(Fcar|Fcdr) ELT' on a non-list, then signal_error for more than one
/// value form (data: the message consed onto ELT when it is a proper list).
fn value_form(elt: &Form) -> Result<Form, Form> {
    if symbol(elt) { return Ok(nil()) }
    let rest = cdr(elt)?;
    if !is_nil(&cdr(&rest)?) {
        let message = Form::String("`let' bindings can have only one value-form".into());
        let data = match elt { Form::List(v) => [vec![message], v.clone()].concat(), _ => vec![message, elt.clone()] };
        return Err(signal("error", data));
    }
    car(&rest)
}

/// ELT's variable: ELT itself or its car.
fn variable(elt: &Form) -> Option<Form> { if symbol(elt) { Some(elt.clone()) } else { car(elt).ok() } }

/// The signal binding ELT's variable raises: CHECK_TYPE (symbolp), then
/// specbind of a constant (nil, t, a keyword).
fn binding_error(elt: &Form) -> Option<Form> {
    let var = variable(elt)?;
    match &var {
        Form::Sym(s) if s == "nil" || s == "t" || s.starts_with(':') => Some(signal("setting-constant", vec![var.clone()])),
        _ if symbol(&var) => None,
        _ => Some(signal("wrong-type-argument", vec![Form::Sym("symbolp".into()), var])),
    }
}

/// VARLIST's elements and the non-nil tail ending it, if any.
fn elements(varlist: &Form) -> (Vec<Form>, Option<Form>) {
    match varlist {
        Form::List(v) => (v.clone(), None),
        Form::Dotted(v, tail) => (v.clone(), Some((**tail).clone())),
        _ if is_nil(varlist) => (vec![], None),
        other => (vec![], Some(other.clone())),
    }
}

/// The form a malformed `(let VARLIST ...)' (STAR: `let*') evaluates as in
/// GNU, or None for a well-formed varlist.
pub(crate) fn malformed_let(star: bool, varlist: &Form) -> Option<Form> {
    let (elts, tail) = elements(varlist);
    if !star {
        // Flet: list_length, every value form into a temporary, then each
        // binding in turn; the bindings before the bad one are made, and
        // unwound by the signal.
        if tail.is_some() { return Some(listp(varlist)) }
        let mut evaluated = Vec::with_capacity(elts.len() + 1);
        for elt in &elts {
            match value_form(elt) {
                Ok(form) => evaluated.push(form),
                Err(raise) => { evaluated.push(raise); return Some(progn(evaluated)) }
            }
        }
        let k = elts.iter().position(|elt| binding_error(elt).is_some())?;
        let temps: Vec<Form> = evaluated.iter().map(|_| Form::Generated(crate::expand::generated_id())).collect();
        let pair = |a: Form, b: &Form| Form::List(vec![a, b.clone()]);
        let inits = temps.iter().zip(&evaluated).map(|(temp, form)| pair(temp.clone(), form)).collect();
        let prefix = elts[..k].iter().zip(&temps).filter_map(|(elt, temp)| Some(pair(variable(elt)?, temp))).collect();
        let bind = Form::List(vec![Form::Sym("let".into()), Form::List(prefix), binding_error(&elts[k])?]);
        return Some(Form::List(vec![Form::Sym("let".into()), Form::List(inits), bind]));
    }
    // FletX: each element's value form and binding in turn, then
    // CHECK_LIST_END with the whole varlist.
    let prefix = |k: usize| Form::List(elts[..k].to_vec());
    for (k, elt) in elts.iter().enumerate() {
        let failure = match value_form(elt) {
            Err(raise) => Some(raise),
            Ok(form) => binding_error(elt).map(|raise| progn(vec![form, raise])),
        };
        if let Some(body) = failure { return Some(Form::List(vec![Form::Sym("let*".into()), prefix(k), body])) }
    }
    tail.map(|_| Form::List(vec![Form::Sym("let*".into()), prefix(elts.len()), listp(varlist)]))
}

/// Fsetq: pair by pair, a variable with no value form is
/// wrong-number-of-arguments, else its value form runs and Fset checks the
/// variable: not a symbol (symbolp), nil or t, or a keyword
/// (set_internal lets a keyword be set only to itself) signals, the
/// earlier pairs set.
pub(crate) fn malformed_setq(args: &[Form]) -> Option<Form> {
    let keyword = |form: &Form| matches!(form, Form::Sym(s) if s.starts_with(':'));
    let bad = |var: &Form| !symbol(var) || is_nil(var) || matches!(var, Form::Sym(s) if s == "t") || keyword(var);
    let k = args.chunks(2).position(|pair| pair.len() == 1 || bad(&pair[0]))?;
    let (var, rest) = (&args[2 * k], &args[2 * k + 1..]);
    let setq = |pairs: &[Form]| Form::List([vec![Form::Sym("setq".into())], pairs.to_vec()].concat());
    let mut forms = Vec::new();
    if k > 0 { forms.push(setq(&args[..2 * k])) }
    let Some((value, rest)) = rest.split_first() else {
        forms.push(signal("wrong-number-of-arguments", vec![Form::Sym("setq".into()), Form::Int(args.len() as i64)]));
        return Some(progn(forms));
    };
    if !symbol(var) {
        forms.extend([value.clone(), signal("wrong-type-argument", vec![Form::Sym("symbolp".into()), var.clone()])]);
    } else if keyword(var) {
        let same = Form::List(vec![Form::Sym("eq".into()), value.clone(), quote(var.clone())]);
        forms.push(Form::List(vec![Form::Sym("or".into()), same, signal("setting-constant", vec![var.clone()])]));
        forms.push(if rest.is_empty() { quote(var.clone()) } else { setq(rest) });
    } else {
        forms.extend([value.clone(), signal("setting-constant", vec![var.clone()])]);
    }
    Some(progn(forms))
}

/// internal_lisp_condition_case's checks before the body runs: VAR a
/// symbol, then each handler nil or a list whose car is a symbol or a list
/// (`error' "Invalid condition handler: HANDLER", printed as princ does).
pub(crate) fn malformed_condition_case(var: &Form, handlers: &[Form]) -> Option<Form> {
    if !symbol(var) { return Some(signal("wrong-type-argument", vec![Form::Sym("symbolp".into()), var.clone()])) }
    let valid = |handler: &Form| is_nil(handler) || match handler {
        Form::List(v) | Form::Dotted(v, _) => symbol(&v[0]) || matches!(v[0], Form::List(_) | Form::Dotted(..)),
        _ => false,
    };
    let bad = handlers.iter().find(|handler| !valid(handler))?;
    let message = Form::List(vec![Form::Sym("format".into()), Form::String("Invalid condition handler: %s".into()), quote(bad.clone())]);
    Some(Form::List(vec![Form::Sym("signal".into()), quote(Form::Sym("error".into())), Form::List(vec![Form::Sym("list".into()), message])]))
}

/// Fdefvar and Fdefconst: CHECK_SYMBOL on the name before anything runs.
pub(crate) fn malformed_definition(name: &Form) -> Option<Form> {
    (!symbol(name)).then(|| signal("wrong-type-argument", vec![Form::Sym("symbolp".into()), name.clone()]))
}
