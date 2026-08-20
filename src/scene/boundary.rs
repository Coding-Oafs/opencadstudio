use super::*;

use cadkernel::geom2d::{
    bounded_faces, contains, distance_to, intersect, segment_crossing, triangulate, Curve, Line,
    SegmentCrossing, Tolerance, Transform as CurveTransform,
};

use crate::command::WorkingPlane;

#[derive(Clone)]
pub struct BoundarySource {
    pub segments: Vec<Line>,
    pub curve: Option<Curve>,
}

/// How far apart two points may be and still be taken for the same one.
///
/// The boundary search runs on already-tessellated wire geometry, so the
/// input is a chord approximation of the drawn curves to begin with; this
/// only has to be coarse enough to close the gaps that leaves and fine
/// enough not to weld genuinely separate corners together.
const WELD_TOLERANCE: f64 = 1.0e-6;

fn wire_segments(wire: &WireModel) -> Vec<Line> {
    let mut segments = Vec::new();
    let mut previous: Option<[f64; 2]> = None;

    for (index, high) in wire.points.iter().copied().enumerate() {
        if !high[0].is_finite() || !high[1].is_finite() {
            previous = None;
            continue;
        }
        let low = wire.points_low.get(index).copied().unwrap_or([0.0; 3]);
        let current = [
            high[0] as f64 + low[0] as f64,
            high[1] as f64 + low[1] as f64,
        ];
        if let Some(start) = previous {
            let (dx, dy) = (current[0] - start[0], current[1] - start[1]);
            if dx.hypot(dy) > WELD_TOLERANCE {
                segments.push(Line { start, end: current });
            }
        }
        previous = Some(current);
    }
    segments
}

fn wire_segments_on_plane(
    wire: &WireModel,
    plane: WorkingPlane,
    tolerance: f64,
) -> Vec<Line> {
    let mut segments = Vec::new();
    let mut previous: Option<glam::DVec3> = None;
    for (index, high) in wire.points.iter().copied().enumerate() {
        if !high.iter().all(|value| value.is_finite()) {
            previous = None;
            continue;
        }
        let low = wire.points_low.get(index).copied().unwrap_or([0.0; 3]);
        let world = glam::DVec3::new(
            high[0] as f64 + low[0] as f64,
            high[1] as f64 + low[1] as f64,
            high[2] as f64 + low[2] as f64,
        );
        let current = plane.to_local(world);
        if current.z.abs() > tolerance {
            previous = None;
            continue;
        }
        if let Some(start) = previous {
            let length = (current.x - start.x).hypot(current.y - start.y);
            if length > tolerance {
                segments.push(Line {
                    start: [start.x, start.y],
                    end: [current.x, current.y],
                });
            }
        }
        previous = Some(current);
    }
    segments
}

fn entity_curve_on_plane(
    entity: &EntityType,
    plane: WorkingPlane,
    tolerance: f64,
) -> Option<Curve> {
    let planar = crate::entities::curve::entity_curve(entity)?;
    let source = planar.plane;
    let origin = plane.to_local(glam::DVec3::from_array(source.origin));
    let x_axis = plane.vector_to_local(glam::DVec3::from_array(source.x_axis));
    let y_axis = plane.vector_to_local(glam::DVec3::from_array(source.y_axis));
    if origin.z.abs() > tolerance || x_axis.z.abs() > 1.0e-9 || y_axis.z.abs() > 1.0e-9 {
        return None;
    }
    planar.curve.transformed(&CurveTransform {
        x_axis: [x_axis.x, x_axis.y].into(),
        y_axis: [y_axis.x, y_axis.y].into(),
        origin: [origin.x, origin.y].into(),
    })
}

fn ring_seed(model: &HatchModel, wanted: usize) -> Option<[f64; 2]> {
    let mut ring = Vec::new();
    let mut index = 0usize;
    for &[x, y] in model.boundary.iter() {
        if x.is_finite() && y.is_finite() {
            if index == wanted {
                ring.push([
                    model.world_origin[0] + x as f64,
                    model.world_origin[1] + y as f64,
                ]);
            }
        } else if index == wanted {
            break;
        } else {
            index += 1;
        }
    }
    if ring.len() < 3 {
        return None;
    }
    let (points, triangles) = triangulate(&ring, &[]);
    if let Some(triangle) = triangles.first() {
        let [a, b, c] = triangle.map(|vertex| points[vertex]);
        return Some([
            (a[0] + b[0] + c[0]) / 3.0,
            (a[1] + b[1] + c[1]) / 3.0,
        ]);
    }
    Some([
        ring.iter().map(|point| point[0]).sum::<f64>() / ring.len() as f64,
        ring.iter().map(|point| point[1]).sum::<f64>() / ring.len() as f64,
    ])
}

