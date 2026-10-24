use romacs_chars::{BytePos, CharPos};
use romacs_text::Text;

fn verify(t: &Text, model: &[u8]) {
    let mut got = vec![];
    t.copy_to(BytePos(0), BytePos(t.len_bytes()), &mut got);
    assert_eq!(got, model);
    let (mut chars, mut lines) = (0, 0);
    for (i, &b) in model.iter().enumerate() {
        assert_eq!(t.newlines_before(BytePos(i)), lines);
        if romacs_chars::char_head_p(b) {
            assert_eq!(t.byte_of_char(CharPos(chars)), BytePos(i));
            assert_eq!(t.char_of_byte(BytePos(i)), CharPos(chars));
            chars += 1;
        }
        lines += usize::from(b == b'\n');
    }
    assert_eq!(t.newlines_before(BytePos(model.len())), lines);
    assert_eq!(t.byte_of_char(CharPos(chars)), BytePos(model.len()));
    t.check_invariants();
}

#[test]
fn tiny_streaming_inserts_across_production_splits() {
    for suffix in ["", "λ\n> "] {
        let mut model = "αx\n".repeat(16384).into_bytes();
        model.extend_from_slice(suffix.as_bytes());
        let mut text = Text::from_bytes(&model, true);
        text.move_gap_to(BytePos(model.len() - suffix.len()));
        for step in 0..32768 {
            let at = text.gap_start();
            text.insert_measured(at, "β\n".as_bytes(), 2, false);
            model.splice(at.byte.0..at.byte.0, "β\n".bytes());
            assert_eq!(
                text.char_of_byte(text.gap_start().byte),
                text.gap_start().char
            );
            assert_eq!(
                text.byte_of_char(text.gap_start().char),
                text.gap_start().byte
            );
            if step % 1024 == 0 {
                text.check_invariants();
            }
        }
        let mut got = vec![];
        text.copy_to(BytePos(0), BytePos(text.len_bytes()), &mut got);
        assert_eq!(got, model);
        assert_eq!(
            text.len_newlines(),
            16384 + 32768 + usize::from(!suffix.is_empty())
        );
        text.check_invariants();
    }
}

#[test]
fn split_merge_preserves_chunk_count() {
    // Keep the generic split/merge fixture above the production 2T bound.
    const SCALE: usize = 4;
    let mut model = vec![b'x'; 8194 * SCALE];
    let mut t = Text::from_bytes(&model, true);
    verify(&t, &model);
    t.insert(BytePos(0), &vec![b'x'; 8193 * SCALE], false);
    model.splice(0..0, vec![b'x'; 8193 * SCALE]);
    verify(&t, &model);
    t.insert(BytePos(12289 * SCALE), &vec![b'\n'; 4095 * SCALE], false);
    model.splice(12289 * SCALE..12289 * SCALE, vec![b'\n'; 4095 * SCALE]);
    assert_eq!(t.newlines_before(BytePos(16386 * SCALE)), 4095 * SCALE);
    verify(&t, &model);
}
