//! Shared monochrome UI-chrome icons rendered from bundled SVGs.
//!
//! Dropdown carets and the undo/redo controls used to be drawn as Unicode
//! glyphs (`▾`, `▲`, `↶`, `↷`). Those depend on the active text font carrying
//! the glyph: on desktop the system fallback fonts supply them, but the web
//! build bundles only Fira Sans, which lacks them, so they rendered as empty
//! boxes. Drawing them from SVG instead makes the chrome font-independent.

use iced::widget::{container, svg, Space};
use iced::{Element, Length, Theme};

const TRI_DOWN: &[u8] = include_bytes!("../../assets/icons/ui/tri_down.svg");
const TRI_UP: &[u8] = include_bytes!("../../assets/icons/ui/tri_up.svg");
const TRI_RIGHT: &[u8] = include_bytes!("../../assets/icons/ui/tri_right.svg");
const TRI_LEFT: &[u8] = include_bytes!("../../assets/icons/ui/tri_left.svg");
const HOME: &[u8] = include_bytes!("../../assets/icons/ui/home.svg");
const UNDO: &[u8] = include_bytes!("../../assets/icons/ui/undo.svg");
const REDO: &[u8] = include_bytes!("../../assets/icons/ui/redo.svg");

// OSNAP marker symbols. Rendered as SVG (not Unicode glyphs) so the snap menu
// shows the right shapes on the web build, whose bundled Fira Sans lacks the
// geometric glyphs and rendered them as tofu boxes. (#138)
const OSNAP_ENDPOINT: &[u8] = include_bytes!("../../assets/icons/osnap/endpoint.svg");
const OSNAP_MIDPOINT: &[u8] = include_bytes!("../../assets/icons/osnap/midpoint.svg");
const OSNAP_CENTER: &[u8] = include_bytes!("../../assets/icons/osnap/center.svg");
const OSNAP_NODE: &[u8] = include_bytes!("../../assets/icons/osnap/node.svg");
const OSNAP_QUADRANT: &[u8] = include_bytes!("../../assets/icons/osnap/quadrant.svg");
const OSNAP_INTERSECTION: &[u8] = include_bytes!("../../assets/icons/osnap/intersection.svg");
const OSNAP_EXTENSION: &[u8] = include_bytes!("../../assets/icons/osnap/extension.svg");
const OSNAP_INSERTION: &[u8] = include_bytes!("../../assets/icons/osnap/insertion.svg");
const OSNAP_PERPENDICULAR: &[u8] =
    include_bytes!("../../assets/icons/osnap/perpendicular.svg");
const OSNAP_TANGENT: &[u8] = include_bytes!("../../assets/icons/osnap/tangent.svg");
const OSNAP_NEAREST: &[u8] = include_bytes!("../../assets/icons/osnap/nearest.svg");
const OSNAP_APPARENT: &[u8] = include_bytes!("../../assets/icons/osnap/apparent.svg");
const OSNAP_PARALLEL: &[u8] = include_bytes!("../../assets/icons/osnap/parallel.svg");
const OSNAP_GRID: &[u8] = include_bytes!("../../assets/icons/osnap/grid.svg");

const LAY_ON: &[u8] = include_bytes!("../../assets/icons/layers/layon.svg");
const LAY_OFF: &[u8] = include_bytes!("../../assets/icons/layers/layoff.svg");
const LAY_FRZ: &[u8] = include_bytes!("../../assets/icons/layers/layfrz.svg");
const LAY_THW: &[u8] = include_bytes!("../../assets/icons/layers/laythw.svg");
const LAY_LCK: &[u8] = include_bytes!("../../assets/icons/layers/laylck.svg");
const LAY_ULK: &[u8] = include_bytes!("../../assets/icons/layers/layulk.svg");

// Monochrome chrome glyphs (replace Unicode glyphs in buttons / menus / toolbars).
// All are black-on-transparent; recolour them at the call site with [`tinted`].
pub const CHECK: &[u8] = include_bytes!("../../assets/icons/ui/check.svg");
pub const CLOSE: &[u8] = include_bytes!("../../assets/icons/ui/close.svg");
pub const PLUS: &[u8] = include_bytes!("../../assets/icons/ui/plus.svg");
pub const MINUS: &[u8] = include_bytes!("../../assets/icons/ui/minus.svg");
pub const TRASH: &[u8] = include_bytes!("../../assets/icons/ui/trash.svg");
pub const COPY: &[u8] = include_bytes!("../../assets/icons/ui/copy.svg");
pub const MENU: &[u8] = include_bytes!("../../assets/icons/ui/menu.svg");
pub const MOVE: &[u8] = include_bytes!("../../assets/icons/ui/move.svg");
pub const RESIZE: &[u8] = include_bytes!("../../assets/icons/ui/resize.svg");
pub const SPLIT_V: &[u8] = include_bytes!("../../assets/icons/ui/split_v.svg");
pub const SPLIT_H: &[u8] = include_bytes!("../../assets/icons/ui/split_h.svg");
pub const GRID: &[u8] = include_bytes!("../../assets/icons/ui/grid.svg");
pub const SNAP: &[u8] = include_bytes!("../../assets/icons/ui/snap.svg");
pub const DOC_NEW: &[u8] = include_bytes!("../../assets/icons/ui/doc_new.svg");
pub const FOLDER_OPEN: &[u8] = include_bytes!("../../assets/icons/ui/folder_open.svg");
pub const SAVE: &[u8] = include_bytes!("../../assets/icons/ui/save.svg");
pub const FILE_EXPORT: &[u8] = include_bytes!("../../assets/icons/ui/file_export.svg");
pub const PRINT: &[u8] = include_bytes!("../../assets/icons/ui/print.svg");
pub const HEART: &[u8] = include_bytes!("../../assets/icons/ui/heart.svg");
pub const DOT: &[u8] = include_bytes!("../../assets/icons/ui/dot.svg");
pub const ARROW_LONG_RIGHT: &[u8] = include_bytes!("../../assets/icons/ui/arrow_long_right.svg");

