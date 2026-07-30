use iced::wgpu;

/// Renderer-relevant limits of the selected GPU device.
///
/// Capabilities, not the compilation target, choose the renderer tier. Browser
/// WebGPU and native Vulkan/Metal/DX12 can therefore share storage-backed
/// pipelines, while WebGL2 and weak native adapters use compatibility paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceCapabilities {
    pub max_storage_buffers_per_shader_stage: u32,
    pub max_inter_stage_shader_components: u32,
    pub max_vertex_attributes: u32,
}

impl DeviceCapabilities {
    /// Indexed wires read one per-wire constants storage buffer.
    const WIRE_STORAGE_BINDINGS: u32 = 1;

    /// Batched hatch uses five storage bindings in one shader stage:
    /// instances, boundaries, families, dashes, and visibility.
    const HATCH_STORAGE_BINDINGS: u32 = 5;

    /// Mesh compute culling reads one item buffer and writes four indirect
    /// command buffers.
    const MESH_CULL_BINDINGS: u32 = 5;

    pub fn detect(device: &wgpu::Device) -> Self {
        Self::from_limits(&device.limits())
    }

    fn from_limits(limits: &wgpu::Limits) -> Self {
        Self {
            max_storage_buffers_per_shader_stage: limits.max_storage_buffers_per_shader_stage,
            max_inter_stage_shader_components: limits.max_inter_stage_shader_variables,
            max_vertex_attributes: limits.max_vertex_attributes,
        }
    }

    /// Mesh instancing needs one read-only storage buffer.
    pub fn supports_mesh_storage_instancing(self) -> bool {
        self.max_storage_buffers_per_shader_stage >= 1
    }

    pub fn supports_wire_storage(self) -> bool {
        self.max_storage_buffers_per_shader_stage >= Self::WIRE_STORAGE_BINDINGS
    }

    pub fn supports_batched_hatch(self) -> bool {
        self.max_storage_buffers_per_shader_stage >= Self::HATCH_STORAGE_BINDINGS
    }

    /// WebGL2 reports zero storage bindings and stays on CPU mesh culling.
    pub fn supports_mesh_compute_culling(self) -> bool {
        self.max_storage_buffers_per_shader_stage >= Self::MESH_CULL_BINDINGS
    }
}

#[cfg(test)]
mod tests {
    use super::DeviceCapabilities;
    use iced::wgpu;

    #[test]
    fn webgl_limits_select_compatibility_paths() {
        let caps = DeviceCapabilities::from_limits(&wgpu::Limits::downlevel_webgl2_defaults());
        assert!(!caps.supports_mesh_storage_instancing());
        assert!(!caps.supports_wire_storage());
        assert!(!caps.supports_batched_hatch());
        assert!(!caps.supports_mesh_compute_culling());
    }

    #[test]
    fn default_limits_select_storage_paths() {
        let caps = DeviceCapabilities::from_limits(&wgpu::Limits::default());
        assert!(caps.supports_mesh_storage_instancing());
        assert!(caps.supports_wire_storage());
        assert!(caps.supports_batched_hatch());
        assert!(caps.supports_mesh_compute_culling());
    }

    #[test]
    fn storage_paths_are_selected_independently() {
        let caps = DeviceCapabilities {
            max_storage_buffers_per_shader_stage: 1,
            max_inter_stage_shader_components: 31,
            max_vertex_attributes: 16,
        };
        assert!(caps.supports_wire_storage());
        assert!(caps.supports_mesh_storage_instancing());
        assert!(!caps.supports_batched_hatch());
        assert!(!caps.supports_mesh_compute_culling());
    }
}
