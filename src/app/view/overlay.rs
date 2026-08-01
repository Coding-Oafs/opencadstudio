use super::super::Message;
use iced::widget::{
    button, column, container, mouse_area, row, stack, text, text_input,
    Space,
};
use iced::{Background, Border, Color, Element, Fill, Theme};

pub(super) fn position_canvas_overlay<'a>(
    anchor: iced::Point,
    panel: Element<'a, Message>,
) -> Element<'a, Message> {
    iced::widget::pin(iced::widget::opaque(panel))
        .position(iced::Point::new(anchor.x.max(0.0), anchor.y.max(0.0)))
        .into()
}

// ── In-place MText editor overlay ───────────────────────────────────────────

/// Widget id for the MText editor's text area (focused when Edit mode opens).
pub(in crate::app) const MTEXT_TEXT_ID: &str = "mtext_editor_text";

/// Widget id for the in-place TEXT editor's input (focused when it opens).
pub(in crate::app) const TEXT_INLINE_ID: &str = "text_inline_input";

/// In-place single-line TEXT editor: a plain text-entry box (no formatting
/// toolbar), anchored at the insertion-point click. Enter commits; Esc cancels.
pub(super) fn text_inline_overlay(
    ed: &super::super::text_inline::TextInlineState,
    canvas: (f32, f32),
) -> Element<'_, Message> {
    let field = text_input("Text", &ed.value)
        .id(iced::widget::Id::new(TEXT_INLINE_ID))
        .on_input(Message::TextInlineInput)
        .on_submit(Message::TextInlineOk)
        .padding(6)
        .size(13)
        .width(iced::Length::Fixed(240.0));

    let panel = container(field)
        .style(move |theme: &Theme| {
            let palette = theme.palette();
            container::Style {
            background: Some(Background::Color(palette.background.weak.color)),
            border: Border {
                color: palette.background.neutral.color,
                width: 1.0,
                radius: 5.0.into(),
            },
            ..Default::default()
            }
        })
        .padding(4);

    // Keep the box on-screen so its field stays clickable at the edges.
    const PANEL_W: f32 = 240.0 + 20.0;
    const PANEL_H: f32 = 46.0;
    let (cw, ch) = canvas;
    let anchor = iced::Point::new(
        (ed.screen_anchor.x - 6.0).clamp(0.0, (cw - PANEL_W).max(0.0)),
        (ed.screen_anchor.y - 18.0).clamp(0.0, (ch - PANEL_H).max(0.0)),
    );
    position_canvas_overlay(anchor, panel.into())
}

// Stroke-font families the renderer ships (LibreCAD LFF; see scene/lff.rs).
const MTEXT_FONTS: [&str; 10] = [
    "[Style default]",
    "Standard",
    "ISO",
    "Simplex",
    "RomanS",
    "RomanD",
    "ItalicC",
    "ScriptS",
    "GothGBT",
    "RomanC",
];
/// (label, ACI). 256 = ByLayer.
/// Canvas program that renders the tessellated MText strokes inside the
/// editor's own preview area (never on the drawing). Strokes lie in the
/// world XY plane; the program fits + vertically flips them into the box.
pub(in crate::app) const MTEXT_PREVIEW_PAD: f32 = 12.0;
pub(in crate::app) const MTEXT_PREVIEW_EM_PX: f32 = 15.0;
// Used only until the shared modal sensor reports the first real layout width.
const MTEXT_EDITOR_FALLBACK_WIDTH: f32 = 660.0;
pub(in crate::app) const MTEXT_EDITOR_WRITING_WIDTH: f32 =
    MTEXT_EDITOR_FALLBACK_WIDTH - 2.0 * MTEXT_PREVIEW_PAD;

struct MTextPreview {
    /// Disconnected polylines as (x, y) world points + colour (NaN-split done).
    segments: Vec<(Vec<(f32, f32)>, Color, f32)>,
    /// Per-visible-character boxes (world frame) for click-to-select.
    boxes: Vec<crate::entities::text_support::GlyphBox>,
    /// Current selection as a visible-char range.
    sel: Option<(usize, usize)>,
    /// Caret position as a visible-char offset.
    caret: usize,
    /// Whether the caret is in its visible blink phase.
    caret_on: bool,
    /// World-space min corner (bbox) and pixels-per-world-unit scale.
    minx: f32,
    miny: f32,
    scale: f32,
    content_h: f32,
    /// On-screen width of the MText wrapping rectangle.
    wrap_width_px: f32,
}