fn face_curves(face: &[[f64; 2]]) -> Vec<Curve> {
    face.iter()
        .copied()
        .zip(face.iter().copied().cycle().skip(1))
        .take(face.len())
        .map(|(start, end)| Curve::Line(Line { start, end }))
        .collect()
}

fn matching_face(faces: &[Vec<[f64; 2]>], seed: Option<[f64; 2]>) -> Option<&Vec<[f64; 2]>> {
    if faces.len() == 1 {
        return faces.first();
    }
    let seed = seed?;
    let tolerance = Tolerance::new(WELD_TOLERANCE);
    if let Some(face) = faces.iter().find(|face| {
        let curves = face_curves(face);
        contains(&curves, seed, tolerance)
    }) {
        return Some(face);
    }
    faces.iter().min_by(|a, b| {
        let nearest = |face: &Vec<[f64; 2]>| {
            face_curves(face)
                .iter()
                .map(|curve| distance_to(curve, seed))
                .fold(f64::INFINITY, f64::min)
        };
        nearest(a).total_cmp(&nearest(b))
    })
}

pub(crate) fn ring_source_handles(
    ring: &[[f64; 2]],
    sources: &rustc_hash::FxHashMap<acadrust::Handle, Vec<Line>>,
) -> Vec<acadrust::Handle> {
    let mut handles = rustc_hash::FxHashSet::default();
    for (&start, &end) in ring
        .iter()
        .zip(ring.iter().cycle().skip(1))
        .take(ring.len())
    {
        let edge = Line { start, end };
        let edge_length = (end[0] - start[0]).hypot(end[1] - start[1]);
        for (&handle, lines) in sources {
            if lines.iter().any(|line| {
                matches!(
                    segment_crossing(edge, *line, Tolerance::new(WELD_TOLERANCE)),
                    SegmentCrossing::Overlap { a, .. }
                        if (a[1] - a[0]).abs() * edge_length > WELD_TOLERANCE
                )
            }) {
                handles.insert(handle);
            }
        }
    }
    let mut handles: Vec<_> = handles.into_iter().collect();
    handles.sort_by_key(|handle| handle.value());
    handles
}

pub(crate) fn boundary_entities(rings: &[Vec<[f64; 2]>]) -> Vec<acadrust::EntityType> {
    rings
        .iter()
        .filter_map(|ring| {
            let mut points = Vec::with_capacity(ring.len());
            for &point in ring {
                if point.iter().all(|value| value.is_finite())
                    && points.last() != Some(&point)
                {
                    points.push(point);
                }
            }
            if points.len() > 1 && points.first() == points.last() {
                points.pop();
            }
            if points.len() < 3 {
                return None;
            }
            let mut polyline = acadrust::entities::LwPolyline::new();
            polyline.is_closed = true;
            polyline.vertices = points
                .into_iter()
                .map(|[x, y]| {
                    acadrust::entities::LwVertex::new(acadrust::types::Vector2::new(x, y))
                })
                .collect();
            Some(acadrust::EntityType::LwPolyline(polyline))
        })
        .collect()
}

fn edge_source(
    edge: Line,
    sources: &rustc_hash::FxHashMap<Handle, BoundarySource>,
    tolerance: f64,
) -> Option<Handle> {
    let edge_length = (edge.end[0] - edge.start[0]).hypot(edge.end[1] - edge.start[1]);
    let mut best = None;
    let mut best_overlap = 0.0;
    for (&handle, source) in sources {
        for segment in &source.segments {
            if let SegmentCrossing::Overlap { a, .. } =
                segment_crossing(edge, *segment, Tolerance::new(tolerance))
            {
                let overlap = (a[1] - a[0]).abs() * edge_length;
                if overlap > best_overlap {
                    best = Some(handle);
                    best_overlap = overlap;
                }
            }
        }
    }
    if best.is_some() {
        return best;
    }
    let midpoint = [
        (edge.start[0] + edge.end[0]) * 0.5,
        (edge.start[1] + edge.end[1]) * 0.5,
    ];
    sources
        .iter()
        .filter_map(|(&handle, source)| {
            let curve = source.curve.as_ref()?;
            Some((handle, distance_to(curve, midpoint)))
        })
        .filter(|(_, distance)| *distance <= tolerance * 4.0)
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(handle, _)| handle)
}

