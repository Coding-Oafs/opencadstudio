use super::*;
use super::super::document::{DynComponent, DynFieldEntry};
use super::super::Message;
use iced::widget::{button, canvas, column, container, mouse_area, row, text, tooltip};
use iced::{Background, Border, Color, Element, Length, Point, Rectangle, Theme};
use std::time::Duration;

fn viewport_tooltip<'a>(
    control: impl Into<Element<'a, Message>>,
    title: String,
    command: &'static str,
) -> Element<'a, Message> {
    let text = format!("{title}\n{} {command}", crate::t!("Command:"));
    tooltip(
        control,
        crate::ui::ribbon::tooltip_content(text),
        tooltip::Position::Bottom,
    )
    .gap(6.0)
    .delay(Duration::from_millis(400))
    .style(crate::ui::ribbon::tooltip_style)
    .into()
}

#[derive(Clone, Copy)]
struct RenderModePreview {
    mode: acadrust::entities::ViewportRenderMode,
}

impl canvas::Program<Message> for RenderModePreview {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        use acadrust::entities::ViewportRenderMode as M;

        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let palette = theme.palette();
        let ink = palette.background.base.text.scale_alpha(0.86);
        let faint = ink.scale_alpha(0.34);
        let accent = palette.primary.base.color;
        let face = palette.primary.weak.color.scale_alpha(0.72);
        let face_alt = palette.primary.strong.color.scale_alpha(0.64);
        let edge = if self.mode == M::Wireframe3D { accent } else { ink };
        let shaded = matches!(
            self.mode,
            M::FlatShaded
                | M::GouraudShaded
                | M::FlatShadedWithEdges
                | M::GouraudShadedWithEdges
        );
        let flat = matches!(self.mode, M::FlatShaded | M::FlatShadedWithEdges);
        let smooth = matches!(self.mode, M::GouraudShaded | M::GouraudShadedWithEdges);
        let with_edges = matches!(
            self.mode,
            M::Wireframe2D
                | M::Wireframe3D
                | M::HiddenLine
                | M::FlatShadedWithEdges
                | M::GouraudShadedWithEdges
        );

        // A small cube makes hidden/back edges and the shaded-edge variants
        // immediately distinguishable without depending on the open drawing.
        let a = Point::new(25.0, 35.0);
        let b = Point::new(87.0, 35.0);
        let c = Point::new(87.0, 94.0);
        let d = Point::new(25.0, 94.0);
        let off = iced::Vector::new(22.0, -17.0);
        let e = a + off;
        let f = b + off;
        let g = c + off;
        let h = d + off;
        let quad = |points: [Point; 4]| {
            canvas::Path::new(|path| {
                path.move_to(points[0]);
                path.line_to(points[1]);
                path.line_to(points[2]);
                path.line_to(points[3]);
                path.close();
            })
        };
        if shaded || self.mode == M::HiddenLine {
            frame.fill(&quad([a, b, f, e]), face_alt);
            frame.fill(&quad([b, c, g, f]), face.scale_alpha(0.82));
            frame.fill(&quad([a, d, c, b]), face);
        }
        let stroke_line = |frame: &mut canvas::Frame, from: Point, to: Point, color: Color| {
            frame.stroke(
                &canvas::Path::line(from, to),
                canvas::Stroke::default().with_color(color).with_width(1.2),
            );
        };
        if matches!(self.mode, M::Wireframe2D | M::Wireframe3D) {
            for (from, to) in [(e, f), (f, g), (g, h), (h, e)] {
                stroke_line(&mut frame, from, to, faint);
            }
        }
        if with_edges {
            for (from, to) in [
                (a, b), (b, c), (c, d), (d, a),
                (a, e), (b, f), (c, g), (d, h),
                (e, f), (f, g),
            ] {
                stroke_line(&mut frame, from, to, edge);
            }
        }