impl MTextPreview {
    /// Visible-char offset (0..=N) nearest the cursor point (bounds-local px).
    fn offset_at(&self, p: iced::Point) -> usize {
        if self.boxes.is_empty() {
            return 0;
        }
        let wx = self.minx + (p.x - MTEXT_PREVIEW_PAD) / self.scale;
        let wy = self.miny + (self.content_h - p.y - MTEXT_PREVIEW_PAD) / self.scale;
        let mut best = 0usize;
        let mut best_d = f32::MAX;
        for b in &self.boxes {
            let dx = if wx < b.xmin {
                b.xmin - wx
            } else if wx > b.xmax {
                wx - b.xmax
            } else {
                0.0
            };
            let dy = if wy < b.ymin {
                b.ymin - wy
            } else if wy > b.ymax {
                wy - b.ymax
            } else {
                0.0
            };
            let d = dy * 1000.0 + dx; // prefer the correct line first
            if d < best_d {
                best_d = d;
                best = b.vis;
                // After the glyph centre → caret sits after this char.
                if wx > (b.xmin + b.xmax) * 0.5 {
                    best = b.vis + 1;
                }
            }
        }
        best
    }
}

#[derive(Default)]
struct MTextPreviewState {
    dragging: bool,
}

impl iced::widget::canvas::Program<Message> for MTextPreview {
    type State = MTextPreviewState;

    fn update(
        &self,
        state: &mut MTextPreviewState,
        event: &iced::Event,
        bounds: iced::Rectangle,
        cursor: iced::mouse::Cursor,
    ) -> Option<iced::widget::canvas::Action<Message>> {
        use iced::mouse::{Button, Event as Me};
        use iced::widget::canvas::Action;
        use iced::Event;
        match event {
            Event::Mouse(Me::ButtonPressed(Button::Left)) => {
                if let Some(p) = cursor.position_in(bounds) {
                    state.dragging = true;
                    let off = self.offset_at(p);
                    return Some(Action::publish(Message::MTextSelStart(off)).and_capture());
                }
            }
            Event::Mouse(Me::CursorMoved { .. }) => {
                if state.dragging {
                    if let Some(p) = cursor.position_in(bounds) {
                        let off = self.offset_at(p);
                        return Some(Action::publish(Message::MTextSelTo(off)));
                    }
                }
            }
            Event::Mouse(Me::ButtonReleased(Button::Left)) => {
                if state.dragging {
                    state.dragging = false;
                    return Some(Action::capture());
                }
            }
            _ => {}
        }
        None
    }

