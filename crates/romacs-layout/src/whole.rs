//! A window laid out whole: every face of its table realised into the
//! frame, every character of its published text walked from the start of
//! its start's line, rows broken and glyphs emitted in one loop
//! (`append_window'), and the measurement that shares its geometry
//! (`measure_inner').
use super::*;

/// Paint SNAPSHOT's window at OX, OY, and report its rows (`WindowEnd`).
pub(super) fn append_window(snapshot: &WindowSnapshot, frame: &mut FrameGlyphBuffer, resolver: &mut FontResolver, ox: f32, oy: f32, cursor: Option<(bool, Color)>, no_font: &NoFont) -> Result<WindowEnd, String> {
    let text = &snapshot.text.utf8;
    if snapshot.height == 0
        || snapshot.faces.is_empty()
        || !text.is_char_boundary(snapshot.start_byte)
        || !text.is_char_boundary(snapshot.point_byte)
    {
        return Err("invalid snapshot dimensions, positions, face table or presentation".into());
    }
    // A body width of zero is legal, and refusing it would kill redisplay.
    //
    // `width` here is already the *body* width: the publisher subtracts the
    // margins (romacs-vm/src/window.rs). GNU validates margins once against
    // the window they are assigned to (src/window.c:8477), copies them to the
    // new window on split without re-validating (:5610-5612), and clamps the
    // resulting body width at zero under an explicit "Don't return a negative
    // value" (:1136-1137) -- redisplay continues. Measured on a 640px root at
    // an 8px cell: margins (30 . 30) leave a 160px body, and splitting into two
    // 320px windows leaves both bodies at 0 with redisplay still returning.
    //
    // So an empty body paints nothing and displays nothing, which makes
    // window-end equal window-start. Returning Err here instead propagated out
    // of the GUI loop and terminated redisplay -- the failure mode s121 was
    // repaired to prevent. A window whose *total* geometry does not fit its
    // parent is still rejected, by layout_frame's placement check.
    if snapshot.width == 0 {
        return Ok(WindowEnd { id: snapshot.id, end: snapshot.start_byte, visible_end: snapshot.start_byte, body_height: 0.0, rows: vec![], point: None });
    }
    validate_runs(&snapshot.text, snapshot.faces.len())?;
    if snapshot.stretches.windows(2).any(|w| w[0].byte >= w[1].byte)
        || snapshot.stretches.iter().any(|s| text.get(s.byte..s.byte + 1) != Some(" ")) {
        return Err("stretches out of order or not on a placeholder space".into());
    }
    if snapshot.images.windows(2).any(|w| w[0].byte >= w[1].byte)
        || snapshot.images.iter().any(|i| text.get(i.byte..i.byte + 1) != Some(" ")) {
        return Err("images out of order or not on a placeholder space".into());
    }
    let mut stretches = snapshot.stretches.iter().peekable();
    let mut images = snapshot.images.iter().peekable();
    // The row's images so far: their glyphs' indexes and ascents.
    let mut row_images: Vec<(usize, f32)> = vec![];
    let bands = [(&snapshot.header_line, snapshot.header_line_face), (&snapshot.mode_line, snapshot.mode_line_face)];
    for (band, face) in bands {
        let Some(band) = band else { continue };
        validate_runs(band, snapshot.faces.len())?;
        if face.0 as usize >= snapshot.faces.len() || band.utf8.chars().any(char::is_control) {
            return Err("invalid mode-line face or control character".into());
        }
    }
    // Above every face id an earlier window took, its band's copies included.
    let face_base = frame.faces.keys().map(|id| id.get() + 1).max().unwrap_or(0);
    let mut slices = vec![(&snapshot.text, 0)];
    slices.extend(bands.iter().filter_map(|(band, face)| Some(((*band).as_ref()?, face.0 as usize))));
    let mut face_text = face_text(snapshot.faces.len(), &slices);
    if let Some(numbers) = &snapshot.line_numbers {
        if numbers.face.0 as usize >= snapshot.faces.len() || numbers.current.0 as usize >= snapshot.faces.len() {
            return Err("invalid line-number face".into());
        }
        for face in [numbers.face, numbers.current] { face_text[face.0 as usize].push_str("0123456789 "); }
    }
    // A display table's truncation and continuation glyphs show in the
    // default face, from whichever font has them.
    if let Some(default) = face_text.first_mut() { default.extend(snapshot.chrome.line_glyphs.iter().flatten()); }
    // A band's own face sets its height even where no character uses it,
    // as the faces of the fringes' bitmaps set their colours.
    let band_faces: Vec<usize> = bands.iter().filter(|(band, _)| band.is_some()).map(|(_, face)| face.0 as usize)
        .chain(fringe::bitmap_faces(&snapshot.chrome))
        .chain(snapshot.text.extends.iter().map(|e| e.face.0 as usize)).collect();
    if snapshot.text.extends.iter().any(|e| e.face.0 as usize >= snapshot.faces.len())
        || snapshot.text.extends.windows(2).any(|w| w[0].byte >= w[1].byte) {
        return Err("invalid extend face".into());
    }
    let (metrics, glyphless) = realize_faces(&snapshot.faces, &face_text, |index| band_faces.contains(&index), frame, resolver, face_base, snapshot.underline_rules, no_font)?;
    if face_base == 0 { frame.char_height = metrics[0].1; frame.char_width = metrics[0].2; }
    // A band is as tall as its tallest face.
    let band_height = |(band, face): (&Option<TextSlice>, FaceHandle)| band.as_ref().map_or(0.0, |band| {
        std::iter::once(face.0 as usize)
            .chain(band.face_runs.iter().map(|run| run.face.0 as usize))
            .map(|index| metrics[index].1)
            .fold(0.0_f32, f32::max)
    });
    let (header_height, mode_height) = (band_height(bands[0]), band_height(bands[1]));
    let body_height = (snapshot.height as f32 - header_height - mode_height).max(0.0);
    // The text area starts below the header line: OY is its origin from here.
    let (window_top, oy) = (oy, oy + header_height);
    frame.set_draw_context(
        DisplayWindowId::new(i64::try_from(snapshot.id).map_err(|e| e.to_string())?),
        GlyphRowRole::Text,
        Some(Rect::new(ox, oy, snapshot.width as f32, body_height)),
    );
    // GNU: a row ends where its next glyph would pass the text area,
    // less the continuation (`\\') or truncation (`$') glyph it shows there
    // on a window without a right fringe, whose indicators show the break
    // instead (display_line, move_it_in_display_line_to, init_iterator).
    // Rows are laid out from the start of the window start's line, so a
    // window starting on a continuation line breaks its rows, and stops
    // its TABs, as from the line start.
    // A horizontally scrolled window truncates its lines (init_iterator).
    let truncate = snapshot.wrap == LineWrap::Truncate || !snapshot.hscroll.is_zero();
    // A display table's glyph, where it sets one, for GNU's `$' and `\\'.
    let [truncation, continuation] = snapshot.chrome.line_glyphs;
    let special = if truncate { truncation.unwrap_or('$') } else { continuation.unwrap_or('\\') };
    let default_id = FaceId::new(face_base);
    let glyphs_mark = snapshot.chrome.right_fringe == 0;
    let last_x = snapshot.width as f32 - if glyphs_mark { advance(frame, default_id, special, metrics[0].2) } else { 0.0 };
    let line_start = text[..snapshot.start_byte].rfind('\n').map_or(0, |at| at + 1);
    // Each line shows from its first visible x (GNU's first_visible_x):
    // point's line from the hscroll of point's row. Its rows are truncated.
    let point_line = text[..snapshot.point_byte].rfind('\n').map_or(0, |at| at + 1);
    let shift_of = |line: usize| if line == point_line { snapshot.hscroll.point_row } else { snapshot.hscroll.rows } as f32;
    // A scrolled row's left truncation glyph, without a left fringe
    // (insert_left_trunc_glyphs): it hides the glyphs it covers.
    let left_glyph = truncation.unwrap_or('$');
    let dollar = if snapshot.chrome.left_fringe == 0 { advance(frame, default_id, left_glyph, metrics[0].2) } else { 0.0 };
    let left_mark = |shift: f32| if shift > 0.0 { dollar } else { 0.0 };
    // Line numbers take each row's first glyphs, its line's label or as
    // many blanks, in cells of their face.
    // (x, the label when the row starts its line, whether point's line).
    let gutter = |byte: usize| -> (f32, Option<&str>, bool) {
        let Some(numbers) = &snapshot.line_numbers else { return (0.0, None, false) };
        let cell = metrics[numbers.face.0 as usize].2;
        let at = numbers.labels.partition_point(|(b, ..)| *b <= byte);
        match at.checked_sub(1).map(|i| &numbers.labels[i]) {
            Some((b, label, current)) if *b == byte => (label.len() as f32 * cell, Some(label.as_str()), *current),
            Some((_, label, current)) => (label.len() as f32 * cell, None, *current),
            None => (numbers.columns as f32 * cell, None, false),
        }
    };
    // A `word-wrap' window's rows break where GNU's word wrap breaks them.
    let word = snapshot.wrap == LineWrap::Word && !truncate;
    // Text that shows no box skips the box runs' work (`box_step').
    let boxes = shows_box(&snapshot.faces, &snapshot.text);
    let mut breaks = WordBreaks { row: line_start, continued: 0.0, next: None, done: !word, boxes, box_open: false };
    let mut x0 = gutter(line_start).0;
    let mut x = x0;
    let mut shift = shift_of(line_start);
    // Where point is (`PointPlace'), and where the row being built was
    // truncated on the right, unscrolled.
    let mut place: Option<PointPlace> = None;
    let mut cut = 0.0f32;
    // The cursor's x on a row scrolled by SHIFT for a position at
    // unscrolled X: on the truncation glyph past the row's last position,
    // or on its first glyph when scrolled out on the left
    // (set_cursor_from_row's glyph_after and glyph_before).
    let cursor_x = |x: f32, x0: f32, shift: f32, hidden: bool, cut: f32| {
        if hidden { cut - shift } else if x < x0 + shift { x0 } else { x - shift }
    };
    let mut y = -(snapshot.vscroll_px as f32);
    let mut row_height = metrics[0].1;
    let mut end_byte = snapshot.start_byte;
    let mut point_rect = None;
    // The width of the rows of this line above this one, and the glyphs
    // on this row; past the truncation glyph, the rest of the line is hidden.
    let (mut continued, mut hpos, mut hidden) = (0.0f32, 0usize, false);
    // Whether the glyph before is in a box run; the tallest box top on the
    // row being built, and where its glyphs start (`settle_row').
    let mut box_open = false;
    let (mut row_box_top, mut row_glyphs) = (0.0f32, frame.glyphs.len());
    // A character on this row is in a font of other metrics than its face's.
    let mut row_mixed = false;
    let mut cluster_rise = (metrics[0].0, metrics[0].1);
    let mut drawing = line_start == snapshot.start_byte;
    // The first byte of the row being built, and the end of the rows
    // above it that show whole: a row is fully visible when it lies
    // within the text area (GNU's cursor_row_fully_visible_p).
    let (mut row_start, mut visible_end) = (snapshot.start_byte, snapshot.start_byte);
    let fully_visible = |y: f32, height: f32| y >= 0.0 && y + height <= body_height;
    // Each drawn row as it ends; the last, as the text ends. Beside each,
    // how it breaks, which the fringes indicate; whether the row being
    // built continues a line a break or the window start split.
    let mut rows: Vec<Row> = vec![];
    let mut marks: Vec<RowMarks> = vec![];
    let mut continuation = line_start != snapshot.start_byte;
    // The end of the composed sequence being laid out, and its glyph's x
    // and advance.
    let (mut inside, mut composed) = (0, (0.0f32, 0.0f32));
    for (byte, ch) in text[line_start..].char_indices().map(|(at, ch)| (line_start + at, ch)) {
        // The window start begins a row: at a line start, or where the row
        // before it breaks, which the character there decides below, so
        // that row's width counts toward `continued' first.
        let mut starting = !drawing && byte >= snapshot.start_byte;
        let top = -(snapshot.vscroll_px as f32);
        if starting && (x == x0 || ch == '\n') {
            (drawing, starting) = (true, false);
            x0 = gutter(byte).0;
            (x, y, row_height, hpos) = (x0, top, metrics[0].1, 0);
        }
        if drawing && y >= body_height { break; }
        if drawing || starting { end_byte = byte + ch.len_utf8(); }
        let at_point = (drawing || starting) && byte == snapshot.point_byte;
        if at_point && ch == '\n' {
            point_rect = Some((cursor_x(x, x0, shift, hidden, cut), y, metrics[0].2, row_height, metrics[0].0));
            place = Some(PointPlace { x: cursor_x(x, x0, shift, hidden, cut), line_x: x + continued, gutter: x0,
                truncated_left: shift > 0.0, truncated_right: hidden, at_eol: true });
        }
        if ch == '\n' {
            if drawing {
                // xdisp.c extend_face_to_end_of_line: the row paints past
                // its text, to the text area's edge, in its extend face.
                if let Some(extend) = snapshot.text.extends.binary_search_by_key(&(byte as u32), |e| e.byte).ok().map(|i| snapshot.text.extends[i]) {
                    let index = extend.face.0 as usize;
                    let from = (x - shift).max(x0);
                    let to = snapshot.width as f32;
                    if !hidden && body_height > 0.0 && y < body_height && y + row_height > 0.0 && to > from {
                        frame.add_stretch(ox + from, oy + y, to - from, row_height, color(snapshot.faces[index].background),
                                          FaceId::new(face_base + index as u32), false);
                    }
                }
                if visible_end == row_start && fully_visible(y, row_height) { visible_end = byte + 1; }
                row_height = settle_row(frame, row_glyphs, oy + y, row_height, row_box_top > 0.0 || row_mixed, &row_images, &metrics, face_base, (&mut point_rect, y));
                (row_box_top, row_mixed, row_glyphs) = (0.0, false, frame.glyphs.len());
                row_images.clear();
                rows.push(Row { start: row_start, y, height: row_height });
                marks.push(RowMarks { continued: false, continuation, truncated: hidden, truncated_left: shift > 0.0 });
                if shift > 0.0 && left_mark(shift) > 0.0 && y < body_height && y + metrics[0].1 > 0.0 {
                    glyph(frame, left_glyph, ox + x0, oy + y, metrics[0], default_id)?;
                    // The glyph is this row's: the next row settles its own.
                    row_glyphs = frame.glyphs.len();
                }
                if let Some(place) = place.as_mut().filter(|_| row_start <= snapshot.point_byte && snapshot.point_byte <= byte) {
                    place.truncated_right = hidden;
                }
                row_start = byte + 1;
            }
            continuation = false;
            box_open = false;
            x0 = gutter(byte + 1).0;
            x = x0;
            shift = shift_of(byte + 1);
            y += row_height;
            row_height = metrics[0].1;
            (continued, hpos, hidden) = (0.0, 0, false);
            continue;
        }
        while stretches.next_if(|s| s.byte < byte).is_some() {}
        let stretch = stretches.next_if(|s| s.byte == byte).map(|s| s.width);
        while images.next_if(|i| i.byte < byte).is_some() {}
        let image = images.next_if(|i| i.byte == byte).copied();
        // Past the truncation, only point's x on its line still counts.
        let measuring = hidden && drawing && row_start >= point_line && byte < snapshot.point_byte;
        if hidden && !measuring {
            if at_point {
                point_rect = Some((cut - shift, y, metrics[0].2, row_height, metrics[0].0));
                place = Some(PointPlace { x: cut - shift, line_x: x + continued, gutter: x0, truncated_left: shift > 0.0, truncated_right: true, at_eol: false });
            }
            continue;
        }
        let index = face_at(&snapshot.text, byte, 0);
        let id = FaceId::new(face_base + index as u32);
        let (ascent, height, cell, space) = metrics[index];
        let shown = match image {
            Some(image) => Shown::Image(image),
            None => shown_at(&snapshot.text, byte, ch, stretch, &mut inside, frame, id, cell),
        };
        // An image's slot is its own height (produce_image_glyph).
        let (ascent, height) = match &shown {
            Shown::Image(i) => (i.ascent as f32, (i.ascent + i.descent) as f32),
            _ if escaped(&snapshot.text, byte, ch) => (ascent, height),
            _ => shown_rise(frame, id, (&snapshot.text.utf8, byte, ch), &shown, (ascent, height), &mut cluster_rise),
        };
        let own_mixed = (ascent, height) != (metrics[index].0, metrics[index].1) && !matches!(shown, Shown::Image(_));
        // X on the row and from the line's start (`continued' before it);
        // a TAB stops as from after the line number (x_produce_glyphs).
        let width = |frame: &FrameGlyphBuffer, x: f32, line_x: f32, x0: f32|
            shown_advance(frame, &shown, id, cell, space, snapshot.tab_width, x, line_x, x0, (!truncate).then_some(last_x));
        // A box run (xdisp.c start_of_box_run_p, end_of_box_run_p): its
        // text's first and last glyphs own its left and right edges; edges
        // outside the text are spacers of their width, the run's advance.
        let step = if boxes { box_step(&snapshot.faces, &snapshot.text, (0, index), byte, ch, shows_glyphs(&shown), box_open, false) } else { BoxStep::NONE };
        // The run's state past this character, on every path below; a row
        // break alone opens no new run (the iterator's box state carries).
        let opening = !box_open;
        box_open = step.open;
        let (lead, trail) = (step.lead, step.trail);
        let mut advance_px = width(frame, x, x + continued, x0) + lead + trail;
        if measuring { x += advance_px; continue; }
        // A word-wrapped row breaks where `WordBreaks' finds; another, at
        // the glyph that would pass the text area.
        // No row breaks inside a composed sequence.
        let breaking = if word { breaks.at(byte, snapshot, frame, &metrics, face_base, last_x, &|byte| gutter(byte).0) }
                       else { !truncate && hpos > 0 && x + advance_px > last_x && !matches!(shown, Shown::Inside) };
        if breaking {
            if glyphs_mark && drawing && y < body_height && y + metrics[0].1 > 0.0 { glyph(frame, special, ox + x, oy + y, metrics[0], default_id)?; }
            if drawing {
                if visible_end == row_start && fully_visible(y, row_height) { visible_end = byte; }
                row_height = settle_row(frame, row_glyphs, oy + y, row_height, row_box_top > 0.0 || row_mixed, &row_images, &metrics, face_base, (&mut point_rect, y));
                (row_box_top, row_mixed, row_glyphs) = (0.0, false, frame.glyphs.len());
                row_images.clear();
                rows.push(Row { start: row_start, y, height: row_height });
                marks.push(RowMarks { continued: true, continuation, truncated: false, truncated_left: false });
                row_start = byte;
            }
            continuation = true;
            // A TAB that does not fit continues from the text area's edge.
            continued += if matches!(shown, Shown::Tab) { last_x } else { x };
            x0 = gutter(byte).0;
            (x, y, row_height, hpos) = (x0, y + row_height, metrics[0].1, 0);
            if starting { (drawing, starting, y) = (true, false, top); }
            // Continued past the last row, this character is not shown.
            if drawing && y >= body_height { end_byte = byte; }
            advance_px = width(frame, x0, x0 + continued, x0) + lead + trail;
            if at_point { point_rect = Some((x, y, metrics[0].2, row_height, metrics[0].0)); }
        }
        if starting {
            // A start inside a row (the motion refuses one): the row
            // breaks there.
            continuation = true;
            continued += x;
            x0 = gutter(byte).0;
            (drawing, x, y, row_height, hpos) = (true, x0, top, metrics[0].1, 0);
            advance_px = width(frame, x0, x0 + continued, x0) + lead + trail;
            if at_point { point_rect = Some((x, y, metrics[0].2, row_height, metrics[0].0)); }
        }
        row_height = row_height.max(height);
        row_mixed |= own_mixed;
        let visible = drawing && body_height > 0.0 && y < body_height && y + height > 0.0 && x - shift < snapshot.width as f32;
        if truncate && x + advance_px > last_x + shift && !matches!(shown, Shown::Inside) {
            if visible && glyphs_mark { glyph(frame, special, ox + x - shift, oy + y, metrics[0], default_id)?; }
            (hidden, cut) = (true, x);
            if at_point {
                point_rect = Some((x - shift, y, metrics[0].2, row_height, ascent));
                place = Some(PointPlace { x: x - shift, line_x: x + continued, gutter: x0, truncated_left: shift > 0.0, truncated_right: true, at_eol: false });
            }
            // The rest of the line still counts toward point's x on it.
            x += advance_px;
            continue;
        }
        // Scrolled out on the left, or under the left truncation glyph; a
        // glyph partly out shows its rest, but none overlaps a line number.
        let left_edge = x0 + shift + left_mark(shift);
        let visible = visible && x + advance_px > x0 + shift && (x >= left_edge || (snapshot.line_numbers.is_none() && left_mark(shift) == 0.0));
        let ox = ox - shift;
        use romacs_display_protocol::BoxVerticalEdges as Edges;
        // A stretch in a box run owns the run's edges at its ends; its
        // advance has the sides in it.
        let stretch_edges = |frame: &mut FrameGlyphBuffer| if step.boxed { set_box_edges(frame, edges(opening, step.ends)); };
        let cursor = match &shown {
            Shown::Tab => {
                if visible {
                    frame.add_stretch(ox + x, oy + y, advance_px, height, color(snapshot.faces[index].background), id, false);
                    stretch_edges(frame);
                }
                // GNU's cursor on a stretch is no wider than a column
                // (`x-stretch-cursor' nil).
                advance_px.min(metrics[0].2)
            }
            // A stretch that is not positive is no glyph (nglyphs 0).
            Shown::Stretch(_) => {
                if visible && advance_px > 0.0 {
                    frame.add_stretch(ox + x, oy + y, advance_px, height, color(snapshot.faces[index].background), id, false);
                    stretch_edges(frame);
                }
                advance_px.clamp(0.0, metrics[0].2)
            }
            // The slot in the face's background, the image's slice in it.
            Shown::Image(image) => {
                if visible {
                    frame.add_stretch(ox + x, oy + y, advance_px, height, color(snapshot.faces[index].background), id, false);
                    // The body: where the image, a surface, or a failed
                    // image's outline goes.
                    let (_, _, sw, sh) = image.source;
                    let (bx, by, bw, bh) = (ox + x + image.body_x as f32, oy + y + image.body_y as f32, sw as f32, sh as f32);
                    if image.image != 0 {
                        let (sx, sy, _, _) = image.source;
                        let (fw, fh) = (image.full.0.max(1) as f32, image.full.1.max(1) as f32);
                        let source = romacs_display_protocol::image::ImageSourceRect::new(sx as f32 / fw, sy as f32 / fh, sw as f32 / fw, sh as f32 / fh)
                            .unwrap_or(romacs_display_protocol::image::ImageSourceRect::FULL);
                        frame.add_image(romacs_display_protocol::ImageId::new(image.image), bx, by, bw, bh);
                        // The face's background under the image is its body's; the
                        // slot's stretch paints the margins, where a relief goes.
                        if let Some(romacs_display_protocol::FrameGlyph::Image { source_rect, slot_rect, face_id, .. }) = frame.glyphs.last_mut() {
                            *source_rect = source;
                            *slot_rect = romacs_display_protocol::Rect::new(ox + x, oy + y, advance_px, height);
                            *face_id = id;
                        }
                    } else if image.surface != 0 {
                        // A native surface (a model) composited in the slot.
                        frame.add_surface(romacs_display_protocol::SurfaceId::new(image.surface), bx, by, bw, bh);
                        if let Some(romacs_display_protocol::FrameGlyph::Surface { face_id, .. }) = frame.glyphs.last_mut() {
                            *face_id = id;
                        }
                    }
                    // The background slot and the image in it move together,
                    // listed in glyph order.
                    let shown = image.image != 0 || image.surface != 0;
                    let slot_at = frame.glyphs.len() - 1 - usize::from(shown);
                    row_images.extend((slot_at..frame.glyphs.len()).map(|at| (at, ascent)));
                    // An image that failed to load: a rectangle of its size in
                    // the face's foreground (x_draw_image_foreground).
                    if !shown {
                        let ink = color(snapshot.faces[index].foreground);
                        for (sx, sy, w, h) in [(bx, by, bw, 1.0), (bx, by + bh - 1.0, bw, 1.0), (bx, by, 1.0, bh), (bx + bw - 1.0, by, 1.0, bh)] {
                            frame.add_stretch(sx, sy, w, h, ink, id, false);
                            row_images.push((frame.glyphs.len() - 1, ascent));
                        }
                    }
                    let base = image.relief_background.unwrap_or_else(|| {
                        let [r, g, b] = snapshot.faces[index].background;
                        u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
                    });
                    for (sx, sy, sw, sh, pixel) in relief_strips(image, ox + x, oy + y, base) {
                        frame.add_stretch(sx, sy, sw, sh, Color::from_pixel(pixel), id, false);
                        row_images.push((frame.glyphs.len() - 1, ascent));
                    }
                }
                advance_px
            }
            // Outside a box run, a character's glyphs are only themselves.
            Shown::Glyphs(glyphs) if !step.boxed => {
                let (mut at, mut first) = (x, None);
                for c in glyphs.chars() {
                    let width = advance(frame, id, c, cell);
                    if visible && at - shift < snapshot.width as f32 {
                        match glyphless.get(index, c) {
                            Some(shown) => { glyphless.paint(frame, index, shown, ox + at, oy + y, (ascent, height), id, false, 0.0)?; }
                            None => glyph_of(frame, c, ox + at, oy + y, width, (ascent, height), id)?,
                        }
                    }
                    first.get_or_insert(width);
                    at += width;
                }
                first.unwrap_or(cell)
            }
            Shown::Glyphs(glyphs) => {
                // A side outside the text: the left a spacer before it, the
                // right the last glyph's own (its cell, the cursor's width).
                if visible && lead > 0.0 {
                    frame.add_stretch(ox + x, oy + y, lead, height, color(snapshot.faces[index].background), id, false);
                    set_box_edges(frame, Edges::Left);
                }
                let (mut at, mut first) = (x + lead, None);
                let count = glyphs.chars().count();
                for (n, c) in glyphs.chars().enumerate() {
                    let last = n + 1 == count;
                    let width = advance(frame, id, c, cell) + if last { trail } else { 0.0 };
                    if visible && at - shift < snapshot.width as f32 {
                        // A glyphless character's cell in the run is its background.
                        let at_cell = match glyphless.get(index, c) {
                            Some(shown) => glyphless.paint(frame, index, shown, ox + at, oy + y, (ascent, height), id, false, width - shown.width as f32)?,
                            None => { glyph_of(frame, c, ox + at, oy + y, width, (ascent, height), id)?; frame.glyphs.len() - 1 }
                        };
                        set_glyph_box_edges(frame, at_cell, edges(step.side == 0.0 && n == 0 && opening, last && step.ends));
                    }
                    first.get_or_insert(width);
                    at += width;
                }
                if visible { row_box_top = row_box_top.max(step.top); }
                // The cursor covers the glyph with its box's sides.
                first.unwrap_or(cell) + lead
            }
            Shown::Composed { end, advance } => {
                // A sequence's glyph takes a box run's sides as a character's does.
                if visible && x - shift < snapshot.width as f32 {
                    if lead > 0.0 {
                        frame.add_stretch(ox + x, oy + y, lead, height, color(snapshot.faces[index].background), id, false);
                        set_box_edges(frame, Edges::Left);
                    }
                    publish_selected_glyph(frame, resolver, id, &text[byte..*end]);
                    frame.add_composed_char(&text[byte..*end], ch, ox + x + lead, oy + y, *advance + trail, height, ascent, false);
                    *frame.glyphs.last_mut().and_then(|g| g.face_id_mut()).ok_or("missing character face")? = id;
                    if step.boxed { set_box_edges(frame, edges(step.side == 0.0 && opening, step.ends)); }
                    row_box_top = row_box_top.max(step.top);
                }
                composed = (x, *advance + lead);
                *advance + lead
            }
            Shown::Inside => 0.0,
        };
        if at_point {
            // Point inside a composed sequence shows on its glyph.
            let (x, cursor) = if matches!(shown, Shown::Inside) { composed } else { (x, cursor) };
            let at = cursor_x(x, x0, shift, false, cut);
            point_rect = Some((at, y, if at == x - shift { cursor } else { metrics[0].2 }, row_height, ascent));
            place = Some(PointPlace { x: at, line_x: x + continued, gutter: x0, truncated_left: shift > 0.0, truncated_right: false, at_eol: false });
        }
        if !matches!(shown, Shown::Stretch(_)) || advance_px > 0.0 { hpos += 1; }
        x += advance_px;
    }
    if snapshot.point_byte == text.len() && end_byte == text.len() {
        let at = cursor_x(x, x0, shift, hidden, cut);
        point_rect = Some((at, y, metrics[0].2, row_height, metrics[0].0));
        // An empty last row shows no character: GNU truncates it on neither side.
        let empty = row_start == text.len();
        place = Some(PointPlace { x: at, line_x: x + continued, gutter: x0, truncated_left: shift > 0.0 && !empty, truncated_right: hidden, at_eol: true });
    }
    if let Some(place) = place.as_mut().filter(|_| row_start <= snapshot.point_byte) { place.truncated_right |= hidden; }
    // The last row holds the text's end position too, when layout reached it.
    if end_byte == text.len() && visible_end == row_start && fully_visible(y, row_height) {
        visible_end = text.len() + 1;
    }
    // The row ending at point-max extends its face too (the default's,
    // which paints only its lines there).
    if end_byte == text.len() && !hidden && y < body_height && y + row_height > 0.0 &&
        let Some(extend) = snapshot.text.extends.last().filter(|e| e.byte as usize == text.len()) {
        let index = extend.face.0 as usize;
        let from = (x - shift).max(x0);
        let to = snapshot.width as f32;
        if to > from {
            frame.add_stretch(ox + from, oy + y, to - from, row_height, color(snapshot.faces[index].background),
                              FaceId::new(face_base + index as u32), false);
        }
    }
    if end_byte == text.len() && y < body_height {
        let scrolled = shift > 0.0 && row_start < text.len();
        row_height = settle_row(frame, row_glyphs, oy + y, row_height, row_box_top > 0.0 || row_mixed, &row_images, &metrics, face_base, (&mut point_rect, y));
        rows.push(Row { start: row_start, y, height: row_height });
        marks.push(RowMarks { continued: false, continuation, truncated: hidden, truncated_left: scrolled });
        if scrolled && left_mark(shift) > 0.0 && y + metrics[0].1 > 0.0 { glyph(frame, left_glyph, ox + x0, oy + y, metrics[0], default_id)?; }
    }
    fringe::paint(frame, snapshot, &rows, &marks, fringe::Area { ox, oy, body_height, line: metrics[0].1, text_len: text.len(), reached_end: end_byte == text.len(), face_base })?;
    // A row starting a numbered line shows its label; any other, blanks,
    // in the face of line numbers.
    if let Some(numbers) = &snapshot.line_numbers {
        for row in rows.iter().filter(|r| r.y < body_height && r.y + r.height > 0.0) {
            // A continuation row of point's line keeps its face, in blanks.
            let (x0, label, current) = gutter(row.start);
            let index = if current { numbers.current.0 } else { numbers.face.0 } as usize;
            let id = FaceId::new(face_base + index as u32);
            let (ascent, height, cell, _) = metrics[index];
            let blanks = " ".repeat((x0 / cell).round() as usize);
            let mut at = 0.0;
            for c in label.unwrap_or(blanks.as_str()).chars() {
                let width = advance(frame, id, c, cell);
                if at < snapshot.width as f32 { glyph_of(frame, c, ox + at, oy + row.y, width, (ascent, height), id)?; }
                at += width;
            }
        }
    }
    if let (Some((active, color)), Some((x, y, width, height, ascent))) = (cursor, point_rect) {
        // No cursor may bleed into the mode line or another window.
        // With a right fringe, a cursor after a row that fills the text
        // area shows in the fringe (`overflow-newline-into-fringe').
        let right = snapshot.width as f32 + snapshot.chrome.right_fringe as f32;
        if x >= 0.0 && x < right && y >= 0.0 && y + height <= body_height {
            use romacs_display_protocol::CursorStyle;
            use romacs_layout_input::CursorKind;
            let width = width.min(right - x);
            // A bar no wider than its glyph, an hbar no taller than its row
            // (GNU's draw_bar_cursor clamps both).
            let style = match if active { snapshot.cursor.selected } else { snapshot.cursor.unselected } {
                CursorKind::Box => Some(CursorStyle::FilledBox),
                CursorKind::Hollow => Some(CursorStyle::Hollow),
                CursorKind::Bar(thick) => Some(CursorStyle::Bar((thick as f32).min(width))),
                CursorKind::Hbar(thick) => Some(CursorStyle::Hbar((thick as f32).min(height))),
                CursorKind::None => None,
            };
            if let Some(style) = style {
                frame.add_cursor(DisplayWindowId::new(snapshot.id as i64), ox + x, oy + y,
                    width, height, style, color);
                let published = frame.window_cursors.last_mut().unwrap();
                published.active = active;
                published.ascent = ascent;
            }
        }
    }
    // Each band's faces are the next table after the window's, so the
    // next window's faces start after them.
    // The header and mode lines span the whole window, margins and
    // fringes included.
    let (left, right) = snapshot.chrome.insets();
    let (band_x, band_width) = (ox - left as f32, (left + snapshot.width + right) as f32);
    let mut copy = 0;
    if let Some(band) = &snapshot.header_line {
        copy += 1;
        let bounds = Rect::new(band_x, window_top, band_width, header_height);
        let stretches = (&snapshot.band_stretches.header_line[..], left as f32);
        paint_band(frame, snapshot, band, snapshot.header_line_face, bounds, stretches, GlyphRowRole::HeaderLine, (face_base, copy), (&metrics, &glyphless))?;
    }
    if let Some(band) = &snapshot.mode_line {
        copy += 1;
        let bounds = Rect::new(band_x, oy + body_height, band_width, snapshot.height as f32 - header_height - body_height);
        let stretches = (&snapshot.band_stretches.mode_line[..], left as f32);
        paint_band(frame, snapshot, band, snapshot.mode_line_face, bounds, stretches, GlyphRowRole::ModeLine, (face_base, copy), (&metrics, &glyphless))?;
    }
    Ok(WindowEnd { id: snapshot.id, end: end_byte, visible_end, body_height, rows, point: place })
}

