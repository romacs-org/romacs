//! Public Text contracts introduced by the S2 facade port.
use super::{BytePos, CharPos, Text, TextPos};

fn contents(text: &Text) -> Vec<u8> {
    let mut bytes = Vec::new();
    text.copy_to(BytePos(0), BytePos(text.len_bytes()), &mut bytes);
    bytes
}

#[test]
fn moving_gap_preserves_bytes_positions_markers_and_ticks() {
    let bytes = "aé\n€z".as_bytes();
    let mut text = Text::from_bytes(bytes, true);
    let markers: Vec<_> = [0, 1, 3, 4, 7, 8]
        .into_iter()
        .map(|byte| text.marker_new(BytePos(byte), byte % 2 == 0))
        .collect();
    let positions: Vec<_> = markers.iter().map(|&m| text.marker_pos(m)).collect();
    let ticks = (text.modiff(), text.chars_modiff(), text.epoch());
    // Cross the gap in both directions, at BOB/EOB and around multibyte/newline.
    for (byte, char) in [(0, 0), (7, 4), (1, 1), (8, 5), (3, 2), (4, 3), (0, 0)] {
        text.move_gap_to(BytePos(byte));
        assert_eq!(
            text.gap_start(),
            TextPos {
                byte: BytePos(byte),
                char: CharPos(char)
            }
        );
        assert_eq!(contents(&text), bytes);
        assert_eq!(
            (text.len_bytes(), text.len_chars(), text.len_newlines()),
            (8, 5, 1)
        );
        assert_eq!((text.modiff(), text.chars_modiff(), text.epoch()), ticks);
        for (&marker, &position) in markers.iter().zip(&positions) {
            assert_eq!(text.marker_pos(marker), position);
        }
        for (char, byte) in [0, 1, 3, 4, 7, 8].into_iter().enumerate() {
            assert_eq!(text.byte_of_char(CharPos(char)), BytePos(byte));
            assert_eq!(text.char_of_byte(BytePos(byte)), CharPos(char));
        }
        text.check_invariants();
    }
}

#[test]
fn contiguous_windows_cover_each_byte_without_lending_gap_capacity() {
    for (bytes, multibyte) in [(b"abcdef".as_slice(), false), ("aéλЖ🙂z".as_bytes(), true)] {
        let mut text = Text::from_bytes(bytes, multibyte);
        let ticks = (text.modiff(), text.chars_modiff(), text.epoch());
        for gap in 0..=bytes.len() {
            if !text.is_char_boundary(BytePos(gap)) {
                continue;
            }
            text.move_gap_to(BytePos(gap));
            for at in 0..bytes.len() {
                let (start, window) = text.contiguous_window_at(BytePos(at)).unwrap();
                assert!(start.0 <= at && at - start.0 < window.len());
                assert_eq!(window, &bytes[start.0..start.0 + window.len()]);
                assert_eq!(window[at - start.0], bytes[at]);
                assert!(start.0 + window.len() <= gap || start.0 >= gap);
            }
            assert!(text.contiguous_window_at(BytePos(bytes.len())).is_none());
            assert!(text.contiguous_window_at(BytePos(usize::MAX)).is_none());
            assert_eq!(text.gap_start().byte, BytePos(gap));
            assert_eq!((text.modiff(), text.chars_modiff(), text.epoch()), ticks);
        }
    }
    assert!(Text::new(true).contiguous_window_at(BytePos(0)).is_none());
}

#[test]
#[should_panic]
fn moving_gap_rejects_an_interior_multibyte_position() {
    let mut text = Text::from_bytes("é".as_bytes(), true);
    text.move_gap_to(BytePos(1));
}

#[test]
fn multibyte_reinterpretation_reindexes_marker_bytes_and_characters() {
    let bytes = "aé\n€z".as_bytes();
    let mut text = Text::from_bytes(bytes, false);
    let markers: Vec<_> = (0..=bytes.len())
        .map(|byte| text.marker_new(BytePos(byte), byte % 2 == 0))
        .collect();
    // This gap starts in what will become the middle of the euro character.
    text.move_gap_to(BytePos(5));
    let ticks = (text.modiff(), text.chars_modiff());
    text.set_multibyte(true);
    assert!(text.is_multibyte());
    assert_eq!(contents(&text), bytes);
    assert_eq!(
        (text.len_bytes(), text.len_chars(), text.len_newlines()),
        (8, 5, 1)
    );
    assert_eq!(
        text.gap_start(),
        TextPos {
            byte: BytePos(4),
            char: CharPos(3)
        }
    );
    let expected = [
        (0, 0),
        (1, 1),
        (1, 1),
        (3, 2),
        (4, 3),
        (4, 3),
        (4, 3),
        (7, 4),
        (8, 5),
    ];
    for (&marker, (byte, char)) in markers.iter().zip(expected) {
        assert_eq!(
            text.marker_pos(marker),
            Some(TextPos {
                byte: BytePos(byte),
                char: CharPos(char)
            })
        );
    }
    assert_eq!((text.modiff(), text.chars_modiff()), ticks);
    text.check_invariants();
    // Same-mode conversion is a no-op, including markers and the gap.
    let epoch = text.epoch();
    text.set_multibyte(true);
    assert_eq!(text.epoch(), epoch);
    text.set_multibyte(false);
    assert!(!text.is_multibyte());
    assert_eq!(contents(&text), bytes);
    assert_eq!(
        (text.len_bytes(), text.len_chars(), text.len_newlines()),
        (8, 8, 1)
    );
    for (&marker, (byte, _)) in markers.iter().zip(expected) {
        assert_eq!(
            text.marker_pos(marker),
            Some(TextPos {
                byte: BytePos(byte),
                char: CharPos(byte)
            })
        );
    }
    assert_eq!((text.modiff(), text.chars_modiff()), ticks);
    text.check_invariants();
}

#[test]
fn primitive_edits_bump_once_with_logarithmic_character_size() {
    let mut text = Text::new(true);
    // Four characters, not seven bytes: floor(log2(4)) + 1 == 3.
    let before = text.modiff();
    text.insert(BytePos(0), "é€ab".as_bytes(), false);
    assert_eq!(text.modiff() - before, 3);
    assert_eq!(text.chars_modiff(), text.modiff());
    let before = text.modiff();
    // Replacement counts two deleted plus two inserted characters once.
    text.replace(BytePos(0), BytePos(5), "XY".as_bytes());
    assert_eq!(text.modiff() - before, 3);
    assert_eq!(text.chars_modiff(), text.modiff());
    let before = text.modiff();
    text.delete(BytePos(0), BytePos(4));
    assert_eq!(text.modiff() - before, 3);
    assert_eq!(text.chars_modiff(), text.modiff());
    let before = (text.modiff(), text.chars_modiff(), text.epoch());
    text.insert(BytePos(0), b"", false);
    text.delete(BytePos(0), BytePos(0));
    text.replace(BytePos(0), BytePos(0), b"");
    assert_eq!((text.modiff(), text.chars_modiff(), text.epoch()), before);
    text.check_invariants();
}
