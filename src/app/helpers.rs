use acadrust::tables::Ucs;

// ── Coordinate parsing ─────────────────────────────────────────────────────

/// How a typed coordinate should be interpreted relative to the last
/// input point.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CoordKind {
    /// `@x,y` prefix — offset from the last input point.
    Relative,
    /// `#x,y` prefix — world/UCS absolute, overriding DYN.
    Absolute,
    /// No prefix — the caller decides (DYN on → relative, off → absolute).
    Default,
}

/// Parse a typed coordinate string into a local Vec3 plus its interpretation.
/// Accepts Cartesian, polar, cylindrical and spherical forms:
/// `x,y`, `x,y,z`, `distance<angle`, `distance<angle,z`, and
/// `distance<angle<elevation`.
/// A leading `@` marks the value relative to the last point; a leading
/// `#` forces absolute. Separators: comma or semicolon.
pub(crate) fn parse_coord(text: &str) -> Option<(glam::DVec3, CoordKind)> {
    let trimmed = text.trim();
    let (kind, rest) = if let Some(r) = trimmed.strip_prefix('@') {
        (CoordKind::Relative, r)
    } else if let Some(r) = trimmed.strip_prefix('#') {
        (CoordKind::Absolute, r)
    } else {
        (CoordKind::Default, trimmed)
    };
    let number = |value: &str| value.trim().parse::<f64>().ok();
    if let Some((distance, angles)) = rest.split_once('<') {
        let distance = number(distance)?;
        if let Some((azimuth, elevation)) = angles.split_once('<') {
            let azimuth = number(azimuth)?.to_radians();
            let elevation = number(elevation)?.to_radians();
            let horizontal = distance * elevation.cos();
            return Some((
                glam::DVec3::new(
                    horizontal * azimuth.cos(),
                    horizontal * azimuth.sin(),
                    distance * elevation.sin(),
                ),
                kind,
            ));
        }
        let angle_and_z: Vec<&str> = angles.split(|c| c == ',' || c == ';').collect();
        let (angle, z) = match angle_and_z.as_slice() {
            [angle] => (number(angle)?, 0.0),
            [angle, z] => (number(angle)?, number(z)?),
            _ => return None,
        };
        let angle = angle.to_radians();
        return Some((
            glam::DVec3::new(distance * angle.cos(), distance * angle.sin(), z),
            kind,
        ));
    }
    let parts: Vec<f64> = rest
        .split(|c| c == ',' || c == ';')
        .map(|s| s.trim())
        .filter_map(|s| s.parse().ok())
        .collect();
    match parts.as_slice() {
        [x, y] => Some((glam::DVec3::new(*x, *y, 0.0), kind)),
        [x, y, z] => Some((glam::DVec3::new(*x, *y, *z), kind)),
        _ => None,
    }
}

#[cfg(test)]
mod coordinate_parsing_tests {
    use super::*;

    #[test]
    fn parses_all_coordinate_forms() {
        let close = |a: glam::DVec3, b: glam::DVec3| (a - b).length() < 1e-9;
        assert!(close(parse_coord("1,2").unwrap().0, glam::dvec3(1.0, 2.0, 0.0)));
        assert!(close(parse_coord("1,2,3").unwrap().0, glam::dvec3(1.0, 2.0, 3.0)));
        assert!(close(parse_coord("10<90").unwrap().0, glam::dvec3(0.0, 10.0, 0.0)));
        assert!(close(parse_coord("10<90,4").unwrap().0, glam::dvec3(0.0, 10.0, 4.0)));
        assert!(close(parse_coord("10<0<30").unwrap().0, glam::dvec3(5.0 * 3.0_f64.sqrt(), 0.0, 5.0)));
        assert_eq!(parse_coord("@10<0").unwrap().1, CoordKind::Relative);
        assert_eq!(parse_coord("#1,2").unwrap().1, CoordKind::Absolute);
    }
}

// ── UCS ↔ WCS converter ─────────────────────────────────────────────────────

/// The single bridge between WCS (how geometry and the file are stored) and the
/// active UCS (the coordinate system the user works in). Build one from the
/// tab's active UCS via [`DocumentTab::ucs_xform`](super::DocumentTab); the
/// `None` UCS yields the identity (plain WCS).
///
/// Every system that has to speak UCS — the coordinate readout, typed input,
/// the UCS icon, snap/ortho, the ViewCube — goes through this one type instead
/// of re-deriving the axis math. Axes are orthonormal, so the inverse rotation
/// is just the transpose (the dot products in `to_ucs`); no matrix inversion.
#[derive(Clone, Copy)]
pub(super) struct UcsXform {
    origin: glam::DVec3,
    x: glam::DVec3,
    y: glam::DVec3,
    z: glam::DVec3,
}