    fn draw(
        &self,
        _state: &MTextPreviewState,
        renderer: &iced::Renderer,
        theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<iced::widget::canvas::Geometry> {
        use iced::widget::canvas::{Frame, Path, Stroke};
        let mut frame = Frame::new(renderer, bounds.size());
        let pad = MTEXT_PREVIEW_PAD;
        // Draw at the real size; flip Y (world up → screen down).
        let map = |x: f32, y: f32| {
            iced::Point::new(
                pad + (x - self.minx) * self.scale,
                self.content_h - (pad + (y - self.miny) * self.scale),
            )
        };
        // Wrap ruler: same screen-space position as the width slider handle.
        let wrap_x = (pad + self.wrap_width_px).clamp(pad, bounds.width.max(pad));
        let wrap_line = Path::new(|path| {
            path.move_to(iced::Point::new(wrap_x, 0.0));
            path.line_to(iced::Point::new(wrap_x, self.content_h));
        });
        frame.stroke(
            &wrap_line,
            Stroke::default()
                .with_color(theme.palette().primary.base.color.scale_alpha(0.65))
                .with_width(1.0),
        );
        // Selection highlight behind the glyphs.
        if let Some((a, b)) = self.sel {
            for bx in &self.boxes {
                if bx.vis >= a && bx.vis < b {
                    let p0 = map(bx.xmin, bx.ymax);
                    let p1 = map(bx.xmax, bx.ymin);
                    let rect = Path::rectangle(
                        iced::Point::new(p0.x.min(p1.x), p0.y.min(p1.y)),
                        iced::Size::new((p1.x - p0.x).abs(), (p1.y - p0.y).abs()),
                    );
                    frame.fill(
                        &rect,
                        theme.palette().primary.base.color.scale_alpha(0.45),
                    );
                }
            }
        }
        for (seg, col, width) in &self.segments {
            if seg.len() < 2 {
                continue;
            }
            let path = Path::new(|p| {
                p.move_to(map(seg[0].0, seg[0].1));
                for &(x, y) in &seg[1..] {
                    p.line_to(map(x, y));
                }
            });
            frame.stroke(&path, Stroke::default().with_color(*col).with_width(*width));
        }
        // Caret — a vertical bar at the caret's glyph boundary, shown when the
        // selection is empty (a plain text cursor).
        // Caret is shown only when the selection is empty and the blink is in
        // its visible phase.
        let collapsed = self.caret_on && self.sel.map(|(a, b)| a == b).unwrap_or(true);
        if collapsed && self.boxes.is_empty() {
            // Empty text: show a caret at the top-left so the user can type.
            // Its height matches the preview's constant em (see EM_PX) so the
            // empty-editor caret isn't tiny.
            let path = Path::new(|p| {
                p.move_to(iced::Point::new(MTEXT_PREVIEW_PAD, MTEXT_PREVIEW_PAD));
                p.line_to(iced::Point::new(
                    MTEXT_PREVIEW_PAD,
                    (MTEXT_PREVIEW_PAD + 40.0).min(self.content_h),
                ));
            });
            frame.stroke(
                &path,
                Stroke::default()
                    .with_color(theme.palette().warning.base.color)
                    .with_width(1.5),
            );
        } else if collapsed {
            let bar = if let Some(b) = self.boxes.iter().find(|b| b.vis == self.caret) {
                Some((b.xmin, b.ymin, b.ymax)) // left edge of the caret's glyph
            } else if self.caret > 0 {
                self.boxes
                    .iter()
                    .find(|b| b.vis == self.caret - 1)
                    .map(|b| (b.xmax, b.ymin, b.ymax)) // after the last glyph
            } else {
                self.boxes.first().map(|b| (b.xmin, b.ymin, b.ymax))
            };
            if let Some((cx, y0, y1)) = bar {
                let p0 = map(cx, y0);
                let p1 = map(cx, y1);
                let path = Path::new(|p| {
                    p.move_to(p0);
                    p.line_to(p1);
                });
                frame.stroke(
                    &path,
                    Stroke::default()
                        .with_color(theme.palette().warning.base.color)
                        .with_width(1.5),
                );
            }
        }
        vec![frame.into_geometry()]
    }
}

/// Split every preview WireModel into finite (x, y) polyline runs, each
/// carrying its wire's colour (so inline `\C` / the colour dropdown shows) and a
/// stroke width (bold runs carry a wider pen via `line_weight_px`).
fn mtext_preview_segments(
    ed: &super::super::mtext_editor::MTextEditorState,
) -> Vec<(Vec<(f32, f32)>, Color, f32)> {
    let mut out: Vec<(Vec<(f32, f32)>, Color, f32)> = Vec::new();
    for w in &ed.preview_wires {
        let col = Color {
            r: w.color[0],
            g: w.color[1],
            b: w.color[2],
            a: 1.0,
        };
        // Bold text wires bake a wider pen (line_weight_px ~2.4); draw them thick.
        let width = if w.line_weight_px > 1.5 { 2.6 } else { 1.4 };
        let mut run: Vec<(f32, f32)> = Vec::new();
        for p in &w.points {
            if p[0].is_finite() && p[1].is_finite() {
                run.push((p[0], p[1]));
            } else if !run.is_empty() {
                out.push((std::mem::take(&mut run), col, width));
            }
        }
        if !run.is_empty() {
            out.push((run, col, width));
        }
    }
    out
}

pub(super) fn mtext_editor_overlay<'a>(
    ed: &'a super::super::mtext_editor::MTextEditorState,
    styles: Vec<String>,
    modal_offset: iced::Vector,
    modal_resize: iced::Vector,
    modal_content_size: Option<iced::Size>,
) -> Element<'a, Message> {
    let writing_area_px = modal_content_size
        .map(|size| size.width - 2.0 * MTEXT_PREVIEW_PAD)
        .unwrap_or(MTEXT_EDITOR_WRITING_WIDTH)
        .max(80.0);
    let measurement = mtext_editor_content(
        ed,
        &styles,
        crate::ui::modal::ModalSizing::INTRINSIC,
        writing_area_px,
        (writing_area_px * 0.5).max(1.0),
    );
    let content = mtext_editor_content(
        ed,
        &styles,
        crate::ui::modal::ModalSizing::FILL,
        writing_area_px,
        (writing_area_px * 0.5).max(1.0),
    );
    let content = crate::ui::modal::intrinsic(
        measurement,
        content,
        iced::Size::INFINITE,
        modal_resize,
    );

    crate::ui::modal::modal(
        iced::widget::Space::new().width(Fill).height(Fill),
        "Text Editor",
        content,
        Message::MTextCancel,
        modal_offset,
        crate::ui::modal::ModalOptions::STANDARD,
    )
}

