use super::*;

use cadkernel::geom2d::{bounded_faces, Line, Tolerance};

/// How far apart two points may be and still be taken for the same one.
///
/// The boundary search runs on already-tessellated wire geometry, so the
/// input is a chord approximation of the drawn curves to begin with; this
/// only has to be coarse enough to close the gaps that leaves and fine
/// enough not to weld genuinely separate corners together.
const WELD_TOLERANCE: f64 = 1.0e-6;

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
            let mut previous: Option<[f64; 2]> = None;

            for (index, high) in wire.points.iter().copied().enumerate() {
                // NaNs delimit independent segments inside some WireModels,
                // notably polylines stored as A-B | B-C | C-D.
                if !high[0].is_finite() || !high[1].is_finite() {
                    previous = None;
                    continue;
                }

                // The wire carries its coordinates as a double-single pair, so
                // both halves are needed to recover the f64 the tessellation
                // produced. Reading only the high half would put every vertex
                // of a survey-coordinate drawing on a grid coarser than the
                // weld tolerance.
                let low = wire.points_low.get(index).copied().unwrap_or([0.0; 3]);
                let current = [
                    high[0] as f64 + low[0] as f64,
                    high[1] as f64 + low[1] as f64,
                ];

                if let Some(start) = previous {
                    let (dx, dy) = (current[0] - start[0], current[1] - start[1]);
                    if dx.hypot(dy) > WELD_TOLERANCE {
                        segments.push(Line {
                            start,
                            end: current,
                        });
                    }
                }

                previous = Some(current);
            }
        }

        bounded_faces(&segments, Tolerance::new(WELD_TOLERANCE))
    }
}
