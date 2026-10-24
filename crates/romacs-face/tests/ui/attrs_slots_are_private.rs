// A face record is immutable once built: its slots are private.
fn edit(attrs: &mut romacs_face::Attrs) {
    attrs.slots[0] = romacs_face::Slot::Reset;
}

fn main() {}