fn nearest_crossing(a: &Curve, b: &Curve, point: [f64; 2], tolerance: f64) -> Option<[f64; 2]> {
    intersect(a, b, Tolerance::new(tolerance))
        .into_iter()
        .min_by(|left, right| {
            let distance = |candidate: [f64; 2]| {
                (candidate[0] - point[0]).hypot(candidate[1] - point[1])
            };
            distance(left.point).total_cmp(&distance(right.point))
        })
        .map(|crossing| crossing.point)
}

fn project_to_curve(curve: &Curve, point: [f64; 2]) -> [f64; 2] {
    curve.point_at(curve.parameter_at(point))
}

fn refined_boundary_ring(
    ring: &[[f64; 2]],
    sources: &rustc_hash::FxHashMap<Handle, BoundarySource>,
    tolerance: f64,
) -> (Vec<[f64; 2]>, Vec<Option<Handle>>) {
    let count = ring.len();
    if count < 3 {
        return (ring.to_vec(), Vec::new());
    }
    let edge_sources: Vec<_> = (0..count)
        .map(|index| {
            edge_source(
                Line {
                    start: ring[index],
                    end: ring[(index + 1) % count],
                },
                sources,
                tolerance,
            )
        })
        .collect();
    let points = (0..count)
        .map(|index| {
            let point = ring[index];
            let incoming = edge_sources[(index + count - 1) % count]
                .and_then(|handle| sources.get(&handle)?.curve.as_ref());
            let outgoing = edge_sources[index]
                .and_then(|handle| sources.get(&handle)?.curve.as_ref());
            match (incoming, outgoing) {
                (Some(a), Some(b)) if !std::ptr::eq(a, b) =>
                    nearest_crossing(a, b, point, tolerance).unwrap_or_else(|| project_to_curve(b, point)),
                (Some(curve), _) | (_, Some(curve)) => project_to_curve(curve, point),
                _ => point,
            }
        })
        .collect();
    (points, edge_sources)
}

fn normalized_delta(from: f64, to: f64) -> f64 {
    (to - from + std::f64::consts::PI)
        .rem_euclid(std::f64::consts::TAU)
        - std::f64::consts::PI
}

fn curve_edge_bulge(curve: &Curve, start: [f64; 2], end: [f64; 2]) -> f64 {
    let direct = match curve {
        Curve::Circle(circle) => Some((circle.centre, circle.radius)),
        Curve::Arc(arc) => Some((arc.centre, arc.radius)),
        _ => None,
    };
    if let Some((centre, radius)) = direct {
        if radius.abs() <= 1.0e-12 {
            return 0.0;
        }
        let start_angle = (start[1] - centre[1]).atan2(start[0] - centre[0]);
        let end_angle = (end[1] - centre[1]).atan2(end[0] - centre[0]);
        return (normalized_delta(start_angle, end_angle) * 0.25).tan();
    }
    if matches!(curve, Curve::Line(_)) {
        return 0.0;
    }
    let chord = [end[0] - start[0], end[1] - start[1]];
    let squared = chord[0] * chord[0] + chord[1] * chord[1];
    if squared <= 1.0e-20 {
        return 0.0;
    }
    let midpoint = [(start[0] + end[0]) * 0.5, (start[1] + end[1]) * 0.5];
    let on_curve = project_to_curve(curve, midpoint);
    let relative = [on_curve[0] - start[0], on_curve[1] - start[1]];
    let bulge = 2.0 * (chord[0] * relative[1] - chord[1] * relative[0]) / squared;
    if bulge.is_finite() { bulge } else { 0.0 }
}

fn boundary_polyline(
    ring: &[[f64; 2]],
    plane: WorkingPlane,
    sources: &rustc_hash::FxHashMap<Handle, BoundarySource>,
    tolerance: f64,
) -> Option<EntityType> {
    let (points, edge_sources) = refined_boundary_ring(ring, sources, tolerance);
    if points.len() < 3 {
        return None;
    }
    let mut polyline = acadrust::entities::LwPolyline::new();
    polyline.is_closed = true;
    polyline.vertices = points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let mut vertex = acadrust::entities::LwVertex::new(acadrust::types::Vector2::new(
                point[0], point[1],
            ));
            vertex.bulge = edge_sources
                .get(index)
                .and_then(|handle| handle.and_then(|value| sources.get(&value)))
                .and_then(|source| source.curve.as_ref())
                .map(|curve| curve_edge_bulge(curve, *point, points[(index + 1) % points.len()]))
                .unwrap_or(0.0);
            vertex
        })
        .collect();
    Some(plane.place_entity(EntityType::LwPolyline(polyline)))
}