/// Whether any character of TEXT shows in a face of FACES with a box: the
/// default face's, or a run's.  A window's face table is the frame's, so
/// most of its boxed faces show nowhere in its text.
fn shows_box(faces: &[Face], text: &TextSlice) -> bool {
    let boxed = |index: usize| faces.get(index).is_some_and(|face| face.decor.boxed.is_some());
    boxed(0) || text.face_runs.iter().any(|run| boxed(run.face.0 as usize))
}

/// What the character CH at BYTE of TEXT shows as, STRETCH the snapshot's
/// stretch there, INSIDE the end of the composed sequence the characters
/// before it began (updated when one begins here), in face ID of nominal
/// advance CELL. A sequence is as wide as the glyph its representative
/// character has (`representative_char_for_cluster').
#[allow(clippy::too_many_arguments)]
#[inline]
fn shown_at(text: &TextSlice, byte: usize, ch: char, stretch: Option<StretchWidth>, inside: &mut usize,
            frame: &FrameGlyphBuffer, id: FaceId, cell: f32) -> Shown {
    if let Some(width) = stretch { return Shown::Stretch(width); }
    // No sequence starts at, or holds, an ASCII character.
    if ch.is_ascii() { return shown(ch, text.is_raw(byte)); }
    sequence_at(text, byte, ch, inside, frame, id, cell)
}

