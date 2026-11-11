//! FSRS, the spaced-repetition scheduler Anki uses, through the `fsrs`
//! crate (Romacs only).  Gnosis schedules reviews with these; the formulas
//! and the optimizer are the crate's, so Romacs answers what fsrs-rs does.
//!
//! PARAMETERS is a vector or list of 17, 19 or 21 numbers (FSRS-4.5, 5 or
//! 6 weights, the older ones filled as the crate fills them), or nil for
//! the crate's FSRS-6 defaults.  MEMORY is nil for a new card, else
//! (STABILITY . DIFFICULTY).  RATING is 1 Again, 2 Hard, 3 Good or 4 Easy.
//! Days are whole days; stability and intervals are in days.  The crate
//! computes in single precision, and floats here carry its values exactly.
use super::Value;
use super::leaf_scalars::number;
use crate::prim::{Fault, Leaf};
use super::orderless::{elements, list};
use ::fsrs::{ComputeParametersInput, FSRSItem, FSRSReview, MemoryState, FSRS};

type Result<T> = std::result::Result<T, Fault>;

// The state after RATING, ELAPSED-DAYS after MEMORY was last reviewed
// under PARAMETERS, and the interval in days that reaches RETENTION:
// (:stability S :difficulty D :interval I).  The interval is unrounded.
defsubr! { name = "romacs-fsrs-next-state", gnu = "romacs:fsrs", effect = Reads,
fn romacs_fsrs_next_state(cx, parameters: Value, memory: Value, elapsed_days: Value, rating: Value, retention: Value) {
    let model = model(cx, parameters)?;
    let memory = memory_state(cx, memory)?;
    let elapsed = whole(elapsed_days, u32::MAX)?;
    let rating = whole(rating, 4).and_then(|r| if r == 0 { Err(Fault::Wrong("natnump", rating)) } else { Ok(r) })?;
    let retention = probability(cx, retention)?;
    let next = model.next_states(memory, retention, elapsed).map_err(|e| failed(&e))?;
    let state = [next.again, next.hard, next.good, next.easy][rating as usize - 1].clone();
    let keys = [":stability", ":difficulty", ":interval"];
    let values = [state.memory.stability, state.memory.difficulty, state.interval];
    Ok(plist(cx, &keys, &values))
}}

// The unrounded interval in days at which a card of STABILITY falls to
// RETENTION under PARAMETERS.
defsubr! { name = "romacs-fsrs-interval", gnu = "romacs:fsrs", effect = Reads,
fn romacs_fsrs_interval(cx, parameters: Value, stability: Value, retention: Value) {
    let model = model(cx, parameters)?;
    let stability = positive(cx, stability)?;
    let retention = probability(cx, retention)?;
    Ok(cx.float(f64::from(model.next_interval(Some(stability), retention, 3))))
}}

// The probability of recalling a card of STABILITY ELAPSED-DAYS after its
// last review, on PARAMETERS' forgetting curve.  ELAPSED-DAYS may be
// fractional.
defsubr! { name = "romacs-fsrs-retrievability", gnu = "romacs:fsrs", effect = Reads,
fn romacs_fsrs_retrievability(cx, parameters: Value, stability: Value, elapsed_days: Value) {
    let weights = weights(cx, parameters)?;
    let stability = positive(cx, stability)?;
    let elapsed = number(cx, elapsed_days)?;
    if !(elapsed >= 0.0 && elapsed.is_finite()) { return Err(Fault::Wrong("natnump", elapsed_days)); }
    let memory = MemoryState { stability, difficulty: 5.0 };
    Ok(cx.float(f64::from(::fsrs::current_retrievability(memory, single(elapsed), weights[20]))))
}}

// Weights fitted to HISTORIES by the crate's optimizer, as Anki trains
// them: (:parameters VECTOR :items N).  HISTORIES holds one list per card
// of (RATING . ELAPSED-DAYS) reviews, oldest first, the first with zero
// elapsed days.  Every later review a day or more after the one before is
// an item to predict, and N counts them.  Under 8 items the result is the
// defaults; under 64 only the initial stabilities are fitted.
defsubr! { name = "romacs-fsrs-compute-parameters", gnu = "romacs:fsrs", effect = Reads,
fn romacs_fsrs_compute_parameters(cx, histories: Value) {
    let mut train_set = vec![];
    let mut card_ids = vec![];
    for (card, history) in elements(cx, histories)?.into_iter().enumerate() {
        let reviews = reviews(cx, history)?;
        for item in items(&reviews) {
            train_set.push(item);
            card_ids.push(i64::try_from(card).map_err(|_| Fault::Wrong("fixnump", histories))?);
        }
    }
    let count = train_set.len();
    let input = ComputeParametersInput { train_set, card_ids: Some(card_ids), ..Default::default() };
    let fitted = ::fsrs::compute_parameters(input).map_err(|e| failed(&e))?;
    let fitted: Vec<Value> = fitted.into_iter().map(|w| cx.float(f64::from(w))).collect();
    let keys = [cx.intern(":parameters"), cx.intern(":items")];
    let values = [cx.vector(&fitted), i64::try_from(count).ok().and_then(Value::fixnum).ok_or(Fault::Wrong("fixnump", histories))?];
    Ok(list(cx, &[keys[0], values[0], keys[1], values[1]]))
}}

