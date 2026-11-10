use crate::app::Message;
use iced::widget::{
    button, column, container, pick_list, progress_bar, row, scrollable, slider, text, text_input,
};
use iced::{Element, Fill};

pub struct TerrainPanel {
    pub tab_id: Option<u64>,
    pub values: [String; 7],
    pub zoom: u32,
    pub outside: String,
    pub error: String,
}
impl Default for TerrainPanel {
    fn default() -> Self {
        Self {
            tab_id: None,
            values: ["", "", "5", "", "1", "0", "400000"].map(String::from),
            zoom: 12,
            outside: "ABORT".into(),
            error: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum TerrainAction {
    SetCrs,
    FetchSite,
    FetchExtent,
    PickBounds,
    Mesh,
    Inspect,
    Drape,
    Cancel,
    Remove,
}

impl TerrainPanel {
    fn value(&self, index: usize, label: &str, min: f64, max: f64) -> Result<f64, String> {
        self.values[index]
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite() && *n >= min && *n <= max)
            .ok_or_else(|| format!("{label}: enter a number between {min} and {max}."))
    }
    pub fn command(&self, action: &TerrainAction) -> Result<String, String> {
        Ok(match action {
            TerrainAction::SetCrs => {
                let epsg = self.value(3, "EPSG code", 1., 65535.)?;
                if epsg.fract() != 0. {
                    return Err("EPSG code must be a whole number.".into());
                }
                format!("CRS {epsg}")
            }
            TerrainAction::FetchSite => format!(
                "TERRAIN AREA {} {} {}",
                self.value(0, "Longitude", -180., 180.)?,
                self.value(1, "Latitude", -85., 85.)?,
                self.value(2, "Radius (km)", 0.001, 10000.)?
            ),
            TerrainAction::FetchExtent => "TERRAIN FETCH".into(),
            TerrainAction::PickBounds => "TERRAIN BOUNDS".into(),
            TerrainAction::Mesh => {
                let budget = self.value(6, "Mesh vertex budget", 16., 4000000.)?;
                if budget.fract() != 0. {
                    return Err("Mesh vertex budget must be a whole number.".into());
                }
                format!("TERRAIN MESH {budget}")
            }
            TerrainAction::Inspect => "TERRAIN INFO".into(),
            TerrainAction::Drape => {
                if !["ABORT", "SKIP", "CLAMP"].contains(&self.outside.as_str()) {
                    return Err("Choose an outside-coverage policy.".into());
                }
                format!(
                    "TERRAIN DRAPE {} {} {}",
                    self.value(4, "Sample spacing", 0.000001, 1e9)?,
                    self.value(5, "Vertical offset", -1e9, 1e9)?,
                    self.outside
                )
            }
            TerrainAction::Cancel => "TERRAIN CANCEL".into(),
            TerrainAction::Remove => "TERRAIN REMOVE".into(),
        })
    }
}

pub struct TerrainView {
    pub crs: String,
    pub has_crs: bool,
    pub has_model: bool,
    pub selected: usize,
    pub model: String,
    pub extent: String,
    pub progress: Option<(usize, usize, usize)>,
    pub feedback: String,
}

pub fn view<'a>(
    state: &'a TerrainPanel,
    data: TerrainView,
    sizing: crate::ui::modal::ModalSizing,
) -> Element<'a, Message> {
    let field = |index: usize, label: &'static str, hint: &'static str| {
        column![
            text(label).size(12),
            text_input(hint, &state.values[index])
                .padding(7)
                .on_input(move |v| Message::TerrainPanelValue(index, v))
        ]
        .spacing(4)
        .width(Fill)
    };
    let action = |label: &'static str, action: TerrainAction, enabled: bool| {
        button(text(label).size(13))
            .padding([8, 12])
            .on_press_maybe(enabled.then_some(Message::TerrainPanelAction(action)))
    };
    let busy = data.progress.is_some();
    let mut body = column![
        text("Build a scene from real elevation").size(23),
        text("1. Set coordinates   →   2. Choose an area   →   3. Download   →   4. Drape your objects").size(13),
        text("Coordinate system").size(17),
        text(data.crs).size(12),
        row![field(3, "Drawing EPSG code", "Use your survey's EPSG code"), action("Apply coordinates", TerrainAction::SetCrs, !busy), button("Use cloud CRS").padding(8).on_press_maybe((!busy).then_some(Message::TerrainPanelCommand("CRS LAS".into())))].spacing(10).align_y(iced::Bottom),
        text("Use the coordinate system of your existing drawing. For a new local site, use its projected CRS (such as the appropriate UTM zone). Units follow the chosen CRS.").size(12),
        text("Site and download").size(17),
        row![field(0, "Longitude (−180 to 180)", "e.g. -71.0589"), field(1, "Latitude (−85 to 85)", "e.g. 42.3601"), field(2, "Radius (kilometers)", "5")].spacing(10),
        text(format!("Source detail: zoom {} (higher = finer; actual source accuracy varies)", state.zoom)).size(12),
        slider(0..=15, state.zoom, Message::TerrainPanelZoom).step(1u32),
        text("Downloads are limited to 256 tiles and may use a coarser level for large areas. The download builds a 3D mesh automatically. Global elevation is for scene context, not a replacement for a site survey.").size(12),
        row![action("Download this site", TerrainAction::FetchSite, data.has_crs && !busy), action("Pick area in drawing", TerrainAction::PickBounds, data.has_crs && !busy), action("Download drawing area", TerrainAction::FetchExtent, data.has_crs && !busy)].spacing(8),
        text(data.extent).size(12),
        button(text("Clear picked area; use drawing extent").size(12)).on_press_maybe((!busy).then_some(Message::TerrainPanelCommand("TERRAIN BOUNDS CLEAR".into()))),
    ].spacing(12);
    if !data.has_crs {
        body = body.push(
            text("Set a drawing coordinate system above to enable terrain downloads.").size(13),
        );
    }
    if let Some((done, total, failed)) = data.progress {
        body = body
            .push(
                text(format!(
                    "Downloading: {done}/{total} tiles • {failed} failed"
                ))
                .size(13),
            )
            .push(progress_bar(0.0..=total.max(1) as f32, done as f32))
            .push(action("Cancel download", TerrainAction::Cancel, true));
    }
    body = body
        .push(text("Resident elevation model").size(17))
        .push(text(data.model).size(12));
    body = body.push(
        row![
            field(6, "Mesh vertex budget", "400000"),
            action("Rebuild mesh", TerrainAction::Mesh, data.has_model && !busy),
            action("Pick elevation", TerrainAction::Inspect, data.has_model)
        ]
        .spacing(10)
        .align_y(iced::Bottom),
    );
    body = body.push(text("Drape selected objects").size(17)).push(text(format!("{} selected object(s). Select curves, polylines or meshes in the drawing first. Draping creates copies and keeps the originals.", data.selected)).size(12));
    body = body.push(
        row![
            field(4, "Curve sample spacing (drawing units)", "1"),
            field(5, "Vertical offset (drawing units)", "0")
        ]
        .spacing(10),
    );
    body = body.push(
        row![
            column![
                text("Outside coverage").size(12),
                pick_list(
                    Some(state.outside.as_str()),
                    vec!["ABORT", "SKIP", "CLAMP"],
                    |s| s.to_string()
                )
                .on_select(|v| Message::TerrainPanelOutside(v.into()))
            ]
            .spacing(4),
            action(
                "Drape selection",
                TerrainAction::Drape,
                data.has_model && data.selected > 0 && !busy
            )
        ]
        .spacing(12)
        .align_y(iced::Bottom),
    );
    body = body.push(text("ABORT stops the operation; SKIP omits objects outside coverage; CLAMP uses the terrain boundary. Spacing controls curve sampling; meshes use their existing vertices.").size(12));
    body = body.push(text("Elevation data stays in this session. Save the drawing to keep generated meshes and draped objects. Rebuild adds another mesh; Undo removes the latest result.").size(12));
    body = body.push(action(
        "Unload elevation data (keep meshes)",
        TerrainAction::Remove,
        data.has_model && !busy,
    ));
    let feedback = if state.error.is_empty() {
        data.feedback
    } else {
        state.error.clone()
    };
    column![
        scrollable(
            container(body.width(Fill))
                .width(Fill)
                .padding(iced::Padding {
                    top: 4.,
                    right: 18.,
                    bottom: 4.,
                    left: 4.
                })
        )
        .width(Fill)
        .height(Fill),
        text(feedback).size(12),
        row![
            button("Fit scene").on_press(Message::TerrainPanelCommand("ZOOM EXTENTS".into())),
            button("3D view").on_press(Message::TerrainPanelCommand("VIEW ISO".into())),
            iced::widget::Space::new().width(Fill),
            button("Close").on_press(Message::CloseModal)
        ]
        .spacing(8)
    ]
    .spacing(12)
    .padding(16)
    .width(sizing.width)
    .height(sizing.height)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geographic_form_preserves_longitude_latitude_order() {
        let mut state = TerrainPanel::default();
        state.values[0] = "-71.0589".into();
        state.values[1] = "42.3601".into();
        assert_eq!(
            state.command(&TerrainAction::FetchSite).unwrap(),
            "TERRAIN AREA -71.0589 42.3601 5"
        );
        state.values[1] = "91".into();
        assert!(state.command(&TerrainAction::FetchSite).is_err());
    }
    #[test]
    fn terrain_forms_validate_before_dispatch() {
        let mut state = TerrainPanel::default();
        for bad in ["0", "NaN", "inf", "-1"] {
            state.values[4] = bad.into();
            assert!(state.command(&TerrainAction::Drape).is_err());
        }
        state.values[6] = "123.5".into();
        assert!(state.command(&TerrainAction::Mesh).is_err());
    }
}
