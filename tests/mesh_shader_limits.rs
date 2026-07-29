use naga::{Binding, ShaderStage, TypeInner};

const MESH_SHADER: &str = include_str!("../src/shaders/mesh.wgsl");
const WEBGL_INTER_STAGE_COMPONENT_LIMIT: u32 = 31;

fn validate(source: &str) -> naga::Module {
    let module = naga::front::wgsl::parse_str(source).expect("mesh WGSL must parse");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .expect("mesh WGSL must validate");
    module
}

fn location_components(module: &naga::Module, type_name: &str) -> u32 {
    let ty = module
        .types
        .iter()
        .find_map(|(_, ty)| (ty.name.as_deref() == Some(type_name)).then_some(ty))
        .unwrap_or_else(|| panic!("{type_name} missing"));
    let TypeInner::Struct { members, .. } = &ty.inner else {
        panic!("{type_name} must remain a struct");
    };
    members
        .iter()
        .filter(|member| matches!(member.binding, Some(Binding::Location { .. })))
        .map(|member| match module.types[member.ty].inner {
            TypeInner::Scalar(_) => 1,
            TypeInner::Vector { size, .. } => u32::from(size),
            ref other => panic!("unsupported inter-stage type: {other:?}"),
        })
        .sum()
}

fn assert_edge_contract(source: &str) {
    let module = validate(source);
    assert!(module
        .entry_points
        .iter()
        .any(|entry| entry.stage == ShaderStage::Vertex && entry.name == "vs_edge"));
    assert!(module
        .entry_points
        .iter()
        .any(|entry| entry.stage == ShaderStage::Fragment && entry.name == "fs_edge"));

    let components = location_components(&module, "EdgeVertexOut");
    assert_eq!(components, 4, "edge output should carry only RGBA color");
    assert!(
        components <= WEBGL_INTER_STAGE_COMPONENT_LIMIT,
        "edge output exceeds WebGL2's inter-stage component budget"
    );
}

#[test]
fn native_mesh_edge_shader_stays_webgl_compatible() {
    assert_edge_contract(MESH_SHADER);
}

#[test]
fn storage_free_mesh_edge_shader_stays_webgl_compatible() {
    let compat = MESH_SHADER.replace(
        "var<storage, read> mesh_instances: array<MeshInstance>;",
        "var<uniform> mesh_instances: array<MeshInstance, 1>;",
    );
    assert_edge_contract(&compat);
}