/// `shown_at' for a character beyond ASCII, out of the ASCII text's way.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn sequence_at(text: &TextSlice, byte: usize, ch: char, inside: &mut usize, frame: &FrameGlyphBuffer, id: FaceId, cell: f32) -> Shown {
    if byte < *inside { return Shown::Inside; }
    let raw = text.is_raw(byte);
    if !raw && let Some(end) = romacs_layout_input::cluster::cluster_end(&text.utf8, byte) {
        *inside = end;
        let representative = romacs_font::cluster::representative_char_for_cluster(&text.utf8[byte..end]);
        return Shown::Composed { end, advance: representative.map_or(cell, |c| advance(frame, id, c, cell)) };
    }
    shown(ch, raw)
}

/// The advance of SHOWN in face ID (CELL wide, SPACE its space's width)
/// at row X, line x LINE_X, on a row whose text starts at X0: a TAB to
/// its next stop as from after the line number, a stretch as GNU's (up to
/// LAST_X on a continued row), glyphs by their own advances.
#[allow(clippy::too_many_arguments)]
fn shown_advance(frame: &FrameGlyphBuffer, shown: &Shown, id: FaceId, cell: f32, space: f32, tab_width: u32,
                 x: f32, line_x: f32, x0: f32, last_x: Option<f32>) -> f32 {
    match shown {
        Shown::Tab => tab_advance(f64::from(line_x - x0), f64::from(space), tab_width) as f32,
        Shown::Stretch(width) => stretch_advance(*width, f64::from(x), f64::from(line_x), last_x.map(f64::from)) as f32,
        Shown::Image(image) => image.width as f32,
        Shown::Glyphs(glyphs) => glyphs.chars().map(|glyph| advance(frame, id, glyph, cell)).sum(),
        Shown::Composed { advance, .. } => *advance,
        Shown::Inside => 0.0,
    }
}