fn mtext_editor_content<'a>(
    ed: &'a super::super::mtext_editor::MTextEditorState,
    styles: &[String],
    sizing: crate::ui::modal::ModalSizing,
    writing_area_px: f32,
    intrinsic_preview_height: f32,
) -> Element<'a, Message> {
    let width = sizing.width;
    let height = sizing.height;
    let preview_height = if matches!(height, iced::Length::Fill) {
        iced::Length::Fill
    } else {
        iced::Length::Fixed(intrinsic_preview_height)
    };
    use super::super::mtext_editor::{JustifyChoice, MTextFmt, ParaAlign};
    use iced::widget::canvas;

    let btn_style = |theme: &Theme, status: button::Status| {
        let palette = theme.palette();
        let pair = match status {
            button::Status::Hovered | button::Status::Pressed => palette.background.strong,
            _ => palette.background.weak,
        };
        button::Style {
        background: Some(Background::Color(pair.color)),
        text_color: pair.text,
        border: Border {
            color: palette.background.neutral.color,
            width: 1.0,
            radius: 3.0.into(),
        },
        shadow: iced::Shadow::default(),
        snap: false,
        }
    };
    let icon_btn = move |bytes: &'static [u8], msg: Message| -> Element<'static, Message> {
        button(crate::ui::icons::themed(bytes, 18.0))
            .on_press(msg)
            .padding(3)
            .style(btn_style)
            .into()
    };
    let lbl = |s: &'static str| text(s).size(11);
    let small_input = |placeholder: &'static str,
                       val: &str,
                       on: fn(String) -> Message,
                       w: f32|
     -> Element<'static, Message> {
        text_input(placeholder, val)
            .on_input(on)
            .width(iced::Length::Fixed(w))
            .padding(3)
            .size(12)
            .into()
    };

    // ── Row 1: style / font / height · format icons · colour ──────────────
    let style_opts: Vec<String> = if styles.is_empty() {
        vec!["Standard".to_string()]
    } else {
        styles.to_vec()
    };
    let style_pl = iced::widget::pick_list(
        Some(ed.style.clone()),
        style_opts,
        |value| value.to_string(),
    )
        .on_select(Message::MTextStyle)
        .text_size(11)
        .width(iced::Length::Fixed(96.0));
    let font_sel = if ed.font.trim().is_empty() {
        "[Style default]".to_string()
    } else {
        ed.font.clone()
    };
    let font_pl = iced::widget::pick_list(
        Some(font_sel),
        MTEXT_FONTS
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
        |value| value.to_string(),
    )
    .on_select(Message::MTextFont)
    .text_size(11)
    .width(iced::Length::Fixed(120.0));
    // Same colour picker as the Properties panel (named swatches + "More…" full
    // palette), applied to the selection or the whole text.
    let color_pl = iced::widget::container(crate::ui::color_select::color_selector(
        acadrust::types::Color::from_index(ed.color_aci as i16),
        ed.color_picker_open,
        crate::ui::color_select::ColorExtras {
            by_layer: true,
            by_block: false,
        },
        Message::MTextColorChanged,
        Message::MTextColorPickerToggle,
        Message::OpenColorWindow(
            crate::app::ColorPickTarget::MText,
            acadrust::types::Color::from_index(ed.color_aci as i16),
        ),
    ))
    .width(iced::Length::Fixed(150.0));

    let row1 = row![
        style_pl,
        font_pl,
        small_input("2.5", &ed.height, Message::MTextHeight, 64.0),
        iced::widget::Space::new().width(6),
        icon_btn(
            include_bytes!("../../../assets/icons/mt_bold.svg"),
            Message::MTextFmt(MTextFmt::Bold)
        ),
        icon_btn(
            include_bytes!("../../../assets/icons/mt_italic.svg"),
            Message::MTextFmt(MTextFmt::Italic)
        ),
        icon_btn(
            include_bytes!("../../../assets/icons/mt_underline.svg"),
            Message::MTextFmt(MTextFmt::Underline)
        ),
        icon_btn(
            include_bytes!("../../../assets/icons/mt_overline.svg"),
            Message::MTextFmt(MTextFmt::Overline)
        ),
        icon_btn(
            include_bytes!("../../../assets/icons/mt_strike.svg"),
            Message::MTextFmt(MTextFmt::Strike)
        ),
        icon_btn(
            include_bytes!("../../../assets/icons/mt_upper.svg"),
            Message::MTextFmt(MTextFmt::Uppercase)
        ),
        icon_btn(
            include_bytes!("../../../assets/icons/mt_lower.svg"),
            Message::MTextFmt(MTextFmt::Lowercase)
        ),
        iced::widget::Space::new().width(width),
        color_pl,
    ]
    .spacing(4)
    .align_y(iced::Alignment::Center)
    .width(width);

    // ── Row 2: oblique / width / char-spacing · align · line spacing · OK ─
    let justify = iced::widget::pick_list(
        Some(JustifyChoice(ed.attachment)),
        JustifyChoice::ALL,
        |value| value.to_string(),
    )
    .on_select(|c| Message::MTextJustify(c.0))
    .text_size(11)
    .width(iced::Length::Fixed(112.0));
    let row2 = row![
        lbl("O"),
        small_input("0", &ed.oblique, Message::MTextOblique, 48.0),
        lbl("W"),
        small_input("1", &ed.width, Message::MTextWidth, 48.0),
        lbl("◊"),
        small_input("0", &ed.char_space, Message::MTextCharSpace, 48.0),
        iced::widget::Space::new().width(6),
        icon_btn(
            include_bytes!("../../../assets/icons/mt_align_left.svg"),
            Message::MTextAlign(ParaAlign::Left)
        ),
        icon_btn(
            include_bytes!("../../../assets/icons/mt_align_center.svg"),
            Message::MTextAlign(ParaAlign::Center)
        ),
        icon_btn(
            include_bytes!("../../../assets/icons/mt_align_right.svg"),
            Message::MTextAlign(ParaAlign::Right)
        ),
        icon_btn(
            include_bytes!("../../../assets/icons/mt_align_justify.svg"),
            Message::MTextAlign(ParaAlign::Justify)
        ),
        iced::widget::Space::new().width(6),
        justify,
        lbl("LS"),
        button(lbl("1"))
            .on_press(Message::MTextLineSpacing(1.0))
            .padding(3)
            .style(btn_style),
        button(lbl("1.5"))
            .on_press(Message::MTextLineSpacing(1.5))
            .padding(3)
            .style(btn_style),
        button(lbl("2"))
            .on_press(Message::MTextLineSpacing(2.0))
            .padding(3)
            .style(btn_style),
    ]
    .spacing(4)
    .align_y(iced::Alignment::Center)
    .width(width);

    let preview_scale = ed.preview_scale();
    let slider_min = 1e-6_f64;
    let slider_max =
        f64::from(writing_area_px / preview_scale).max(slider_min * 2.0);
    let width_slider = column![
        row![
            text(format!("Width: {:.3}", ed.rect_width)).size(11),
            Space::new().width(width),
            text(format!("{:.0}%", ed.rect_width / slider_max * 100.0)).size(11),
        ]
        .width(width),
        container(
            iced::widget::slider(
                slider_min..=slider_max,
                ed.rect_width.clamp(slider_min, slider_max),
                Message::MTextRectWidth,
            )
            .step((slider_max * 0.01).max(1e-6))
            .width(width),
        )
        .padding([0.0, MTEXT_PREVIEW_PAD])
        .width(width),
    ]
    .spacing(2)
    .width(width);

    // ── Body: the rendered preview (the editor is preview-only). It fills the
    // space left by the toolbars, so the resizable modal's extra height flows
    // into the text area. ─────────────────────────────────────────────────
    let body: Element<'a, Message> = if !matches!(height, iced::Length::Fill) {
        // The preview is a flexible scroll viewport. Measuring its real canvas
        // would make long/wide MText dictate the dialog size, so the intrinsic
        // pass uses an empty viewport proxy. The toolbars alone determine its
        // width; the preview height follows that measured writing width.
        container(Space::new())
            .style(move |theme: &Theme| {
                let palette = theme.palette();
                container::Style {
                background: Some(Background::Color(palette.background.base.color)),
                border: Border {
                    color: palette.background.neutral.color,
                    width: 1.0,
                    radius: 3.0.into(),
                },
                ..Default::default()
                }
            })
            .padding(2)
            .width(width)
            .height(preview_height)
            .into()
    } else {
        let segments = mtext_preview_segments(ed);
        let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (seg, _, _) in &segments {
            for &(x, y) in seg {
                minx = minx.min(x);
                miny = miny.min(y);
                maxx = maxx.max(x);
                maxy = maxy.max(y);
            }
        }
        // Include glyph boxes so all-whitespace / box-only lines still anchor
        // the transform (hit-testing relies on minx/miny).
        for b in &ed.glyph_boxes {
            minx = minx.min(b.xmin);
            miny = miny.min(b.ymin);
            maxx = maxx.max(b.xmax);
            maxy = maxy.max(b.ymax);
        }
        let scale = preview_scale;
        let content_h = if maxx >= minx {
            ((maxy - miny) * scale + 2.0 * MTEXT_PREVIEW_PAD).max(40.0)
        } else {
            40.0
        };
        // Multi-column MTEXT lays its columns out side by side, so the content
        // can be wider than the editor; size the canvas to the real content
        // width and let the scroll area pan horizontally to reach later columns.
        let content_w = if maxx >= minx {
            ((maxx - minx).max(ed.rect_width as f32) * scale
                + 2.0 * MTEXT_PREVIEW_PAD)
                .max(40.0)
        } else {
            ed.rect_width as f32 * scale + 2.0 * MTEXT_PREVIEW_PAD
        };
        let prog = MTextPreview {
            segments,
            boxes: ed.glyph_boxes.clone(),
            sel: ed.sel,
            caret: ed.caret,
            caret_on: ed.caret_blink_on,
            minx,
            miny,
            scale,
            content_h,
            wrap_width_px: ed.rect_width as f32 * scale,
        };
        let cv = canvas(prog)
            .width(iced::Length::Fixed(content_w))
            .height(iced::Length::Fixed(content_h));
        // The preview fills the space the toolbars leave; it scrolls in BOTH
        // directions so a taller-than-view text scrolls vertically and a
        // wider-than-view multi-column text scrolls horizontally.
        use iced::widget::scrollable::{Direction, Scrollbar};
        container(
            iced::widget::scrollable(cv)
                .direction(Direction::Both {
                    vertical: Scrollbar::default(),
                    horizontal: Scrollbar::default(),
                })
                .width(width)
                .height(preview_height),
        )
            .style(move |theme: &Theme| {
                let palette = theme.palette();
                container::Style {
                background: Some(Background::Color(palette.background.base.color)),
                border: Border {
                    color: palette.background.neutral.color,
                    width: 1.0,
                    radius: 3.0.into(),
                },
                ..Default::default()
                }
            })
            .padding(2)
            .width(width)
            .height(preview_height)
            .into()
    };

    // ── Top action bar: Apply on the right, exactly like the style managers'
    // toolbar strip. Closing (the modal ✕) without applying discards the
    // buffer, so there is no separate Cancel.
    let action_bar = container(
        row![
            iced::widget::Space::new().width(width),
            crate::ui::style::style_manager::tb_button("Apply", Message::MTextApply, true),
        ]
        .align_y(iced::Alignment::Center),
    )
    .style(|theme: &Theme| container::Style {
        background: Some(Background::Color(
            theme.palette().background.weak.color
        )),
        ..Default::default()
    })
    .width(width)
    .padding([5, 8]);

    container(
        column![action_bar, row1, row2, width_slider, body]
            .spacing(6)
            .width(width)
            .height(height),
    )
    .width(width)
    .height(height)
    .into()
}