// ── Status-bar toggle icons (issue #216) ──────────────────────────────────
pub const ST_ORTHO: &[u8] = include_bytes!("../../assets/icons/status/ortho.svg");
pub const ST_POLAR: &[u8] = include_bytes!("../../assets/icons/status/polar.svg");
pub const ST_OSNAP: &[u8] = include_bytes!("../../assets/icons/status/osnap.svg");
pub const ST_OTRACK: &[u8] = include_bytes!("../../assets/icons/status/otrack.svg");
pub const ST_DYN: &[u8] = include_bytes!("../../assets/icons/status/dyn.svg");
pub const ST_LWT: &[u8] = include_bytes!("../../assets/icons/status/lwt.svg");
pub const ST_TRANSPARENCY: &[u8] = include_bytes!("../../assets/icons/status/transparency.svg");
pub const ST_ISOLATE: &[u8] = include_bytes!("../../assets/icons/status/isolate.svg");
pub const ST_QUICKPROPS: &[u8] = include_bytes!("../../assets/icons/status/quickprops.svg");
pub const ST_FILTER: &[u8] = include_bytes!("../../assets/icons/status/filter.svg");
pub const ST_SELCYCLE: &[u8] = include_bytes!("../../assets/icons/status/selcycle.svg");
pub const ST_CLEANSCREEN: &[u8] = include_bytes!("../../assets/icons/status/cleanscreen.svg");