/// GNU's word wrap (move_it_in_display_line_to), one row ahead of the
/// layout: the bytes where a `word-wrap' window's rows break, from ROW, a
/// line start. A row breaks when its next glyph would pass LAST_X, at its
/// last wrap point, the first glyph after whitespace (a space or TAB, as
/// `wrap_flips' classes the snapshot's elements), or
/// with none at that glyph; the next row continues the line from there,
/// its TABs stopping as from the line's start (a TAB that does not fit,
/// from the text area's edge). Rows are found only as the layout reaches
/// them, so a long line costs what the window shows.
struct WordBreaks { row: usize, continued: f32, next: Option<usize>, done: bool, boxes: bool, box_open: bool }

impl WordBreaks {
    /// Whether a row breaks at BYTE; BYTEs ascend.
    #[allow(clippy::too_many_arguments)]
    fn at(&mut self, byte: usize, snapshot: &WindowSnapshot, frame: &FrameGlyphBuffer, metrics: &[(f32, f32, f32, f32)],
          face_base: u32, last_x: f32, gutter: &dyn Fn(usize) -> f32) -> bool {
        while !self.done && self.next.is_none_or(|next| next < byte) {
            self.next = self.row_break(snapshot, frame, metrics, face_base, last_x, gutter);
            self.done = self.next.is_none();
        }
        self.next == Some(byte)
    }

