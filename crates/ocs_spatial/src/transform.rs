//! Explicit transformation planning, execution, and provenance.
//!
//! A transformation never "just happens": callers build a
//! [`TransformationPlan`] naming every CRS hop, the plan reports the best
//! known accuracy, and executing it returns provenance suitable for
//! attaching to derived outputs. The math currently runs through the pure
//! Rust `proj4rs` backend; grid-based datum shifts and geoid models are
//! modelled here and will execute behind the same plan interface once a
//! validated backend is bundled.

use crate::catalog::{AxisUnit, CrsCatalog, CrsKind};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// One CRS hop of a planned transformation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "method")]
pub enum TransformationMethod {
    /// Same EPSG on both sides; coordinates pass through.
    Identity,
    /// proj4rs pipeline between the two EPSG codes.
    Proj4Pipeline,
    /// Declared but not executed: a grid shift or geoid evaluation that a
    /// future backend will perform. Planning includes it so provenance can
    /// state honestly that it did not run.
    DeclaredOnly { description: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransformationStep {
    pub from_epsg: u16,
    pub to_epsg: u16,
    pub method: TransformationMethod,
    /// Best-known nominal accuracy in metres, when documented.
    pub accuracy_metres: Option<f64>,
}

/// A validated chain of steps from one CRS to another.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransformationPlan {
    pub source_epsg: u16,
    pub target_epsg: u16,
    pub steps: Vec<TransformationStep>,
    /// Combined nominal accuracy in metres; `None` means "not documented".
    pub accuracy_metres: Option<f64>,
}

impl TransformationPlan {
    /// Root-sum-square of the step accuracies; unknown steps keep the whole
    /// plan unknown rather than inventing a number.
    pub fn combined_accuracy(steps: &[TransformationStep]) -> Option<f64> {
        let mut total = 0.0;
        for step in steps {
            total += step.accuracy_metres?.powi(2);
        }
        Some(total.sqrt())
    }

    pub fn describe(&self) -> String {
        let chain = self
            .steps
            .iter()
            .map(|step| format!("{}->{}", step.from_epsg, step.to_epsg))
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "EPSG:{} -> EPSG:{} via [{}], nominal accuracy {}",
            self.source_epsg,
            self.target_epsg,
            chain,
            match self.accuracy_metres {
                Some(metres) => format!("{metres:.3} m"),
                None => "undocumented".to_string(),
            }
        )
    }
}

/// Provenance block to attach to derived outputs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransformationProvenance {
    pub source_epsg: u16,
    pub target_epsg: u16,
    pub source_unit: String,
    pub target_unit: String,
    pub steps: Vec<String>,
    pub backend: String,
    pub accuracy_metres: Option<f64>,
    /// Coordinate epoch of dynamic frames, when declared.
    pub coordinate_epoch: Option<String>,
    pub point_count: u64,
    pub executed_utc: String,
}

/// Errors from planning or executing a transformation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpatialError {
    UnknownCrs(u16),
    /// The requested unit conversion cannot be exact (degree involved).
    NotALinearConversion {
        from: AxisUnit,
        to: AxisUnit,
    },
    TransformFailed(String),
    VerticalNotExecuted(String),
}

impl std::fmt::Display for SpatialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownCrs(epsg) => write!(f, "EPSG:{epsg} is not usable"),
            Self::NotALinearConversion { from, to } => {
                write!(f, "no linear conversion between {from:?} and {to:?}")
            }
            Self::TransformFailed(message) => write!(f, "transformation failed: {message}"),
            Self::VerticalNotExecuted(message) => {
                write!(f, "vertical operation declared but not executed: {message}")
            }
        }
    }
}

impl std::error::Error for SpatialError {}

/// Nominal accuracies for documented datum hops. Same-datum reprojections
/// are exact-in-principle (sub-millimetre at survey scales) and omitted.
fn hop_accuracy(from: u16, to: u16) -> Option<f64> {
    let pair = (from.min(to), from.max(to));
    match pair {
        // NAD83 <-> WGS 84 realizations: nominal ~1 m (NAD83(2011)/G1150).
        (4326, 6318) | (4326, 4269) => Some(1.0),
        _ => None,
    }
}

