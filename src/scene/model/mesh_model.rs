// Triangle mesh model — produced by truck Shell/Solid tessellation.
//
// Stored alongside WireModels in the scene; rendered by the mesh pipeline
// (wgpu TriangleList with depth test, flat normals).

/// A tessellated triangle mesh ready to upload to the GPU.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct MeshModel {
    /// Unique identifier (entity handle value as decimal string).
    pub name: String,
    /// World-space vertex positions (high half of the double-single pair).
    pub verts: Vec<[f32; 3]>,
    /// Low residual paired with `verts` so meshes stay precise at UTM scale.
    /// Empty = all-zero (legacy / interactive meshes near the origin).
    pub verts_low: Vec<[f32; 3]>,
    /// Per-vertex normals (may be empty if not available).
    pub normals: Vec<[f32; 3]>,
    /// Triangle indices into `verts` (every 3 values = one triangle).
    pub indices: Vec<u32>,
    /// RGBA colour in [0, 1].
    pub color: [f32; 4],
    /// Whether this mesh is currently selected.
    pub selected: bool,
}

/// Bundle of mesh tessellations at different sampling densities, picked
/// per frame by the render pipeline based on the projected pixel size of
/// `world_aabb`. Phase 3.4 LOD ladder:
///
/// | LOD | Source     | Use when projected diagonal |
/// |-----|------------|------------------------------|
/// | 0   | HIGH       | > 200 px                     |
/// | 1   | MID (½)    | 50–200 px                    |
/// | 2   | LOW (¼)    | < 50 px                      |
///
/// `lods` holds up to one MeshModel per LOD level (high → low). Empty
/// slots fall back to the nearest available LOD at render time.
/// A curved face's generator, kept so a view-dependent silhouette (DISPSILH)
/// can be computed per frame — the silhouette is where the surface turns away
/// from the eye, which no baked edge can capture. World-space, post body
/// transform; base/centre points carry a double-single low half so they stay
/// precise at UTM scale like the mesh verts. Each variant also carries the
/// face's parametric extent so the silhouette is clipped to the actual face
/// rather than drawn across the whole (possibly partial) surface.
#[derive(Clone, Copy, Debug)]
pub enum CurvedGen {
    /// Cone / cylinder: two edge-on lines up the surface.
    Cone {
        base: [f32; 3],
        base_low: [f32; 3],
        axis: [f32; 3],
        /// Radial frame: `u` is the θ=0 direction, `v = axis × u`.
        u_dir: [f32; 3],
        v_dir: [f32; 3],
        /// Radius at the base (`h = 0`).
        radius: f32,
        /// `tan(half-angle)`: radius at height `h` is `radius + h * tan_a`.
        tan_a: f32,
        /// Height span along the axis the face covers (base is `h = 0`).
        h_max: f32,
        theta_min: f32,
        theta_span: f32,
        full: bool,
    },
    /// Sphere: the great circle perpendicular to the view, clipped to the
    /// face's longitude/colatitude window.
    Sphere {
        center: [f32; 3],
        center_low: [f32; 3],
        pole: [f32; 3],
        u_dir: [f32; 3],
        v_dir: [f32; 3],
        radius: f32,
        theta_min: f32,
        theta_span: f32,
        full: bool,
        phi_min: f32,
        phi_max: f32,
    },
    /// Torus: the two extreme circles of the tube (outer/inner), clipped to the
    /// revolution window the face covers.
    Torus {
        center: [f32; 3],
        center_low: [f32; 3],
        axis: [f32; 3],
        u_dir: [f32; 3],
        v_dir: [f32; 3],
        major: f32,
        minor: f32,
        phi_min: f32,
        phi_span: f32,
        full: bool,
    },
}

#[derive(Clone, Debug)]
pub struct MeshLodSet {
    pub lods: Vec<MeshModel>,
    /// True only when every source face produced triangles. False keeps
    /// downstream solid-edit code from treating a display-only partial shell
    /// as a closed, valid solid.
    pub complete: bool,
    /// Feature-edge line list (LOD-independent): pairs of endpoints, high half
    /// of the double-single. Populated for ACIS solids (the B-rep face-boundary
    /// edges) so their wireframe shows real edges rather than the triangulation.
    /// Empty for plain meshes — those fall back to triangle edges at batch time.
    pub edge_verts: Vec<[f32; 3]>,
    /// Low residual paired with `edge_verts`.
    pub edge_verts_low: Vec<[f32; 3]>,
    /// Curved-face generators for per-frame silhouette (DISPSILH). Empty for a
    /// solid with no curved faces, or when silhouettes aren't wanted.
    pub curved_gens: Vec<CurvedGen>,
    /// World XY AABB `[min_x, min_y, max_x, max_y]` of the mesh — used
    /// by the per-frame LOD selector to compute the projected pixel
    /// diagonal.
    pub world_aabb: [f32; 4],
    /// World Z extent `[min_z, max_z]`. With `world_aabb` this is the full 3D
    /// box, which the pick path projects to a screen rect to skip solids whose
    /// footprint isn't under the cursor (O(solids) instead of ray-testing every
    /// triangle). `verts` carry only the high half of the double-single
    /// position, so the bound is f32-precise — fine for a conservative cull.
    pub z_aabb: [f32; 2],
}

/// 3D bounds of every LOD's vertices: `([min_x, min_y, max_x, max_y], [min_z, max_z])`.
pub fn compute_mesh_aabb(lods: &[MeshModel]) -> ([f32; 4], [f32; 2]) {
    let (mut min_x, mut min_y, mut min_z) = (f32::INFINITY, f32::INFINITY, f32::INFINITY);
    let (mut max_x, mut max_y, mut max_z) =
        (f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for lod in lods {
        for &[x, y, z] in &lod.verts {
            if !x.is_finite() || !y.is_finite() || !z.is_finite() {
                continue;
            }
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            min_z = min_z.min(z);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
            max_z = max_z.max(z);
        }
    }
    ([min_x, min_y, max_x, max_y], [min_z, max_z])
}

impl MeshLodSet {
    /// Build a set from its LODs, computing the 3D AABB.
    pub fn from_lods(lods: Vec<MeshModel>) -> Self {
        let (world_aabb, z_aabb) = compute_mesh_aabb(&lods);
        Self {
            lods,
            complete: true,
            edge_verts: Vec::new(),
            edge_verts_low: Vec::new(),
            curved_gens: Vec::new(),
            world_aabb,
            z_aabb,
        }
    }

    /// Wrap a single MeshModel as a one-LOD set. Used by interactive
    /// commands that only produce one tessellation (e.g. truck-based
    /// BOX/CYLINDER creation). The LOD selector will pick slot 0 for
    /// every zoom level.
    pub fn from_single(mesh: MeshModel) -> Self {
        Self::from_lods(vec![mesh])
    }

    /// Recompute `world_aabb` / `z_aabb` after the LODs' vertices were rewritten
    /// (relative-to-eye re-split, INSERT transform).
    pub fn recompute_aabb(&mut self) {
        let (xy, z) = compute_mesh_aabb(&self.lods);
        self.world_aabb = xy;
        self.z_aabb = z;
    }
}