    /// The next break after the rows already found, or None at the text's end.
    fn row_break(&mut self, snapshot: &WindowSnapshot, frame: &FrameGlyphBuffer, metrics: &[(f32, f32, f32, f32)],
                 face_base: u32, last_x: f32, gutter: &dyn Fn(usize) -> f32) -> Option<usize> {
        let text = &snapshot.text.utf8;
        let stretch_at = |byte: usize| snapshot.stretches.binary_search_by_key(&byte, |s| s.byte).ok().map(|i| snapshot.stretches[i].width);
        let image_at = |byte: usize| snapshot.images.binary_search_by_key(&byte, |i| i.byte).ok().map(|i| snapshot.images[i]);
        while self.row < text.len() {
            let (row, x0) = (self.row, gutter(self.row));
            let (mut x, mut hpos, mut may_wrap, mut wrap_at) = (x0, 0usize, false, None::<(usize, f32, bool)>);
            let (mut end, mut inside) = (None, 0);
            for (byte, ch) in text[row..].char_indices().map(|(at, ch)| (row + at, ch)) {
                if ch == '\n' { end = Some((byte + 1, None)); self.box_open = false; break; }
                let whitespace = (ch == ' ' || ch == '\t') != snapshot.wrap_flips.binary_search(&(byte as u32)).is_ok();
                // The box state before BYTE, where the row may restart.
                let before = self.box_open;
                if may_wrap && !whitespace { wrap_at = Some((byte, x, before)); }
                may_wrap = whitespace;
                let index = face_at(&snapshot.text, byte, 0);
                let (_, _, cell, space) = metrics[index];
                let id = FaceId::new(face_base + index as u32);
                let shown = match image_at(byte) {
                    Some(image) => Shown::Image(image),
                    None => shown_at(&snapshot.text, byte, ch, stretch_at(byte), &mut inside, frame, id, cell),
                };
                // A box run's sides count as painting counts them.
                let step = if self.boxes { box_step(&snapshot.faces, &snapshot.text, (0, index), byte, ch, shows_glyphs(&shown), self.box_open, false) } else { BoxStep::NONE };
                self.box_open = step.open;
                let advance = shown_advance(frame, &shown, id, cell, space, snapshot.tab_width, x, x + self.continued, x0, Some(last_x))
                    + step.lead + step.trail;
                // No row breaks inside a composed sequence.
                if hpos > 0 && x + advance > last_x && !matches!(shown, Shown::Inside) {
                    let (at, x, open) = wrap_at.unwrap_or((byte, if matches!(shown, Shown::Tab) { last_x } else { x }, before));
                    self.box_open = open;
                    end = Some((at, Some(x)));
                    break;
                }
                if !matches!(shown, Shown::Stretch(_)) || advance > 0.0 { hpos += 1; }
                x += advance;
            }
            match end {
                None => { self.row = text.len(); }
                Some((at, None)) => { (self.row, self.continued) = (at, 0.0); }
                Some((at, Some(x))) => { (self.row, self.continued) = (at, self.continued + x); return Some(at); }
            }
        }
        None
    }
}