// ── Viewport right-click context menu ──────────────────────────────────────

pub(super) fn viewport_context_menu_overlay(
    pos: iced::Point,
    has_cmd: bool,
    has_selection: bool,
    isolation_active: bool,
    last_cmds: Vec<String>,
    draworder_open: bool,
) -> Element<'static, Message> {
    let item = |label: String, msg: Message| -> Element<'static, Message> {
        button(text(label).size(12))
            .on_press(msg)
            .style(button::subtle)
            .padding([4, 12])
            .width(Fill)
            .into()
    };

    let sep = || -> Element<'static, Message> {
        container(iced::widget::Space::new().width(Fill).height(1))
            .style(|theme: &Theme| container::Style {
                background: Some(Background::Color(
                    theme.palette().background.weak.color,
                )),
                ..Default::default()
            })
            .width(Fill)
            .height(1)
            .padding([0, 6])
            .into()
    };

    // Indented variant for sub-menu rows (e.g. Draw Order children).
    let subitem = |label: String, msg: Message| -> Element<'static, Message> {
        button(text(label).size(12))
            .on_press(msg)
            .style(button::subtle)
            .padding(iced::Padding {
                top: 4.0,
                right: 12.0,
                bottom: 4.0,
                left: 26.0,
            })
            .width(Fill)
            .into()
    };

    let mut items: Vec<Element<'static, Message>> = Vec::new();

    if has_cmd {
        items.push(item("Cancel".to_string(), Message::CommandEscape));
        items.push(item("Enter".to_string(), Message::CommandFinalize));
    } else {
        if !last_cmds.is_empty() {
            let last = last_cmds[0].clone();
            items.push(item(
                format!("Repeat {last}"),
                Message::Command(last.to_uppercase()),
            ));
            if last_cmds.len() > 1 {
                for cmd in last_cmds.iter().skip(1) {
                    let c = cmd.clone();
                    items.push(item(c.clone(), Message::Command(c.to_uppercase())));
                }
            }
            items.push(sep());
        }
        if has_selection {
            items.push(item("Delete".to_string(), Message::DeleteSelected));
            items.push(item(
                "Move".to_string(),
                Message::Command("MOVE".to_string()),
            ));
            items.push(item(
                "Copy".to_string(),
                Message::Command("COPY".to_string()),
            ));
            items.push(sep());
            let do_caret = if draworder_open {
                crate::ui::icons::themed_arrow_down(9.0)
            } else {
                crate::ui::icons::themed_arrow_right(9.0)
            };
            items.push(
                button(
                    row![
                        text("Draw Order").size(12),
                        iced::widget::Space::new().width(Fill),
                        do_caret,
                    ]
                    .align_y(iced::Center),
                )
                .on_press(Message::DrawOrderSubmenuToggle)
                .style(button::subtle)
                .padding([4, 12])
                .width(Fill)
                .into(),
            );
            if draworder_open {
                items.push(subitem(
                    "Bring to Front".to_string(),
                    Message::Command("DRAWORDER F".to_string()),
                ));
                items.push(subitem(
                    "Send to Back".to_string(),
                    Message::Command("DRAWORDER B".to_string()),
                ));
                items.push(subitem(
                    "Bring Above Object".to_string(),
                    Message::DrawOrderPickRef(true),
                ));
                items.push(subitem(
                    "Send Under Object".to_string(),
                    Message::DrawOrderPickRef(false),
                ));
            }
            items.push(sep());
            items.push(item(
                "Isolate Objects".to_string(),
                Message::Command("ISOLATEOBJECTS".to_string()),
            ));
            items.push(item(
                "Hide Objects".to_string(),
                Message::Command("HIDEOBJECTS".to_string()),
            ));
            items.push(sep());
            items.push(item("Select Similar".to_string(), Message::SelectSimilar));
            items.push(item(
                "Invert Selection".to_string(),
                Message::InvertSelection,
            ));
        }
        if isolation_active {
            items.push(item(
                "End Object Isolation".to_string(),
                Message::Command("UNISOLATEOBJECTS".to_string()),
            ));
        }
        items.push(item(
            "Select All".to_string(),
            Message::Command("SELECTALL".to_string()),
        ));
        items.push(item("Quick Select...".to_string(), Message::QSelectOpen));
        items.push(item(
            "Zoom Extents".to_string(),
            Message::Command("ZOOM EXTENTS".to_string()),
        ));
    }

    let menu_col = column(items).spacing(0).width(iced::Length::Fixed(180.0));

    let menu = container(menu_col)
        .style(container::bordered_box)
        .padding([4, 0])
        .width(iced::Length::Fixed(180.0));

    position_canvas_overlay(pos, menu.into())
}