impl UcsXform {
    /// Plain WCS — no active UCS.
    pub(super) fn identity() -> Self {
        Self {
            origin: glam::DVec3::ZERO,
            x: glam::DVec3::X,
            y: glam::DVec3::Y,
            z: glam::DVec3::Z,
        }
    }

    pub(super) fn from_ucs(ucs: &Ucs) -> Self {
        let v = |a: acadrust::types::Vector3| glam::DVec3::new(a.x, a.y, a.z);
        let x = v(ucs.x_axis).normalize_or(glam::DVec3::X);
        let raw_y = v(ucs.y_axis).normalize_or(glam::DVec3::Y);
        let fallback_z = if x.dot(glam::DVec3::Z).abs() < 0.999 {
            glam::DVec3::Z
        } else {
            glam::DVec3::Y
        };
        let z = x.cross(raw_y).normalize_or(x.cross(fallback_z).normalize());
        let y = z.cross(x).normalize();
        Self { origin: v(ucs.origin), x, y, z }
    }

    pub(super) fn from_active(ucs: Option<&Ucs>) -> Self {
        ucs.map(Self::from_ucs).unwrap_or_else(Self::identity)
    }

    /// True when this is plain WCS — lets callers skip the conversion.
    pub(super) fn is_identity(&self) -> bool {
        self.origin == glam::DVec3::ZERO
            && self.x == glam::DVec3::X
            && self.y == glam::DVec3::Y
            && self.z == glam::DVec3::Z
    }

    /// UCS point → WCS.
    pub(super) fn to_wcs(&self, p: glam::DVec3) -> glam::DVec3 {
        self.origin + self.x * p.x + self.y * p.y + self.z * p.z
    }

    /// WCS point → UCS.
    pub(super) fn to_ucs(&self, p: glam::DVec3) -> glam::DVec3 {
        let d = p - self.origin;
        glam::DVec3::new(d.dot(self.x), d.dot(self.y), d.dot(self.z))
    }

    /// UCS direction → WCS (rotation only, no origin shift).
    pub(super) fn vec_to_wcs(&self, v: glam::DVec3) -> glam::DVec3 {
        self.x * v.x + self.y * v.y + self.z * v.z
    }

    /// WCS direction → UCS (rotation only, no origin shift).
    pub(super) fn vec_to_ucs(&self, v: glam::DVec3) -> glam::DVec3 {
        glam::DVec3::new(v.dot(self.x), v.dot(self.y), v.dot(self.z))
    }

    /// `(origin, x, y, z)` axes in WCS — for drawing the UCS icon.
    pub(super) fn axes(&self) -> (glam::DVec3, glam::DVec3, glam::DVec3, glam::DVec3) {
        (self.origin, self.x, self.y, self.z)
    }

    pub(super) fn working_plane(&self) -> crate::command::WorkingPlane {
        crate::command::WorkingPlane::new(self.origin, self.x, self.y)
    }

    /// UCS→world rotation matrix (columns = UCS axes). For consumers that take
    /// a `Mat4` rotation directly (ViewCube, OTRACK ray directions). The GPU /
    /// screen layer is f32, so the axes downcast here at the boundary.
    pub(super) fn rotation_mat(&self) -> glam::Mat4 {
        glam::Mat4::from_cols(
            self.x.as_vec3().extend(0.0),
            self.y.as_vec3().extend(0.0),
            self.z.as_vec3().extend(0.0),
            glam::Vec4::W,
        )
    }

    /// Full UCS-local → WCS transform, using `origin` as local zero while
    /// retaining this UCS's orthonormal axes.
    pub(super) fn to_wcs_transform_at(
        &self,
        origin: glam::DVec3,
    ) -> acadrust::types::Transform {
        use acadrust::types::{Matrix4, Transform};
        Transform::from_matrix(Matrix4 {
            m: [
                [self.x.x, self.y.x, self.z.x, origin.x],
                [self.x.y, self.y.y, self.z.y, origin.y],
                [self.x.z, self.y.z, self.z.z, origin.z],
                [0.0, 0.0, 0.0, 1.0],
            ],
        })
    }

