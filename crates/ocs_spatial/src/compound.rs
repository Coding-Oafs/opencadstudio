//! Compound horizontal/vertical transformations with epoch-aware provenance.
//!
//! Constant-offset operations support local engineering datums. Grid
//! operations execute only through the checksum-pinned bundled PROJ worker;
//! missing or modified resources fail closed and are never approximated.

use crate::{
    plan_transformation, transform_xy, CrsCatalog, ProjGridBackend, SpatialError,
    TransformationPlan, TransformationProvenance,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoordinateEpoch(pub f64);

impl CoordinateEpoch {
    pub fn validate(self) -> Result<Self, SpatialError> {
        if self.0.is_finite() && (1800.0..=2300.0).contains(&self.0) {
            Ok(self)
        } else {
            Err(SpatialError::TransformFailed(format!(
                "coordinate epoch {} is outside the supported range",
                self.0
            )))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "method")]
pub enum VerticalOperation {
    Identity,
    /// Add a surveyed constant in metres after horizontal reprojection.
    ConstantOffset {
        metres: f64,
        description: String,
        accuracy_metres: Option<f64>,
    },
    /// A required geoid/datum grid. Execution deliberately fails until the
    /// named, checksummed resource is available to a validated backend.
    Grid {
        name: String,
        checksum_sha256: String,
        accuracy_metres: Option<f64>,
        #[serde(default)]
        inverse: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompoundTransformationPlan {
    pub horizontal: TransformationPlan,
    pub source_vertical_epsg: Option<u16>,
    pub target_vertical_epsg: Option<u16>,
    pub vertical: VerticalOperation,
    pub coordinate_epoch: Option<CoordinateEpoch>,
}

impl CompoundTransformationPlan {
    pub fn validate(&self) -> Result<(), SpatialError> {
        if let Some(epoch) = self.coordinate_epoch {
            epoch.validate()?;
        }
        if self.source_vertical_epsg != self.target_vertical_epsg
            && matches!(self.vertical, VerticalOperation::Identity)
        {
            return Err(SpatialError::VerticalNotExecuted(
                "different vertical CRSs require an explicit operation".into(),
            ));
        }
        match &self.vertical {
            VerticalOperation::ConstantOffset {
                metres,
                accuracy_metres,
                ..
            } => {
                if !metres.is_finite()
                    || accuracy_metres
                        .is_some_and(|accuracy| !accuracy.is_finite() || accuracy < 0.0)
                {
                    return Err(SpatialError::TransformFailed(
                        "vertical offset and accuracy must be finite".into(),
                    ));
                }
            }
            VerticalOperation::Grid {
                name,
                checksum_sha256,
                accuracy_metres,
                ..
            } => {
                if name.trim().is_empty()
                    || checksum_sha256.len() != 64
                    || !checksum_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
                    || accuracy_metres
                        .is_some_and(|accuracy| !accuracy.is_finite() || accuracy < 0.0)
                {
                    return Err(SpatialError::TransformFailed(
                        "vertical grid requires a name, SHA-256 checksum, and valid accuracy"
                            .into(),
                    ));
                }
            }
            VerticalOperation::Identity => {}
        }
        Ok(())
    }
}

pub fn transform_xyz(
    plan: &CompoundTransformationPlan,
    points: &mut [[f64; 3]],
) -> Result<TransformationProvenance, SpatialError> {
    plan.validate()?;
    if let VerticalOperation::Grid {
        name,
        checksum_sha256,
        accuracy_metres,
        inverse,
    } = &plan.vertical
    {
        let catalog = CrsCatalog::well_known();
        let to_geographic = plan_transformation(&catalog, plan.horizontal.source_epsg, 4326, &[])?;
        let from_geographic =
            plan_transformation(&catalog, 4326, plan.horizontal.target_epsg, &[])?;
        let mut geographic: Vec<[f64; 3]> = points.to_vec();
        let mut xy: Vec<[f64; 2]> = geographic
            .iter()
            .map(|point| [point[0], point[1]])
            .collect();
        let before = transform_xy(&to_geographic, &mut xy)?;
        for (point, transformed) in geographic.iter_mut().zip(xy) {
            point[0] = transformed[0];
            point[1] = transformed[1];
        }
        ProjGridBackend::discover()?.transform_vertical_grid(
            name,
            checksum_sha256,
            *inverse,
            &mut geographic,
        )?;
        let mut xy: Vec<[f64; 2]> = geographic
            .iter()
            .map(|point| [point[0], point[1]])
            .collect();
        let after = transform_xy(&from_geographic, &mut xy)?;
        for ((output, geographic), transformed) in points.iter_mut().zip(geographic).zip(xy) {
            *output = [transformed[0], transformed[1], geographic[2]];
        }
        let mut steps = before.steps;
        steps.push(format!(
            "PROJ vgridshift {} grid '{}' (SHA-256 {})",
            if *inverse { "inverse" } else { "forward" },
            name,
            checksum_sha256
        ));
        steps.extend(after.steps);
        return Ok(TransformationProvenance {
            source_epsg: plan.horizontal.source_epsg,
            target_epsg: plan.horizontal.target_epsg,
            source_unit: before.source_unit,
            target_unit: after.target_unit,
            steps,
            backend: "proj4rs + bundled PROJ grid worker".into(),
            accuracy_metres: combine_accuracy(
                combine_accuracy(before.accuracy_metres, after.accuracy_metres),
                *accuracy_metres,
            ),
            coordinate_epoch: plan.coordinate_epoch.map(|epoch| format!("{:.4}", epoch.0)),
            point_count: points.len() as u64,
            executed_utc: after.executed_utc,
        });
    }
    let mut horizontal: Vec<[f64; 2]> = points.iter().map(|point| [point[0], point[1]]).collect();
    let mut provenance = transform_xy(&plan.horizontal, &mut horizontal)?;
    for (point, xy) in points.iter_mut().zip(horizontal) {
        point[0] = xy[0];
        point[1] = xy[1];
        if let VerticalOperation::ConstantOffset { metres, .. } = plan.vertical {
            point[2] += metres;
        }
    }
    match &plan.vertical {
        VerticalOperation::Identity => provenance.steps.push("vertical identity".into()),
        VerticalOperation::ConstantOffset {
            metres,
            description,
            accuracy_metres,
        } => {
            provenance
                .steps
                .push(format!("vertical offset {metres:+.6} m ({description})"));
            provenance.accuracy_metres =
                combine_accuracy(provenance.accuracy_metres, *accuracy_metres);
        }
        VerticalOperation::Grid { .. } => unreachable!(),
    }
    provenance.coordinate_epoch = plan.coordinate_epoch.map(|epoch| format!("{:.4}", epoch.0));
    provenance.point_count = points.len() as u64;
    Ok(provenance)
}

fn combine_accuracy(horizontal: Option<f64>, vertical: Option<f64>) -> Option<f64> {
    Some((horizontal?.powi(2) + vertical?.powi(2)).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{plan_transformation, CrsCatalog};

    #[test]
    fn compound_offset_transforms_xy_z_and_records_epoch() {
        let horizontal = plan_transformation(&CrsCatalog::well_known(), 4326, 3857, &[]).unwrap();
        let plan = CompoundTransformationPlan {
            horizontal,
            source_vertical_epsg: Some(5703),
            target_vertical_epsg: Some(5703),
            vertical: VerticalOperation::ConstantOffset {
                metres: 1.25,
                description: "project benchmark adjustment".into(),
                accuracy_metres: Some(0.02),
            },
            coordinate_epoch: Some(CoordinateEpoch(2026.5)),
        };
        let mut points = [[-71.0, 42.0, 10.0]];
        let provenance = transform_xyz(&plan, &mut points).unwrap();
        assert!(points[0][0] < -7_000_000.0);
        assert_eq!(points[0][2], 11.25);
        assert_eq!(provenance.coordinate_epoch.as_deref(), Some("2026.5000"));
        assert!(provenance.steps.last().unwrap().contains("+1.250000"));
    }

    #[test]
    fn grid_and_implicit_vertical_changes_fail_closed() {
        let horizontal = plan_transformation(&CrsCatalog::well_known(), 4326, 4326, &[]).unwrap();
        let implicit = CompoundTransformationPlan {
            horizontal: horizontal.clone(),
            source_vertical_epsg: Some(5703),
            target_vertical_epsg: Some(6360),
            vertical: VerticalOperation::Identity,
            coordinate_epoch: None,
        };
        assert!(implicit.validate().is_err());
        let grid = CompoundTransformationPlan {
            horizontal,
            source_vertical_epsg: Some(5703),
            target_vertical_epsg: Some(6360),
            vertical: VerticalOperation::Grid {
                name: "g2018u0.bin".into(),
                checksum_sha256: "a".repeat(64),
                accuracy_metres: Some(0.03),
                inverse: false,
            },
            coordinate_epoch: None,
        };
        let mut points = [[0.0, 0.0, 0.0]];
        assert!(matches!(
            transform_xyz(&grid, &mut points),
            Err(SpatialError::VerticalNotExecuted(_))
        ));
    }

    #[test]
    #[ignore = "requires the packaged PROJ worker and official GEOID18 grid"]
    fn bundled_geoid18_grid_changes_vertical_coordinate() {
        let horizontal = plan_transformation(&CrsCatalog::well_known(), 4326, 4326, &[]).unwrap();
        let plan = CompoundTransformationPlan {
            horizontal,
            source_vertical_epsg: Some(4979),
            target_vertical_epsg: Some(5703),
            vertical: VerticalOperation::Grid {
                name: "us_noaa_g2018u0.tif".into(),
                checksum_sha256: "fa9a407ac7ee3f5a3694008e4bcd09ce9cc250452f0c3b11700a4960340abce2"
                    .into(),
                accuracy_metres: Some(0.03),
                inverse: false,
            },
            coordinate_epoch: Some(CoordinateEpoch(2020.0)),
        };
        let mut points = [[-71.0589, 42.3601, 0.0]];
        let provenance = transform_xyz(&plan, &mut points).unwrap();
        assert!(points[0][2].abs() > 1.0);
        assert!(provenance.backend.contains("bundled PROJ"));
    }
}