pub(crate) fn boundary_polyline_entities(
    regions: &[Vec<Vec<[f64; 2]>>],
    plane: WorkingPlane,
    sources: &rustc_hash::FxHashMap<Handle, BoundarySource>,
    tolerance: f64,
) -> Vec<EntityType> {
    regions
        .iter()
        .flat_map(|region| region.iter())
        .filter_map(|ring| boundary_polyline(ring, plane, sources, tolerance))
        .collect()
}

pub(crate) fn boundary_region_entities(
    regions: &[Vec<Vec<[f64; 2]>>],
    plane: WorkingPlane,
    sources: &rustc_hash::FxHashMap<Handle, BoundarySource>,
    tolerance: f64,
) -> Vec<EntityType> {
    regions
        .iter()
        .filter_map(|region| {
            let mut wires = Vec::new();
            for ring in region {
                let (points, _) = refined_boundary_ring(ring, sources, tolerance);
                if points.len() < 3 {
                    continue;
                }
                let mut wire = acadrust::entities::Wire::new();
                wire.points = points
                    .into_iter()
                    .map(|[x, y]| {
                        let world = plane.to_world(glam::DVec3::new(x, y, 0.0));
                        acadrust::types::Vector3::new(world.x, world.y, world.z)
                    })
                    .collect();
                wires.push(wire);
            }
            let first = wires.first()?.points.first().copied()?;
            let mut region = acadrust::entities::Region::new();
            region.point_of_reference = first;
            region.wires = wires;
            Some(EntityType::Region(region))
        })
        .collect()
}

impl Scene {
    fn associative_boundary_segments(&self, handles: &[Handle]) -> Vec<Line> {
        self.wire_models_for(handles)
            .iter()
            .flat_map(wire_segments)
            .collect()
    }

    fn associative_hatch_dependents(
        &self,
        changed: &rustc_hash::FxHashSet<Handle>,
    ) -> Vec<Handle> {
        if self.associative_hatch_source_cache.borrow().is_none() {
            let mut index: HashMap<Handle, Vec<Handle>> = HashMap::default();
            for entity in self.document.entities() {
                let EntityType::Hatch(hatch) = entity else {
                    continue;
                };
                if !hatch.is_associative {
                    continue;
                }
                for source in hatch
                    .paths
                    .iter()
                    .flat_map(|path| path.boundary_handles.iter().copied())
                {
                    let dependents = index.entry(source).or_default();
                    if !dependents.contains(&hatch.common.handle) {
                        dependents.push(hatch.common.handle);
                    }
                }
            }
            *self.associative_hatch_source_cache.borrow_mut() = Some(index);
        }
        let cache = self.associative_hatch_source_cache.borrow();
        let index = cache.as_ref().expect("associative hatch index");
        let mut handles = rustc_hash::FxHashSet::default();
        for source in changed {
            if let Some(dependents) = index.get(source) {
                handles.extend(dependents.iter().copied());
            }
        }
        handles.into_iter().collect()
    }

