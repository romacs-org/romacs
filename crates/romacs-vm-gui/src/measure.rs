//! The VM's host layout hook (DIVERGENCES.md VM2-WTPS1) over the engine
//! that paints: `window-text-pixel-size', `pos-visible-in-window-p' and
//! `posn-at-point' measure a displayed window with the fonts, advances
//! and row heights the painter uses, inside the call, presenting nothing.
use std::{cell::RefCell, rc::Rc};

pub struct Host {
    pub engine: Rc<RefCell<romacs_layout2::LayoutEngine>>,
}

impl romacs_vm::LayoutHost for Host {
    fn measure(&mut self, input: &romacs_layout_input::MeasureInput) -> Result<romacs_layout_input::Measured, String> {
        // Lisp runs only between paints, never inside one.
        let mut engine = self.engine.try_borrow_mut().map_err(|_| "the layout engine is painting".to_owned())?;
        engine.measure(input)
    }
    fn lay_out(&mut self, frame: &romacs_layout_input::FrameSnapshot) -> Result<Vec<romacs_vm::WindowEnd>, String> {
        let mut engine = self.engine.try_borrow_mut().map_err(|_| "the layout engine is painting".to_owned())?;
        engine.layout_frame(frame, 1, None).map(|(_, ends)| ends)
    }
}

#[cfg(test)]
mod tests {
    //! The VM's answers for a displayed window against the host's real
    //! layout of it: every position's glyph and cursor, every row's width
    //! and height, and the mode-line band, for text in several fonts,
    //! sizes and widths. What the host does not lay out as GNU displays
    //! it is refused, and its plain twin answers.
    use super::*;
    use romacs_display_protocol::{FrameGlyph, frame_glyphs::GlyphRowRole};
    use romacs_vm::Vm;

    struct Hosted { vm: Vm, engine: Rc<RefCell<romacs_layout2::LayoutEngine>>, width: u32, height: u32 }