/// Each face's painted characters. Publishing the whole window for every
/// color-only face multiplies Unicode bindings by the face-table size,
/// even when those faces are unused.
pub(super) fn face_text(faces: usize, slices: &[(&TextSlice, usize)]) -> Vec<String> {
    let mut text = vec![String::new(); faces];
    // The continuation and truncation glyphs, in the default face.
    text[0].push_str("\\$");
    for (slice, default) in slices {
        // A composed sequence's characters are all its first one's face's,
        // which draws it (and needs its representative's font).
        let mut inside = 0;
        for (byte, ch) in slice.utf8.char_indices() {
            if byte < inside { continue; }
            let face = &mut text[face_at(slice, byte, *default)];
            if !ch.is_ascii() && !slice.is_raw(byte) && let Some(end) = romacs_layout_input::cluster::cluster_end(&slice.utf8, byte) {
                face.push_str(&slice.utf8[byte..end]);
                inside = end;
                continue;
            }
            match shown(ch, slice.is_raw(byte)) {
                Shown::Tab | Shown::Stretch(_) | Shown::Image(_) => face.push(' '),
                Shown::Glyphs(glyphs) => face.extend(glyphs.chars()),
                Shown::Composed { .. } | Shown::Inside => face.push(ch),
            }
        }
    }
    text
}