    /// Full WCS → UCS-local transform, using `origin` as the local zero.
    pub(super) fn to_ucs_transform_at(
        &self,
        origin: glam::DVec3,
    ) -> acadrust::types::Transform {
        use acadrust::types::{Matrix4, Transform};
        Transform::from_matrix(Matrix4 {
            m: [
                [self.x.x, self.x.y, self.x.z, -origin.dot(self.x)],
                [self.y.x, self.y.y, self.y.z, -origin.dot(self.y)],
                [self.z.x, self.z.y, self.z.z, -origin.dot(self.z)],
                [0.0, 0.0, 0.0, 1.0],
            ],
        })
    }

    /// Convert from the represented UCS into its canonical local frame.
    pub(super) fn to_ucs_transform(&self) -> acadrust::types::Transform {
        self.to_ucs_transform_at(self.origin)
    }
}

// ── UCS ↔ WCS transforms (thin wrappers over `UcsXform`) ────────────────────

/// Rotate a UCS-local offset into WCS without applying the origin
/// translation — used for relative coordinate entry, where only the
/// axis orientation matters, not the UCS origin.
pub(super) fn ucs_rotate_vec(offset: glam::DVec3, ucs: &Ucs) -> glam::DVec3 {
    UcsXform::from_ucs(ucs).vec_to_wcs(offset)
}

/// Convert a point from UCS local coordinates to WCS.
pub(super) fn ucs_to_wcs(pt: glam::DVec3, ucs: &Ucs) -> glam::DVec3 {
    UcsXform::from_ucs(ucs).to_wcs(pt)
}

/// Return the normalised Z axis of a UCS (cross product of X and Y axes).
pub(super) fn ucs_z_axis(ucs: &Ucs) -> glam::DVec3 {
    UcsXform::from_ucs(ucs).axes().3
}

/// Build a UCS with `origin` and axes rotated by `angle_z_rad` around the Z axis.
pub(super) fn ucs_rotated_z(origin: glam::DVec3, angle_z: f32) -> Ucs {
    let cos = angle_z.cos() as f64;
    let sin = angle_z.sin() as f64;
    let mut ucs = Ucs::new("*ACTIVE*");
    ucs.origin = acadrust::types::Vector3::new(origin.x, origin.y, origin.z);
    ucs.x_axis = acadrust::types::Vector3::new(cos, sin, 0.0);
    ucs.y_axis = acadrust::types::Vector3::new(-sin, cos, 0.0);
    ucs
}

// ── Drawing constraint helpers ─────────────────────────────────────────────

/// Constrain `pt` to the nearest 90° direction from `base`, in the active UCS
/// plane — ortho follows the user's coordinate system, not world axes. `xf` is
/// identity for plain WCS, so the world-XY behaviour is unchanged there.
pub(super) fn ortho_constrain(pt: glam::DVec3, base: glam::DVec3, xf: &UcsXform) -> glam::DVec3 {
    let p = xf.to_ucs(pt);
    let b = xf.to_ucs(base);
    let dx = (p.x - b.x).abs();
    let dy = (p.y - b.y).abs();
    let c = if dx >= dy {
        glam::DVec3::new(p.x, b.y, p.z)
    } else {
        glam::DVec3::new(b.x, p.y, p.z)
    };
    xf.to_wcs(c)
}

/// Constrain `pt` to the nearest polar angle multiple from `base`, measured in
/// the active UCS plane (identity `xf` = world XY, Z-up).
pub(super) fn polar_constrain(
    pt: glam::DVec3,
    base: glam::DVec3,
    step_deg: f32,
    xf: &UcsXform,
) -> glam::DVec3 {
    let p = xf.to_ucs(pt);
    let b = xf.to_ucs(base);
    let dx = p.x - b.x;
    let dy = p.y - b.y;
    let dist = (dx * dx + dy * dy).sqrt();
    if dist < 1e-6 {
        return pt;
    }
    let step = (step_deg as f64).to_radians();
    let angle = dy.atan2(dx);
    let snapped = (angle / step).round() * step;
    xf.to_wcs(glam::DVec3::new(
        b.x + dist * snapped.cos(),
        b.y + dist * snapped.sin(),
        p.z,
    ))
}