/// The items Anki trains on from one card's REVIEWS: each prefix ending
/// in a review at least a day after the one before.
fn items(reviews: &[FSRSReview]) -> impl Iterator<Item = FSRSItem> + '_ {
    (1..reviews.len())
        .filter(|&end| reviews[end].delta_t > 0)
        .map(|end| FSRSItem { reviews: reviews[..=end].to_vec() })
}

/// One card's (RATING . ELAPSED-DAYS) list as crate reviews.
fn reviews(cx: &mut Leaf<'_>, history: Value) -> Result<Vec<FSRSReview>> {
    let mut out = vec![];
    for review in elements(cx, history)? {
        let (rating, delta) = cx.pair(review).ok_or(Fault::Wrong("consp", review))?;
        let rating = whole(rating, 4)?;
        if rating == 0 { return Err(Fault::Wrong("natnump", review)); }
        let delta_t = whole(delta, u32::MAX)?;
        if out.is_empty() && delta_t != 0 { return Err(Fault::Wrong("zerop", delta)); }
        out.push(FSRSReview { rating, delta_t });
    }
    Ok(out)
}

/// The crate's model for PARAMETERS, nil meaning the defaults.
fn model(cx: &mut Leaf<'_>, parameters: Value) -> Result<FSRS> {
    let given = given_weights(cx, parameters)?;
    FSRS::new(&given).map_err(|e| failed(&e))
}

/// PARAMETERS filled to the 21 FSRS-6 weights, as the crate fills them.
fn weights(cx: &mut Leaf<'_>, parameters: Value) -> Result<Vec<f32>> {
    let given = given_weights(cx, parameters)?;
    ::fsrs::check_and_fill_parameters(&given).map_err(|e| failed(&e))
}

/// PARAMETERS' numbers as given, empty for nil (the crate's defaults).
fn given_weights(cx: &mut Leaf<'_>, parameters: Value) -> Result<Vec<f32>> {
    let values = if parameters == Value::NIL { vec![] }
        else if cx.vector_len(parameters).is_ok() { cx.vector_values(parameters)? }
        else { elements(cx, parameters)? };
    values.into_iter().map(|w| {
        let x = number(cx, w)?;
        if x.is_finite() { Ok(single(x)) } else { Err(Fault::Wrong("numberp", w)) }
    }).collect()
}

/// MEMORY, nil or (STABILITY . DIFFICULTY).
fn memory_state(cx: &mut Leaf<'_>, memory: Value) -> Result<Option<MemoryState>> {
    if memory == Value::NIL { return Ok(None); }
    let (stability, difficulty) = cx.pair(memory).ok_or(Fault::Wrong("consp", memory))?;
    let stability = positive(cx, stability)?;
    let d = number(cx, difficulty)?;
    if !(1.0..=10.0).contains(&d) { return Err(Fault::Range(difficulty, Value::integer(10).unwrap())); }
    Ok(Some(MemoryState { stability, difficulty: single(d) }))
}

/// VALUE as a finite number above zero.
fn positive(cx: &mut Leaf<'_>, value: Value) -> Result<f32> {
    let x = number(cx, value)?;
    if x > 0.0 && x.is_finite() { Ok(single(x)) } else { Err(Fault::Wrong("cl-plusp", value)) }
}

/// VALUE as a probability strictly between 0 and 1.
fn probability(cx: &mut Leaf<'_>, value: Value) -> Result<f32> {
    let x = number(cx, value)?;
    if x > 0.0 && x < 1.0 { Ok(single(x)) } else { Err(Fault::Range(value, Value::integer(1).unwrap())) }
}

/// VALUE as an integer in 0..=MAX.
fn whole(value: Value, max: u32) -> Result<u32> {
    value.integer_value()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|&n| n <= max)
        .ok_or(Fault::Wrong("natnump", value))
}

/// KEYS and their single-precision VALUES as a plist of floats.
fn plist(cx: &mut Leaf<'_>, keys: &[&str], values: &[f32]) -> Value {
    let mut items = vec![];
    for (key, &value) in keys.iter().zip(values) {
        items.push(cx.intern(key));
        items.push(cx.float(f64::from(value)));
    }
    list(cx, &items)
}

/// X at the crate's single precision, the nearest f32.
#[expect(clippy::cast_possible_truncation, reason = "the crate computes in f32; narrowing is its input contract")]
fn single(x: f64) -> f32 {
    x as f32
}

