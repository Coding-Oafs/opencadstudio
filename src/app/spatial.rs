//! Drawing-owned spatial reference and working-unit settings.
//!
//! CAD geometry remains plain coordinates and INSUNITS keeps its DWG-defined
//! insertion-scaling role. These settings describe the coordinate space used
//! by GIS underlays and the unit displayed by survey/inquiry tools.

use super::{Message, OpenCADStudio};
use iced::Task;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkingUnit {
    #[default]
    Meters,
    Centimeters,
    Feet,
    Inches,
    /// Geographic CRSs are angular. This value is assigned by CRS and is not
    /// offered in the CRS-free working-unit picker.
    Degrees,
}

impl WorkingUnit {
    pub fn label(self) -> &'static str {
        match self {
            Self::Meters => "meters",
            Self::Centimeters => "centimeters",
            Self::Feet => "feet",
            Self::Inches => "inches",
            Self::Degrees => "degrees",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Self::Meters => "m",
            Self::Centimeters => "cm",
            Self::Feet => "ft",
            Self::Inches => "in",
            Self::Degrees => "deg",
        }
    }

    pub fn from_keyword(value: &str) -> Option<Self> {
        match value.trim().to_ascii_uppercase().as_str() {
            "M" | "METER" | "METERS" | "METRE" | "METRES" => Some(Self::Meters),
            "CM" | "CENTIMETER" | "CENTIMETERS" | "CENTIMETRE" | "CENTIMETRES" => {
                Some(Self::Centimeters)
            }
            "FT" | "FOOT" | "FEET" => Some(Self::Feet),
            "IN" | "INCH" | "INCHES" => Some(Self::Inches),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoordinateUnit {
    Meters,
    Feet,
    UsSurveyFeet,
    Degrees,
}

impl CoordinateUnit {
    fn from_proj_unit(value: &str) -> Option<Self> {
        match value {
            "m" => Some(Self::Meters),
            "ft" => Some(Self::Feet),
            "us-ft" => Some(Self::UsSurveyFeet),
            "degrees" => Some(Self::Degrees),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Meters => "meters",
            Self::Feet => "international feet",
            Self::UsSurveyFeet => "U.S. survey feet",
            Self::Degrees => "degrees",
        }
    }

    fn required_working_unit(self) -> WorkingUnit {
        match self {
            Self::Meters => WorkingUnit::Meters,
            Self::Feet | Self::UsSurveyFeet => WorkingUnit::Feet,
            Self::Degrees => WorkingUnit::Degrees,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrawingCrs {
    pub epsg: u16,
    pub coordinate_unit: CoordinateUnit,
}

impl DrawingCrs {
    pub fn label(&self) -> String {
        format!("EPSG:{} ({})", self.epsg, self.coordinate_unit.label())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn from_epsg(epsg: u16) -> Result<Self, String> {
        let unit = ocs_pointcloud::epsg_horizontal_unit(epsg)
            .ok_or_else(|| format!("EPSG:{epsg} is unavailable in the bundled CRS database"))?;
        let coordinate_unit = CoordinateUnit::from_proj_unit(unit)
            .ok_or_else(|| format!("EPSG:{epsg} uses unsupported horizontal units \"{unit}\""))?;
        Ok(Self {
            epsg,
            coordinate_unit,
        })
    }

    #[cfg(target_arch = "wasm32")]
    fn from_epsg(epsg: u16) -> Result<Self, String> {
        // Native performs authoritative validation through proj4rs. The web
        // build currently has no CRS database; retain a useful drawing label.
        Ok(Self {
            epsg,
            coordinate_unit: CoordinateUnit::Meters,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DrawingSpatialSettings {
    pub drawing_crs: Option<DrawingCrs>,
    pub working_unit: WorkingUnit,
    /// Optional drawing-coordinate envelope supplied before geometry or LiDAR
    /// exists: `[min_x, min_y, max_x, max_y]`.
    pub basemap_bounds: Option<[f64; 4]>,
}

impl Default for DrawingSpatialSettings {
    fn default() -> Self {
        Self {
            drawing_crs: None,
            working_unit: WorkingUnit::Meters,
            basemap_bounds: None,
        }
    }
}

impl DrawingSpatialSettings {
    pub fn working_unit_allowed(&self, unit: WorkingUnit) -> bool {
        self.drawing_crs
            .as_ref()
            .is_none_or(|crs| crs.coordinate_unit.required_working_unit() == unit)
    }

    fn normalize(&mut self) {
        match self.drawing_crs.as_ref() {
            Some(crs) => self.working_unit = crs.coordinate_unit.required_working_unit(),
            None if self.working_unit == WorkingUnit::Degrees => {
                self.working_unit = WorkingUnit::Meters
            }
            None => {}
        }
    }
}

impl OpenCADStudio {
    pub(super) fn drawing_crs_command(&mut self, argument: &str) -> Task<Message> {
        let i = self.active_tab;
        let tab_id = self.tabs[i].id;
        let argument = argument.trim();
        if argument.is_empty() || argument.eq_ignore_ascii_case("STATUS") {
            let spatial = &self.tabs[i].spatial;
            let crs = spatial
                .drawing_crs
                .as_ref()
                .map(DrawingCrs::label)
                .unwrap_or_else(|| "unset".to_string());
            self.command_line.push_output(
                format!(
                    "Drawing CRS: {crs}; working units: {}.",
                    spatial.working_unit.label()
                )
                .as_str(),
            );
            return Task::none();
        }
        if matches!(
            argument.to_ascii_uppercase().as_str(),
            "NONE" | "UNSET" | "CLEAR"
        ) {
            self.tabs[i].spatial.drawing_crs = None;
            self.tabs[i].spatial.normalize();
            self.persist_spatial_settings(i);
            self.command_line.push_output(
                format!(
                    "Drawing CRS cleared; working units remain {}.",
                    self.tabs[i].spatial.working_unit.label()
                )
                .as_str(),
            );
            return self.refresh_basemap(tab_id);
        }

        #[cfg(not(target_arch = "wasm32"))]
        let epsg = if argument.eq_ignore_ascii_case("LAS")
            || argument.eq_ignore_ascii_case("POINTCLOUD")
        {
            self.tabs[i]
                .point_cloud
                .active()
                .and_then(|source| source.sample.metadata.crs.horizontal_epsg)
        } else {
            argument
                .trim_start_matches("EPSG:")
                .trim_start_matches("epsg:")
                .parse::<u16>()
                .ok()
        };
        #[cfg(target_arch = "wasm32")]
        let epsg = argument
            .trim_start_matches("EPSG:")
            .trim_start_matches("epsg:")
            .parse::<u16>()
            .ok();

        let Some(epsg) = epsg else {
            self.command_line
                .push_error("CRS <EPSG code|LAS|UNSET> (example: CRS 32615).");
            return Task::none();
        };
        let crs = match DrawingCrs::from_epsg(epsg) {
            Ok(crs) => crs,
            Err(error) => {
                self.command_line.push_error(&error);
                return Task::none();
            }
        };
        let unit = crs.coordinate_unit.required_working_unit();
        let label = crs.label();
        let previous_epsg = self.tabs[i]
            .spatial
            .drawing_crs
            .as_ref()
            .map(|current| current.epsg);
        let cleared_stale_bounds = previous_epsg.is_some_and(|current| current != epsg)
            && self.tabs[i].spatial.basemap_bounds.take().is_some();
        self.tabs[i].spatial.drawing_crs = Some(crs);
        self.tabs[i].spatial.working_unit = unit;
        self.basemap.projection = crate::scene::basemap::BasemapProjection::FromDrawing;
        self.sync_basemap_dropdown();
        self.persist_spatial_settings(i);
        self.command_line.push_output(
            format!(
                "Drawing CRS set to {label}; working units locked to {}.",
                unit.label()
            )
            .as_str(),
        );
        if cleared_stale_bounds {
            self.command_line.push_info(
                "Basemap: cleared the previous manual extent because drawing coordinates changed CRS; use Set Location to choose the site again.",
            );
        }
        self.refresh_basemap(tab_id)
    }

    pub(super) fn working_units_command(&mut self, argument: &str) {
        let i = self.active_tab;
        let argument = argument.trim();
        if argument.is_empty() || argument.eq_ignore_ascii_case("STATUS") {
            self.command_line.push_output(
                format!(
                    "Working units: {}.",
                    self.tabs[i].spatial.working_unit.label()
                )
                .as_str(),
            );
            return;
        }
        let Some(unit) = WorkingUnit::from_keyword(argument) else {
            self.command_line
                .push_error("WORKINGUNITS <METERS|CENTIMETERS|FEET|INCHES>.");
            return;
        };
        if !self.tabs[i].spatial.working_unit_allowed(unit) {
            let crs = self.tabs[i]
                .spatial
                .drawing_crs
                .as_ref()
                .map(DrawingCrs::label)
                .unwrap_or_default();
            self.command_line.push_error(
                format!(
                    "WORKINGUNITS: {} is incompatible with {crs}; clear or change the CRS first.",
                    unit.label()
                )
                .as_str(),
            );
            return;
        }
        self.tabs[i].spatial.working_unit = unit;
        self.sync_basemap_dropdown();
        self.persist_spatial_settings(i);
        self.command_line
            .push_output(format!("Working units set to {}.", unit.label()).as_str());
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn persist_spatial_settings(&mut self, tab_index: usize) {
        let Some(path) = self.tabs[tab_index].current_path.clone() else {
            return;
        };
        let result =
            ocs_pointcloud::SidecarStore::open(ocs_pointcloud::sidecar_path_for_drawing(path))
                .and_then(|store| store.save_drawing_settings(&self.tabs[tab_index].spatial));
        if let Err(error) = result {
            self.command_line
                .push_error(format!("SPATIALSETTINGS: {error}").as_str());
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn persist_spatial_settings(&mut self, _tab_index: usize) {}

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn load_spatial_settings(&mut self, tab_index: usize) {
        let Some(path) = self.tabs[tab_index].current_path.clone() else {
            return;
        };
        let sidecar = ocs_pointcloud::sidecar_path_for_drawing(path);
        if !sidecar.exists() {
            return;
        }
        match ocs_pointcloud::SidecarStore::open(sidecar)
            .and_then(|store| store.load_drawing_settings::<DrawingSpatialSettings>())
        {
            Ok(Some(mut settings)) => {
                settings.normalize();
                self.tabs[tab_index].spatial = settings;
            }
            Ok(None) => {}
            Err(error) => self
                .command_line
                .push_error(format!("SPATIALSETTINGS: {error}").as_str()),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn load_spatial_settings(&mut self, _tab_index: usize) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crs_free_units_offer_both_systems() {
        let settings = DrawingSpatialSettings::default();
        for unit in [
            WorkingUnit::Meters,
            WorkingUnit::Centimeters,
            WorkingUnit::Feet,
            WorkingUnit::Inches,
        ] {
            assert!(settings.working_unit_allowed(unit));
        }
    }

    #[test]
    fn projected_crs_locks_units_to_its_coordinate_unit() {
        let settings = DrawingSpatialSettings {
            drawing_crs: Some(DrawingCrs {
                epsg: 32615,
                coordinate_unit: CoordinateUnit::Meters,
            }),
            ..Default::default()
        };
        assert!(settings.working_unit_allowed(WorkingUnit::Meters));
        assert!(!settings.working_unit_allowed(WorkingUnit::Feet));
        assert!(!settings.working_unit_allowed(WorkingUnit::Centimeters));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn epsg_validation_derives_metric_foot_and_geographic_units() {
        assert_eq!(
            DrawingCrs::from_epsg(3857).unwrap().coordinate_unit,
            CoordinateUnit::Meters
        );
        assert_eq!(
            DrawingCrs::from_epsg(2263).unwrap().coordinate_unit,
            CoordinateUnit::UsSurveyFeet
        );
        assert_eq!(
            DrawingCrs::from_epsg(4326).unwrap().coordinate_unit,
            CoordinateUnit::Degrees
        );
    }
}
