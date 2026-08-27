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

/// Colorization must come from the style uniform so color mode, class
/// visibility and class-table edits never rebuild the instance buffer.
#[test]
fn shader_colorizes_from_style_uniform() {
    assert!(SHADER.contains("@group(1) @binding(0) var<uniform> style: Style"));
    assert!(SHADER.contains("style.class_visible"));
    assert!(SHADER.contains("style.class_colors"));
    assert!(SHADER.contains("style.color_mode"));
    // Hidden classes collapse to a zero-size quad instead of being filtered
    // out of the point set on the CPU.
    assert!(SHADER.contains("class_is_visible(scheme_class)"));
}

/// The UPCP label mode colors and filters through the same class tables as
/// ASPRS classification, driven by the label byte packed into the free
/// attribute slot of the instance layout.
#[test]
fn shader_labels_color_through_class_tables() {
    assert!(SHADER.contains("let label = u32(point.attributes.w);"));
    assert!(SHADER.contains("style.color_mode == 6u"));
    assert!(SHADER.contains("class_color(label)"));
    // Visibility switches to the label scheme in label mode.
    assert!(SHADER.contains("select(classification, label, style.color_mode == 6u)"));
}

/// The vertical cross-section must be a shader-side band test (style uniform),
/// not a CPU point filter, so moving/rotating the section is one uniform write.
#[test]
fn shader_sections_clip_from_style_uniform() {
    assert!(SHADER.contains("section_outside"));
    assert!(SHADER.contains("style.section_p0"));
    assert!(SHADER.contains("style.section_params"));
    assert!(SHADER.contains("let half = 0.5 * style.section_params.x;"));
    assert!(!SHADER.contains("style.section_params.x * u.world_per_pixel"));
}
