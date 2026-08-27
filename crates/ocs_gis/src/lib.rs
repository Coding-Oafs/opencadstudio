//! Native GIS feature layers for OpenCADStudio.
//!
//! v1.2 GIS foundation: an OGC simple-feature geometry model with WKB and
//! GeoJSON coding, feature layers with typed attributes, and GeoPackage
//! (the OGC SQLite container) as the primary editable open format.

pub mod feature;
pub mod geometry;
pub mod geopackage;

pub use feature::{Feature, FeatureLayer, FieldValue};
pub use geometry::{
    geometry_from_geojson, geometry_from_wkb, geometry_to_geojson, geometry_to_wkb, Geometry,
};
pub use geopackage::{create_geopackage, feature_tables, open_geopackage, read_layer, write_layer};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn sample_layer() -> FeatureLayer {
        let mut layer = FeatureLayer::new("boston_buildings", 6492);
        let mut first = BTreeMap::new();
        first.insert("HEIGHT_FT".to_string(), FieldValue::Real(42.5));
        first.insert(
            "NAME".to_string(),
            FieldValue::Text("Old State House".into()),
        );
        layer.push(
            Geometry::Polygon(vec![
                vec![
                    [0.0, 0.0],
                    [100.0, 0.0],
                    [100.0, 50.0],
                    [0.0, 50.0],
                    [0.0, 0.0],
                ],
                vec![
                    [40.0, 20.0],
                    [60.0, 20.0],
                    [60.0, 30.0],
                    [40.0, 30.0],
                    [40.0, 20.0],
                ],
            ]),
            first,
        );
        let mut second = BTreeMap::new();
        second.insert("HEIGHT_FT".to_string(), FieldValue::Integer(88));
        second.insert("NAME".to_string(), FieldValue::Text("Custom House".into()));
        layer.push(
            Geometry::LineString(vec![[0.0, 0.0], [500.0, 250.0]]),
            second,
        );
        layer
    }

    #[test]
    fn wkb_round_trips_every_geometry_kind() {
        let geometries = vec![
            Geometry::Point([1.5, -2.5]),
            Geometry::MultiPoint(vec![[0.0, 0.0], [9.0, 9.0]]),
            Geometry::LineString(vec![[0.0, 0.0], [5.0, 5.0], [9.0, 0.0]]),
            Geometry::MultiLineString(vec![
                vec![[0.0, 0.0], [1.0, 1.0]],
                vec![[2.0, 2.0], [3.0, 3.0]],
            ]),
            Geometry::Polygon(vec![vec![
                [0.0, 0.0],
                [10.0, 0.0],
                [10.0, 10.0],
                [0.0, 10.0],
                [0.0, 0.0],
            ]]),
            Geometry::MultiPolygon(vec![
                vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0]]],
                vec![vec![[5.0, 5.0], [6.0, 5.0], [6.0, 6.0], [5.0, 5.0]]],
            ]),
        ];
        for geometry in &geometries {
            let wkb = geometry_to_wkb(geometry);
            let decoded = geometry_from_wkb(&wkb).unwrap();
            assert_eq!(&decoded, geometry);
            // GeoJSON agrees with WKB.
            let geojson = geometry_to_geojson(geometry);
            assert_eq!(&geometry_from_geojson(&geojson).unwrap(), geometry);
        }
    }

    #[test]
    fn polygons_contain_their_interior_and_boundary() {
        let polygon = Geometry::Polygon(vec![vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0],
            [0.0, 0.0],
        ]]);
        assert!(polygon.contains(5.0, 5.0));
        assert!(polygon.contains(0.0, 5.0)); // boundary inclusive
        assert!(!polygon.contains(15.0, 5.0));
        let with_hole = Geometry::Polygon(vec![
            vec![
                [0.0, 0.0],
                [10.0, 0.0],
                [10.0, 10.0],
                [0.0, 10.0],
                [0.0, 0.0],
            ],
            vec![[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0], [4.0, 4.0]],
        ]);
        assert!(!with_hole.contains(5.0, 5.0));
        assert!(with_hole.contains(2.0, 2.0));
    }

    #[test]
    fn geopackage_round_trips_layers_and_attributes() {
        let dir = std::env::temp_dir().join(format!("ocs-gis-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.gpkg");
        let _ = std::fs::remove_file(&path);
        let connection = create_geopackage(&path).unwrap();
        let layer = sample_layer();
        write_layer(&connection, &layer).unwrap();
        let tables = feature_tables(&connection).unwrap();
        assert_eq!(tables, vec!["boston_buildings".to_string()]);
        let read = read_layer(&connection, "boston_buildings").unwrap();
        assert_eq!(read.name, "boston_buildings");
        assert_eq!(read.epsg, 6492);
        assert_eq!(read.features.len(), 2);
        assert_eq!(read.fields.len(), 2);
        assert_eq!(
            read.features[0].properties.get("NAME"),
            Some(&FieldValue::Text("Old State House".into()))
        );
        assert_eq!(
            read.features[1].properties.get("HEIGHT_FT"),
            Some(&FieldValue::Integer(88))
        );
        assert_eq!(read.features[0].geometry, layer.features[0].geometry);
        // The contents envelope was recorded from real bounds.
        let (min_x, max_y): (f64, f64) = connection
            .query_row(
                "SELECT min_x, max_y FROM gpkg_contents WHERE table_name = 'boston_buildings'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(min_x, 0.0);
        assert_eq!(max_y, 250.0);
        drop(connection);
        // Reopen and read again: the container stands on its own.
        let reopened = open_geopackage(&path).unwrap();
        let again = read_layer(&reopened, "boston_buildings").unwrap();
        assert_eq!(again.features.len(), 2);
    }

    #[test]
    fn refuses_to_overwrite_existing_gpkg() {
        let dir = std::env::temp_dir().join(format!("ocs-gis-refuse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("exists.gpkg");
        std::fs::write(&path, b"already there").unwrap();
        assert!(create_geopackage(&path).is_err());
        assert!(open_geopackage(&dir.join("missing.gpkg")).is_err());
    }

    #[test]
    fn geojson_round_trips_layers_with_crs() {
        let layer = sample_layer();
        let text = layer.to_geojson();
        let parsed = FeatureLayer::from_geojson(&text, "fallback", 4326).unwrap();
        assert_eq!(parsed.name, "boston_buildings");
        assert_eq!(parsed.epsg, 6492); // carried by the crs member
        assert_eq!(parsed.features.len(), 2);
        assert_eq!(parsed.fields.len(), 2);
        assert_eq!(parsed.features[0].geometry, layer.features[0].geometry);
        assert_eq!(
            parsed.features[0].properties.get("HEIGHT_FT"),
            Some(&FieldValue::Real(42.5))
        );
        // A bare collection falls back to the default CRS.
        let bare = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":{"type":"Point","coordinates":[1,2]},"properties":{}}]}"#;
        let parsed = FeatureLayer::from_geojson(bare, "bare", 4326).unwrap();
        assert_eq!(parsed.epsg, 4326);
        assert_eq!(parsed.features.len(), 1);
    }

    #[test]
    fn field_values_parse_and_serialize() {
        assert_eq!(FieldValue::from_text("42"), FieldValue::Integer(42));
        assert_eq!(FieldValue::from_text("42.5"), FieldValue::Real(42.5));
        assert_eq!(FieldValue::from_text("true"), FieldValue::Boolean(true));
        assert_eq!(
            FieldValue::from_text("Custom House"),
            FieldValue::Text("Custom House".into())
        );
        assert_eq!(FieldValue::from_text(""), FieldValue::Null);
        assert_eq!(FieldValue::Integer(-7).to_sql_text(), "-7");
    }
}