/// One-shot snap override menu (Shift+RMB, #337): a cursor-anchored grid of
/// snap ICONS only — the names show as hover tooltips. Picking one applies
/// that snap to just the next point pick.
pub(super) fn snap_override_overlay(pos: iced::Point) -> Element<'static, Message> {
    const COLS: usize = 4;

    let cell = |snap_type: crate::snap::SnapType, label: &'static str| -> Element<'static, Message> {
        let icon = container(crate::ui::icons::themed::<Message>(
            crate::ui::icons::osnap(snap_type),
            16.0,
        ))
        .width(26)
        .height(26)
        .align_x(iced::Center)
        .align_y(iced::Center);
        let btn = button(icon)
            .on_press(Message::SnapOverridePick(snap_type))
            .style(|theme: &Theme, status| button::Style {
                background: matches!(
                    status,
                    button::Status::Hovered | button::Status::Pressed
                )
                .then_some(Background::Color(
                    theme.palette().primary.weak.color
                )),
                border: Border::default(),
                text_color: theme.palette().background.base.text,
                ..Default::default()
            })
            .padding(2);
        iced::widget::tooltip(
            btn,
            container(text(label).size(11))
                .style(|theme: &Theme| {
                    let palette = theme.palette();
                    container::Style {
                    background: Some(Background::Color(palette.background.strong.color)),
                    border: Border {
                        color: palette.background.neutral.color,
                        width: 1.0,
                        radius: 2.0.into(),
                    },
                    text_color: Some(palette.background.strong.text),
                    ..Default::default()
                    }
                })
                .padding([2, 6]),
            iced::widget::tooltip::Position::Bottom,
        )
        .into()
    };

    let mut grid = column![].spacing(2);
    for chunk in crate::snap::ALL_SNAP_MODES.chunks(COLS) {
        let mut r = row![].spacing(2);
        for &(snap_type, _glyph, label) in chunk {
            r = r.push(cell(snap_type, label));
        }
        grid = grid.push(r);
    }

    let panel = container(grid)
        .style(|theme: &Theme| {
            let palette = theme.palette();
            container::Style {
            background: Some(Background::Color(palette.background.weak.color)),
            border: Border {
                color: palette.background.neutral.color,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..Default::default()
            }
        })
        .padding(4);

    // Full-screen click-catcher closes on an outside click.
    let catcher = mouse_area(
        container(iced::widget::Space::new().width(Fill).height(Fill))
            .width(Fill)
            .height(Fill),
    )
    .on_press(Message::SnapOverrideClose)
    .on_right_press(Message::SnapOverrideClose);

    stack![catcher, position_canvas_overlay(pos, panel.into())].into()
}