/// Render a chrome icon with the active Iced theme's normal text color.
pub fn themed<'a, M: 'a>(bytes: &'static [u8], size: f32) -> Element<'a, M> {
    svg(svg::Handle::from_memory(bytes))
        .width(size)
        .height(size)
        .style(|theme: &Theme, _| svg::Style {
            color: Some(theme.extended_palette().background.base.text),
        })
        .into()
}

/// Render secondary chrome with the active Iced theme's text color.
pub fn themed_secondary<'a, M: 'a>(bytes: &'static [u8], size: f32) -> Element<'a, M> {
    svg(svg::Handle::from_memory(bytes))
        .width(size)
        .height(size)
        .style(|theme: &Theme, _| svg::Style {
            color: Some(
                theme
                    .extended_palette()
                    .background
                    .base
                    .text
                    .scale_alpha(0.72),
            ),
        })
        .into()
}

/// Render disabled chrome with the active Iced theme's text color.
pub fn themed_disabled<'a, M: 'a>(bytes: &'static [u8], size: f32) -> Element<'a, M> {
    svg(svg::Handle::from_memory(bytes))
        .width(size)
        .height(size)
        .style(|theme: &Theme, _| svg::Style {
            color: Some(
                theme
                    .extended_palette()
                    .background
                    .base
                    .text
                    .scale_alpha(0.42),
            ),
        })
        .into()
}

/// Render an emphasized chrome icon with the active Iced theme's primary color.
pub fn themed_primary<'a, M: 'a>(bytes: &'static [u8], size: f32) -> Element<'a, M> {
    svg(svg::Handle::from_memory(bytes))
        .width(size)
        .height(size)
        .style(|theme: &Theme, _| svg::Style {
            color: Some(theme.extended_palette().primary.base.color),
        })
        .into()
}

/// Render a positive-state chrome icon with the active Iced theme's success color.
pub fn themed_success<'a, M: 'a>(bytes: &'static [u8], size: f32) -> Element<'a, M> {
    svg(svg::Handle::from_memory(bytes))
        .width(size)
        .height(size)
        .style(|theme: &Theme, _| svg::Style {
            color: Some(theme.extended_palette().success.base.color),
        })
        .into()
}

/// Render a warning-state chrome icon from the active Iced theme.
pub fn themed_warning<'a, M: 'a>(bytes: &'static [u8], size: f32) -> Element<'a, M> {
    svg(svg::Handle::from_memory(bytes))
        .width(size)
        .height(size)
        .style(|theme: &Theme, _| svg::Style {
            color: Some(theme.extended_palette().warning.base.color),
        })
        .into()
}

/// Render a destructive-state chrome icon from the active Iced theme.
pub fn themed_danger<'a, M: 'a>(bytes: &'static [u8], size: f32) -> Element<'a, M> {
    svg(svg::Handle::from_memory(bytes))
        .width(size)
        .height(size)
        .style(|theme: &Theme, _| svg::Style {
            color: Some(theme.extended_palette().danger.base.color),
        })
        .into()
}

/// Render an icon with the foreground chosen for a danger-coloured surface.
pub fn themed_danger_text<'a, M: 'a>(bytes: &'static [u8], size: f32) -> Element<'a, M> {
    svg(svg::Handle::from_memory(bytes))
        .width(size)
        .height(size)
        .style(|theme: &Theme, _| svg::Style {
            color: Some(theme.extended_palette().danger.base.text),
        })
        .into()
}

/// Fixed-width check column colored from the active Iced theme.
pub fn themed_check_cell<'a, M: 'a>(active: bool) -> Element<'a, M> {
    let inner: Element<'a, M> = if active {
        themed_primary(CHECK, 11.0)
    } else {
        Space::new().width(0).into()
    };
    container(inner).width(Length::Fixed(14.0)).into()
}

/// Render a bundled SVG at its native colours (no tint) at a square `size`.
pub fn raw<'a, M: 'a>(bytes: &'static [u8], size: f32) -> Element<'a, M> {
    svg(svg::Handle::from_memory(bytes))
        .width(size)
        .height(size)
        .into()
}

/// SVG bytes for an OSNAP mode's marker symbol, for the snap menu. (#138)
pub fn osnap(snap: crate::snap::SnapType) -> &'static [u8] {
    use crate::snap::SnapType as S;
    match snap {
        S::Endpoint => OSNAP_ENDPOINT,
        S::Midpoint => OSNAP_MIDPOINT,
        S::Center => OSNAP_CENTER,
        S::Node => OSNAP_NODE,
        S::Quadrant => OSNAP_QUADRANT,
        S::Intersection => OSNAP_INTERSECTION,
        S::Extension => OSNAP_EXTENSION,
        S::Insertion => OSNAP_INSERTION,
        S::Perpendicular => OSNAP_PERPENDICULAR,
        S::Tangent => OSNAP_TANGENT,
        S::Nearest => OSNAP_NEAREST,
        S::ApparentIntersection => OSNAP_APPARENT,
        S::Parallel => OSNAP_PARALLEL,
        S::Grid => OSNAP_GRID,
        // Not shown in the snap menu; fall back to a neutral marker.
        S::ObjectPick => OSNAP_NEAREST,
    }
}

/// Layer visibility icon bytes (on / off).
pub fn layer_visible(visible: bool) -> &'static [u8] {
    if visible {
        LAY_ON
    } else {
        LAY_OFF
    }
}

/// Layer freeze icon bytes (frozen / thawed).
pub fn layer_freeze(frozen: bool) -> &'static [u8] {
    if frozen {
        LAY_FRZ
    } else {
        LAY_THW
    }
}

/// Layer lock icon bytes (locked / unlocked).
pub fn layer_lock(locked: bool) -> &'static [u8] {
    if locked {
        LAY_LCK
    } else {
        LAY_ULK
    }
}

pub fn themed_arrow_down<'a, M: 'a>(size: f32) -> Element<'a, M> {
    themed(TRI_DOWN, size)
}

pub fn themed_arrow_up<'a, M: 'a>(size: f32) -> Element<'a, M> {
    themed(TRI_UP, size)
}

pub fn themed_arrow_right<'a, M: 'a>(size: f32) -> Element<'a, M> {
    themed(TRI_RIGHT, size)
}

pub fn themed_arrow_left<'a, M: 'a>(size: f32) -> Element<'a, M> {
    themed(TRI_LEFT, size)
}

pub fn themed_primary_arrow_down<'a, M: 'a>(size: f32) -> Element<'a, M> {
    themed_primary(TRI_DOWN, size)
}

pub fn themed_secondary_arrow_down<'a, M: 'a>(size: f32) -> Element<'a, M> {
    themed_secondary(TRI_DOWN, size)
}

pub fn themed_disabled_arrow_down<'a, M: 'a>(size: f32) -> Element<'a, M> {
    themed_disabled(TRI_DOWN, size)
}

pub fn themed_home<'a, M: 'a>(size: f32) -> Element<'a, M> {
    themed(HOME, size)
}

/// Caret that flips up/down with `open`.
pub fn themed_arrow_toggle<'a, M: 'a>(open: bool, size: f32) -> Element<'a, M> {
    if open {
        themed_arrow_up(size)
    } else {
        themed_arrow_down(size)
    }
}

pub fn themed_undo<'a, M: 'a>(size: f32, enabled: bool) -> Element<'a, M> {
    if enabled {
        themed(UNDO, size)
    } else {
        themed_disabled(UNDO, size)
    }
}

pub fn themed_redo<'a, M: 'a>(size: f32, enabled: bool) -> Element<'a, M> {
    if enabled {
        themed(REDO, size)
    } else {
        themed_disabled(REDO, size)
    }
}