    pub(crate) fn refresh_associative_hatches(
        &mut self,
        changes: &[(Handle, ChangeKind)],
    ) -> Vec<(Handle, ChangeKind)> {
        if changes.is_empty() {
            return Vec::new();
        }
        let changed: rustc_hash::FxHashSet<_> =
            changes.iter().map(|(handle, _)| *handle).collect();
        let candidates: Vec<_> = self
            .associative_hatch_dependents(&changed)
            .into_iter()
            .filter_map(|handle| {
                let EntityType::Hatch(hatch) = self.document.get_entity(handle)? else {
                    return None;
                };
                Some({
                    let seeds = self
                        .hatches
                        .get(&hatch.common.handle)
                        .map(|model| {
                            (0..hatch.paths.len())
                                .map(|index| ring_seed(model, index))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    (handle, hatch.clone(), seeds)
                })
            })
            .collect();

        let mut refreshed = Vec::new();
        for (handle, mut hatch, seeds) in candidates {
            let normal = hatch.normal;
            if normal.x.abs() > 1.0e-8 || normal.y.abs() > 1.0e-8 {
                continue;
            }
            let mut modified = false;
            let mut association_changed = false;
            for (index, path) in hatch.paths.iter_mut().enumerate() {
                if !path
                    .boundary_handles
                    .iter()
                    .any(|source| changed.contains(source))
                {
                    continue;
                }
                let old_count = path.boundary_handles.len();
                path.boundary_handles
                    .retain(|source| self.document.get_entity(*source).is_some());
                association_changed |= path.boundary_handles.len() != old_count;
                modified |= association_changed;
                let segments = self.associative_boundary_segments(&path.boundary_handles);
                let faces = bounded_faces(&segments, Tolerance::new(WELD_TOLERANCE));
                let Some(face) = matching_face(&faces, seeds.get(index).copied().flatten()) else {
                    continue;
                };
                path.edges = vec![acadrust::entities::hatch::BoundaryEdge::Polyline(
                    acadrust::entities::hatch::PolylineEdge::new(
                        face.iter()
                            .map(|point| acadrust::types::Vector2::new(point[0], point[1]))
                            .collect(),
                        true,
                    ),
                )];
                modified = true;
            }
            hatch.is_associative = hatch
                .paths
                .iter()
                .any(|path| !path.boundary_handles.is_empty());
            if !modified {
                continue;
            }
            if association_changed {
                self.associative_hatch_source_cache.borrow_mut().take();
            }
            if self.is_recording_undo() {
                let before = self.document.get_entity_arc(handle);
                self.record_undo_before(handle, before);
            }
            if let Some(slot) = self.document.get_entity_mut(handle) {
                *slot = EntityType::Hatch(hatch);
                self.refresh_fill_model(handle);
                refreshed.push((handle, ChangeKind::Modified));
            }
        }
        refreshed
    }

    /// Build closed planar regions from the visible wire geometry.
    ///
    /// Unlike `closed_outlines()`, the source entities do not need to be closed
    /// individually. Intersections are inserted as temporary graph vertices and
    /// the bounded faces of that planar graph are returned as hatch candidates.
    ///
    /// Curved entities participate through their already-tessellated WireModel
    /// geometry, so arcs, circles, ellipses and splines can take part in the
    /// boundary search without modifying the source entities.
    ///
    /// The arrangement itself is the kernel's: splitting at crossings, welding
    /// coincident ends and tracing the bounded faces is the same problem a
    /// B-rep boolean solves in a face's parameter space, and it is solved
    /// once. What stays here is reading the wires — which is where the
    /// drawing's own conventions live.
    pub fn hatch_boundary_outlines(&self) -> Vec<Vec<[f64; 2]>> {
        let mut segments = Vec::<Line>::new();

        for wire in self.entity_wires().iter() {
            segments.extend(wire_segments(wire));
        }

        bounded_faces(&segments, Tolerance::new(WELD_TOLERANCE))
    }

    /// Tessellated boundary segments grouped by their selectable entity.
    pub fn hatch_boundary_sources(
        &self,
    ) -> rustc_hash::FxHashMap<acadrust::Handle, Vec<Line>> {
        let mut sources = rustc_hash::FxHashMap::default();
        for wire in self.entity_wires().iter() {
            let Some(handle) = Self::handle_from_wire_name(&wire.name) else {
                continue;
            };
            sources
                .entry(handle)
                .or_insert_with(Vec::new)
                .extend(wire_segments(wire));
        }
        sources
    }

    /// Boundary candidates expressed in the active working plane. Rendered
    /// segments cover transformed/block geometry; direct curve entities also
    /// retain their exact kernel curve so output polylines can restore arc
    /// bulges instead of saving the display tessellation as straight chords.
    pub fn boundary_sources_on_plane(
        &self,
        plane: WorkingPlane,
        tolerance: f64,
    ) -> rustc_hash::FxHashMap<Handle, BoundarySource> {
        let mut sources: rustc_hash::FxHashMap<Handle, BoundarySource> =
            rustc_hash::FxHashMap::default();
        for wire in self.entity_wires().iter() {
            let Some(handle) = Self::handle_from_wire_name(&wire.name) else {
                continue;
            };
            let segments = wire_segments_on_plane(wire, plane, tolerance);
            if segments.is_empty() {
                continue;
            }
            sources
                .entry(handle)
                .or_insert_with(|| BoundarySource {
                    segments: Vec::new(),
                    curve: None,
                })
                .segments
                .extend(segments);
        }
        for (&handle, source) in &mut sources {
            source.curve = self
                .document
                .get_entity(handle)
                .and_then(|entity| entity_curve_on_plane(entity, plane, tolerance));
        }
        sources
    }
}

pub(crate) fn boundary_faces(
    sources: &rustc_hash::FxHashMap<Handle, BoundarySource>,
    tolerance: f64,
) -> Vec<Vec<[f64; 2]>> {
    let segments: Vec<_> = sources
        .values()
        .flat_map(|source| source.segments.iter().copied())
        .collect();
    bounded_faces(&segments, Tolerance::new(tolerance))
}
