use super::*;

fn frames(kind: Kind, limit: Option<usize>, chunks: &[&[u8]]) -> Vec<Frame> {
    let mut framer = Framer::new(kind, limit);
    let mut out = Vec::new();
    for chunk in chunks {
        framer.push(chunk, &mut out);
    }
    out
}

fn bytes(s: &str) -> Frame {
    Frame::Bytes(s.as_bytes().to_vec())
}

#[test]
fn lines_split_across_chunks() {
    let out = frames(Kind::Lines, None, &[b"{\"a\"", b":1}\n{\"b\":2}\n{\"c", b"\":3}\n"]);
    assert_eq!(out, vec![bytes("{\"a\":1}"), bytes("{\"b\":2}"), bytes("{\"c\":3}")]);
}

#[test]
fn lines_keep_empty_lines_and_cr() {
    assert_eq!(frames(Kind::Lines, None, &[b"a\r\n\nb\n"]), vec![bytes("a\r"), bytes(""), bytes("b")]);
}

#[test]
fn lines_hold_a_partial_until_its_end() {
    let mut framer = Framer::new(Kind::Lines, None);
    let mut out = Vec::new();
    framer.push(b"abc", &mut out);
    framer.push(b"def", &mut out);
    assert!(out.is_empty());
    framer.push(b"\nx", &mut out);
    assert_eq!(out, vec![bytes("abcdef")]);
    assert_eq!(framer.into_rest(), b"x");
}

#[test]
fn lines_at_the_limit_and_one_over() {
    assert_eq!(frames(Kind::Lines, Some(4), &[b"abcd\n"]), vec![bytes("abcd")]);
    assert_eq!(frames(Kind::Lines, Some(4), &[b"abcde\nok\n"]), vec![Frame::Over, bytes("ok")]);
    // At the limit with its LF still to come: not yet over.
    assert_eq!(frames(Kind::Lines, Some(4), &[b"ab", b"cd", b"\n"]), vec![bytes("abcd")]);
}

#[test]
fn lines_over_the_limit_are_never_held() {
    let mut framer = Framer::new(Kind::Lines, Some(4));
    let mut out = Vec::new();
    framer.push(b"abcdef", &mut out);
    assert_eq!(out, vec![Frame::Over]);
    framer.push(&[b'x'; 100], &mut out);
    assert!(framer.buf.is_empty(), "a skipped line is not buffered");
    framer.push(b"yy\nok\n", &mut out);
    assert_eq!(out, vec![Frame::Over, bytes("ok")]);
}

#[test]
fn lines_multibyte_split_across_chunks() {
    let text = "λ→€😀";
    let line = format!("{text}\n");
    let raw = line.as_bytes();
    for cut in 1..raw.len() {
        let out = frames(Kind::Lines, None, &[&raw[..cut], &raw[cut..]]);
        assert_eq!(out, vec![bytes(text)], "cut at {cut}");
    }
}

#[test]
fn lines_last_line_without_newline_at_eof() {
    let mut framer = Framer::new(Kind::Lines, None);
    let mut out = Vec::new();
    framer.push(b"a\nlast", &mut out);
    framer.finish(&mut out);
    assert_eq!(out, vec![bytes("a"), bytes("last")]);
}