/// A crate refusal as a Lisp error.
fn failed(error: &::fsrs::FSRSError) -> Fault {
    Fault::Error(format!("FSRS: {error:?}"))
}

#[cfg(test)]
mod tests {
    fn eval(source: &str) -> String {
        let mut vm = crate::Vm::new();
        let value = vm.eval_source_raw(source).unwrap();
        vm.display(value).unwrap()
    }

    /// fsrs-rs's `test_memory_state`: Again, Good on the same day, then
    /// Good at 1, 3, 8 and 21 days, under the default weights.
    #[test]
    fn follows_the_crates_reference_review_sequence() {
        let shown = eval(
            "(let ((memory nil))
               (dolist (step '((1 . 0) (3 . 0) (3 . 1) (3 . 3) (3 . 8) (3 . 21)))
                 (let ((next (romacs-fsrs-next-state nil memory (cdr step) (car step) 0.9)))
                   (setq memory (cons (plist-get next :stability) (plist-get next :difficulty)))))
               (list (< (abs (- (car memory) 53.62691)) 1e-4)
                     (< (abs (- (cdr memory) 6.3574867)) 1e-4)))");
        assert_eq!(shown, "(t t)");
    }

    /// fsrs-rs's `test_next_interval`: stability 1 at retention 0.1..1.0.
    #[test]
    fn matches_the_crates_reference_intervals() {
        let shown = eval(
            "(let (out)
               (dolist (r '(9 8 7 6 5 4 3 2 1) out)
                 (setq out (cons (max 1 (round (romacs-fsrs-interval nil 1.0 (/ r 10.0)))) out))))");
        assert_eq!(shown, "(3116766 34793 2508 387 90 27 9 3 1)");
    }

    #[test]
    fn explicit_default_weights_equal_nil() {
        let shown = eval(
            "(let ((w [0.212 1.2931 2.3065 8.2956 6.4133 0.8334 3.0194 0.001 1.8722 0.1666 0.796
                      1.4835 0.0614 0.2629 1.6483 0.6014 1.8729 0.5425 0.0912 0.0658 0.1542]))
               (list (equal (romacs-fsrs-next-state w '(4.0 . 5.0) 3 3 0.9)
                            (romacs-fsrs-next-state nil '(4.0 . 5.0) 3 3 0.9))
                     (= (romacs-fsrs-retrievability w 4.0 4) (romacs-fsrs-retrievability nil 4.0 4))
                     (< (abs (- 0.9 (romacs-fsrs-retrievability nil 4.0 4))) 1e-6)))");
        assert_eq!(shown, "(t t t)");
    }

    #[test]
    fn rejects_malformed_input() {
        let shown = eval(
            "(let (out)
               (dolist (form '((romacs-fsrs-compute-parameters '(((3 . 2))))
                               (romacs-fsrs-interval [1 2 3] 1.0 0.9)
                               (romacs-fsrs-interval nil 1.0 1.0)
                               (romacs-fsrs-next-state nil nil -1 3 0.9)
                               (romacs-fsrs-next-state nil '(1.0 . 11) 1 3 0.9)
                               (romacs-fsrs-next-state nil '(0 . 5) 1 3 0.9)
                               (romacs-fsrs-next-state nil nil 0 0 0.9)
                               (romacs-fsrs-next-state nil nil 0 5 0.9))
                             out)
                 (setq out (cons (condition-case e (eval form t) (error (car e))) out))))");
        assert_eq!(shown, "(wrong-type-argument wrong-type-argument wrong-type-argument args-out-of-range \
                           wrong-type-argument args-out-of-range error wrong-type-argument)");
    }

    /// Few items keep the defaults; enough fit the initial stabilities.
    #[test]
    fn optimizer_counts_items_and_fits_what_the_data_supports() {
        let shown = eval(
            "(let ((few (romacs-fsrs-compute-parameters '(((3 . 0) (3 . 1) (3 . 0) (1 . 4)))))
                   (cards nil) (n 0))
               (while (< n 40)
                 (setq cards (cons (list (cons (if (= 0 (% n 3)) 1 3) 0)
                                         (cons (if (= 0 (% n 4)) 1 3) (1+ (% n 3))))
                                   cards)
                       n (1+ n)))
               (let ((many (romacs-fsrs-compute-parameters cards)))
                 (list (plist-get few :items) (length (plist-get few :parameters))
                       (< (abs (- (aref (plist-get few :parameters) 0) 0.212)) 1e-6)
                       (plist-get many :items)
                       (equal (substring (plist-get many :parameters) 4)
                              (substring (plist-get few :parameters) 4))
                       (equal (plist-get many :parameters) (plist-get few :parameters)))))");
        assert_eq!(shown, "(2 21 t 40 t nil)");
    }
}
