// 3D solid modelling support on the App: committing Model-tab primitives,
// and the Design-group boolean operations (truck-shapeops) over the scene's
// session-cached truck B-reps.

use acadrust::entities::Solid3D;
use acadrust::{EntityType, Handle};
use iced::Task;
use truck_modeling::Solid;

use super::Message;
use crate::modules::model::boolean_cmd::BoolOp;
use crate::scene::model::solid_model::{self, Bool};

impl super::OpenCADStudio {
    /// Commit a Model-tab solid: add its acadrust entity to the document, then
    /// register the truck B-rep (caches it for booleans + tessellates it into
    /// the shaded mesh pipeline). Returns the new entity handle.
    pub(super) fn add_solid_model(&mut self, entity: EntityType, solid: Solid) -> Handle {
        let i = self.active_tab;
        let Some(handle) = self.commit_entity_handle(entity) else {
            return Handle::NULL;
        };
        self.tabs[i].scene.register_solid_model(handle, solid);
        handle
    }

    /// Run a boolean (`union` / `subtract` / `intersect`) on exactly two
    /// selected solids whose truck B-reps are in the session cache.
    pub(super) fn solid_boolean(&mut self, op: BoolOp) -> Task<Message> {
        let i = self.active_tab;
        // Selected entities that have a cached truck B-rep.
        let handles: Vec<Handle> = self.tabs[i]
            .scene
            .selected
            .iter()
            .copied()
            .filter(|h| self.tabs[i].scene.solid_models.contains_key(h))
            .collect();
        if handles.len() != 2 {
            self.command_line
                .push_error(crate::t!("Boolean: select exactly two solids created this session.").as_ref());
            return Task::none();
        }
        let a = self.tabs[i].scene.solid_models[&handles[0]].clone();
        let b = self.tabs[i].scene.solid_models[&handles[1]].clone();
        let kind = match op {
            BoolOp::Union => Bool::Union,
            BoolOp::Subtract => Bool::Subtract,
            BoolOp::Intersect => Bool::Intersect,
        };
        let Some(result) = solid_model::boolean(kind, &a, &b) else {
            self.command_line
                .push_error(crate::t!("Boolean failed — the solids may not overlap.").as_ref());
            return Task::none();
        };

        self.push_undo_snapshot(i, "BOOLEAN");
        // Remove the two operands (entity + mesh + cached B-rep).
        self.tabs[i].scene.erase_entities(&handles);
        // The result is freshly combined geometry with no ACIS parametrisation,
        // so it lives as a Solid3D whose render/boolean data is the injected
        // truck mesh + cached B-rep; its edge wires make it pickable.
        let mut s3d = Solid3D::new();
        s3d.wires = solid_model::edge_wires(&result);
        let handle = self.add_solid_model(EntityType::Solid3D(s3d), result);
        self.tabs[i].scene.deselect_all();
        if !handle.is_null() {
            self.tabs[i].scene.select_entity(handle, false);
        }
        self.tabs[i].dirty = true;
        self.refresh_properties();
        Task::none()
    }

