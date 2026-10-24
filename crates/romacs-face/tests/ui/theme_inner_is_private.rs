// A built theme cannot be edited in place: its table is private and shared.
fn edit(theme: &mut romacs_face::Theme) {
    let _ = std::sync::Arc::get_mut(&mut theme.0);
}

fn main() {}