// ── Quick Select panel ─────────────────────────────────────────────────────

const QSELECT_ANY_TYPE: &str = "(Any type)";
const QSELECT_ANY_PROP: &str = "(Any property)";

/// Floating panel for the Quick Select feature. Single-row filter:
/// object type → property → operator → value, plus an "Append to current
/// selection" checkbox. The property dropdown is type-aware — Common
/// properties (Layer, Color, Linetype, Lineweight) are always shown;
/// picking a specific Object type adds that type's `geometry_properties`
/// fields (Start X, Length, Radius, …) so type-specific filtering works.
pub(super) fn qselect_overlay<'a>(
    state: &'a crate::app::QSelectState,
    types: &[String],
    properties: &[(String, String)],
    modal_offset: iced::Vector,
    modal_resize: iced::Vector,
) -> Element<'a, Message> {
    let measurement = qselect_content(
        state,
        types,
        properties,
        crate::ui::modal::ModalSizing::INTRINSIC,
    );
    let content = qselect_content(
        state,
        types,
        properties,
        crate::ui::modal::ModalSizing::FILL,
    );
    let content = crate::ui::modal::intrinsic(
        measurement,
        content,
        iced::Size::INFINITE,
        modal_resize,
    );

    crate::ui::modal::modal(
        Space::new().width(Fill).height(Fill),
        "Quick Select",
        content,
        Message::QSelectClose,
        modal_offset,
        crate::ui::modal::ModalOptions::STANDARD,
    )
}