    /// Slice the one selected solid with an axis-aligned plane (axis 0/1/2 =
    /// X/Y/Z at `value`), keeping the lower side when `keep_low` is true. The
    /// kept half is the intersection of the solid with a half-space box, reusing
    /// the same truck-shapeops path as the boolean tools.
    pub(super) fn solid_slice(&mut self, axis: usize, value: f64, keep_low: bool) -> Task<Message> {
        let i = self.active_tab;
        let handles: Vec<Handle> = self.tabs[i]
            .scene
            .selected
            .iter()
            .copied()
            .filter(|h| self.tabs[i].scene.solid_models.contains_key(h))
            .collect();
        if handles.len() != 1 {
            self.command_line
                .push_error(crate::t!("SLICE: select exactly one solid created this session.").as_ref());
            return Task::none();
        }
        let solid = self.tabs[i].scene.solid_models[&handles[0]].clone();
        // Bounding box from the solid's edge wires.
        let wires = solid_model::edge_wires(&solid);
        let (mut min, mut max) = ([f64::MAX; 3], [f64::MIN; 3]);
        for w in &wires {
            for p in &w.points {
                let c = [p.x, p.y, p.z];
                for k in 0..3 {
                    min[k] = min[k].min(c[k]);
                    max[k] = max[k].max(c[k]);
                }
            }
        }
        if min[0] > max[0] {
            self.command_line
                .push_error(crate::t!("SLICE: could not determine the solid's extent.").as_ref());
            return Task::none();
        }
        // Generous margin so the box fully spans the solid in the free axes.
        let m = [
            (max[0] - min[0]).max(1.0),
            (max[1] - min[1]).max(1.0),
            (max[2] - min[2]).max(1.0),
        ];
        let mut lo = [min[0] - m[0], min[1] - m[1], min[2] - m[2]];
        let mut hi = [max[0] + m[0], max[1] + m[1], max[2] + m[2]];
        if keep_low {
            hi[axis] = value;
        } else {
            lo[axis] = value;
        }
        if hi[axis] <= lo[axis] {
            self.command_line
                .push_error(crate::t!("SLICE: the plane does not cross the solid on the kept side.").as_ref());
            return Task::none();
        }
        let center = [
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ];
        let halfspace = solid_model::box_solid(center, hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]);
        let Some(result) = solid_model::boolean(Bool::Intersect, &solid, &halfspace) else {
            self.command_line
                .push_error(crate::t!("SLICE failed — the plane may not cross the solid.").as_ref());
            return Task::none();
        };
        self.push_undo_snapshot(i, "SLICE");
        self.tabs[i].scene.erase_entities(&handles);
        let mut s3d = Solid3D::new();
        s3d.wires = solid_model::edge_wires(&result);
        let handle = self.add_solid_model(EntityType::Solid3D(s3d), result);
        self.tabs[i].scene.deselect_all();
        if !handle.is_null() {
            self.tabs[i].scene.select_entity(handle, false);
        }
        self.tabs[i].dirty = true;
        self.refresh_properties();
        let ax = ["X", "Y", "Z"][axis];
        self.command_line.push_output(crate::tf!(
            "SLICE: cut at {ax}={value}, kept the {} half.",
            if keep_low { "lower" } else { "upper" }
        ).as_ref());
        Task::none()
    }

    /// INTERFERE — create a solid from the overlap of the two selected solids,
    /// leaving the originals in place (a non-destructive boolean intersect).
    pub(super) fn solid_interfere(&mut self) -> Task<Message> {
        let i = self.active_tab;
        let handles: Vec<Handle> = self.tabs[i]
            .scene
            .selected
            .iter()
            .copied()
            .filter(|h| self.tabs[i].scene.solid_models.contains_key(h))
            .collect();
        if handles.len() != 2 {
            self.command_line
                .push_error(crate::t!("INTERFERE: select exactly two solids created this session.").as_ref());
            return Task::none();
        }
        let a = self.tabs[i].scene.solid_models[&handles[0]].clone();
        let b = self.tabs[i].scene.solid_models[&handles[1]].clone();
        match solid_model::boolean(Bool::Intersect, &a, &b) {
            Some(result) => {
                self.push_undo_snapshot(i, "INTERFERE");
                // Keep both originals; add the interference solid.
                let mut s3d = Solid3D::new();
                s3d.wires = solid_model::edge_wires(&result);
                self.add_solid_model(EntityType::Solid3D(s3d), result);
                self.tabs[i].dirty = true;
                self.refresh_properties();
                self.command_line
                    .push_output(crate::t!("INTERFERE: created an interference solid from the overlap.").as_ref());
            }
            None => self
                .command_line
                .push_output(crate::t!("INTERFERE: the selected solids do not overlap.").as_ref()),
        }
        Task::none()
    }

    /// 3DROTATE — rotate the one selected solid about the X/Y/Z axis (0/1/2)
    /// through its centre by `angle_deg` degrees. Rotation preserves the solid's
    /// orientation, so it reuses the cached truck B-rep directly.
    pub(super) fn solid_rotate3d(&mut self, axis: usize, angle_deg: f64) -> Task<Message> {
        use truck_modeling::{builder, Point3, Rad, Vector3 as TVec3};
        let i = self.active_tab;
        let handles: Vec<Handle> = self.tabs[i]
            .scene
            .selected
            .iter()
            .copied()
            .filter(|h| self.tabs[i].scene.solid_models.contains_key(h))
            .collect();
        if handles.len() != 1 {
            self.command_line
                .push_error(crate::t!("3DROTATE: select exactly one solid created this session.").as_ref());
            return Task::none();
        }
        let solid = self.tabs[i].scene.solid_models[&handles[0]].clone();
        let wires = solid_model::edge_wires(&solid);
        let (mut min, mut max) = ([f64::MAX; 3], [f64::MIN; 3]);
        for w in &wires {
            for p in &w.points {
                let c = [p.x, p.y, p.z];
                for k in 0..3 {
                    min[k] = min[k].min(c[k]);
                    max[k] = max[k].max(c[k]);
                }
            }
        }
        if min[0] > max[0] {
            self.command_line
                .push_error(crate::t!("3DROTATE: could not determine the solid's extent.").as_ref());
            return Task::none();
        }
        let origin = Point3::new(
            (min[0] + max[0]) / 2.0,
            (min[1] + max[1]) / 2.0,
            (min[2] + max[2]) / 2.0,
        );
        let axis_v = match axis {
            0 => TVec3::unit_x(),
            1 => TVec3::unit_y(),
            _ => TVec3::unit_z(),
        };
        let rotated = builder::rotated(&solid, origin, axis_v, Rad(angle_deg.to_radians()));
        self.push_undo_snapshot(i, "3DROTATE");
        self.tabs[i].scene.erase_entities(&handles);
        let mut s3d = Solid3D::new();
        s3d.wires = solid_model::edge_wires(&rotated);
        let handle = self.add_solid_model(EntityType::Solid3D(s3d), rotated);
        self.tabs[i].scene.deselect_all();
        if !handle.is_null() {
            self.tabs[i].scene.select_entity(handle, false);
        }
        self.tabs[i].dirty = true;
        self.refresh_properties();
        self.command_line.push_output(crate::tf!(
            "3DROTATE: rotated {angle_deg}° about the {} axis.",
            ["X", "Y", "Z"][axis]
        ).as_ref());
        Task::none()
    }

    /// POLYSOLID — build a wall-like solid from a selected polyline: one box per
    /// segment (oriented along it, `width` wide, `height` tall) unioned together,
    /// reusing the box primitive + the boolean union path.
    pub(super) fn solid_polysolid(&mut self, width: f64, height: f64) -> Task<Message> {
        use truck_modeling::{builder, Point3, Rad, Vector3 as TVec3};
        let i = self.active_tab;
        let found: Option<(Handle, Vec<[f64; 2]>, bool)> = self.tabs[i]
            .scene
            .selected_entities()
            .iter()
            .find_map(|(h, e)| match e {
                EntityType::LwPolyline(pl) => Some((
                    *h,
                    pl.vertices
                        .iter()
                        .map(|v| [v.location.x, v.location.y])
                        .collect(),
                    pl.is_closed,
                )),
                _ => None,
            });
        let Some((handle, pts, closed)) = found else {
            self.command_line
                .push_error(crate::t!("POLYSOLID: select a polyline first.").as_ref());
            return Task::none();
        };
        let mut segs: Vec<([f64; 2], [f64; 2])> = pts.windows(2).map(|w| (w[0], w[1])).collect();
        if closed && pts.len() > 2 {
            segs.push((pts[pts.len() - 1], pts[0]));
        }
        let mut acc: Option<Solid> = None;
        let mut used = 0usize;
        for (a, b) in &segs {
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-9 {
                continue;
            }
            let angle = dy.atan2(dx);
            // Local box: X 0..len, Y -w/2..w/2, Z 0..h, then orient to the segment.
            let bx = solid_model::box_solid([len / 2.0, 0.0, height / 2.0], len, width, height);
            let bx = builder::rotated(&bx, Point3::new(0.0, 0.0, 0.0), TVec3::unit_z(), Rad(angle));
            let bx = builder::translated(&bx, TVec3::new(a[0], a[1], 0.0));
            acc = Some(match acc {
                None => bx,
                Some(prev) => solid_model::boolean(Bool::Union, &prev, &bx).unwrap_or(prev),
            });
            used += 1;
        }
        let Some(result) = acc else {
            self.command_line
                .push_error(crate::t!("POLYSOLID: the polyline has no usable segments.").as_ref());
            return Task::none();
        };
        self.push_undo_snapshot(i, "POLYSOLID");
        self.tabs[i].scene.erase_entities(&[handle]);
        let mut s3d = Solid3D::new();
        s3d.wires = solid_model::edge_wires(&result);
        let h = self.add_solid_model(EntityType::Solid3D(s3d), result);
        self.tabs[i].scene.deselect_all();
        if !h.is_null() {
            self.tabs[i].scene.select_entity(h, false);
        }
        self.tabs[i].dirty = true;
        self.refresh_properties();
        self.command_line.push_output(crate::tf!(
            "POLYSOLID: built a wall solid from {used} segment(s) (width {width}, height {height})."
        ).as_ref());
        Task::none()
    }

    /// 3DMIRROR — add a mirrored copy of the one selected solid across the plane
    /// perpendicular to the X/Y/Z axis (0/1/2) through its centre, keeping the
    /// original. Reflection reverses face orientation, so `Solid::not()` restores
    /// outward normals (the same inversion the boolean subtraction relies on).
    pub(super) fn solid_mirror3d(&mut self, axis: usize) -> Task<Message> {
        use truck_modeling::{builder, Matrix4, Vector3 as TVec3};
        let i = self.active_tab;
        let handles: Vec<Handle> = self.tabs[i]
            .scene
            .selected
            .iter()
            .copied()
            .filter(|h| self.tabs[i].scene.solid_models.contains_key(h))
            .collect();
        if handles.len() != 1 {
            self.command_line
                .push_error(crate::t!("3DMIRROR: select exactly one solid created this session.").as_ref());
            return Task::none();
        }
        let solid = self.tabs[i].scene.solid_models[&handles[0]].clone();
        let wires = solid_model::edge_wires(&solid);
        let (mut min, mut max) = ([f64::MAX; 3], [f64::MIN; 3]);
        for w in &wires {
            for p in &w.points {
                let c = [p.x, p.y, p.z];
                for k in 0..3 {
                    min[k] = min[k].min(c[k]);
                    max[k] = max[k].max(c[k]);
                }
            }
        }
        if min[0] > max[0] {
            self.command_line
                .push_error(crate::t!("3DMIRROR: could not determine the solid's extent.").as_ref());
            return Task::none();
        }
        let c = TVec3::new(
            (min[0] + max[0]) / 2.0,
            (min[1] + max[1]) / 2.0,
            (min[2] + max[2]) / 2.0,
        );
        let s = match axis {
            0 => TVec3::new(-1.0, 1.0, 1.0),
            1 => TVec3::new(1.0, -1.0, 1.0),
            _ => TVec3::new(1.0, 1.0, -1.0),
        };
        // Reflect about the plane through the centre: T(c) · scale · T(-c).
        let mat = Matrix4::from_translation(c)
            * Matrix4::from_nonuniform_scale(s.x, s.y, s.z)
            * Matrix4::from_translation(-c);
        let mut reflected = builder::transformed(&solid, mat);
        reflected.not(); // restore outward orientation after the reflection
        self.push_undo_snapshot(i, "3DMIRROR");
        let mut s3d = Solid3D::new();
        s3d.wires = solid_model::edge_wires(&reflected);
        let h = self.add_solid_model(EntityType::Solid3D(s3d), reflected);
        self.tabs[i].scene.deselect_all();
        if !h.is_null() {
            self.tabs[i].scene.select_entity(h, false);
        }
        self.tabs[i].dirty = true;
        self.refresh_properties();
        self.command_line.push_output(crate::tf!(
            "3DMIRROR: added a mirror across the {} plane.",
            ["X", "Y", "Z"][axis]
        ).as_ref());
        Task::none()
    }

    /// 3DALIGN — move/rotate the one selected solid so its three source points
    /// land on the three destination points. The frame-to-frame transform is
    /// computed in glam (`M = D · S⁻¹`) and handed to truck as a raw matrix; both
    /// frames are right-handed so the result is a pure rotation+translation.
    pub(super) fn solid_align3d(
        &mut self,
        src: [[f64; 3]; 3],
        dst: [[f64; 3]; 3],
    ) -> Task<Message> {
        use truck_modeling::{builder, Matrix4};
        let i = self.active_tab;
        let handles: Vec<Handle> = self.tabs[i]
            .scene
            .selected
            .iter()
            .copied()
            .filter(|h| self.tabs[i].scene.solid_models.contains_key(h))
            .collect();
        if handles.len() != 1 {
            self.command_line
                .push_error(crate::t!("3DALIGN: select exactly one solid created this session.").as_ref());
            return Task::none();
        }
        // Build a right-handed frame (origin + orthonormal axes) from 3 points.
        let frame = |p: [[f64; 3]; 3]| -> Option<glam::DMat4> {
            let p1 = glam::DVec3::from_array(p[0]);
            let p2 = glam::DVec3::from_array(p[1]);
            let p3 = glam::DVec3::from_array(p[2]);
            let x = (p2 - p1).normalize_or_zero();
            let z = (p2 - p1).cross(p3 - p1).normalize_or_zero();
            if x.length_squared() < 1e-12 || z.length_squared() < 1e-12 {
                return None; // coincident or collinear points
            }
            let y = z.cross(x);
            Some(glam::DMat4::from_cols(
                x.extend(0.0),
                y.extend(0.0),
                z.extend(0.0),
                p1.extend(1.0),
            ))
        };
        let (Some(s), Some(d)) = (frame(src), frame(dst)) else {
            self.command_line
                .push_error(crate::t!("3DALIGN: each point triple must be non-coincident and non-collinear.").as_ref());
            return Task::none();
        };
        let a = (d * s.inverse()).to_cols_array(); // column-major [f64; 16]
        let mat = Matrix4::new(
            a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7], a[8], a[9], a[10], a[11], a[12], a[13],
            a[14], a[15],
        );
        let solid = self.tabs[i].scene.solid_models[&handles[0]].clone();
        let aligned = builder::transformed(&solid, mat);
        self.push_undo_snapshot(i, "3DALIGN");
        self.tabs[i].scene.erase_entities(&handles);
        let mut s3d = Solid3D::new();
        s3d.wires = solid_model::edge_wires(&aligned);
        let h = self.add_solid_model(EntityType::Solid3D(s3d), aligned);
        self.tabs[i].scene.deselect_all();
        if !h.is_null() {
            self.tabs[i].scene.select_entity(h, false);
        }
        self.tabs[i].dirty = true;
        self.refresh_properties();
        self.command_line
            .push_output(crate::t!("3DALIGN: aligned the solid to the destination points.").as_ref());
        Task::none()
    }

    /// SECTION — draw the cross-section outline where an axis-aligned plane
    /// (X/Y/Z = `axis` at `value`) cuts the one selected solid, as Line entities.
    /// Reuses the mesh-interference analyzer to find the cut segments.
    #[cfg(feature = "solid3d")]
    pub(super) fn solid_section(&mut self, axis: usize, value: f64) -> Task<Message> {
        use acadrust::types::Vector3;
        use acadrust::Line;
        use truck_meshalgo::analyzers::Collision;
        use truck_meshalgo::tessellation::{MeshableShape, MeshedShape};
        use truck_modeling::{builder, Point3, Shell, Wire};

        let i = self.active_tab;
        let handles: Vec<Handle> = self.tabs[i]
            .scene
            .selected
            .iter()
            .copied()
            .filter(|h| self.tabs[i].scene.solid_models.contains_key(h))
            .collect();
        if handles.len() != 1 {
            self.command_line
                .push_error(crate::t!("SECTION: select exactly one solid created this session.").as_ref());
            return Task::none();
        }
        let solid = self.tabs[i].scene.solid_models[&handles[0]].clone();
        let wires = solid_model::edge_wires(&solid);
        let (mut min, mut max) = ([f64::MAX; 3], [f64::MIN; 3]);
        for w in &wires {
            for p in &w.points {
                let c = [p.x, p.y, p.z];
                for k in 0..3 {
                    min[k] = min[k].min(c[k]);
                    max[k] = max[k].max(c[k]);
                }
            }
        }
        if min[0] > max[0] {
            self.command_line
                .push_error(crate::t!("SECTION: could not determine the solid's extent.").as_ref());
            return Task::none();
        }
        // Margin so the cutting plane fully spans the solid in the free axes.
        let m = [
            (max[0] - min[0]).max(1.0) * 0.1,
            (max[1] - min[1]).max(1.0) * 0.1,
            (max[2] - min[2]).max(1.0) * 0.1,
        ];
        let lo = [min[0] - m[0], min[1] - m[1], min[2] - m[2]];
        let hi = [max[0] + m[0], max[1] + m[1], max[2] + m[2]];
        let corners: [Point3; 4] = match axis {
            0 => [
                Point3::new(value, lo[1], lo[2]),
                Point3::new(value, hi[1], lo[2]),
                Point3::new(value, hi[1], hi[2]),
                Point3::new(value, lo[1], hi[2]),
            ],
            1 => [
                Point3::new(lo[0], value, lo[2]),
                Point3::new(hi[0], value, lo[2]),
                Point3::new(hi[0], value, hi[2]),
                Point3::new(lo[0], value, hi[2]),
            ],
            _ => [
                Point3::new(lo[0], lo[1], value),
                Point3::new(hi[0], lo[1], value),
                Point3::new(hi[0], hi[1], value),
                Point3::new(lo[0], hi[1], value),
            ],
        };
        let v: Vec<_> = corners.iter().map(|p| builder::vertex(*p)).collect();
        let wire: Wire = vec![
            builder::line(&v[0], &v[1]),
            builder::line(&v[1], &v[2]),
            builder::line(&v[2], &v[3]),
            builder::line(&v[3], &v[0]),
        ]
        .into_iter()
        .collect();
        let Ok(face) = builder::try_attach_plane(&[wire]) else {
            self.command_line
                .push_error(crate::t!("SECTION: could not build the cutting plane.").as_ref());
            return Task::none();
        };
        let shell: Shell = std::iter::once(face).collect();
        let tol = 0.02;
        let solid_mesh = solid.triangulation(tol).to_polygon();
        let plane_mesh = shell.triangulation(tol).to_polygon();
        let segs = solid_mesh.extract_interference(&plane_mesh);
        if segs.is_empty() {
            self.command_line
                .push_output(crate::t!("SECTION: the plane does not cross the solid.").as_ref());
            return Task::none();
        }
        self.push_undo_snapshot(i, "SECTION");
        for (p1, p2) in &segs {
            let line = Line::from_points(
                Vector3::new(p1.x, p1.y, p1.z),
                Vector3::new(p2.x, p2.y, p2.z),
            );
            self.tabs[i].scene.add_entity(EntityType::Line(line));
        }
        self.tabs[i].dirty = true;
        self.refresh_properties();
        self.command_line.push_output(crate::tf!(
            "SECTION: created {} section line(s) at {}={value}.",
            segs.len(),
            ["X", "Y", "Z"][axis]
        ).as_ref());
        Task::none()
    }

    /// Without `solid3d` (e.g. wasm) there is no mesh-interference kernel.
    #[cfg(not(feature = "solid3d"))]
    pub(super) fn solid_section(&mut self, _axis: usize, _value: f64) -> Task<Message> {
        self.command_line
            .push_error(crate::t!("SECTION: solid modelling is unavailable in this build.").as_ref());
        Task::none()
    }

    /// PYRAMID — create an `n`-sided pyramid (regular polygon base of the given
    /// circumradius, apex at `height`) as a tessellated mesh, reusing the same
    /// face→Shell→mesh path as LOFT. Each face is independent, so no closed-shell
    /// topology is required; the windings give outward normals.
    pub(super) fn solid_pyramid(
        &mut self,
        radius: f64,
        height: f64,
        sides: usize,
    ) -> Task<Message> {
        use crate::modules::insert::solid3d_cmds::empty_solid3d;
        use crate::scene::convert::truck_tess;
        use crate::scene::model::mesh_model::MeshModel;
        use truck_modeling::{builder, Point3, Shell, Wire};

        let i = self.active_tab;
        let n = sides.max(3);
        if radius <= 0.0 || height <= 0.0 {
            self.command_line
                .push_error(crate::t!("PYRAMID: radius and height must be positive.").as_ref());
            return Task::none();
        }
        let corners: Vec<Point3> = (0..n)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / n as f64;
                Point3::new(radius * a.cos(), radius * a.sin(), 0.0)
            })
            .collect();
        let apex = builder::vertex(Point3::new(0.0, 0.0, height));
        let mut faces = Vec::new();
        // Base: reversed winding so the normal points down (outward at the base).
        {
            let bv: Vec<_> = corners.iter().rev().map(|p| builder::vertex(*p)).collect();
            let wire: Wire = (0..n)
                .map(|k| builder::line(&bv[k], &bv[(k + 1) % n]))
                .collect();
            if let Ok(f) = builder::try_attach_plane(&[wire]) {
                faces.push(f);
            }
        }
        // Side triangles [corner k, corner k+1, apex] → outward normals.
        let sv: Vec<_> = corners.iter().map(|p| builder::vertex(*p)).collect();
        for k in 0..n {
            let k1 = (k + 1) % n;
            let wire: Wire = vec![
                builder::line(&sv[k], &sv[k1]),
                builder::line(&sv[k1], &apex),
                builder::line(&apex, &sv[k]),
            ]
            .into_iter()
            .collect();
            if let Ok(f) = builder::try_attach_plane(&[wire]) {
                faces.push(f);
            }
        }
        if faces.len() < n + 1 {
            self.command_line
                .push_error(crate::t!("PYRAMID: could not build all faces.").as_ref());
            return Task::none();
        }
        let shell = Shell::from(faces);
        let color = self.tabs[i].scene.layer_color(&self.tabs[i].active_layer);
        let truck_tess::TruckTessResult::Mesh {
            verts,
            verts_low,
            normals,
            indices,
        } = truck_tess::tessellate_shell(&shell)
        else {
            self.command_line
                .push_error(crate::t!("PYRAMID: tessellation failed.").as_ref());
            return Task::none();
        };
        if verts.is_empty() {
            self.command_line
                .push_error(crate::t!("PYRAMID: tessellation produced no geometry.").as_ref());
            return Task::none();
        }
        self.push_undo_snapshot(i, "PYRAMID");
        let new_handle = self.tabs[i].scene.add_entity(empty_solid3d());
        let mesh = MeshModel {
            name: format!("{}", new_handle.value()),
            verts,
            verts_low,
            normals,
            indices,
            triangle_material_handles: Vec::new(),
            triangle_colors: Vec::new(),
            color,
            selected: false,
        };
        self.tabs[i]
            .scene
            .meshes
            .insert(new_handle, crate::scene::MeshLodSet::from_single(mesh));
        self.tabs[i].dirty = true;
        self.refresh_properties();
        self.command_line.push_output(crate::tf!(
            "PYRAMID: created a {n}-sided pyramid (radius {radius}, height {height})."
        ).as_ref());
        Task::none()
    }

    /// SPLINEFIT — replace the selected polyline with a cubic spline that passes
    /// through its vertices. Control points come from the Catmull-Rom → cubic
    /// Bézier formula (the curve provably interpolates each vertex), with a
    /// clamped piecewise-Bézier knot vector the spline renderer reads directly.
    pub(super) fn fit_spline(&mut self) -> Task<Message> {
        use acadrust::entities::Spline;
        use acadrust::types::Vector3;

        let i = self.active_tab;
        let found: Option<(Handle, Vec<[f64; 3]>)> = self.tabs[i]
            .scene
            .selected_entities()
            .iter()
            .find_map(|(h, e)| match e {
                EntityType::LwPolyline(pl) => Some((
                    *h,
                    pl.vertices
                        .iter()
                        .map(|v| [v.location.x, v.location.y, 0.0])
                        .collect(),
                )),
                _ => None,
            });
        let Some((handle, fit)) = found else {
            self.command_line
                .push_error(crate::t!("SPLINEFIT: select a polyline to fit a spline through.").as_ref());
            return Task::none();
        };
        if fit.len() < 3 {
            self.command_line
                .push_error(crate::t!("SPLINEFIT: need at least 3 points.").as_ref());
            return Task::none();
        }
        let n = fit.len();
        let m = n - 1; // Bézier segments
        let p = |k: usize| glam::DVec3::new(fit[k][0], fit[k][1], fit[k][2]);
        // Catmull-Rom → cubic Bézier control points: [P0, b1,b2,P1, b1,b2,P2, …].
        let mut ctrl: Vec<Vector3> = Vec::with_capacity(3 * m + 1);
        ctrl.push(Vector3::new(fit[0][0], fit[0][1], 0.0));
        for seg in 0..m {
            let p0 = p(seg);
            let p1 = p(seg + 1);
            let prev = if seg > 0 { p(seg - 1) } else { p0 };
            let next = if seg + 2 <= m { p(seg + 2) } else { p1 };
            let b1 = p0 + (p1 - prev) / 6.0;
            let b2 = p1 - (next - p0) / 6.0;
            ctrl.push(Vector3::new(b1.x, b1.y, 0.0));
            ctrl.push(Vector3::new(b2.x, b2.y, 0.0));
            ctrl.push(Vector3::new(p1.x, p1.y, 0.0));
        }
        // Clamped piecewise-Bézier knots (degree 3): len == ctrl.len()+degree+1.
        let mut knots: Vec<f64> = vec![0.0; 4];
        for s in 1..m {
            knots.extend_from_slice(&[s as f64, s as f64, s as f64]);
        }
        knots.extend_from_slice(&[m as f64; 4]);
        let mut spl = Spline::new();
        spl.degree = 3;
        spl.control_points = ctrl;
        spl.knots = knots;
        spl.fit_points = fit.iter().map(|q| Vector3::new(q[0], q[1], 0.0)).collect();
        // flags.rational defaults to false (non-rational) — exactly what we want.
        self.push_undo_snapshot(i, "SPLINEFIT");
        self.tabs[i].scene.erase_entities(&[handle]);
        self.tabs[i].scene.add_entity(EntityType::Spline(spl));
        self.tabs[i].dirty = true;
        self.refresh_properties();
        self.command_line
            .push_output(crate::tf!("SPLINEFIT: fit a spline through {n} points.").as_ref());
        Task::none()
    }

    /// FLATSHOT — project the selected solid's edges onto the XY plane (Z=0) as
    /// Line entities, giving a flattened 2D shot of the model. Reuses the cached
    /// solid's edge wires (the same source SECTION uses).
    pub(super) fn solid_flatshot(&mut self) -> Task<Message> {
        use acadrust::types::Vector3;
        use acadrust::Line;
        let i = self.active_tab;
        let handles: Vec<Handle> = self.tabs[i]
            .scene
            .selected
            .iter()
            .copied()
            .filter(|h| self.tabs[i].scene.solid_models.contains_key(h))
            .collect();
        if handles.is_empty() {
            self.command_line
                .push_error(crate::t!("FLATSHOT: select a solid created this session.").as_ref());
            return Task::none();
        }
        self.push_undo_snapshot(i, "FLATSHOT");
        let mut n = 0usize;
        for h in &handles {
            let solid = self.tabs[i].scene.solid_models[h].clone();
            for w in solid_model::edge_wires(&solid) {
                for seg in w.points.windows(2) {
                    let line = Line::from_points(
                        Vector3::new(seg[0].x, seg[0].y, 0.0),
                        Vector3::new(seg[1].x, seg[1].y, 0.0),
                    );
                    self.tabs[i].scene.add_entity(EntityType::Line(line));
                    n += 1;
                }
            }
        }
        self.tabs[i].dirty = true;
        self.refresh_properties();
        self.command_line
            .push_output(crate::tf!("FLATSHOT: created {n} projected edge(s) at Z=0.").as_ref());
        Task::none()
    }

    /// CONVTOSURFACE — convert the selected solid(s) into Surface entities,
    /// carrying the solid's edge wires (reuses the cached B-rep's edge wires).
    pub(super) fn solid_convtosurface(&mut self) -> Task<Message> {
        use acadrust::entities::{Surface, SurfaceKind, Wire as AWire};
        use acadrust::types::Vector3;
        let i = self.active_tab;
        let handles: Vec<Handle> = self.tabs[i]
            .scene
            .selected
            .iter()
            .copied()
            .filter(|h| self.tabs[i].scene.solid_models.contains_key(h))
            .collect();
        if handles.is_empty() {
            self.command_line
                .push_error(crate::t!("CONVTOSURFACE: select a solid created this session.").as_ref());
            return Task::none();
        }
        let mut surfaces: Vec<Surface> = Vec::new();
        for h in &handles {
            let solid = self.tabs[i].scene.solid_models[h].clone();
            let awires: Vec<AWire> = solid_model::edge_wires(&solid)
                .into_iter()
                .map(|w| {
                    let mut aw = AWire::new();
                    aw.points = w
                        .points
                        .iter()
                        .map(|p| Vector3::new(p.x, p.y, p.z))
                        .collect();
                    aw
                })
                .collect();
            let mut surf = Surface::new(SurfaceKind::Generic);
            surf.wires = awires;
            surf.common.layer = self.tabs[i].active_layer.clone();
            surfaces.push(surf);
        }
        self.push_undo_snapshot(i, "CONVTOSURFACE");
        self.tabs[i].scene.erase_entities(&handles);
        let n = surfaces.len();
        for surf in surfaces {
            self.tabs[i].scene.add_entity(EntityType::Surface(surf));
        }
        self.tabs[i].dirty = true;
        self.refresh_properties();
        self.command_line
            .push_output(crate::tf!("CONVTOSURFACE: converted {n} solid(s) to surface(s).").as_ref());
        Task::none()
    }
}