    fn hosted() -> Hosted {
        let engine = Rc::new(RefCell::new(romacs_layout2::LayoutEngine::new().unwrap()));
        let mut vm = Vm::new();
        crate::startup::initialize(&mut vm).unwrap();
        // As the GUI does after startup Lisp chose the default face.
        let (line, column) = romacs_layout2::frame_metrics(&vm.display_default_face().unwrap()).unwrap();
        let (width, height) = (column * 60, line * 20);
        vm.set_frame_geometry(width, height, line, column).unwrap();
        vm.set_layout_host(Some(Box::new(Host { engine: engine.clone() })));
        vm.eval_source(r#"(progn
          (defface hosted-big '((t :height 280)) "Taller.")
          (defface hosted-small '((t :height 90)) "Shorter.")
          (defface hosted-sans '((t :family "Sans")) "Proportional.")
          (set-buffer (get-buffer-create "hosted")) (set-window-buffer nil (current-buffer)))"#).unwrap();
        let mut hosted = Hosted { vm, engine, width, height };
        hosted.paint();
        hosted
    }

    /// The host's layout of the frame, the selected window's cursor and
    /// that window's id and height.
    struct Paint { glyphs: Vec<(char, f32, f32, f32, f32, GlyphRowRole)>, cursor: Option<(f32, f32, f32, f32)>, window_width: u32, window_height: u32 }

    impl Hosted {
        fn eval(&mut self, source: &str) -> String {
            let probe = format!("(condition-case e {source} (error (list 'signal e)))");
            let value = self.vm.eval_source(&probe).unwrap_or_else(|e| panic!("{source}: {e}")).display().unwrap();
            assert!(!value.starts_with("(signal "), "{source}: {value}");
            value
        }
        fn paint(&mut self) -> Paint {
            let snapshot = self.vm.frame_snapshot(self.width, self.height).unwrap();
            let window = &snapshot.windows[0];
            let (id, window_width, window_height) = (window.snapshot.id, window.snapshot.width, window.snapshot.height);
            let color = romacs_display_protocol::Color::from_pixel(0xffffff);
            let (frame, ends) = self.engine.borrow_mut().layout_frame(&snapshot, 1, Some((id, color))).unwrap();
            self.vm.display_apply_layout(&ends).unwrap();
            // A TAB's stretch stands for it as '\t'.
            let glyphs = frame.glyphs.iter().filter_map(|g| match g {
                FrameGlyph::Char { window_id, char, x, y, width, height, row_role, .. } if window_id.get() == id as i64 =>
                    Some((*char, *x, *y, *width, *height, *row_role)),
                FrameGlyph::Stretch { window_id, x, y, width, height, row_role, .. } if window_id.get() == id as i64 =>
                    Some(('\t', *x, *y, *width, *height, *row_role)),
                _ => None,
            }).collect();
            let cursor = frame.window_cursors.iter().find(|c| c.window_id.get() == id as i64).map(|c| (c.x, c.y, c.width, c.height));
            Paint { glyphs, cursor, window_width, window_height }
        }
    }

    fn cons(x: f32, y: f32) -> String { format!("({} . {})", x as i64, y as i64) }

    /// Evaluate INSERT, which inserts the text under test (without `\\'
    /// or `$', the continuation and truncation glyphs), add a sentinel
    /// line, and compare the VM's answers with the host's layout.
    fn agree(h: &mut Hosted, insert: &str) {
        let text = h.eval(&format!("(progn (erase-buffer) {insert} (prog1 (buffer-string) (insert \"\\n#\") (goto-char 1) (set-window-start nil 1)))"));
        let text: String = h.vm.eval_source(&format!("(substring-no-properties {text})")).unwrap().display().unwrap();
        let text: String = serde_json::from_str(&text).unwrap();
        assert!(!text.contains(['\\', '$']), "{text:?}");
        let end = text.chars().count() + 1;
        let paint = h.paint();
        let line = h.eval("(frame-char-height)").parse::<f32>().unwrap();
        let text_glyphs = || paint.glyphs.iter().filter(|g| g.5 == GlyphRowRole::Text);
        // The text area less the continuation or truncation glyph, if painted.
        let special = text_glyphs().find(|g| g.0 == '\\' || g.0 == '$').map(|g| paint.window_width as f32 - g.3);
        let mut painted = text_glyphs().filter(|g| g.0 != '\\' && g.0 != '$').peekable();
        // Each position's cursor, if the host shows it, and the painted
        // glyph of each character.
        let mut at = vec![];
        for (index, ch) in text.chars().chain("\n#".chars()).enumerate() {
            let pos = index + 1;
            h.eval(&format!("(goto-char {pos})"));
            let cursor = h.paint().cursor;
            let glyph = if ch == '\n' || cursor.is_none() { None }
                else { painted.next_if(|g| (g.1, g.2) == (cursor.unwrap().0, cursor.unwrap().1)) };
            if let Some(g) = glyph { assert_eq!(g.0, ch, "{text:?}: glyph {pos}"); }
            at.push((pos, ch, cursor, glyph.map(|g| (g.3, g.4))));
        }
        assert!(painted.next().is_none(), "{text:?}: every painted glyph is a character's");
        let mut rows: Vec<f32> = at.iter().filter_map(|a| a.2.map(|c| c.1)).collect();
        rows.sort_by(f32::total_cmp);
        rows.dedup();
        for (index, &(pos, ch, cursor, glyph)) in at.iter().enumerate() {
            let posn = h.eval(&format!("(let ((p (posn-at-point {pos}))) (list (eq (car p) (selected-window)) (cdr p)))"));
            let visible = h.eval(&format!("(pos-visible-in-window-p {pos} nil t)"));
            // Past a truncation glyph the host shows nothing; GNU's
            // iterator still places the position (not compared).
            let Some((x, y, cursor_width, _)) = cursor else { continue };
            assert_eq!(visible, format!("({} {})", x as i64, y as i64), "{text:?}: visible {pos}");
            // A truncated row's last character is under its `$'.
            if glyph.is_none() && ch != '\n' { continue; }
            let (w, ht) = glyph.unwrap_or((cursor_width, line));
            let row = rows.iter().position(|&r| r == y).unwrap();
            let col = at[..index].iter().filter(|a| a.2.is_some_and(|c| c.1 == y)).count();
            let want = format!("(t ({pos} {} 0 nil {pos} ({col} . {row}) nil (0 . 0) {}))", cons(x, y), cons(w, ht));
            assert_eq!(posn, want, "{text:?}: posn-at-point {pos} {ch:?}");
        }
        // A line is as wide as its newline's x, or the last visible x when
        // continued or truncated, and as tall as the next line's y less its own.
        let y_at = |pos: usize| at[pos - 1].2.unwrap().1;
        let (mut start, mut widest) = (1, 0.0f32);
        for &(pos, _, cursor, _) in at.iter().filter(|a| a.1 == '\n') {
            let (x, y) = cursor.map_or((0.0, f32::NAN), |c| (c.0, c.1));
            let truncated = at[start - 1..pos].iter().any(|a| a.2.is_none() || a.3.is_none() && a.1 != '\n');
            let x = if truncated || y != y_at(start) { special.unwrap() } else { x };
            widest = widest.max(x);
            // Measuring past a truncated line's edge counts the row after
            // it too, as GNU's window_text_pixel_size does.
            let height = (y_at(pos + 1) - y_at(start)) * if truncated { 2.0 } else { 1.0 };
            // An empty range measures nothing (GNU, and the batch cases).
            if pos != start { assert_eq!(h.eval(&format!("(window-text-pixel-size nil {start} {pos})")), cons(x, height), "{text:?}: line from {start}"); }
            start = pos + 1;
        }
        let whole = cons(widest, y_at(end + 1));
        assert_eq!(h.eval(&format!("(window-text-pixel-size nil nil {})", end + 1)), whole, "{text:?}: whole");
        assert_eq!(h.eval(&format!("(buffer-text-pixel-size nil nil nil nil nil {})", end + 1)), whole, "{text:?}: buffer");
        let band_y = paint.glyphs.iter().find(|g| g.5 == GlyphRowRole::ModeLine).map(|g| g.2).unwrap();
        let band = paint.window_height as f32 - band_y;
        assert_eq!(h.eval(&format!("(window-text-pixel-size nil nil {} nil nil t)", end + 1)), cons(widest, y_at(end + 1) + band), "{text:?}: mode line");
    }

    #[test]
    fn displayed_window_answers_agree_with_the_host_layout() {
        let mut h = hosted();
        for insert in [
            r#"(insert "abc\n\nde fgh\n  x")"#,
            r#"(insert "λx é → ü\n日本語テキスト\nmixed 日本 abc")"#,
            r#"(insert "")"#,
            r#"(insert "one\n")"#,
            r#"(insert "a" (propertize "BIG" 'face 'hosted-big) "b\nplain\n" (propertize "tiny" 'face 'hosted-small) "\nx")"#,
            r#"(insert (propertize "Proportional iiii WWWW" 'face 'hosted-sans) "\nmono iiii WWWW")"#,
            r#"(insert (propertize "x" 'face '(hosted-big)) "\n" (propertize "日本" 'face 'hosted-big))"#,
            // TABs stop at multiples of `tab-width' spaces of their face,
            // counted from the line start in pixels.
            r#"(insert "\ta\tbc\n12345678\tz\n1234567\tq\n\t\t")"#,
            r#"(setq tab-width 3) (insert "ab\tc\t\td\nλ\t日本\tx")"#,
            r#"(insert (propertize "iii\tW\tx" 'face 'hosted-sans) "\n" (propertize "a\tb" 'face 'hosted-big) "\tc")"#,
            // Lines wider than the window continue on the next row, from
            // its left edge; TABs stop as from the line's start. The window
            // is half the frame: 29 columns of text before the `\\'.
            r#"(setq truncate-partial-width-windows nil) (insert (make-string 150 ?x) "\nshort\n" (make-string 28 ?y) "\n" (make-string 29 ?z) "\n" (make-string 30 ?w))"#,
            r#"(insert "λόγος " (make-string 30 ?a) "\t" (make-string 20 ?b) "\tc\td")"#,
            r#"(insert (propertize (make-string 50 ?i) 'face 'hosted-sans) (propertize "WWW" 'face 'hosted-big) (make-string 40 ?m))"#,
            r#"(insert (make-string 57 ?q) "\t\t" (make-string 5 ?r))"#,
            // Truncated lines show to their `$' and hide the rest, here as
            // `truncate-partial-width-windows' asks of this window.
            r#"(setq truncate-partial-width-windows 50) (insert (make-string 40 ?t) "\nnext\n" (make-string 25 ?u) "\t" (make-string 3 ?v))"#,
            r#"(setq truncate-partial-width-windows nil truncate-lines t) (insert (make-string 33 ?t) "\nend")"#,
        ] { agree(&mut h, &format!("(kill-local-variable 'tab-width) (kill-local-variable 'truncate-lines) {insert}")); }
        // Not a cell grid: proportional advances differ per glyph, and a
        // taller face makes its row taller than the frame's line.
        h.eval(r#"(progn (erase-buffer) (insert (propertize "iW" 'face 'hosted-sans) "\n" (propertize "B" 'face 'hosted-big)))"#);
        h.paint();
        let i = h.eval("(car (nth 9 (posn-at-point 1)))");
        let w = h.eval("(car (nth 9 (posn-at-point 2)))");
        assert_ne!(i, w, "Sans advances");
        let line = h.eval("(frame-char-height)").parse::<i64>().unwrap();
        let tall = h.eval("(cdr (window-text-pixel-size nil 4 5))").parse::<i64>().unwrap();
        assert!(tall > line, "{tall} > {line}");
    }

    /// Redisplay keeps the window start exactly when point is on a row the
    /// start shows whole, as the iterator measures it over the host's
    /// layout (`pos-visible-in-window-p'), whether point moved or text
    /// changed; every paint then shows the cursor on a whole row.
    #[test]
    fn the_start_stays_exactly_while_point_shows_on_a_whole_row() {
        let mut h = hosted();
        h.eval(r#"(progn (setq truncate-partial-width-windows nil) (erase-buffer)
          (let ((i 0))
            (while (< i 50)
              (insert (format "line %d " i) (make-string (* 13 (% i 5)) ?x)
                      (if (= (% i 7) 3) (propertize "BIG" 'face 'hosted-big) "") "\n")
              (setq i (1+ i))))
          (goto-char 1) (set-window-start nil 1))"#);
        let first = h.paint();
        let band = first.glyphs.iter().find(|g| g.5 == GlyphRowRole::ModeLine).map(|g| g.2).unwrap();
        let steps = ["(forward-char 1)", "(forward-char 1)", "(forward-line 1)", "(insert \"abc\")",
                     "(insert \"\\n\")", "(delete-char -1)", "(insert (make-string 40 ?y))", "(backward-char 3)"];
        let (mut kept, mut moved) = (0, 0);
        for step in 0..400 {
            let form = steps[step % steps.len()];
            let start = h.eval(&format!("(progn (if (eobp) (goto-char 1) {form}) (window-start))"));
            let visible = h.eval("(pos-visible-in-window-p (point) nil t)");
            let whole = visible != "nil" && visible.split(' ').count() == 2;
            let paint = h.paint();
            let (_, y, _, height) = paint.cursor.expect("the selected window's cursor");
            assert!(y >= 0.0 && y + height <= band, "step {step} {form}: cursor at {y}+{height} past the text area {band}");
            let after = h.eval("(window-start)");
            assert_eq!(after == start, whole, "step {step} {form}: start {start} -> {after}, point visible {visible}");
            if whole { kept += 1 } else { moved += 1 }
        }
        assert!(kept > 300 && moved > 3, "kept {kept}, moved {moved}");
    }

    /// A start redisplay chooses from the painted rows puts point where
    /// GNU's do, as the display iterator measures them: with
    /// `scroll-conservatively' 101 on the last whole row going down and the
    /// first going up, and one row more would not show it whole; with 0,
    /// on the row GNU's recentring moves back to.
    #[test]
    fn a_new_start_puts_point_where_gnus_rules_do() {
        let mut h = hosted();
        h.eval(r#"(progn (setq truncate-partial-width-windows nil) (erase-buffer)
          (let ((i 0))
            (while (< i 120)
              (insert (format "line %d " i) (make-string (* 17 (% i 4)) ?x)
                      (if (= (% i 9) 4) (propertize "BIG" 'face 'hosted-big) "") "\n")
              (setq i (1+ i))))
          (goto-char 1) (set-window-start nil 1))"#);
        h.paint();
        let whole = |h: &mut Hosted, form: &str| {
            let visible = h.eval(&format!("(pos-visible-in-window-p {form} nil t)"));
            visible != "nil" && visible.split(' ').count() == 2
        };
        let line = h.eval("(frame-char-height)").parse::<u32>().unwrap();
        let body = h.eval("(window-body-height nil t)").parse::<u32>().unwrap();
        let back = { let dy = body / 2; let mut back = (dy / line).max(1);
            if dy > back * line && dy - back * line > body.min(line * 2 / 3) { back += 1; } back };
        let mut scrolls = 0;
        for (conservative, step) in [(101, "(forward-line 1)"), (101, "(forward-line -1)"), (0, "(forward-line 7)"), (0, "(forward-line -9)")] {
            h.eval(&format!("(setq scroll-conservatively {conservative})"));
            for _ in 0..60 {
                let start = h.eval(&format!("(progn {step} (window-start))"));
                h.paint();
                if h.eval("(window-start)") == start { continue; }
                scrolls += 1;
                assert!(whole(&mut h, "(point)"), "{step}: point shows whole after the scroll");
                let row = h.eval("(save-excursion (vertical-motion 0) (point))");
                if conservative == 0 {
                    // Recentring: point's row is BACK rows below the start, or the buffer starts sooner.
                    let down = h.eval(&format!("(save-excursion (goto-char (window-start)) (vertical-motion {back}) (point))"));
                    assert!(down == row || h.eval("(window-start)") == "1", "{step}: row {row}, {back} rows down {down}");
                } else if step.contains(" -") {
                    assert_eq!(h.eval("(window-start)"), row, "{step}: point on the first row");
                } else {
                    // One row more above would not show point whole.
                    let up = h.eval("(save-excursion (goto-char (window-start)) (vertical-motion -1) (point))");
                    let shown = h.eval(&format!("(let ((s (window-start))) (set-window-start nil {up} t) (prog1 (pos-visible-in-window-p (point) nil t) (set-window-start nil s t)))"));
                    assert!(shown == "nil" || shown.split(' ').count() != 2, "{step}: start {up} shows point too: {shown}");
                }
            }
        }
        assert!(scrolls > 20, "{scrolls} scrolls");
    }

    /// A default face shrunk live, with no frame resize, makes rows shorter
    /// than the frame's line: the window still fills with text, and point
    /// moved below its last whole row still scrolls it, the text it has
    /// not shown being no end of the buffer.
    #[test]
    fn a_shrunk_default_face_fills_and_scrolls_the_window() {
        let mut h = hosted();
        h.eval(r#"(progn (erase-buffer)
          (let ((i 0)) (while (< i 300) (insert (format "line %d\n" i)) (setq i (1+ i))))
          (goto-char 1) (set-window-start nil 1))"#);
        h.paint();
        h.eval("(internal-set-lisp-face-attribute 'default :height 60 (selected-frame))");
        let paint = h.paint();
        let band = paint.glyphs.iter().find(|g| g.5 == GlyphRowRole::ModeLine).map(|g| g.2).unwrap();
        let text = || paint.glyphs.iter().filter(|g| g.5 == GlyphRowRole::Text);
        let row = text().map(|g| g.4).fold(f32::MAX, f32::min);
        let bottom = text().map(|g| g.2 + g.4).fold(0.0, f32::max);
        assert!(row < h.eval("(frame-char-height)").parse::<f32>().unwrap(), "rows of {row}px are shorter than the frame's line");
        assert!(bottom + row > band, "text fills the window: its last row ends at {bottom}, the band starts at {band}");
        // Down a line at a time: the start moves once point passes the last whole row.
        let mut scrolled = false;
        for step in 0..((band / row) as usize + 5) {
            h.eval("(forward-line 1)");
            let paint = h.paint();
            let (_, y, _, height) = paint.cursor.unwrap_or_else(|| panic!("step {step}: no cursor on a whole row"));
            assert!(y + height <= band, "step {step}: cursor at {y}+{height} past {band}");
            scrolled |= h.eval("(window-start)") != "1";
        }
        assert!(scrolled, "the window scrolled");
    }

    /// A mode line enabled while point is on the window's bottom whole row
    /// takes that row: redisplay scrolls so point shows above the band, as
    /// pinned GNU does (start 1 -> 8 at its geometry). Removing the band, or
    /// adding a header line, which Romacs does not paint, keeps the cursor
    /// on a whole row.
    #[test]
    fn a_mode_line_enabled_over_points_row_scrolls_the_window() {
        let mut h = hosted();
        h.eval(r#"(progn (setq mode-line-format nil) (erase-buffer)
          (let ((i 0)) (while (< i 100) (insert (format "line %d\n" i)) (setq i (1+ i))))
          (goto-char 1) (set-window-start nil 1))"#);
        h.paint();
        // Point to the last whole row.
        h.eval("(while (let ((v (pos-visible-in-window-p (line-beginning-position 2) nil t))) (and v (= (length v) 2))) (forward-line 1))");
        let paint = h.paint();
        assert_eq!(h.eval("(window-start)"), "1", "point on the bottom whole row keeps the start");
        let (_, y, _, height) = paint.cursor.expect("cursor on the bottom row");
        assert!(y + height <= paint.window_height as f32 && y + 2.0 * height > paint.window_height as f32, "point is on the bottom row: {y}+{height}");
        let cursor_in_text = |h: &mut Hosted, what: &str| {
            let paint = h.paint();
            let bottom = paint.glyphs.iter().find(|g| g.5 == GlyphRowRole::ModeLine).map_or(paint.window_height as f32, |g| g.2);
            let (_, y, _, height) = paint.cursor.unwrap_or_else(|| panic!("{what}: the cursor is clipped"));
            assert!(y + height <= bottom, "{what}: cursor at {y}+{height} past {bottom}");
        };
        h.eval("(setq mode-line-format '(\"%b\"))");
        cursor_in_text(&mut h, "mode line enabled");
        assert_eq!(h.eval("(window-start)"), "8", "the band took point's row: the window scrolls as GNU does");
        h.eval("(setq mode-line-format nil)");
        cursor_in_text(&mut h, "mode line removed");
        h.eval("(setq header-line-format \"header\")");
        cursor_in_text(&mut h, "header line set");
        h.eval("(setq mode-line-format '(\"%b\"))");
        cursor_in_text(&mut h, "mode line again, with a header line");
    }

    /// A window that starts on a continuation row: the host's glyphs and
    /// the VM's answers agree there, TABs stopping as from the line start.
    #[test]
    fn a_window_starting_on_a_continuation_row_agrees() {
        let mut h = hosted();
        // 29 columns before the `\'; the second row starts at 30.
        h.eval("(progn (setq truncate-partial-width-windows nil) (erase-buffer) (insert (make-string 34 ?a) \"\\tz\\nnext\") (goto-char 36) (set-window-start nil 30))");
        let paint = h.paint();
        let z = paint.glyphs.iter().find(|g| g.0 == 'z').unwrap();
        let first = paint.glyphs.iter().filter(|g| g.5 == GlyphRowRole::Text && g.0 != '\\').map(|g| g.2).fold(f32::MAX, f32::min);
        assert_eq!((h.eval("(window-start)").as_str(), first), ("30", 0.0), "starts on the continuation row");
        assert_eq!(h.eval("(pos-visible-in-window-p 36 nil t)"), format!("({} {})", z.1 as i64, z.2 as i64));
        assert_eq!(h.eval("(car (nth 2 (posn-at-point 36)))"), (z.1 as i64).to_string());
        // From the line start z stands past a TAB stop counted over both rows.
        assert_eq!(z.1 as i64, 110);
    }

    /// Redisplay fontifies the rows a window shows as the host continues
    /// them: lines several screen rows long fill the window before as many
    /// logical lines as it has rows, and those past its end stay unfontified.
    #[test]
    fn redisplay_fontifies_the_screen_rows_a_window_shows() {
        let mut h = hosted();
        h.eval(r#"(progn (setq truncate-partial-width-windows nil) (erase-buffer)
          (let ((i 0)) (while (< i 40) (insert (make-string 200 ?x) "\n") (setq i (1+ i))))
          (goto-char 1) (set-window-start nil 1)
          (setq hosted-fontified nil)
          (setq fontification-functions
                (list (lambda (pos)
                        (setq hosted-fontified (cons pos hosted-fontified))
                        (put-text-property pos (save-excursion (goto-char pos) (forward-line 1) (point)) 'fontified t)))))"#);
        h.paint();
        let called = h.eval("(reverse hosted-fontified)");
        let shown = h.eval("(let ((p 1) starts) (while (< p (window-end)) (setq starts (cons p starts) p (+ p 201))) (reverse starts))");
        assert_eq!(called, shown, "fontified line starts against the window's end");
        let rows = h.eval("(window-body-height)").parse::<usize>().unwrap();
        assert!(called.split(' ').count() < rows, "{called}: fewer lines than the window's {rows} rows");
    }

    /// Fontification that swaps a tall face for a small one shows more of a
    /// continued line than the walk measured with the tall face, so
    /// redisplay fontifies what comes into view, for as many passes as that
    /// takes: every position it paints is fontified.
    #[test]
    fn redisplay_fontifies_what_a_shrinking_face_brings_into_view() {
        let mut h = hosted();
        for chunk in [300, 50] {
            h.eval(&format!(r#"(progn (setq truncate-partial-width-windows nil) (erase-buffer)
              (insert (propertize (make-string 3000 ?x) 'face 'hosted-big))
              (goto-char 1) (set-window-start nil 1)
              (setq fontification-functions
                    (list (lambda (pos)
                            (let ((end (min (point-max) (+ pos {chunk}))))
                              (remove-text-properties pos end '(face nil))
                              (put-text-property pos end 'fontified t))))))"#));
            h.paint();
            let end = h.eval("(window-end)").parse::<i64>().unwrap();
            // Past 300 characters: six 50-character chunks, more passes
            // than the retry bound.
            assert!(end > 301, "{chunk}: the window shows past 300 characters: {end}");
            assert_eq!(h.eval("(text-property-not-all 1 (window-end) 'fontified t)"), "nil", "{chunk}: unfontified text before the window end {end}");
        }
    }

    /// A function that shrinks faces but never sets `fontified': redisplay
    /// walks only the text each pass brings into view, so it calls the
    /// function at ever later positions and stops once the view stops
    /// growing.
    #[test]
    fn redisplay_stops_when_a_shrinking_fontifier_never_fontifies() {
        let mut h = hosted();
        h.eval(r#"(progn (setq truncate-partial-width-windows nil) (erase-buffer)
          (insert (propertize (make-string 3000 ?x) 'face 'hosted-big))
          (goto-char 1) (set-window-start nil 1)
          (setq hosted-calls nil)
          (setq fontification-functions
                (list (lambda (pos)
                        (setq hosted-calls (cons pos hosted-calls))
                        (remove-text-properties pos (min (point-max) (+ pos 50)) '(face nil))))))"#);
        h.paint();
        let calls = h.eval("(reverse hosted-calls)");
        let calls: Vec<i64> = calls.trim_matches(['(', ')']).split(' ').map(|n| n.parse().unwrap()).collect();
        assert!(calls.len() > 1, "{calls:?}: a pass for the text the first one brought into view");
        assert!(calls.windows(2).all(|w| w[0] < w[1]), "{calls:?}: each pass walks only text newly in view");
    }

    /// What the host does not lay out as GNU displays it refuses; its
    /// plain twin answers.
    #[test]
    fn displayed_window_refuses_what_the_host_does_not_lay_out() {
        let mut h = hosted();
        let long = "(make-string (+ (window-body-width) 2) ?x)";
        for (refused, twin, what) in [
            // The host continues lines at the window edge, never at a word.
            (format!("(setq truncate-partial-width-windows nil word-wrap t) (insert {long})"), "(setq word-wrap t) (insert (make-string (- (window-body-width) 2) ?x))".to_owned(), "of a continued or truncated line"),
            ("(insert \"a\\fb\")".into(), "(insert \"a b\")".into(), "of text the host does not lay out as GNU displays it"),
            ("(insert \"a\" (propertize \"b\" 'display \"D\") \"c\")".into(), "(insert \"abc\")".into(), "of text the host does not lay out as GNU displays it"),
            ("(insert \"a\" (propertize \"b\" 'invisible t) \"c\")".into(), "(insert \"abc\")".into(), "of text the host does not lay out as GNU displays it"),
            ("(insert \"abc\") (overlay-put (make-overlay 2 3) 'before-string \"S\")".into(), "(insert \"abc\") (make-overlay 2 3)".into(), "of text the host does not lay out as GNU displays it"),
            ("(insert \"abc\") (overlay-put (make-overlay 2 3) 'face 'hosted-big)".into(), "(insert \"abc\") (overlay-put (make-overlay 2 3) 'mouse-face 'hosted-big)".into(), "of a face the host does not paint"),
            ("(insert (propertize \"abc\" 'face '(hosted-big bold)))".into(), "(insert (propertize \"abc\" 'face 'hosted-big))".into(), "of a face the host does not paint"),
            ("(insert (propertize \"abc\" 'font-lock-face 'hosted-big))".into(), "(insert (propertize \"abc\" 'face 'hosted-big))".into(), "of a face the host does not paint"),
            ("(setq line-prefix \"> \") (insert \"abc\")".into(), "(insert \"abc\")".into(), "of a line or wrap prefix"),
            // Hidden text at the end, or all of it, skips characters the
            // host still paints; so does a multibyte control's escape.
            ("(insert \"a\" (propertize \"b\" 'invisible t))".into(), "(insert \"ab\")".into(), "of text the host does not lay out as GNU displays it"),
            ("(insert \"a\" (propertize \"b\" 'display \"\"))".into(), "(insert \"ab\")".into(), "of text the host does not lay out as GNU displays it"),
            ("(insert (propertize \"ab\" 'invisible t))".into(), "(insert \"ab\")".into(), "of text the host does not lay out as GNU displays it"),
            ("(insert \"a\\u0085b\")".into(), "(insert \"a b\")".into(), "of text the host does not lay out as GNU displays it"),
        ] {
            for (form, refuse) in [(refused.as_str(), true), (twin.as_str(), false)] {
                // Hosted since setup's paint.
                h.eval(&format!("(progn (erase-buffer) (delete-all-overlays) (kill-all-local-variables) {form} (goto-char 1) (set-window-start nil 1))"));
                for call in ["(window-text-pixel-size)", "(posn-at-point 2)", "(pos-visible-in-window-p 2)"] {
                    let answer = h.eval(&format!("(condition-case e {call} (error (car (cdr e))))"));
                    if refuse { assert_eq!(answer, format!("\"VM2 window-text-pixel-size {what} in a displayed window is unsupported\""), "{form} {call}"); }
                    else { assert!(!answer.starts_with('"'), "{form} {call}: {answer}"); }
                }
            }
        }
    }
}
