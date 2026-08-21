//! Embedded version metadata generated at build time.

/// The embedded version info as a JSON string.
pub const EMBEDDED_VERSION_INFO_JSON: &str =
    include_str!(concat!(env!("OUT_DIR"), "/version_info.json"));

/// Convenience accessor for the embedded version info JSON.
pub fn get_embedded_version_info_json() -> &'static str {
    EMBEDDED_VERSION_INFO_JSON
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_version_info_is_valid_json() {
        let json = get_embedded_version_info_json();
        assert!(!json.is_empty());
        let value: serde_json::Value = serde_json::from_str(json).expect("valid JSON");
        assert!(value.get("ocs_version").is_some());
        assert!(value.get("acadrust_version").is_some());
    }

    #[test]
    fn embedded_version_info_has_expected_keys() {
        let value: serde_json::Value =
            serde_json::from_str(get_embedded_version_info_json()).expect("valid JSON");
        for key in [
            "ocs_version",
            "ocs_plugin_api_version",
            "acadrust_version",
            "acadrust_source",
            "api_version",
            "api_version_min_supported",
            "build_timestamp",
        ] {
            assert!(value.get(key).is_some(), "missing version info key {}", key);
        }
    }

    #[test]
    fn embedded_version_info_matches_package_versions() {
        let value: serde_json::Value =
            serde_json::from_str(get_embedded_version_info_json()).expect("valid JSON");
        assert_eq!(
            value["ocs_plugin_api_version"].as_str(),
            Some(env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(value["api_version"].as_i64(), Some(4));
        assert_eq!(value["api_version_min_supported"].as_i64(), Some(2));

        // acadrust_version should have a patch component (e.g. "0.4.0").
        let acadrust = value["acadrust_version"].as_str().expect("acadrust_version string");
        assert_eq!(acadrust.split('.').count(), 3);
    }
}
