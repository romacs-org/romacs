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