        // Curved sample: faceted tones for Flat, concentric highlight for
        // Gouraud, and latitude/longitude lines for the wireframe modes.
        let center = Point::new((bounds.width - 55.0).max(145.0), 59.0);
        let radius = 32.0;
        if flat {
            for i in 0..12 {
                let a0 = i as f32 * std::f32::consts::TAU / 12.0;
                let a1 = (i + 1) as f32 * std::f32::consts::TAU / 12.0;
                let wedge = canvas::Path::new(|path| {
                    path.move_to(center);
                    path.line_to(Point::new(center.x + radius * a0.cos(), center.y + radius * a0.sin()));
                    path.line_to(Point::new(center.x + radius * a1.cos(), center.y + radius * a1.sin()));
                    path.close();
                });
                frame.fill(&wedge, if i % 3 == 0 { face_alt } else { face });
            }
        } else if smooth {
            frame.fill(&canvas::Path::circle(center, radius), face_alt);
            frame.fill(
                &canvas::Path::circle(Point::new(center.x - 7.0, center.y - 8.0), radius * 0.72),
                face,
            );
            frame.fill(
                &canvas::Path::circle(Point::new(center.x - 12.0, center.y - 13.0), radius * 0.34),
                palette.primary.weak.text.scale_alpha(0.38),
            );
        }
        if with_edges || self.mode == M::HiddenLine {
            frame.stroke(
                &canvas::Path::circle(center, radius),
                canvas::Stroke::default().with_color(edge).with_width(1.2),
            );
        }
        if matches!(self.mode, M::Wireframe2D | M::Wireframe3D) {
            for scale in [0.38_f32, 0.72] {
                let y = radius * scale;
                let half = (radius * radius - y * y).sqrt();
                stroke_line(
                    &mut frame,
                    Point::new(center.x - half, center.y - y),
                    Point::new(center.x + half, center.y - y),
                    faint,
                );
                stroke_line(
                    &mut frame,
                    Point::new(center.x - half, center.y + y),
                    Point::new(center.x + half, center.y + y),
                    faint,
                );
            }
            stroke_line(
                &mut frame,
                Point::new(center.x, center.y - radius),
                Point::new(center.x, center.y + radius),
                faint,
            );
        }

        // Bottom samples show the intentional Wireframe 2D/3D distinction:
        // the legacy planar solid loses only its interior in the 3D style,
        // while a hatch-like pattern remains visible in both.
        let solid = quad([
            Point::new(18.0, 118.0),
            Point::new(74.0, 112.0),
            Point::new(69.0, 140.0),
            Point::new(23.0, 143.0),
        ]);
        if self.mode != M::Wireframe3D {
            frame.fill(&solid, accent.scale_alpha(0.48));
        }
        frame.stroke(
            &solid,
            canvas::Stroke::default().with_color(edge).with_width(1.0),
        );
        let hatch_box = canvas::Path::rectangle(Point::new(105.0, 112.0), iced::Size::new(82.0, 31.0));
        frame.stroke(
            &hatch_box,
            canvas::Stroke::default().with_color(ink).with_width(1.0),
        );
        for x in (94..188).step_by(9) {
            let x = x as f32;
            stroke_line(
                &mut frame,
                Point::new(x.max(105.0), 143.0),
                Point::new((x + 24.0).min(187.0), 112.0),
                faint,
            );
        }

        vec![frame.into_geometry()]
    }
}

