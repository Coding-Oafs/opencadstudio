//! Geodesy boundary for OpenCADStudio: typed CRS metadata, explicit
//! transformation planning, exact unit normalization, and provenance.
//!
//! The crate is the policy layer the roadmap calls for: every CRS hop is
//! planned and reported before it runs, accuracies are stated (or honestly
//! missing), and outputs can carry [`TransformationProvenance`] describing
//! exactly what executed. Ordinary horizontal math uses the pure-Rust
//! `proj4rs` backend; checksum-pinned datum/geoid grids execute in the bundled
//! out-of-process PROJ backend shipped with desktop releases.

pub mod catalog;
pub mod compound;
pub mod proj_backend;
pub mod transform;

pub use catalog::{AxisOrder, AxisUnit, CrsCatalog, CrsDefinition, CrsKind, VerticalReference};
pub use compound::{transform_xyz, CompoundTransformationPlan, CoordinateEpoch, VerticalOperation};
pub use proj_backend::{ProjBackendHealth, ProjGridBackend};
pub use transform::{
    convert_linear, plan_transformation, transform_xy, SpatialError, TransformationMethod,
    TransformationPlan, TransformationProvenance, TransformationStep,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn us_survey_foot_factor_is_exact() {
        let factor = AxisUnit::UsSurveyFoot.metres_per_unit().unwrap();
        assert!((factor - 1200.0 / 3937.0).abs() < 1e-18);
        // The classic drift trap: int'l vs US survey foot differ by ~2 ppm,
        // about 0.6 m across a 1,000,000 ft state plane coordinate.
        let intl = AxisUnit::InternationalFoot.metres_per_unit().unwrap();
        assert!((factor - intl).abs() > 0.0);
        assert!((1_000_000.0 * factor - 1_000_000.0 * intl).abs() > 0.5);
        assert_eq!(AxisUnit::Degree.metres_per_unit(), None);
    }

    #[test]
    fn catalog_covers_survey_grids_and_infers_units() {
        let catalog = CrsCatalog::well_known();
        assert_eq!(
            catalog.get(6492).unwrap().unit,
            AxisUnit::UsSurveyFoot,
            "the Boston LiDAR grid is US survey feet"
        );
        assert_eq!(catalog.get(3857).unwrap().unit, AxisUnit::Metre);
        assert_eq!(catalog.get(4326).unwrap().axis_order, AxisOrder::LatLon);
        // A code outside the curated set still infers metadata from proj4.
        let utm = catalog.get_or_infer(32619).unwrap(); // WGS 84 / UTM 19N
        assert_eq!(utm.unit, AxisUnit::Metre);
        assert!(catalog.get_or_infer(u16::MAX).is_err());
    }

    #[test]
    fn boston_round_trip_through_web_mercator() {
        let catalog = CrsCatalog::well_known();
        let plan = plan_transformation(&catalog, 6492, 3857, &[]).unwrap();
        assert_eq!(plan.steps.len(), 1);
        let mut points = [[700_000.0, 2_960_000.0]];
        let provenance = transform_xy(&plan, &mut points).unwrap();
        assert_eq!(provenance.source_unit, "US survey foot");
        assert_eq!(provenance.target_unit, "metre");
        assert_eq!(provenance.backend, "proj4rs");
        // Boston-ish coordinates must land in the right hemisphere: about
        // -71.3E / 42.35N, i.e. x ≈ -7.94M m and y ≈ 5.22M m in 3857.
        assert!(points[0][0] > -8_100_000.0 && points[0][0] < -7_800_000.0);
        assert!(points[0][1] > 5_100_000.0 && points[0][1] < 5_300_000.0);
        // Round trip back to survey feet closes well under a foot.
        let back = plan_transformation(&catalog, 3857, 6492, &[]).unwrap();
        let round_tripped = transform_xy(&back, &mut points).unwrap();
        let original = [700_000.0, 2_960_000.0];
        assert!((points[0][0] - original[0]).abs() < 0.05);
        assert!((points[0][1] - original[1]).abs() < 0.05);
        assert!(serde_json::to_string(&round_tripped)
            .unwrap()
            .contains("proj4rs"));
    }

    #[test]
    fn via_chains_are_explicit_and_identities_are_free() {
        let catalog = CrsCatalog::well_known();
        let plan = plan_transformation(&catalog, 26986, 3857, &[4326]).unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].to_epsg, 4326);
        assert_eq!(plan.steps[1].from_epsg, 4326);
        let identity = plan_transformation(&catalog, 6492, 6492, &[]).unwrap();
        assert!(matches!(
            identity.steps[0].method,
            TransformationMethod::Identity
        ));
        assert_eq!(identity.accuracy_metres, Some(0.0));
        assert!(plan.describe().contains("EPSG:"));
    }

    #[test]
    fn declared_only_steps_refuse_to_run() {
        let catalog = CrsCatalog::well_known();
        let mut plan = plan_transformation(&catalog, 6492, 3857, &[]).unwrap();
        plan.steps[0].method = TransformationMethod::DeclaredOnly {
            description: "GEOID18 grid shift".to_string(),
        };
        let mut points = [[1.0, 1.0]];
        let result = transform_xy(&plan, &mut points);
        assert!(matches!(result, Err(SpatialError::VerticalNotExecuted(_))));
    }

    #[test]
    fn linear_conversions_are_exact() {
        // 1 US survey foot in metres, then back, and across to int'l feet.
        let metres = convert_linear(AxisUnit::UsSurveyFoot, AxisUnit::Metre, 1.0).unwrap();
        assert!((metres - 0.30480060960121924).abs() < 1e-15);
        let feet = convert_linear(AxisUnit::Metre, AxisUnit::UsSurveyFoot, metres).unwrap();
        assert!((feet - 1.0).abs() < 1e-15);
        assert!(convert_linear(AxisUnit::Degree, AxisUnit::Metre, 1.0).is_err());
    }

    #[test]
    fn vertical_crs_rejects_horizontal_plans() {
        let catalog = CrsCatalog::well_known();
        let plan = plan_transformation(&catalog, 4326, 5703, &[]).unwrap();
        let mut points = [[1.0, 1.0]];
        assert!(transform_xy(&plan, &mut points).is_err());
    }
}