fn qselect_content<'a>(
    state: &'a crate::app::QSelectState,
    types: &[String],
    properties: &[(String, String)],
    sizing: crate::ui::modal::ModalSizing,
) -> Element<'a, Message> {
    use iced::widget::{checkbox};
    let mut type_options: Vec<String> = vec![QSELECT_ANY_TYPE.to_string()];
    type_options.extend(types.iter().cloned());

    let mut prop_options: Vec<crate::app::QSelectPropertyChoice> =
        vec![crate::app::QSelectPropertyChoice {
            field: String::new(),
            label: QSELECT_ANY_PROP.to_string(),
        }];
    prop_options.extend(properties.iter().map(|(field, label)| {
        crate::app::QSelectPropertyChoice {
            field: field.clone(),
            label: label.clone(),
        }
    }));

    let op_options: Vec<crate::app::QSelectOp> = vec![
        crate::app::QSelectOp::Eq,
        crate::app::QSelectOp::Neq,
        crate::app::QSelectOp::Gt,
        crate::app::QSelectOp::Lt,
        crate::app::QSelectOp::Any,
    ];

    let type_sel = state
        .type_filter
        .clone()
        .unwrap_or_else(|| QSELECT_ANY_TYPE.to_string());
    let prop_sel = state
        .property
        .clone()
        .unwrap_or(crate::app::QSelectPropertyChoice {
            field: String::new(),
            label: QSELECT_ANY_PROP.to_string(),
        });

    // The value field is disabled (visually de-emphasised; we still
    // render the same widget) when no property is picked or the
    // operator is "*Any value" — both of those skip the value test.
    let value_enabled =
        state.property.is_some() && !matches!(state.operator, crate::app::QSelectOp::Any);
    let field_width = if matches!(sizing.width, iced::Length::Fill) {
        Fill
    } else {
        iced::Length::Shrink
    };

    let label = |s: &'static str| {
        text(s)
            .size(12)
            .width(iced::Length::Fixed(90.0))
    };

    let btn = |lbl: &'static str, msg: Message, primary: bool| {
        button(text(lbl).size(12))
            .on_press(msg)
            .style(move |theme: &Theme, st| {
                let palette = theme.palette();
                let pair = match (
                    primary,
                    matches!(st, button::Status::Hovered | button::Status::Pressed),
                ) {
                    (true, true) => palette.primary.strong,
                    (true, false) => palette.primary.base,
                    (false, true) => palette.background.strong,
                    (false, false) => palette.background.weak,
                };
                button::Style {
                background: Some(Background::Color(pair.color)),
                text_color: pair.text,
                border: Border {
                    color: palette.background.neutral.color,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..Default::default()
                }
            })
            .padding([4, 14])
    };

    let mut value_input = text_input("", &state.value)
        .size(12)
        .width(field_width);
    if value_enabled {
        value_input = value_input.on_input(Message::QSelectSetValue);
    }

    let panel_body = column![
        row![
            label("Object type:"),
            iced::widget::pick_list(
                Some(type_sel),
                type_options,
                |value| value.to_string(),
            )
            .on_select(|s: String| {
                if s == QSELECT_ANY_TYPE {
                    Message::QSelectSetType(None)
                } else {
                    Message::QSelectSetType(Some(s))
                }
            })
            .width(field_width),
        ]
        .align_y(iced::Alignment::Center)
        .spacing(8)
        .width(sizing.width),
        Space::new().height(6),
        row![
            label("Property:"),
            iced::widget::pick_list(
                Some(prop_sel),
                prop_options,
                |value| value.to_string(),
            )
            .on_select(|p: crate::app::QSelectPropertyChoice| {
                    if p.field.is_empty() {
                        Message::QSelectSetProperty(None)
                    } else {
                        Message::QSelectSetProperty(Some(p))
                    }
                })
            .width(field_width),
        ]
        .align_y(iced::Alignment::Center)
        .spacing(8)
        .width(sizing.width),
        Space::new().height(6),
        row![
            label("Operator:"),
            iced::widget::pick_list(
                Some(state.operator),
                op_options,
                |value| value.to_string(),
            )
            .on_select(Message::QSelectSetOperator)
            .width(field_width),
        ]
        .align_y(iced::Alignment::Center)
        .spacing(8)
        .width(sizing.width),
        Space::new().height(6),
        row![label("Value:"), value_input,]
            .align_y(iced::Alignment::Center)
            .spacing(8)
            .width(sizing.width),
        Space::new().height(10),
        row![
            checkbox(state.append)
                .on_toggle(Message::QSelectSetAppend)
                .size(14),
            Space::new().width(6),
            text("Append to current selection").size(12),
        ]
        .align_y(iced::Alignment::Center),
        Space::new().height(14),
        row![
            Space::new().width(field_width),
            btn("Cancel", Message::QSelectClose, false),
            Space::new().width(8),
            btn("Apply", Message::QSelectApply, true),
        ]
        .align_y(iced::Alignment::Center),
    ]
    .spacing(0)
    .width(sizing.width)
    .height(sizing.height);

    let panel = container(panel_body)
        .padding(16)
        .width(sizing.width)
        .height(sizing.height)
        .style(|theme: &Theme| {
            let palette = theme.palette();
            container::Style {
            background: Some(Background::Color(palette.background.weak.color)),
            border: Border {
                color: palette.background.neutral.color,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..Default::default()
            }
        });

    panel.into()
}
