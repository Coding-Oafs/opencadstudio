use super::*;

use cadkernel::geom2d::{bounded_faces, Line, Tolerance};

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

impl Scene {
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
}
