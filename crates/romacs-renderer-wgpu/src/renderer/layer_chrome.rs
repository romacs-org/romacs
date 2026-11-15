//! Cursor, window border, and scroll bar phases of `render_frame_glyphs`
//! (z-order steps 2-3 collection and step 8 draws).

use romacs_display_protocol::frame_glyphs::FrameGlyph;
use romacs_display_protocol::types::Color;

use super::super::vertex::RectVertex;
use super::WgpuRenderer;
use super::frame_pass::{ChromeLayerVertices, FrameParams, FramePassCtx};

/// The smallest rectangle holding POINTS; None for none.
fn bounding_box(points: impl Iterator<Item = [f32; 2]>) -> Option<romacs_display_protocol::types::Rect> {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for [x, y] in points {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    (x1 >= x0 && y1 >= y0).then(|| romacs_display_protocol::types::Rect::new(x0, y0, x1 - x0, y1 - y0))
}

impl WgpuRenderer {
    /// Collect cursor rects (inverse-video bg, behind-text trail, front
    /// cursors), window borders, and scroll bar tracks/thumbs.
    pub(super) fn collect_chrome_layers(
        &mut self,
        params: &FrameParams<'_>,
    ) -> ChromeLayerVertices {
        let frame_glyphs = params.frame_glyphs;
        let cursor_visible = params.cursor_visible;
        let animated_cursor = params.animated_cursor;
        let hovered_scroll_bar = params.hovered_scroll_bar;
        // === Collect cursor bg rect for inverse video (drawn before text) ===
        // For filled box cursor (style 0), we draw the cursor background BEFORE text
        // so the character under the cursor can be re-drawn with inverse colors on top.
        let cursor_bg_vertices: Vec<RectVertex> = Vec::new();
        let cursor_glow_vertices = Vec::new();

        // === Collect behind-text cursor shapes (animated trail for filled box) ===
        let behind_text_cursor_vertices: Vec<RectVertex> = Vec::new();

        // === Collect front cursors and borders (drawn after text) ===
        // Bar (1), hbar (2), hollow (3), borders — all drawn on top of text.
        // Filled box (0) is EXCLUDED here — handled by bg rect + trail + fg swap.
        // Scroll bar tracks AND thumbs are collected into cursor_vertices too,
        // so both draw through the reliable rect pipeline in draw_cursor_layer.
        let mut cursor_vertices: Vec<RectVertex> = Vec::new();

        for glyph in &frame_glyphs.glyphs {
            match glyph {
                FrameGlyph::Border {
                    x,
                    y,
                    width,
                    height,
                    color,
                    clip_rect,
                    ..
                } => {
                    let mut draw_y = *y;
                    let mut draw_h = *height;
                    if let Some(clip) = clip_rect {
                        let top = clip.y;
                        let bottom = clip.y + clip.height;
                        if draw_y < top {
                            let cut = top - draw_y;
                            if cut >= draw_h {
                                continue;
                            }
                            draw_y = top;
                            draw_h -= cut;
                        }
                        if draw_y + draw_h > bottom {
                            let cut = (draw_y + draw_h) - bottom;
                            if cut >= draw_h {
                                continue;
                            }
                            draw_h -= cut;
                        }
                    }
                    if draw_h > 0.0 {
                        self.add_rect(&mut cursor_vertices, *x, draw_y, *width, draw_h, color);
                    }
                }
                FrameGlyph::ScrollBar {
                    window_id: _,
                    row_role: _,
                    clip_rect: _,
                    horizontal,
                    x,
                    y,
                    width,
                    height,
                    position: _,
                    portion: _,
                    whole: _,
                    thumb_start,
                    thumb_size,
                    track_color,
                    thumb_color,
                } => {
                    // Draw scroll bar track (subtle, configurable opacity)
                    let subtle_track = Color::new(
                        track_color.r,
                        track_color.g,
                        track_color.b,
                        track_color.a * self.effects.scroll_bar.track_opacity,
                    );
                    self.add_rect(&mut cursor_vertices, *x, *y, *width, *height, &subtle_track);

                    // Compute thumb bounds
                    let (tx, ty, tw, th) = if *horizontal {
                        (*x + *thumb_start, *y, *thumb_size, *height)
                    } else {
                        (*x, *y + *thumb_start, *width, *thumb_size)
                    };

                    // Brighten the thumb of the bar the compositor says the
                    // pointer is over. Recognising it by identity rather than
                    // by comparing the pointer's coordinates with this rect is
                    // the whole point: the two live in different spaces for as
                    // long as this bar's pane is moving, and nothing at this
                    // draw site could tell.
                    let hovered = hovered_scroll_bar
                        .is_some_and(|bar| glyph.scroll_bar_identity() == Some(bar));
                    let bright = self.effects.scroll_bar.hover_brightness;
                    let effective_thumb = if hovered {
                        Color::new(
                            (thumb_color.r * bright).min(1.0),
                            (thumb_color.g * bright).min(1.0),
                            (thumb_color.b * bright).min(1.0),
                            thumb_color.a.min(1.0),
                        )
                    } else {
                        *thumb_color
                    };

                    // Draw the thumb as a solid rect through the proven rect
                    // pipeline (into cursor_vertices, after the track, so it lands
                    // on top of it). The rounded_rect "filled" path (border_width
                    // = 0) renders nothing — that pipeline paints no fragments
                    // here — so this is the reliable way to make the thumb visible.
                    // It still tracks GNU's scroll position and portion, just with
                    // square corners instead of GTK's pill.
                    self.add_rect(&mut cursor_vertices, tx, ty, tw, th, &effective_thumb);
                }
                _ => {}
            }
        }

        let mut layers = ChromeLayerVertices {
            cursor_bg: cursor_bg_vertices,
            behind_text_cursor: behind_text_cursor_vertices,
            cursor_glow: cursor_glow_vertices,
            cursors: cursor_vertices,
        };
        // One entry per window (selected window's entry is `active`); every
        // cursor draws through the shared cursor_draw_rect, so a non-selected
        // window's box lands on its glyph cell just like the selected one,
        // under line numbers or scaled fonts.
        for cursor in &frame_glyphs.window_cursors {
            self.emit_window_cursor(frame_glyphs, cursor, cursor_visible, animated_cursor.as_ref(), &mut layers);
        }
        layers
    }

    /// CURSOR of FRAME's visuals into LAYERS.
    fn emit_window_cursor(
        &mut self,
        frame: &romacs_display_protocol::frame_glyphs::FrameGlyphBuffer,
        cursor: &romacs_display_protocol::frame_glyphs::WindowCursor,
        cursor_visible: bool,
        animated_cursor: Option<&romacs_display_protocol::types::AnimatedCursor>,
        layers: &mut ChromeLayerVertices,
    ) {
        let animated_cursor = &animated_cursor.copied();
        let cursor_effects = frame
            .effective_window_cursor_effects(cursor.window_id, &self.effects)
            .clone();
        self.emit_cursor_visual(
            cursor.window_id,
            frame.cursor_draw_rect(
                cursor.slot_id,
                cursor.style,
                cursor.ascent,
                (cursor.x, cursor.y, cursor.width, cursor.height),
            ),
            cursor.slot_id,
            cursor.style,
            &cursor.color,
            cursor.cursor_fg,
            &cursor_effects,
            cursor_visible,
            animated_cursor,
            cursor.active,
            &mut layers.cursor_bg,
            &mut layers.behind_text_cursor,
            &mut layers.cursor_glow,
            &mut layers.cursors,
        );
    }

    /// Where each cursor of FRAME draws this frame, in `window_cursors`'
    /// order: the box of every vertex its layers emit (body, inverse-video
    /// background, trail, light), None for one that draws nothing.
    pub(crate) fn cursor_extents(
        &mut self,
        frame: &romacs_display_protocol::frame_glyphs::FrameGlyphBuffer,
        cursor_visible: bool,
        animated_cursor: Option<&romacs_display_protocol::types::AnimatedCursor>,
    ) -> Vec<Option<romacs_display_protocol::types::Rect>> {
        frame
            .window_cursors
            .iter()
            .map(|cursor| {
                let mut layers = ChromeLayerVertices {
                    cursor_bg: Vec::new(),
                    behind_text_cursor: Vec::new(),
                    cursor_glow: Vec::new(),
                    cursors: Vec::new(),
                };
                self.emit_window_cursor(frame, cursor, cursor_visible, animated_cursor, &mut layers);
                bounding_box(
                    layers
                        .cursor_bg
                        .iter()
                        .chain(&layers.behind_text_cursor)
                        .chain(&layers.cursors)
                        .map(|v| v.position)
                        .chain(layers.cursor_glow.iter().map(|v| v.position)),
                )
            })
            .collect()
    }

    pub(super) fn draw_pre_text_cursor_layers(
        &mut self,
        render_pass: &mut wgpu::RenderPass<'_>,
        cursor_bg_vertices: &[RectVertex],
        behind_text_cursor_vertices: &[RectVertex],
    ) {
        // === Step 2: Draw cursor bg rect (inverse video background) ===
        // Drawn after window/char backgrounds but before text, so the cursor
        // background color is visible behind the inverse-video character.
        self.draw_rect_vertex_layer(render_pass, cursor_bg_vertices);

        // === Step 3: Draw animated cursor trail behind text ===
        // The spring trail or animated rect for filled box cursor appears
        // behind text so characters remain readable during cursor motion.
        self.draw_rect_vertex_layer(render_pass, behind_text_cursor_vertices);
    }

    /// The light around lit cursors, drawn with the cursors, after text: the
    /// cursor-only fast path draws over a retained cursorless scene, so light
    /// drawn anywhere else would sit behind text in one frame and over it in
    /// the next.
    pub(super) fn draw_cursor_glow(
        &mut self,
        render_pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
        glow: &[crate::vertex::RoundedRectVertex],
    ) {
        let Some(upload) = self.arenas.rounded.upload(&self.device, &self.queue, glow) else { return };
        render_pass.set_pipeline(pipeline);
        render_pass.set_vertex_buffer(0, upload.buffer_slice());
        render_pass.draw(0..glow.len() as u32, 0..1);
    }

    /// Draw front cursors and window borders (after text).
    pub(super) fn draw_cursor_layer(
        &mut self,
        ctx: &mut FramePassCtx<'_, '_>,
        chrome: &ChromeLayerVertices,
    ) {
        let render_pass = &mut ctx.pass;
        let rounded = wgpu::RenderPipeline::clone(&self.pipelines.rounded_rect);
        self.draw_cursor_glow(render_pass, &rounded, &chrome.cursor_glow);
        let cursor_vertices = &chrome.cursors;
        // Draw cursors and borders (after text)
        if let Some(upload) = self
            .arenas
            .rect
            .upload(&self.device, &self.queue, &chrome.cursors)
        {
            render_pass.set_pipeline(&self.pipelines.rect);
            render_pass.set_vertex_buffer(0, upload.buffer_slice());
            render_pass.draw(0..cursor_vertices.len() as u32, 0..1);
        }
    }
}
