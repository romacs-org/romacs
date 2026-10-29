//! eval.c Flet and FletX for a malformed VARLIST. GNU checks the varlist
//! as it evaluates the form, so a bad binding signals only when the `let'
//! runs, after the value forms evaluated before it. VM2 compiles a `let'
//! ahead of time; a malformed one compiles to exactly those evaluations
//! followed by GNU's signal, so a closure holding it can still be made.
use crate::Form;

fn nil() -> Form { Form::Sym("nil".into()) }
fn is_nil(form: &Form) -> bool { matches!(form, Form::Sym(s) if s == "nil") || matches!(form, Form::List(v) if v.is_empty()) }
fn quote(form: Form) -> Form { Form::List(vec![Form::Sym("quote".into()), form]) }
fn signal(condition: &str, data: Vec<Form>) -> Form {
    Form::List(vec![Form::Sym("signal".into()), quote(Form::Sym(condition.into())), quote(Form::List(data))])
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

/// The signal binding ELT's variable raises: CHECK_TYPE (symbolp), then
/// specbind of a constant (nil, t, a keyword).
fn binding_error(elt: &Form) -> Option<Form> {
    let var = if symbol(elt) { elt.clone() } else { car(elt).ok()? };
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
        // Flet: list_length, every value form, then every binding.
        if tail.is_some() { return Some(listp(varlist)) }
        let mut evaluated = Vec::with_capacity(elts.len() + 1);
        for elt in &elts {
            match value_form(elt) {
                Ok(form) => evaluated.push(form),
                Err(raise) => { evaluated.push(raise); return Some(progn(evaluated)) }
            }
        }
        let raise = elts.iter().find_map(binding_error)?;
        evaluated.push(raise);
        return Some(progn(evaluated));
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