/// Each face's ascent, height, nominal advance and space width, as
/// `realize_face' publishes it from FACE_BASE on, and the glyphless glyphs
/// of the characters no font has (`glyphless'), in faces published above
/// every copy of the table a window's bands make. A face no character uses
/// is not published, except the default and those KEEP accepts; its
/// metrics are unused.
#[allow(clippy::too_many_arguments)]
pub(super) fn realize_faces(faces: &[Face], face_text: &[String], keep: impl Fn(usize) -> bool, frame: &mut FrameGlyphBuffer, resolver: &mut FontResolver,
                 face_base: u32, rules: romacs_layout_input::UnderlineRules, no_font: &NoFont) -> Result<(Vec<Metrics>, Glyphless), String> {
    let mut metrics = Vec::new();
    let mut glyphless = Glyphless::default();
    let len = u32::try_from(faces.len()).map_err(|e| e.to_string())?;
    // Above a window's band copies (face_base + 1 and 2 tables): each
    // face's digit and box-line faces, then two copies of its digit face.
    let derived = len.checked_mul(3).and_then(|n| n.checked_add(face_base)).ok_or("too many faces")?;
    glyphless.end = len.checked_mul(4).and_then(|n| n.checked_add(derived)).ok_or("too many faces")?;
    let id_at = |base: u32, index: usize| u32::try_from(index).ok().and_then(|i| base.checked_add(i)).map(FaceId::new).ok_or("too many faces");
    for (index, face) in faces.iter().enumerate() {
        if index != 0 && !keep(index) && face_text[index].is_empty() {
            metrics.push((0.0, 0.0, 0.0, 0.0));
            continue;
        }
        let id = id_at(face_base, index)?;
        let realized = realize_face(face, faces.first(), id, &face_text[index], frame, resolver, rules)?;
        metrics.push(realized.metrics);
        if realized.unresolved.is_empty() { continue; }
        // GNU's no-font glyph (produce_glyphless_glyph), its code in
        // `glyphless-char' (the character's face at 0.6 of its height) and
        // its box in the face's foreground.
        let (digits, line) = (id_at(derived, 2 * index)?, id_at(derived, 2 * index + 1)?);
        let spare = [id_at(derived, 2 * faces.len() + 2 * index)?, id_at(derived, 2 * faces.len() + 2 * index + 1)?];
        let small = Face { pixel_size: (face.pixel_size * glyphless::DIGIT_SCALE).round().max(1.0), decor: Default::default(), ..face.clone() };
        let shown: String = realized.unresolved.iter().filter_map(|&ch| glyphless::text_of(no_font, ch)).collect();
        let digit_face = realize_face(&small, None, digits, &shown, frame, resolver, Default::default()).ok();
        let fg = color(face.foreground);
        frame.set_face_with_font(line, fg, Some(fg), &face.family, face.weight, face.italic, face.pixel_size, 0, None, 0, None, 0, None, false);
        let primary = frame.fonts[&realized.primary].clone();
        let digit_font = digit_face.as_ref().map(|d| frame.fonts[&d.primary].clone());
        let base_width = resolver.ink_extents(&primary, "x").map_or(realized.metrics.2, |x| x.width).round() as i32;
        let base = glyphless::Base { ascent: primary.ascent_px.round() as i32, descent: primary.descent_px.round() as i32, width: base_width };
        let shown_in = glyphless::Faces { digits: digit_face.as_ref().map(|d| (digits, d.metrics.2)), line, spare };
        for &ch in &realized.unresolved {
            let mut extents = |text: &str| digit_font.as_ref().and_then(|font| resolver.ink_extents(font, text));
            let glyph = glyphless::layout(no_font, ch, base, &mut extents);
            glyphless.bind(frame, (index, id), &primary, ch, glyph, shown_in);
        }
    }
    Ok((metrics, glyphless))
}

/// What `realize_face' made of one face: its metrics, its primary font,
/// and the characters of its text no installed font has a glyph for.
struct Realized {
    metrics: Metrics,
    primary: romacs_display_protocol::font::ResolvedFontId,
    unresolved: Vec<char>,
}

/// Publish FACE as ID with its font, binding the characters of TEXT, and
/// its metrics: its ascent, height, nominal advance and space width.
/// Nothing here stops the display: a face whose font cannot be had falls
/// back to the DEFAULT face's family and then to `monospace' (GNU falls
/// back to the frame's font), a font without a positive advance to its
/// space or half its size, and a character no font has is left for the
/// caller to show glyphless.
#[allow(clippy::too_many_arguments)]
fn realize_face(face: &Face, default: Option<&Face>, id: FaceId, text: &str, frame: &mut FrameGlyphBuffer, resolver: &mut FontResolver,
                rules: romacs_layout_input::UnderlineRules) -> Result<Realized, String> {
    let valid = |f: &Face| !f.family.is_empty() && f.pixel_size.is_finite() && f.pixel_size > 0.0;
    let pixel_size = if face.pixel_size.is_finite() && face.pixel_size > 0.0 { face.pixel_size }
        else { default.filter(|d| valid(d)).map_or(16.0, |d| d.pixel_size) };
    let asked = FontRequest { pixel_size, ..font_request(face) };
    // The families tried after the face's own, built only when it fails.
    let fallbacks = [default.filter(|d| valid(d)).map(|d| d.family.as_str()), Some("monospace")];
    let line = |line: Option<romacs_layout_input::Line>| line.map_or((0, None), |l| (l.style, l.color.map(color)));
    let ((underline, underline_color), (strike, strike_color), (overline, overline_color)) =
        (line(face.decor.underline), line(face.decor.strike), line(face.decor.overline));
    // Icons are bound from the resolver's one answer for them, not
    // resolved through the face's fallback on every layout.
    let plain: std::borrow::Cow<str> = if text.chars().any(is_icon) { text.chars().filter(|&c| !is_icon(c)).collect::<String>().into() } else { text.into() };
    let mut failure = None;
    let (request, published) = 'found: {
        let mut tried: Vec<&str> = vec![];
        for family in std::iter::once(None).chain(fallbacks.into_iter().flatten().map(Some)) {
            let request = match family {
                None => asked.clone(),
                Some(family) if family == asked.family || tried.contains(&family) => continue,
                Some(family) => { tried.push(family); FontRequest { family: family.to_owned(), ..asked.clone() } }
            };
            if request.family.is_empty() { continue; }
            frame.set_face_with_font(id, color(face.foreground), Some(color(face.background)), &request.family, face.weight, face.italic, pixel_size,
                underline, underline_color, strike, strike_color, overline, overline_color, false);
            match publish_face_text(resolver, frame, id, &request, &plain) {
                Ok(published) => break 'found (request, published),
                Err(error) => { failure.get_or_insert(error); }
            }
        }
        return Err(format!("no font for face {:?}: {}", face.family, failure.map_or_else(String::new, |e| e.to_string())));
    };
    if let Some(error) = failure { warn_once(format!("face family {:?} shows in {:?}: {error}", face.family, request.family)); }
    let mut unresolved: Vec<char> = published.unresolved.iter().map(|&(ch, _)| ch).collect();
    for icon in text.chars().filter(|&c| is_icon(c)) {
        match resolver.resolve_sole(&request, icon) {
            Ok(resolution) => {
                frame.fonts.entry(resolution.font.id).or_insert_with(|| resolution.font.clone());
                frame.char_fonts.entry(id).or_default().insert(icon, resolution.glyph);
            }
            Err(error) => {
                warn_once(format!("icon U+{:04X} shows glyphless: {error}", icon as u32));
                unresolved.push(icon);
            }
        }
    }
    for (ch, error) in &published.unresolved { warn_once(format!("U+{:04X} shows glyphless: {error}", *ch as u32)); }
    let font = &frame.fonts[&published.primary];
    let placement = underline_placement(face, font, rules);
    let (ascent_px, descent_px) = (font.ascent_px, font.descent_px);
    if let Some(published) = frame.faces.get_mut(&id) {
        (published.underline_placement, published.underline_thickness) = placement;
        (published.font_ascent, published.font_descent) = (ascent_px.round() as i32, descent_px.round() as i32);
    }
    let font = &frame.fonts[&published.primary];
    let height = (font.ascent_px + font.descent_px).ceil();
    let space = font.space_advance_px;
    let per_glyph = font.glyph_advance.cell_advance_px().is_none();
    let mut cell = font.glyph_advance.cell_advance_px().unwrap_or(space);
    let ascent = font.ascent_px;
    if !cell.is_finite() || cell <= 0.0 {
        warn_once(format!("font {:?} has no positive nominal advance", font.family));
        cell = if space.is_finite() && space > 0.0 { space } else { (pixel_size / 2.0).round().max(1.0) };
    }
    if per_glyph {
        // Dual-width fonts such as Iosevka report per-glyph advances. The
        // nominal space is only the frame column metric; text uses the
        // resolver's exact cached advance, including ASCII.
        // Each printable ASCII character once, in code order, without
        // sorting the face's text.
        let mut seen = [false; 128];
        for byte in text.bytes().filter(|b| b.is_ascii() && !b.is_ascii_control()) { seen[usize::from(byte)] = true; }
        for ch in (0u8..128).filter(|&b| seen[usize::from(b)]).map(char::from) {
            match resolver.resolve_char(&request, ch) {
                Ok(resolved) => {
                    frame.fonts.insert(resolved.font.id, resolved.font);
                    frame.char_fonts.entry(id).or_default().insert(ch, resolved.glyph);
                }
                Err(error) => warn_once(format!("{ch:?} in {:?}: {error}", request.family)),
            }
        }
    }
    // A box's edges outside the text grow the face's row by their width
    // above and below (produce_glyphs: `it->ascent += thick').
    let thick = face.decor.boxed.map_or(0.0, |b| f32::from(b.horizontal.max(0)));
    if let Some(boxed) = face.decor.boxed && let Some(published) = frame.faces.get_mut(&id) {
        use romacs_display_protocol::face::{BoxLineWidth, BoxType};
        published.box_type = BoxType::from_gnu_code(boxed.style).unwrap_or(BoxType::Line);
        published.box_line_width = BoxLineWidth::from_gnu(i32::from(boxed.horizontal));
        published.box_vertical_line_width = Some(BoxLineWidth::from_gnu(i32::from(boxed.vertical.unsigned_abs())));
        published.box_color = boxed.color.map(color);
    }
    Ok(Realized { metrics: (ascent + thick, height + 2.0 * thick, cell, space), primary: published.primary, unresolved })
}

