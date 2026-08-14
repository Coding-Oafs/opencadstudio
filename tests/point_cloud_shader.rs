const SHADER: &str = include_str!("../src/shaders/point_cloud.wgsl");

#[test]
fn native_point_cloud_shader_parses_and_validates() {
    let module = naga::front::wgsl::parse_str(SHADER).expect("point-cloud WGSL must parse");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .expect("point-cloud WGSL must validate");
}

#[test]
fn shader_uses_fixed_pixel_quads_and_rte_coordinates() {
    assert!(SHADER.contains("half_size_px"));
    assert!(SHADER.contains("u.viewport_size"));
    assert!(SHADER.contains("point.position_low.xyz - u.eye_low"));
    assert!(SHADER.contains("discard"));
}