fn message(body: &str) -> Vec<u8> {
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

#[test]
fn content_length_frames_byte_by_byte() {
    let stream = [message("{\"id\":1}"), message("{\"id\":2}")].concat();
    let chunks: Vec<&[u8]> = stream.chunks(1).collect();
    assert_eq!(frames(Kind::ContentLength, None, &chunks), vec![bytes("{\"id\":1}"), bytes("{\"id\":2}")]);
}

#[test]
fn content_length_crlf_split_across_chunks() {
    let stream = message("{}");
    for cut in 1..stream.len() {
        let out = frames(Kind::ContentLength, None, &[&stream[..cut], &stream[cut..]]);
        assert_eq!(out, vec![bytes("{}")], "cut at {cut}");
    }
}

#[test]
fn content_length_other_headers_and_case() {
    let stream = b"content-length: 2\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{}";
    assert_eq!(frames(Kind::ContentLength, None, &[stream]), vec![bytes("{}")]);
}

#[test]
fn content_length_counts_bytes_not_characters() {
    let body = "{\"t\":\"λ😀\"}";
    let stream = message(body);
    let (a, b) = stream.split_at(stream.len() - 3);
    assert_eq!(frames(Kind::ContentLength, None, &[a, b]), vec![bytes(body)]);
}

#[test]
fn content_length_at_the_limit_and_one_over() {
    assert_eq!(frames(Kind::ContentLength, Some(2), &[&message("{}")]), vec![bytes("{}")]);
    let stream = [message("{ }"), message("{}")].concat();
    let chunks: Vec<&[u8]> = stream.chunks(2).collect();
    assert_eq!(frames(Kind::ContentLength, Some(2), &chunks), vec![Frame::Over, bytes("{}")]);
}

#[test]
fn content_length_stray_block_is_dropped() {
    let stream = [b"garbage\r\n\r\n".to_vec(), message("{}")].concat();
    assert_eq!(frames(Kind::ContentLength, None, &[&stream]), vec![bytes("{}")]);
}

#[test]
fn content_length_endless_header_is_malformed() {
    let mut framer = Framer::new(Kind::ContentLength, None);
    let mut out = Vec::new();
    framer.push(&vec![b'x'; HEADER_LIMIT + 1], &mut out);
    framer.push(&message("{}"), &mut out);
    assert_eq!(out, vec![Frame::Malformed]);
    assert!(framer.buf.is_empty());
}

#[test]
fn content_length_incomplete_body_dropped_at_eof() {
    let mut framer = Framer::new(Kind::ContentLength, None);
    let mut out = Vec::new();
    framer.push(b"Content-Length: 5\r\n\r\n{}", &mut out);
    framer.finish(&mut out);
    assert!(out.is_empty());
}

#[test]
fn a_long_line_in_small_chunks_moves_each_byte_once() {
    // Linear: the partial is never copied while it grows.
    let mut framer = Framer::new(Kind::Lines, None);
    let mut out = Vec::new();
    let chunk = [b'y'; 4096];
    for _ in 0..256 {
        framer.push(&chunk, &mut out);
        assert_eq!(framer.start, 0);
    }
    framer.push(b"\n", &mut out);
    assert!(matches!(&out[..], [Frame::Bytes(b)] if b.len() == 256 * 4096));
}

#[test]
fn content_length_header_limit_whatever_the_reads() {
    // A header block over the limit is malformed whether its end came in
    // the same read or a later one.
    let mut stream = b"X-Pad: ".to_vec();
    stream.extend(vec![b'p'; HEADER_LIMIT + 100]);
    stream.extend_from_slice(b"\r\nContent-Length: 2\r\n\r\n{}");
    assert_eq!(frames(Kind::ContentLength, None, &[&stream]), vec![Frame::Malformed]);
    let (a, b) = stream.split_at(HEADER_LIMIT + 1);
    assert_eq!(frames(Kind::ContentLength, None, &[a, b]), vec![Frame::Malformed]);
}

#[test]
fn content_length_rest_keeps_its_header() {
    // Framing stopped in a body gives back the header with the body so far.
    let mut framer = Framer::new(Kind::ContentLength, None);
    let mut out = Vec::new();
    framer.push(b"Content-Length: 5\r\n\r\nab", &mut out);
    assert!(out.is_empty());
    assert_eq!(framer.into_rest(), b"Content-Length: 5\r\n\r\nab");
}

#[test]
fn a_new_limit_keeps_the_frame_in_progress() {
    let mut framer = Framer::new(Kind::ContentLength, None);
    let mut out = Vec::new();
    framer.push(b"Content-Length: 5\r\n\r\nab", &mut out);
    framer.set_limit(Some(100));
    framer.push(b"cde", &mut out);
    assert_eq!(out, vec![bytes("abcde")]);
}

#[test]
fn a_large_frame_gone_gives_its_buffer_back() {
    let mut framer = Framer::new(Kind::Lines, None);
    let mut out = Vec::new();
    framer.push(&vec![b'x'; 4 << 20], &mut out);
    framer.push(b"\nsmall", &mut out);
    assert!(framer.buf.capacity() <= KEPT_CAPACITY);
}