/// The ascent and height CH adds to its row (gui_produce_glyphs): those of
/// the font it is shown in, which a character outside face ID's font takes
/// from its fontset; FACE's otherwise, as for every ASCII character.
fn font_rise(frame: &FrameGlyphBuffer, id: FaceId, ch: char, face: (f32, f32)) -> (f32, f32) {
    if ch.is_ascii() { return face; }
    let font = frame.char_fonts.get(&id).and_then(|chars| chars.get(&ch)).and_then(|glyph| frame.fonts.get(&glyph.resolved_font_id));
    font.map_or(face, |font| { let ascent = font.ascent_px.round(); (ascent, ascent + font.descent_px.round()) })
}

/// The ascent and height a character adds to its row, shown as SHOWN at
/// byte AT of TEXT: a composed sequence's are its representative's, whose
/// font shows it (`shown_at'), and a character within one takes them
/// (CLUSTER); others show nothing of a font.
fn shown_rise(frame: &FrameGlyphBuffer, id: FaceId, (text, at, ch): (&str, usize, char), shown: &Shown, face: (f32, f32), cluster: &mut (f32, f32)) -> (f32, f32) {
    match shown {
        Shown::Glyphs(_) => font_rise(frame, id, ch, face),
        Shown::Composed { end, .. } => {
            let representative = romacs_font::cluster::representative_char_for_cluster(&text[at..*end]).unwrap_or(ch);
            *cluster = font_rise(frame, id, representative, face);
            *cluster
        }
        Shown::Inside => *cluster,
        _ => face,
    }
}

/// Lay INPUT out as `append_window` paints its text, without presenting
/// it: the advance and face row height of each character, from the same
/// face realization and advances. A TAB answers its face's space, from
/// which the caller makes its stretch (`tab_advance`); another control
/// character, its escape's glyphs.
pub(super) fn measure_inner(input: &MeasureInput, resolver: &mut FontResolver) -> Result<Measured, String> {
    resolver.set_selection_state(input.font_selection);
    resolver.set_alternative_state(input.font_alternatives.clone());
    resolver.set_fontsets(input.fontsets.clone());
    if input.faces.is_empty() { return Err("missing face".into()); }
    validate_runs(&input.text, input.faces.len())?;
    let mut frame = new_frame(1, 1, 1)?;
    let face_text = face_text(input.faces.len(), &[(&input.text, 0)]);
    // A face no character uses answers no row height (0).
    let (metrics, _) = realize_faces(&input.faces, &face_text, |_| false, &mut frame, resolver, 0, Default::default(), &input.no_font)?;
    let pixels = |value: f32| if value.fract() == 0.0 && value >= 0.0 && value <= u32::MAX as f32 { Ok(value as u32) }
        else { Err(format!("fractional or negative advance {value}")) };
    let face_heights = metrics.iter().map(|m| pixels(m.1)).collect::<Result<Vec<_>, _>>()?;
    let face_ascents: Vec<u32> = metrics.iter().map(|m| m.0.round().max(0.0) as u32).collect();
    let mut glyphs = Vec::with_capacity(input.text.utf8.len());
    let mut ascents = Vec::with_capacity(input.text.utf8.len());
    let mut fallback = Vec::with_capacity(input.text.utf8.len());
    let mut cluster_rise = (face_ascents[0] as f32, face_heights[0] as f32);
    let mut inside = 0;
    // A box run's outside side edges add to its first and last glyphs'
    // advances, as append_window paints them.
    let (boxes, mut box_open) = (shows_box(&input.faces, &input.text), false);
    for (byte, ch) in input.text.utf8.char_indices() {
        if ch == '\n' { glyphs.push((0, face_heights[0])); ascents.push(face_ascents[0]); fallback.push(false); box_open = false; continue; }
        let index = face_at(&input.text, byte, 0);
        let (_, _, cell, space) = metrics[index];
        let id = FaceId::new(index as u32);
        let shown = shown_at(&input.text, byte, ch, None, &mut inside, &frame, id, cell);
        let step = if boxes { box_step(&input.faces, &input.text, (0, index), byte, ch, shows_glyphs(&shown), box_open, false) } else { BoxStep::NONE };
        box_open = step.open;
        let width = step.lead + step.trail + match shown {
            Shown::Tab | Shown::Stretch(_) | Shown::Image(_) => space,
            Shown::Glyphs(shown) => shown.chars().map(|glyph| advance(&frame, id, glyph, cell)).sum::<f32>(),
            Shown::Composed { advance, .. } => advance,
            Shown::Inside => 0.0,
        };
        let face = (face_ascents[index] as f32, face_heights[index] as f32);
        let (ascent, height) = if escaped(&input.text, byte, ch) { face } else { shown_rise(&frame, id, (&input.text.utf8, byte, ch), &shown, face, &mut cluster_rise) };
        glyphs.push((pixels(width)?, pixels(height)?));
        ascents.push(ascent as u32);
        fallback.push((ascent, height) != face);
    }
    Ok(Measured { glyphs, face_heights, ascents, face_ascents, fallback })
}