/// Polar constraint that only engages when the cursor is within `tol_px`
/// screen pixels of the nearest polar ray; otherwise the cursor is left free
/// so POLAR behaves as if off when pointing away from every angle (issue #70).
pub(super) fn polar_constrain_near(
    pt: glam::DVec3,
    base: glam::DVec3,
    step_deg: f32,
    view_rot: glam::Mat4,
    eye: glam::DVec3,
    bounds: iced::Rectangle,
    tol_px: f32,
    xf: &UcsXform,
) -> glam::DVec3 {
    let snapped = polar_constrain(pt, base, step_deg, xf);
    let to_screen = |w: glam::DVec3| {
        let ndc = view_rot.project_point3((w - eye).as_vec3());
        (
            (ndc.x + 1.0) * 0.5 * bounds.width,
            (1.0 - ndc.y) * 0.5 * bounds.height,
        )
    };
    let (cx, cy) = to_screen(pt);
    let (sx, sy) = to_screen(snapped);
    if ((cx - sx).powi(2) + (cy - sy).powi(2)).sqrt() <= tol_px {
        snapped
    } else {
        pt
    }
}

/// Hard axis lock (#312): the locked ray's direction — the nearest polar
/// increment (or ortho axis) from `base` toward `cursor`, in the active UCS
/// plane. `None` when the cursor sits on the base point.
pub(super) fn axis_lock_capture(
    cursor: glam::DVec3,
    base: glam::DVec3,
    polar: bool,
    step_deg: f32,
    xf: &UcsXform,
) -> Option<glam::DVec3> {
    let p = xf.to_ucs(cursor);
    let b = xf.to_ucs(base);
    let dx = p.x - b.x;
    let dy = p.y - b.y;
    if dx.hypot(dy) < 1e-9 {
        return None;
    }
    let step = (if polar { step_deg as f64 } else { 90.0 }).to_radians();
    let ang = (dy.atan2(dx) / step).round() * step;
    let dir_ucs = glam::DVec3::new(ang.cos(), ang.sin(), 0.0);
    let dir = xf.to_wcs(b + dir_ucs) - xf.to_wcs(b);
    (dir.length_squared() > 1e-12).then(|| dir.normalize())
}

/// Project `pt` onto the locked ray through `base` — the hard lock applies to
/// EVERYTHING, including an osnap hit, so a snap far off-axis contributes only
/// its along-axis component (#312).
pub(super) fn axis_lock_apply(
    pt: glam::DVec3,
    base: glam::DVec3,
    dir: glam::DVec3,
) -> glam::DVec3 {
    base + dir * (pt - base).dot(dir)
}

// ── Clipboard / selection helpers ──────────────────────────────────────────

/// Copy/paste anchor: the lower-left corner of the bounding box that encloses
/// every copied entity. Standard clipboard behaviour puts this corner under the
/// cursor at paste time, so the whole selection drops down-and-right of the pick.
///
/// Unions each entity's own bounding box (O(entities)) rather than averaging
/// every tessellated wire vertex, which cost O(total geometry) and stalled a
/// whole-drawing copy. The per-entity `min` corners give the exact enclosing
/// box's lower-left.
pub(super) fn entities_lower_left_by_bbox(
    doc: &acadrust::CadDocument,
    handles: &[acadrust::Handle],
) -> glam::DVec3 {
    let mut min = glam::DVec3::splat(f64::INFINITY);
    let mut any = false;
    for &h in handles {
        let Some(e) = doc.get_entity(h) else { continue };
        let bb = e.as_entity().bounding_box();
        let lo = glam::DVec3::new(bb.min.x, bb.min.y, bb.min.z);
        if lo.x.is_finite() && lo.y.is_finite() && lo.z.is_finite() {
            min = min.min(lo);
            any = true;
        }
    }
    if any {
        min
    } else {
        glam::DVec3::ZERO
    }
}

/// Generate the next available auto group name ("*A1", "*A2", …).
pub(super) fn next_group_auto_name(scene: &crate::scene::Scene) -> String {
    let existing: rustc_hash::FxHashSet<String> =
        scene.groups().map(|g| g.name.clone()).collect();
    for n in 1..=9999 {
        let name = format!("*A{n}");
        if !existing.contains(&name) {
            return name;
        }
    }
    "*A".to_string()
}

// ── Entity type labels ─────────────────────────────────────────────────────

pub(super) fn entity_type_label(entity: &acadrust::EntityType) -> String {
    crate::t!(crate::entities::names::ui_name_or_class(entity)).into_owned()
}