/// Build an explicit transformation plan through an optional via-chain.
///
/// `via` lists intermediate EPSG codes; an empty chain transforms directly
/// (proj4rs composes the datum path internally). Every hop is reported so
/// callers can reject plans they do not trust.
pub fn plan_transformation(
    catalog: &CrsCatalog,
    source_epsg: u16,
    target_epsg: u16,
    via: &[u16],
) -> Result<TransformationPlan, SpatialError> {
    for epsg in std::iter::once(source_epsg)
        .chain(via.iter().copied())
        .chain(std::iter::once(target_epsg))
    {
        catalog
            .get_or_infer(epsg)
            .map_err(|_| SpatialError::UnknownCrs(epsg))?;
    }
    let mut chain: Vec<u16> = Vec::with_capacity(via.len() + 2);
    chain.push(source_epsg);
    for epsg in via.iter().copied().filter(|epsg| *epsg != source_epsg) {
        chain.push(epsg);
    }
    if chain.len() < 2 || chain.last() != Some(&target_epsg) {
        chain.push(target_epsg);
    }
    let steps: Vec<TransformationStep> = chain
        .windows(2)
        .map(|window| {
            let (from, to) = (window[0], window[1]);
            TransformationStep {
                from_epsg: from,
                to_epsg: to,
                method: if from == to {
                    TransformationMethod::Identity
                } else {
                    TransformationMethod::Proj4Pipeline
                },
                accuracy_metres: if from == to {
                    Some(0.0)
                } else {
                    hop_accuracy(from, to)
                },
            }
        })
        .collect();
    let accuracy_metres = TransformationPlan::combined_accuracy(&steps);
    Ok(TransformationPlan {
        source_epsg,
        target_epsg,
        steps,
        accuracy_metres,
    })
}

fn iso_utc_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default();
    let days = seconds.div_euclid(86_400);
    let time_of_day = seconds.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = if month <= 2 { year + 1 } else { year };
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time_of_day / 3600,
        (time_of_day % 3600) / 60,
        time_of_day % 60
    )
}

/// Execute a horizontal plan over x/easting-northing pairs, returning
/// provenance describing exactly what ran.
pub fn transform_xy(
    plan: &TransformationPlan,
    points: &mut [[f64; 2]],
) -> Result<TransformationProvenance, SpatialError> {
    let catalog = CrsCatalog::well_known();
    let source = catalog
        .get_or_infer(plan.source_epsg)
        .map_err(|_| SpatialError::UnknownCrs(plan.source_epsg))?;
    let target = catalog
        .get_or_infer(plan.target_epsg)
        .map_err(|_| SpatialError::UnknownCrs(plan.target_epsg))?;
    for step in &plan.steps {
        if let TransformationMethod::DeclaredOnly { description } = &step.method {
            return Err(SpatialError::VerticalNotExecuted(description.clone()));
        }
    }
    if matches!(source.kind, CrsKind::Vertical) || matches!(target.kind, CrsKind::Vertical) {
        return Err(SpatialError::TransformFailed(
            "vertical CRSs cannot take part in a horizontal transformation".to_string(),
        ));
    }
    if plan.source_epsg != plan.target_epsg && !points.is_empty() {
        let source_projection = proj4rs::Proj::from_epsg_code(plan.source_epsg)
            .map_err(|error| SpatialError::TransformFailed(error.to_string()))?;
        let target_projection = proj4rs::Proj::from_epsg_code(plan.target_epsg)
            .map_err(|error| SpatialError::TransformFailed(error.to_string()))?;
        for point in points.iter_mut() {
            let mut coordinate = (point[0], point[1], 0.0);
            if source_projection.is_latlong() {
                coordinate.0 = coordinate.0.to_radians();
                coordinate.1 = coordinate.1.to_radians();
            }
            proj4rs::transform::transform(&source_projection, &target_projection, &mut coordinate)
                .map_err(|error| SpatialError::TransformFailed(error.to_string()))?;
            if target_projection.is_latlong() {
                coordinate.0 = coordinate.0.to_degrees();
                coordinate.1 = coordinate.1.to_degrees();
            }
            *point = [coordinate.0, coordinate.1];
        }
    }
    Ok(TransformationProvenance {
        source_epsg: plan.source_epsg,
        target_epsg: plan.target_epsg,
        source_unit: source.unit.label().to_string(),
        target_unit: target.unit.label().to_string(),
        steps: plan
            .steps
            .iter()
            .map(|step| {
                format!(
                    "EPSG:{} -> EPSG:{} ({})",
                    step.from_epsg,
                    step.to_epsg,
                    match &step.method {
                        TransformationMethod::Identity => "identity".to_string(),
                        TransformationMethod::Proj4Pipeline => "proj4rs".to_string(),
                        TransformationMethod::DeclaredOnly { description } => description.clone(),
                    }
                )
            })
            .collect(),
        backend: "proj4rs".to_string(),
        accuracy_metres: plan.accuracy_metres,
        coordinate_epoch: None,
        point_count: points.len() as u64,
        executed_utc: iso_utc_now(),
    })
}

/// Convert a linear quantity between CRS units without changing the datum —
/// exact, because the factors are exact.
pub fn convert_linear(from: AxisUnit, to: AxisUnit, value: f64) -> Result<f64, SpatialError> {
    let from_factor = from
        .metres_per_unit()
        .ok_or(SpatialError::NotALinearConversion { from, to })?;
    let to_factor = to
        .metres_per_unit()
        .ok_or(SpatialError::NotALinearConversion { from, to })?;
    Ok(value * from_factor / to_factor)
}
