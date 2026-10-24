// Review reproducer: safe native payload with a cons edge outside its Slot prefix.
struct P(raw::RawValue);
impl Payload for P {
    fn bytes(&self) -> usize {
        let _ = self.0;
        0
    }
}
let child = h.cons(h.nil(), h.nil()).unwrap();
let raw = raw::RawValue::from_value(child);
let container = h.make_payload(PvecType::UserPtr, &[], Box::new(P(raw))).unwrap();
let v = h.as_vectorlike(container).unwrap();
assert_eq!(v.len(), 0);
assert_eq!(v.payload().unwrap().bytes(), 0);
