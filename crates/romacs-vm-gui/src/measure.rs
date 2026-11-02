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
        engine.lay_out(frame)
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
    struct Paint { glyphs: Vec<(char, f32, f32, f32, f32, GlyphRowRole)>, cursor: Option<(f32, f32, f32, f32)>, window_width: u32, window_height: u32,
        rows: Vec<romacs_layout_input::Row> }

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
            let rows = ends.iter().find(|e| e.id == id).map(|e| e.rows.clone()).unwrap_or_default();
            Paint { glyphs, cursor, window_width, window_height, rows }
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

    /// With the pane gap's bottom divider and internal border
    /// (VM2-PANE-CHROME), moving point down a line at a time keeps the
    /// cursor on a whole row above the mode line: the visibility check lays
    /// the window out at the height the painted snapshot has, its bottom
    /// divider excluded, and so scrolls when the painted window does.
    #[test]
    fn point_below_a_window_with_a_bottom_divider_scrolls_it() {
        let mut h = hosted();
        h.vm.set_display_host(Some(Box::new(crate::fonts::Display::new().unwrap())));
        assert!(h.vm.open_display());
        // A body 3 pixels short of 20 whole rows: with its 6-pixel divider
        // counted, it would seem to hold 20.
        let [line, column] = ["(frame-char-height)", "(frame-char-width)"].map(|form| h.eval(form).parse::<u32>().unwrap());
        h.height = 22 * line + 15;
        h.vm.set_frame_geometry(h.width, h.height, line, column).unwrap();
        h.eval(r#"(progn (modify-frame-parameters nil '((bottom-divider-width . 6) (internal-border-width . 6)))
          (erase-buffer) (let ((i 0)) (while (< i 100) (insert (format "line %d\n" i)) (setq i (1+ i))))
          (goto-char 1) (set-window-start nil 1))"#);
        assert_eq!(h.eval("(list (window-bottom-divider-width) (- (window-body-height nil t) (* 20 (frame-char-height))))"), "(6 -3)");
        h.paint();
        let mut scrolled = false;
        for step in 0..30 {
            h.eval("(forward-line 1)");
            let paint = h.paint();
            let band = paint.glyphs.iter().find(|g| g.5 == GlyphRowRole::ModeLine).map(|g| g.2).unwrap();
            let (_, y, _, height) = paint.cursor.unwrap_or_else(|| panic!("step {step}: no cursor on a whole row"));
            assert!(y + height <= band, "step {step}: cursor at {y}+{height} past the mode line at {band}");
            scrolled |= h.eval("(window-start)") != "1";
        }
        assert!(scrolled, "the window scrolled");
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

    /// A row a plain newline starts measures alone, whatever the line
    /// before holds: gnosis's review centres each line with an overlay
    /// string, then measures the next line.  The line holding the string
    /// measures as the host paints it: the space, aligned past the window's
    /// edge, stretches to a pixel short of the continuation glyph, and its
    /// text continues on the next row, where pinned GNU's measure starts
    /// (move_it_by_lines 0 to the screen line showing FROM; harness/wtps_live.py
    /// `own-row-before-string').
    #[test]
    fn a_row_after_a_plain_newline_measures_without_the_line_before() {
        let mut h = hosted();
        h.eval(r#"(progn (erase-buffer) (insert "first line\nwhich fuse") (goto-char 1) (set-window-start nil 1))"#);
        let plain = h.eval("(window-text-pixel-size nil 12 22 1000)");
        h.eval(r#"(overlay-put (make-overlay 1 11) 'before-string (propertize " " 'display '(space :align-to 30)))"#);
        assert_eq!(h.eval("(window-text-pixel-size nil 12 22 1000)"), plain);
        h.eval("(setq truncate-partial-width-windows nil)");
        let paint = h.paint();
        let column = h.eval("(frame-char-width)").parse::<f32>().unwrap();
        let first: Vec<_> = paint.glyphs.iter().filter(|g| g.5 == GlyphRowRole::Text && g.2 == 0.0).collect();
        assert_eq!((first[0].0, first[0].1, first[0].1 + first[0].3), ('\t', 0.0, paint.window_width as f32 - first.last().unwrap().3 - 1.0),
                   "the space stretches to a pixel short of the continuation glyph, column {column}");
        let second: f32 = paint.glyphs.iter().filter(|g| g.5 == GlyphRowRole::Text && g.2 == paint.rows[1].y).map(|g| g.3).sum();
        assert_eq!(h.eval("(window-text-pixel-size nil 1 11)"), cons(second, paint.rows[1].height));
    }

    /// `window-end' with UPDATE lays the window out from its start as
    /// Fwindow_end (window.c:1782) does: the end the host's paint then
    /// records, asked before that paint catches up with the edit.
    #[test]
    fn window_end_update_answers_the_end_the_host_paints() {
        let mut h = hosted();
        for insert in [
            r#"(dotimes (i 40) (insert (format "line %d\n" i)))"#,
            r#"(insert "short\nno newline")"#,
            r#"(insert "short\n")"#,
            r#"(insert "")"#,
            r#"(dotimes (i 18) (insert "x\n")) (insert (propertize "BIG\n" 'face 'hosted-big) "after\nmore\n")"#,
            r#"(dotimes (i 40) (insert (format "line %d\n" i))) (narrow-to-region 20 60)"#,
        ] {
            h.eval(&format!("(progn (widen) (erase-buffer) {insert} (goto-char (point-min)) (set-window-start nil (point-min)))"));
            let update = h.eval("(window-end nil t)");
            h.paint();
            assert_eq!(update, h.eval("(window-end)"), "{insert}");
        }
        h.eval("(widen)");
    }

    /// A header line paints above the text, and pos-visible-in-window-p
    /// counts from the window's top as GNU's does (window_top_y), and
    /// posn-at-point from the text area: the text starts below the band,
    /// whose height the body loses.
    #[test]
    fn a_header_line_paints_above_the_text_and_shrinks_the_body() {
        let mut h = hosted();
        h.eval(r#"(progn (erase-buffer) (dotimes (i 40) (insert (format "line %d\n" i)))
                         (setq header-line-format "Header") (goto-char 1) (set-window-start nil 1))"#);
        let update = h.eval("(window-end nil t)");
        let paint = h.paint();
        let header: String = paint.glyphs.iter().filter(|g| g.5 == GlyphRowRole::HeaderLine).map(|g| g.0).collect();
        assert_eq!(header, "Header");
        let band = paint.glyphs.iter().find(|g| g.5 == GlyphRowRole::HeaderLine).map(|g| (g.2, g.4)).unwrap();
        let first = paint.glyphs.iter().find(|g| g.5 == GlyphRowRole::Text).unwrap();
        assert_eq!((band.0, first.0, first.2), (0.0, 'l', band.1), "the text starts below the band");
        let mode_y = paint.glyphs.iter().find(|g| g.5 == GlyphRowRole::ModeLine).map(|g| g.2).unwrap();
        let body = (mode_y - band.1) as i64;
        assert_eq!(h.eval("(list (window-header-line-height) (window-body-height nil t))"), format!("({} {body})", band.1 as i64));
        assert_eq!(h.eval("(pos-visible-in-window-p 1 nil t)"), format!("(0 {})", band.1 as i64));
        // posn-at-point's (X . Y) counts from the text area (harness/wtps_live.py `tabulated-list').
        assert_eq!(h.eval("(nth 2 (posn-at-point 1))"), "(0 . 0)");
        assert_eq!(update, h.eval("(window-end)"));
        h.eval("(setq header-line-format nil)");
    }

    /// `recenter' and `scroll-up' move by the host's screen lines:
    /// M->'s `(recenter -3)' paints point three lines up from the bottom
    /// row, and `scroll-up' paints from the start it set, keeping
    /// `next-screen-context-lines' of the old page.
    #[test]
    fn recenter_and_scroll_up_set_the_start_the_host_paints() {
        let mut h = hosted();
        h.eval(r#"(progn (erase-buffer) (dotimes (i 100) (insert (format "line %d\n" i))) (goto-char (point-max)) (recenter -3))"#);
        let paint = h.paint();
        let line = h.eval("(frame-char-height)").parse::<f32>().unwrap();
        let rows = h.eval("(window-body-height)").parse::<f32>().unwrap();
        let cursor = paint.cursor.expect("cursor");
        assert_eq!(cursor.1, (rows - 3.0) * line, "point on the third row from the bottom");
        let first: String = paint.glyphs.iter().filter(|g| g.5 == GlyphRowRole::Text).take_while(|g| g.2 == 0.0).map(|g| g.0).collect();
        let start = h.eval("(save-excursion (goto-char (window-start)) (buffer-substring-no-properties (point) (line-end-position)))");
        assert_eq!(format!("{first:?}"), start);
        h.eval("(progn (goto-char 1) (set-window-start nil 1) (scroll-up))");
        let paint = h.paint();
        let first: String = paint.glyphs.iter().filter(|g| g.5 == GlyphRowRole::Text).take_while(|g| g.2 == 0.0).map(|g| g.0).collect();
        let expected = h.eval("(format \"line %d\" (- (window-body-height) next-screen-context-lines))");
        assert_eq!(format!("{first:?}"), expected, "scroll-up keeps the old page's last lines");
    }

    /// The display elements GNU's iterator delivers, as the host paints
    /// them: each row's glyphs and stretches, and the VM's answers over
    /// them. ROWS is each painted row's text, a stretch as a TAB; each
    /// logical line is one row, measured to its newline, and where the host
    /// shows the cursor for a position, pos-visible-in-window-p and
    /// posn-at-point answer its place and glyph.
    fn agree_painted(h: &mut Hosted, insert: &str, rows: &[&str]) {
        // GNU's kill-all-local-variables resets the invisibility spec
        // (buffer.c:1163); Romacs's keeps the last case's.
        h.eval(&format!("(progn (erase-buffer) (delete-all-overlays) (kill-all-local-variables) (setq buffer-invisibility-spec t truncate-partial-width-windows nil) {insert} (goto-char 1) (set-window-start nil 1))"));
        let paint = h.paint();
        let mut text: Vec<_> = paint.glyphs.iter().filter(|g| g.5 == GlyphRowRole::Text).copied().collect();
        text.sort_by(|a, b| (a.2, a.1).partial_cmp(&(b.2, b.1)).unwrap());
        let row_of = |y: f32| paint.rows.iter().position(|r| r.y == y).unwrap();
        let mut painted = vec![String::new(); paint.rows.len()];
        let mut right = vec![0.0f32; paint.rows.len()];
        for g in &text {
            painted[row_of(g.2)].push(g.0);
            right[row_of(g.2)] = right[row_of(g.2)].max(g.1 + g.3);
        }
        assert_eq!(painted, rows, "{insert}: painted rows");
        let last = h.eval("(point-max)").parse::<usize>().unwrap();
        let line_ends: Vec<usize> = h.eval("(let (ends) (save-excursion (goto-char 1) (while (search-forward \"\\n\" nil t) (push (1- (point)) ends))) (nreverse ends))")
            .trim_matches(['(', ')']).split(' ').filter(|n| !n.is_empty() && *n != "nil").map(|n| n.parse().unwrap()).collect();
        let mut start = 1;
        for (row, &end) in line_ends.iter().chain([last].iter()).enumerate() {
            let size = cons(right[row], paint.rows[row].height);
            if end > start { assert_eq!(h.eval(&format!("(window-text-pixel-size nil {start} {end})")), size, "{insert}: line {row}"); }
            start = end + 1;
        }
        let whole = cons(right.iter().copied().fold(0.0, f32::max), paint.rows.iter().map(|r| r.height).sum());
        assert_eq!(h.eval(&format!("(window-text-pixel-size nil nil {last})")), whole, "{insert}: whole");
        assert_eq!(h.eval("(buffer-text-pixel-size)"), whole, "{insert}: buffer");
        for pos in 1..=last {
            h.eval(&format!("(goto-char {pos})"));
            let Some((x, y, _, _)) = h.paint().cursor else { continue };
            let visible = h.eval(&format!("(pos-visible-in-window-p {pos} nil t)"));
            // Around an ellipsis pos_visible_p answers where the text before
            // it ends, not where the cursor shows (harness/wtps_live.py
            // `ellipsis').
            if h.eval(&format!("(and (or (invisible-p {pos}) (invisible-p (max 1 (1- {pos})))) t)")) == "t" { continue; }
            assert_eq!(visible, format!("({} {})", x as i64, y as i64), "{insert}: visible {pos}");
            let posn = h.eval(&format!("(let ((p (posn-at-point {pos}))) (list (nth 2 p) (nth 9 p)))"));
            let size = text.iter().find(|g| (g.1, g.2) == (x, y)).map(|g| cons(g.3, g.4));
            if let Some(size) = size { assert_eq!(posn, format!("({} {size})", cons(x, y)), "{insert}: posn-at-point {pos}"); }
        }
    }

    /// `display' strings and spaces, invisible text, overlay strings and
    /// faces, face lists: the host paints what GNU's iterator delivers in
    /// the faces GNU merges, and the VM answers from that layout.
    #[test]
    fn display_elements_answer_as_the_host_paints_them() {
        let mut h = hosted();
        for (insert, rows) in [
            (r#"(insert "a" (propertize "b" 'display "DISP") "c\nnext")"#, &["aDISPc", "next"][..]),
            (r#"(insert "ab" (propertize "hid" 'invisible t) "cd\nx")"#, &["abcd", "x"]),
            (r#"(insert "a" (propertize " " 'display '(space :width 3)) "b\nx" (propertize " " 'display '(space :align-to 12)) "y")"#, &["a\tb", "x\ty"]),
            (r#"(insert "abc" (propertize " " 'display '(space :align-to right)) "\nz")"#, &["abc\t", "z"]),
            (r#"(insert (propertize " " 'display '(space :width text)) "\nz")"#, &["\t", "z"]),
            (r#"(insert "abc\ndef") (overlay-put (make-overlay 2 3) 'before-string "BEF") (overlay-put (make-overlay 5 6) 'after-string (propertize "AFT" 'face 'hosted-big))"#, &["aBEFbc", "dAFTef"]),
            (r#"(insert "abc\ndef") (overlay-put (make-overlay 2 3) 'face 'hosted-big)"#, &["abc", "def"]),
            (r#"(insert "a" (propertize "b" 'display (propertize "WIDE" 'face 'hosted-big)) "c")"#, &["aWIDEc"]),
            (r#"(setq buffer-invisibility-spec '((hide . t))) (insert "ab" (propertize "xyz" 'invisible 'hide) "cd")"#, &["ab...cd"]),
            (r#"(insert (propertize " " 'display '(space :width 2) 'face 'hosted-small) " " (propertize "Run" 'face 'hosted-sans) " 1s" (propertize " " 'display '(space :align-to right)))"#, &["\t Run 1s\t"]),
            (r#"(insert (propertize "abc" 'face '(hosted-big bold)) "\n" (propertize "xyz" 'font-lock-face 'hosted-big))"#, &["abc", "xyz"]),
            (r#"(insert "a" (propertize "b" 'display "") "c")"#, &["ac"]),
            (r#"(insert "a" (propertize "bc" 'invisible t) "\nd")"#, &["a", "d"]),
            // Org's table separators: a space as wide as the glyph it covers.
            (r#"(insert "a" (propertize "-" 'display '(space :relative-width 1)) "b" (propertize "W" 'display '(space :relative-width 2) 'face 'hosted-sans) "c")"#, &["a\tb\tc"]),
            // Line numbers start every row, before its TAB stops and
            // alignments; point's line shows its own number.
            (r#"(setq display-line-numbers t) (insert "a\tb\nc" (propertize " " 'display '(space :align-to 6)) "d")"#, &["  1 a\tb", "  2 c\td"]),
            (r#"(setq display-line-numbers 'relative) (insert "one\ntwo\nthree")"#, &["  1 one", "  1 two", "  2 three"]),
        ] { agree_painted(&mut h, insert, rows); }
        // A line after hidden ones shows its own, longer number.
        h.eval(r#"(progn (erase-buffer) (setq buffer-invisibility-spec t display-line-numbers t) (insert "a\n" (propertize (make-string 120 ?\n) 'invisible t) "b") (goto-char 1) (set-window-start nil 1))"#);
        let paint = h.paint();
        let mut glyphs: Vec<_> = paint.glyphs.iter().filter(|g| g.5 == GlyphRowRole::Text).collect();
        glyphs.sort_by(|a, b| (a.2, a.1).partial_cmp(&(b.2, b.1)).unwrap());
        let second: String = glyphs.iter().filter(|g| g.2 > 0.0).map(|g| g.0).collect();
        assert_eq!(second, "122 b");
        let b = glyphs.iter().find(|g| g.0 == 'b').unwrap().1;
        assert_eq!(h.eval("(car (pos-visible-in-window-p (point-max) nil t))"), (b as i64 + 10).to_string());
        // A continued row's gutter is blanks in the line numbers' face.
        h.eval(r#"(progn (erase-buffer) (setq truncate-partial-width-windows nil display-line-numbers t) (insert (make-string 40 ?x)) (goto-char 1) (set-window-start nil 1))"#);
        let paint = h.paint();
        let mut glyphs: Vec<_> = paint.glyphs.iter().filter(|g| g.5 == GlyphRowRole::Text).collect();
        glyphs.sort_by(|a, b| (a.2, a.1).partial_cmp(&(b.2, b.1)).unwrap());
        let second: String = glyphs.iter().filter(|g| g.2 > 0.0).map(|g| g.0).collect();
        assert!(second.starts_with("    x"), "{second:?}");
        // With line numbers, vertical motion moves to the next line's start.
        h.eval(r#"(progn (erase-buffer) (setq display-line-numbers t) (insert "first\nsecond\nthird") (goto-char 3) (set-window-start nil 1))"#);
        h.paint();
        assert_eq!(h.eval("(list (vertical-motion 1) (point) (line-number-display-width) (line-number-display-width 'columns))"), "(1 7 2 4.0)");
        h.eval("(setq display-line-numbers nil)");
        // The face lists and overlay faces GNU merges size the glyphs.
        h.eval(r#"(progn (erase-buffer) (insert "abc") (goto-char 1))"#);
        let plain = h.eval("(window-text-pixel-size)");
        h.eval("(overlay-put (make-overlay 2 3) 'face 'hosted-big)");
        assert_ne!(h.eval("(window-text-pixel-size)"), plain, "an overlay's face makes its text bigger");
        // A string's glyph names the string in posn-at-point.
        h.eval(r#"(progn (erase-buffer) (delete-all-overlays) (insert "a" (propertize "b" 'display "XY") "c") (goto-char 2))"#);
        h.paint();
        assert_eq!(h.eval("(nth 4 (posn-at-point 2))"), "(\"XY\" . 0)");
    }

    /// A window publishes the elements its rows show: past lines it hides,
    /// with the strings an overlay ending at its start shows there.
    #[test]
    fn publication_reaches_what_the_rows_show() {
        let mut h = hosted();
        h.eval(r#"(progn (erase-buffer) (setq buffer-invisibility-spec t) (insert (propertize (make-string 100 ?\n) 'invisible t) "VISIBLE\nend") (goto-char 1) (set-window-start nil 1))"#);
        let paint = h.paint();
        let text: String = paint.glyphs.iter().filter(|g| g.5 == GlyphRowRole::Text).map(|g| g.0).collect();
        assert_eq!(text, "VISIBLEend");
        assert_eq!(h.eval("(window-end)"), h.eval("(window-end nil t)"));
        h.eval(r#"(progn (erase-buffer) (insert "old\nnew\nend") (overlay-put (make-overlay 1 5) 'after-string "AFTER") (goto-char 5) (set-window-start nil 5))"#);
        let paint = h.paint();
        let first: String = paint.glyphs.iter().filter(|g| g.5 == GlyphRowRole::Text && g.2 == 0.0).map(|g| g.0).collect();
        assert_eq!(first, "AFTERnew");
        let x = paint.glyphs.iter().find(|g| g.0 == 'n').unwrap().1;
        assert_eq!(h.eval("(car (pos-visible-in-window-p 5 nil t))"), (x as i64).to_string());
        // An empty buffer still shows an overlay's string, before point.
        h.eval(r#"(progn (delete-all-overlays) (erase-buffer) (overlay-put (make-overlay 1 1) 'before-string "BEFORE") (set-window-start nil 1))"#);
        let paint = h.paint();
        let shown: String = paint.glyphs.iter().filter(|g| g.5 == GlyphRowRole::Text).map(|g| g.0).collect();
        assert_eq!(shown, "BEFORE");
        assert_eq!(h.eval("(car (pos-visible-in-window-p 1 nil t))"), (paint.cursor.unwrap().0 as i64).to_string());
        // An overlay string's space is no text point shows on.
        h.eval(r#"(progn (delete-all-overlays) (insert "ab") (overlay-put (make-overlay 1 1) 'before-string (propertize " " 'display '(space :relative-width 2))) (goto-char 1) (set-window-start nil 1))"#);
        let paint = h.paint();
        let a = paint.glyphs.iter().find(|g| g.0 == 'a').unwrap().1;
        assert!(a > 0.0);
        assert_eq!(paint.cursor.unwrap().0, a);
        assert_eq!(h.eval("(car (pos-visible-in-window-p 1 nil t))"), (a as i64).to_string());
        h.eval("(delete-all-overlays)");
    }

    /// A package's `make-display-table' leaves `standard-display-table' an
    /// empty table; vertical motion (next-line) still runs on the host's
    /// layout rather than refusing.
    #[test]
    fn an_empty_standard_display_table_leaves_motion_to_the_host() {
        let mut h = hosted();
        // disp-table.el's display tables, which this VM has not loaded.
        h.eval("(put 'display-table 'char-table-extra-slots 18)");
        h.eval("(progn (erase-buffer) (insert \"ab\\ncd\\n\") (goto-char 1) (set-window-start nil 1))");
        assert_eq!(h.eval("(let ((standard-display-table (make-char-table 'display-table))) (list (vertical-motion 1) (point)))"), "(1 4)");
    }

    /// What the host does not lay out as GNU displays it refuses; its
    /// plain twin answers.
    #[test]
    fn displayed_window_refuses_what_the_host_does_not_lay_out() {
        let mut h = hosted();
        h.eval("(put 'display-table 'char-table-extra-slots 18)");
        let long = "(make-string (+ (window-body-width) 2) ?x)";
        for (refused, twin, what) in [
            // The host continues lines at the window edge, never at a word.
            (format!("(setq truncate-partial-width-windows nil word-wrap t) (insert {long})"), "(setq word-wrap t) (insert (make-string (- (window-body-width) 2) ?x))".to_owned(), "of a continued or truncated line"),
            // It draws a control character's escape itself, as one glyph.
            ("(insert \"a\\fb\")".into(), "(insert \"a b\")".into(), "of text the host does not lay out as GNU displays it"),
            ("(insert \"a\\u0085b\")".into(), "(insert \"a b\")".into(), "of text the host does not lay out as GNU displays it"),
            ("(setq line-prefix \"> \") (insert \"abc\")".into(), "(insert \"abc\")".into(), "of a line or wrap prefix"),
            ("(insert \"a\" (propertize \" \" 'display '(space :width 2 :height 2)) \"c\")".into(), "(insert \"a\" (propertize \" \" 'display '(space :width 2)) \"c\")".into(), "of a space with its own height"),
            // A display table that maps glyphs refuses; an empty one does not.
            ("(setq buffer-display-table (make-char-table 'display-table)) (aset buffer-display-table ?b [?B]) (insert \"abc\")".into(),
             "(setq buffer-display-table (make-char-table 'display-table)) (insert \"abc\")".into(), "with a display table"),
            ("(setq buffer-display-table (make-char-table 'display-table)) (set-char-table-extra-slot buffer-display-table 1 ?|) (insert \"abc\")".into(),
             "(setq buffer-display-table (make-char-table 'display-table)) (insert \"abc\")".into(), "with a display table"),
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