pub(super) fn viewport_controls<'a>(
    render_mode: acadrust::entities::ViewportRenderMode,
    show_grid: bool,
    snap_on: bool,
    include_split: bool,
    tile_count: usize,
    render_mode_menu_open: bool,
    render_mode_preview: Option<acadrust::entities::ViewportRenderMode>,
) -> Element<'a, Message> {
    use acadrust::entities::ViewportRenderMode as M;
    let render_modes = [
        RenderModeChoice(M::Wireframe2D),
        RenderModeChoice(M::Wireframe3D),
        RenderModeChoice(M::HiddenLine),
        RenderModeChoice(M::FlatShaded),
        RenderModeChoice(M::GouraudShaded),
        RenderModeChoice(M::FlatShadedWithEdges),
        RenderModeChoice(M::GouraudShadedWithEdges),
    ];
    let danger_btn = move |bytes: &'static [u8],
                           msg: Message,
                           title: String,
                           command: &'static str| {
        let button = button(crate::ui::icons::themed_danger(bytes, 15.0))
            .on_press(msg)
            .padding([4, 6])
            .style(move |theme: &Theme, status| iced::widget::button::Style {
                background: matches!(
                    status,
                    iced::widget::button::Status::Hovered
                        | iced::widget::button::Status::Pressed
                )
                .then_some(Background::Color(theme.palette().danger.weak.color)),
                border: Border {
                    radius: 3.0.into(),
                    ..Default::default()
                },
                text_color: theme.palette().danger.base.color,
                ..Default::default()
            });
        viewport_tooltip(button, title, command)
    };

    // Borderless icon button; an `active` toggle gets an accent tint + fill.
    let icon_btn = move |bytes: &'static [u8],
                         active: bool,
                         msg: Message,
                         title: String,
                         command: &'static str| {
        let icon = if active {
            crate::ui::icons::themed_primary(bytes, 15.0)
        } else {
            crate::ui::icons::themed(bytes, 15.0)
        };
        let button = button(icon)
            .on_press(msg)
            .padding([4, 6])
            .style(move |theme: &Theme, status| {
                let palette = theme.palette();
                let pair = match (active, status) {
                    (_, iced::widget::button::Status::Hovered) => {
                        Some(palette.background.strong)
                    }
                    (true, _) => Some(palette.primary.weak),
                    (false, _) => None,
                };
                iced::widget::button::Style {
                background: pair.map(|p| Background::Color(p.color)),
                border: Border {
                    radius: 3.0.into(),
                    ..Default::default()
                },
                text_color: pair
                    .map(|p| p.text)
                    .unwrap_or(palette.background.base.text),
                ..Default::default()
                }
            });
        viewport_tooltip(button, title, command)
    };

    // The standard picker cannot show a live sample beside its rows. This
    // flyout keeps the current style unchanged while hovering; a click commits.
    let picker_head = button(
        row![
            text(RenderModeChoice(render_mode).to_string()).size(11),
            crate::ui::icons::themed_arrow_toggle(render_mode_menu_open, 9.0),
        ]
        .spacing(6)
        .align_y(iced::Center),
    )
    .on_press(Message::ToggleRenderModeMenu(render_mode))
    .padding([4, 6])
    .style(move |theme: &Theme, status| {
        let palette = theme.palette();
        let active = render_mode_menu_open
            || matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: active.then_some(Background::Color(palette.background.strong.color)),
            text_color: if active {
                palette.background.strong.text
            } else {
                palette.background.base.text
            },
            border: Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    });
    let picker_head = viewport_tooltip(
        picker_head,
        crate::t!("Visual Style").into_owned(),
        "VISUALSTYLES",
    );

    let preview_mode = render_mode_preview.unwrap_or(render_mode);
    let mut choices = column![].spacing(2).width(Length::Fixed(174.0));
    for choice in render_modes {
        let highlighted = choice.0 == preview_mode;
        let selected = choice.0 == render_mode;
        let option = container(text(choice.to_string()).size(11).width(Length::Fill))
            .padding([6, 8])
            .width(Length::Fill)
            .style(move |theme: &Theme| {
                let palette = theme.palette();
                let active = highlighted;
                container::Style {
                    background: active.then_some(Background::Color(
                        if selected { palette.primary.weak.color } else { palette.background.strong.color },
                    )),
                    text_color: if selected {
                        Some(palette.primary.weak.text)
                    } else if active {
                        Some(palette.background.strong.text)
                    } else {
                        Some(palette.background.base.text)
                    },
                    border: Border {
                        radius: 3.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            });
        choices = choices.push(mouse_area(option)
            .interaction(iced::mouse::Interaction::Pointer)
            .on_enter(Message::PreviewRenderMode(choice.0))
            .on_press(Message::SetRenderMode(choice.0)));
    }
    let preview = container(
        column![
            container(
                text(RenderModeChoice(preview_mode).to_string())
                    .size(12)
                    .wrapping(iced::advanced::text::Wrapping::None),
            )
            .width(Length::Fixed(215.0))
            .height(Length::Fixed(16.0))
            .align_y(iced::alignment::Vertical::Center),
            canvas(RenderModePreview { mode: preview_mode })
                .width(Length::Fixed(215.0))
                .height(Length::Fixed(154.0)),
        ]
        .spacing(6),
    )
    .padding(8)
    .style(|theme: &Theme| {
        let palette = theme.palette();
        container::Style {
            background: Some(Background::Color(palette.background.base.color)),
            border: Border {
                color: palette.background.neutral.color,
                width: 1.0,
                radius: 3.0.into(),
            },
            text_color: Some(palette.background.base.text),
            ..Default::default()
        }
    });
    let popup = container(
        row![choices, preview]
            .spacing(6)
            .align_y(iced::alignment::Vertical::Top),
    )
        .padding(6)
        .style(|theme: &Theme| {
            let palette = theme.palette();
            container::Style {
                background: Some(Background::Color(palette.background.weak.color)),
                border: Border {
                    color: palette.background.neutral.color,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                text_color: Some(palette.background.weak.text),
                ..Default::default()
            }
        });
    let picker: Element<'a, Message> = iced_aw::DropDown::new(
        picker_head,
        popup,
        render_mode_menu_open,
    )
    .alignment(iced_aw::drop_down::Alignment::Bottom)
    .offset(3.0)
    .on_dismiss(Message::DismissRenderModeMenu)
    .into();

    // Thin vertical divider between control groups.
    let sep = || {
        container(iced::widget::Space::new().width(1.0).height(16.0)).style(|theme: &Theme| {
            iced::widget::container::Style {
                background: Some(Background::Color(
                    theme.palette().background.neutral.color.scale_alpha(0.7)
                )),
                ..Default::default()
            }
        })
    };

    let mut bar = row![]
        .spacing(3)
        .align_y(iced::alignment::Vertical::Center);
    bar = bar
        .push(icon_btn(
            crate::ui::icons::GRID,
            show_grid,
            Message::ToggleGrid,
            crate::t!("Toggle Grid").into_owned(),
            "GRID",
        ))
        .push(sep())
        .push(icon_btn(
            crate::ui::icons::SNAP,
            snap_on,
            Message::ToggleGridSnap,
            crate::t!("Toggle Grid Snap").into_owned(),
            "SNAP",
        ))
        .push(sep())
        .push(picker);
    if include_split {
        bar = bar
            .push(sep())
            .push(icon_btn(
                crate::ui::icons::SPLIT_V,
                false,
                Message::SplitModelViewport(false),
                crate::tr!("viewport-split-vertical"),
                "VPORTS 2V",
            ))
            .push(sep())
            .push(icon_btn(
                crate::ui::icons::SPLIT_H,
                false,
                Message::SplitModelViewport(true),
                crate::tr!("viewport-split-horizontal"),
                "VPORTS 2H",
            ));
        // Drag handle + close: only meaningful with more than one model tile.
        // The handle is a `mouse_area` (not a button) so it fires on press-DOWN,
        // letting the drag continue onto the target pane to swap them (a button
        // would only fire on release). Placed just left of Close.
        if tile_count > 1 {
            let drag = mouse_area(
                container(crate::ui::icons::themed_success(crate::ui::icons::MOVE, 15.0))
                    .padding([4, 6])
                    .style(|_: &Theme| iced::widget::container::Style {
                        border: Border {
                            radius: 3.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            )
            .interaction(iced::mouse::Interaction::Grab)
            .on_press(Message::PaneMoveStart);
            let drag = viewport_tooltip(drag, crate::tr!("viewport-move"), "VPORTS");
            bar = bar
                .push(sep())
                .push(drag)
                .push(sep())
                .push(danger_btn(
                    crate::ui::icons::CLOSE,
                    Message::CloseModelViewport,
                    crate::tr!("viewport-close"),
                    "VPORTS SINGLE",
                ));
        }
    }

    container(bar)
        .padding(2)
        .style(|theme: &Theme| {
            let palette = theme.palette();
            iced::widget::container::Style {
            background: Some(Background::Color(
                palette.background.weak.color.scale_alpha(0.92)
            )),
            border: Border {
                color: palette.background.neutral.color,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..Default::default()
            }
        })
        .into()
}

// ── Dynamic-input field formatting ─────────────────────────────────────────

/// Short prefix shown before a dynamic-input box's value.
/// The string shown inside a dynamic-input box: the typed buffer when the
/// field is locked, otherwise the live value derived from the cursor
/// world position (and the base point for polar quantities).
pub(super) fn dyn_component_value(
    f: &DynFieldEntry,
    w: glam::DVec3,
    base: Option<glam::DVec3>,
    xf: &super::super::helpers::UcsXform,
    comma_cartesian: bool,
    absolute: bool,
) -> String {
    if let Some(b) = &f.buffer {
        return b.clone();
    }
    let b = base.unwrap_or(glam::DVec3::ZERO);
    let p = xf.to_ucs(w);
    // Relative deltas and the polar angle read in the active UCS plane. The
    // delta is offset-invariant, so only the axis rotation matters (identity
    // xf reproduces the world-frame deltas).
    let d = xf.vec_to_ucs(w - b);
    let dx = d.x as f64;
    let dy = d.y as f64;
    // When a base point exists (DYN-on after the first pick) the cartesian
    // fields show relative deltas — matching the typed-value convention
    // in `dyn_resolve_point` so the live preview and the committed
    // coordinate use the same frame. See #35.
    let relative = base.is_some() && !absolute;
    // Width / Height read as unsigned magnitudes (the sign is taken from the
    // cursor side on commit), matching the rectangle's two-edge entry. But once
    // the user separates the values with `,` the entry is a cartesian
    // coordinate pair, so the fields read as signed X/Y deltas to match the
    // committed point (#269).
    let wh = matches!(f.role, crate::command::DynRole::Width | crate::command::DynRole::Height)
        && relative
        && !comma_cartesian;
    match f.component {
        DynComponent::X if relative => format!("{:.4}", if wh { dx.abs() } else { dx }),
        DynComponent::Y if relative => format!("{:.4}", if wh { dy.abs() } else { dy }),
        DynComponent::Z if relative => "0.0000".to_string(),
        DynComponent::X => format!("{:.4}", p.x),
        DynComponent::Y => format!("{:.4}", p.y),
        DynComponent::Z => format!("{:.4}", p.z),
        // Scaled by the role so a diameter box reads twice the radius.
        DynComponent::Distance => {
            format!("{:.4}", (dx * dx + dy * dy).sqrt() * f.role.value_scale() as f64)
        }
        // Shared rule: unsigned magnitude of the short angle, so CW (below the
        // reference axis) reads positive (e.g. 30°, not -30°/330°). The
        // committed value stays signed (see dyn_resolve_point).
        DynComponent::Angle => {
            format!("{:.1}", crate::command::dyn_display_angle_deg(dy.atan2(dx) as f32))
        }
        // Typed-only scalar — no geometric value to track when empty.
        DynComponent::Scalar => String::new(),
    }
}