pub(super) fn entity_type_key(entity: &acadrust::EntityType) -> String {
    use acadrust::EntityType::*;
    match entity {
        Point(_) => "point",
        Line(_) => "line",
        Circle(_) => "circle",
        Arc(_) => "arc",
        Ellipse(_) => "ellipse",
        Spline(_) => "spline",
        Helix(_) => "helix",
        LwPolyline(_) | Polyline(_) => "pline",
        Polyline2D(_) => "pline2d",
        Polyline3D(_) => "pline3d",
        PolyfaceMesh(_) => "polyface",
        PolygonMesh(_) => "polymesh",
        Text(_) => "text",
        MText(_) => "mtext",
        Dimension(_) => "dimension",
        Leader(_) => "leader",
        MultiLeader(_) => "multileader",
        Tolerance(_) => "tolerance",
        Insert(_) => "insert",
        Block(_) => "block",
        BlockEnd(_) => "blockend",
        Hatch(_) => "hatch",
        Solid(_) => "solid",
        Face3D(_) => "face3d",
        Solid3D(_) => "solid3d",
        Region(_) => "region",
        Body(_) => "body",
        Surface(_) => "surface",
        Mesh(_) => "mesh",
        Ray(_) => "ray",
        XLine(_) => "xline",
        MLine(_) => "mline",
        Viewport(_) => "viewport",
        RasterImage(_) => "rasterimage",
        Wipeout(_) => "wipeout",
        Underlay(_) => "underlay",
        Shape(_) => "shape",
        Table(_) => "table",
        AttributeDefinition(_) => "attdef",
        AttributeEntity(_) => "attrib",
        Ole2Frame(_) => "ole2frame",
        Light(_) => "light",
        SectionSymbol(_) => "sectionsymbol",
        ViewBorder(_) => "viewborder",
        Extended(entity) => return entity.class_name().to_ascii_lowercase(),
        Seqend(_) => "seqend",
        Unknown(_) => "unknown",
    }
    .to_string()
}

pub(super) fn title_case_word(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => {
            let mut out = first.to_uppercase().collect::<String>();
            out.push_str(chars.as_str());
            out
        }
        None => String::new(),
    }
}

// ── Window icon ────────────────────────────────────────────────────────────

/// Builds a 32×32 RGBA icon: red background with OCS drawn in white pixels.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn build_window_icon() -> Vec<u8> {
    const W: usize = 32;
    const SZ: usize = W * W * 4;

    let bg = [176u8, 48, 32, 255];
    let fg = [255u8, 255, 255, 255];

    let mut px = vec![0u8; SZ];
    for i in 0..W * W {
        px[i * 4..i * 4 + 4].copy_from_slice(&bg);
    }

    fn stroke(px: &mut Vec<u8>, ax: i32, ay: i32, bx: i32, by: i32, fg: [u8; 4]) {
        let steps = ((bx - ax).abs().max((by - ay).abs()) * 3).max(1);
        for s in 0..=steps {
            let t = s as f32 / steps as f32;
            let cx = ax as f32 + (bx - ax) as f32 * t;
            let cy = ay as f32 + (by - ay) as f32 * t;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let ix = cx.round() as i32 + dx;
                    let iy = cy.round() as i32 + dy;
                    if ix >= 0 && ix < W as i32 && iy >= 0 && iy < W as i32 {
                        let idx = (iy as usize * W + ix as usize) * 4;
                        px[idx..idx + 4].copy_from_slice(&fg);
                    }
                }
            }
        }
    }

    // O
    stroke(&mut px, 3, 6, 9, 6, fg);
    stroke(&mut px, 3, 25, 9, 25, fg);
    stroke(&mut px, 3, 6, 3, 25, fg);
    stroke(&mut px, 9, 6, 9, 25, fg);
    // C
    stroke(&mut px, 12, 6, 18, 6, fg);
    stroke(&mut px, 12, 25, 18, 25, fg);
    stroke(&mut px, 12, 6, 12, 25, fg);
    // S
    stroke(&mut px, 21, 6, 27, 6, fg);
    stroke(&mut px, 21, 6, 21, 15, fg);
    stroke(&mut px, 21, 15, 27, 15, fg);
    stroke(&mut px, 27, 15, 27, 25, fg);
    stroke(&mut px, 21, 25, 27, 25, fg);

    px
}
