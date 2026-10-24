//! T2 example (PRIMITIVE-DOCTRINE section 3): a namespaced callback primitive.
//! Test-only: generated module and table entry are both gated by cfg(test).
//! It exists to exercise `Step::Call`, so it adds a romacs-namespaced function
//! (policy F5) and changes no GNU function.
use super::*;

// GNU `Fassoc` with TESTFN (fns.c:2006), namespaced and with TESTFN required:
// (romacs--t2-assoc KEY ALIST TESTFN). Every candidate costs one
// `(funcall TESTFN (car ELT) KEY)`, GNU's argument order, that the ENGINE
// performs; KEY, ALIST, the tail, the candidate, TESTFN and the step count wait
// in the continuation's registers, so a collection inside TESTFN forwards them
// like any frame. Deliberate differences, which keep the example small: a nil
// TESTFN compares with `eq` alone (GNU also tries `equal`) and never enters
// Lisp (doctrine C5), and a cycle is caught by the list limit, naming ALIST.
defsubr! { name = "romacs--t2-assoc", gnu = "fns.c:2006", effect = Global,
fn romacs_t2_assoc(cx, key: Value, alist: Value, testfn: Value) -> Step {
    scan(cx, key, alist, alist, testfn, 0)
}}

fn scan(cx: &mut Leaf<'_>, key: Value, alist: Value, mut tail: Value, testfn: Value, mut n: i64) -> Next {
    while let Some((car, cdr)) = cx.pair(tail) {
        n += 1;
        if n as usize > cx.list_limit() {
            return Err(Fault::Circular(alist));
        }
        if let Some((candidate, _)) = cx.pair(car) {
            if testfn == Value::NIL {
                if candidate == key {
                    return Ok(Step::Done(car));
                }
            } else {
                let n = Value::integer(n).expect("list limit is a fixnum");
                return Ok(Step::Call {
                    f: testfn,
                    args: vec![candidate, key],
                    then: tested,
                    saved: vec![key, alist, tail, car, testfn, n],
                });
            }
        }
        tail = cdr;
    }
    // CHECK_LIST_END (tail, alist): GNU names the whole list.
    if tail == Value::NIL { Ok(Step::Done(Value::NIL)) } else { Err(Fault::Wrong("listp", alist)) }
}

// Like GNU, the matched element is the one read before the call, and the walk
// continues from the tail's cdr as it is after the call.
fn tested(cx: &mut Leaf<'_>, matched: Value, saved: &[Value]) -> Next {
    let &[key, alist, tail, car, testfn, n] = saved else {
        unreachable!("romacs--t2-assoc saves six registers")
    };
    if matched != Value::NIL {
        return Ok(Step::Done(car));
    }
    let next = cx.pair(tail).map_or(Value::NIL, |(_, cdr)| cdr);
    scan(cx, key, alist, next, testfn, n.integer_value().expect("saved count"))
}
