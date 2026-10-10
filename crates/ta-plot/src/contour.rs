//! Contour extraction by marching squares.
//!
//! A matrix is a sampled grid, not a function, so a contour line is the linear
//! interpolation of `value == level` along the cell edges it crosses. Cells with
//! a missing corner are skipped entirely: drawing a line through a region that
//! was never measured would invent data.

use ta_core::{Axis, CoreError, Result};

/// A sampled grid in display orientation: `x` runs along the rows of `values`
/// and `y` along its columns.
///
/// The matrix convention is `[wavelength, delay]`, so x is normally wavelength
/// and y is delay; the constructor is the only place that mapping is written
/// down.
#[derive(Clone, Debug)]
pub struct ContourGrid<'a> {
    pub x: &'a Axis,
    pub y: &'a Axis,
    pub values: &'a ndarray::Array2<f64>,
}

impl<'a> ContourGrid<'a> {
    /// Builds a grid from a wavelength-major matrix.
    pub fn from_matrix(
        wavelength: &'a Axis,
        delay: &'a Axis,
        values: &'a ndarray::Array2<f64>,
    ) -> Result<Self> {
        let (rows, cols) = values.dim();
        if rows != wavelength.len() || cols != delay.len() {
            return Err(CoreError::ShapeMismatch {
                rows,
                cols,
                wavelengths: wavelength.len(),
                delays: delay.len(),
            });
        }
        Ok(Self {
            x: wavelength,
            y: delay,
            values,
        })
    }

    pub fn nx(&self) -> usize {
        self.values.nrows()
    }

    pub fn ny(&self) -> usize {
        self.values.ncols()
    }

    pub fn value(&self, i: usize, j: usize) -> f64 {
        self.values[[i, j]]
    }
}

/// One end of a contour segment: which grid edge it sits on, and where.
///
/// Stitching is done on the edge identity rather than on the coordinates, so
/// two cells that share an edge produce the identical point and the polylines
/// join exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum EdgeKey {
    /// Between `(i, j)` and `(i + 1, j)`.
    Horizontal { i: usize, j: usize },
    /// Between `(i, j)` and `(i, j + 1)`.
    Vertical { i: usize, j: usize },
}

#[derive(Clone, Copy, Debug)]
struct Crossing {
    key: EdgeKey,
    point: (f64, f64),
}

/// Extracts the contour lines at one level, as polylines in data coordinates.
///
/// Ambiguous saddle cells are resolved against the cell's mean, which is the
/// standard choice and keeps a closed line closed.
pub fn isolines(grid: &ContourGrid<'_>, level: f64) -> Vec<Vec<(f64, f64)>> {
    if !level.is_finite() {
        return Vec::new();
    }
    let mut segments: Vec<(Crossing, Crossing)> = Vec::new();
    for i in 0..grid.nx().saturating_sub(1) {
        for j in 0..grid.ny().saturating_sub(1) {
            for segment in cell_segments(grid, i, j, level).into_iter().flatten() {
                segments.push(segment);
            }
        }
    }
    stitch(segments)
}

/// The rings that bound the region where `low <= value < high`.
///
/// Used for the cursor-free "highlight this band" interactions and for tests
/// that need a closed region; the filled contour display itself is rendered
/// from a level-quantised raster, because drawing 200 000 cell quads per frame
/// is exactly what `spec.md` §6.5 forbids.
pub fn band_rings(grid: &ContourGrid<'_>, low: f64, high: f64) -> Vec<Vec<(f64, f64)>> {
    if !(low.is_finite() && high.is_finite()) || high <= low {
        return Vec::new();
    }
    // A ring of the band is a contour of the indicator field at the halfway
    // value, interpolated on the same edges the value contours would use.
    let indicator = grid
        .values
        .mapv(|value| if value >= low && value < high { 1.0 } else { 0.0 });
    let grid = ContourGrid {
        x: grid.x,
        y: grid.y,
        values: &indicator,
    };
    isolines(&grid, 0.5)
}

/// The segments one cell contributes: none, one, or two for a saddle.
fn cell_segments(
    grid: &ContourGrid<'_>,
    i: usize,
    j: usize,
    level: f64,
) -> [Option<(Crossing, Crossing)>; 2] {
    // Cell corners: c0 bottom-left, c1 bottom-right, c2 top-right, c3 top-left.
    let c0 = grid.value(i, j);
    let c1 = grid.value(i + 1, j);
    let c2 = grid.value(i + 1, j + 1);
    let c3 = grid.value(i, j + 1);
    if !(c0.is_finite() && c1.is_finite() && c2.is_finite() && c3.is_finite()) {
        return [None, None];
    }

    let inside = |value: f64| value > level;
    let case = (inside(c0) as u8)
        | ((inside(c1) as u8) << 1)
        | ((inside(c2) as u8) << 2)
        | ((inside(c3) as u8) << 3);

    // Edge crossings, computed on demand so no work happens for a missed cell.
    let bottom = Crossing {
        key: EdgeKey::Horizontal { i, j },
        point: (
            lerp(
                grid.x.values()[i],
                grid.x.values()[i + 1],
                c0,
                c1,
                level,
            ),
            grid.y.values()[j],
        ),
    };
    let right = Crossing {
        key: EdgeKey::Vertical { i: i + 1, j },
        point: (
            grid.x.values()[i + 1],
            lerp(
                grid.y.values()[j],
                grid.y.values()[j + 1],
                c1,
                c2,
                level,
            ),
        ),
    };
    let top = Crossing {
        key: EdgeKey::Horizontal { i, j: j + 1 },
        point: (
            lerp(
                grid.x.values()[i],
                grid.x.values()[i + 1],
                c3,
                c2,
                level,
            ),
            grid.y.values()[j + 1],
        ),
    };
    let left = Crossing {
        key: EdgeKey::Vertical { i, j },
        point: (
            grid.x.values()[i],
            lerp(
                grid.y.values()[j],
                grid.y.values()[j + 1],
                c0,
                c3,
                level,
            ),
        ),
    };

    let single = |segment: (Crossing, Crossing)| -> [Option<(Crossing, Crossing)>; 2] {
        if segment.0.key == segment.1.key {
            [None, None]
        } else {
            [Some(segment), None]
        }
    };
    let pair = |a: (Crossing, Crossing),
                b: (Crossing, Crossing)|
     -> [Option<(Crossing, Crossing)>; 2] {
        [Some(a), Some(b)]
    };

    match case {
        0 | 15 => [None, None],
        1 => single((left, bottom)),
        2 => single((bottom, right)),
        3 => single((left, right)),
        4 => single((right, top)),
        6 => single((bottom, top)),
        7 => single((left, top)),
        8 => single((top, left)),
        9 => single((top, bottom)),
        11 => single((top, right)),
        12 => single((right, left)),
        13 => single((right, bottom)),
        14 => single((bottom, left)),
        // Saddle cells carry two lines. The cell's mean decides whether the
        // inside corners are joined through the middle or separated: when they
        // are joined, the two outside corners are each cut off on their own.
        5 => {
            let mean = (c0 + c1 + c2 + c3) / 4.0;
            if inside(mean) {
                pair((bottom, right), (top, left))
            } else {
                pair((left, bottom), (right, top))
            }
        }
        10 => {
            let mean = (c0 + c1 + c2 + c3) / 4.0;
            if inside(mean) {
                pair((left, bottom), (right, top))
            } else {
                pair((bottom, right), (top, left))
            }
        }
        _ => [None, None],
    }
}

/// Where along an edge from `a0` to `a1` the level sits, in that axis'
/// coordinates.
fn lerp(a0: f64, a1: f64, v0: f64, v1: f64, level: f64) -> f64 {
    let span = v1 - v0;
    let t = if span == 0.0 {
        0.5
    } else {
        ((level - v0) / span).clamp(0.0, 1.0)
    };
    a0 + (a1 - a0) * t
}

/// Joins segments that share an edge into polylines.
fn stitch(segments: Vec<(Crossing, Crossing)>) -> Vec<Vec<(f64, f64)>> {
    use std::collections::HashMap;

    // Neighbour lookup: which segments touch each edge.
    let mut touching: HashMap<EdgeKey, Vec<usize>> = HashMap::new();
    for (index, (a, b)) in segments.iter().enumerate() {
        touching.entry(a.key).or_default().push(index);
        touching.entry(b.key).or_default().push(index);
    }

    let mut used = vec![false; segments.len()];
    let mut lines = Vec::new();

    for start in 0..segments.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let (first, second) = segments[start];
        let mut forward = vec![second.point];
        let mut backward = vec![first.point];

        // Walk in both directions until the chain runs out.
        for (from, chain) in [(second.key, &mut forward), (first.key, &mut backward)] {
            let mut current = from;
            loop {
                let next = touching
                    .get(&current)
                    .and_then(|candidates| {
                        candidates.iter().copied().find(|index| !used[*index])
                    })
                    .map(|index| {
                        used[index] = true;
                        let (a, b) = segments[index];
                        if a.key == current {
                            b
                        } else {
                            a
                        }
                    });
                match next {
                    Some(crossing) => {
                        chain.push(crossing.point);
                        current = crossing.key;
                    }
                    None => break,
                }
            }
        }

        // `backward` runs away from the start, so it is reversed and the
        // forward walk appended to it.
        backward.reverse();
        backward.extend(forward);
        if backward.len() >= 2 {
            lines.push(backward);
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;
    use ta_core::{AxisKind, AxisUnit};

    fn axis(start: f64, step: f64, count: usize) -> Axis {
        Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "x",
            (0..count).map(|i| start + step * i as f64).collect(),
        )
        .unwrap()
    }

    fn delay_axis(start: f64, step: f64, count: usize) -> Axis {
        Axis::new(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            "y",
            (0..count).map(|i| start + step * i as f64).collect(),
        )
        .unwrap()
    }

    #[test]
    fn a_monotonic_grid_crossing_zero_gives_one_straight_line() {
        let x = axis(0.0, 1.0, 5);
        let y = delay_axis(0.0, 1.0, 3);
        let mut values = Array2::zeros((5, 3));
        for i in 0..5 {
            for j in 0..3 {
                values[[i, j]] = i as f64 - 2.0;
            }
        }
        let grid = ContourGrid::from_matrix(&x, &y, &values).unwrap();
        let lines = isolines(&grid, 0.0);
        assert_eq!(lines.len(), 1, "expected one line, got {lines:?}");
        // The line sits exactly on x = 2.0 and covers the whole y range.
        for (px, py) in &lines[0] {
            assert!((px - 2.0).abs() < 1e-12, "x is {px}");
            assert!((0.0..=2.0).contains(py), "y is {py}");
        }
        assert!(lines[0].len() >= 2);
    }

    #[test]
    fn the_line_is_straight_and_stitched_not_fragmented() {
        let x = axis(0.0, 1.0, 21);
        let y = delay_axis(0.0, 1.0, 11);
        let mut values = Array2::zeros((21, 11));
        for i in 0..21 {
            for j in 0..11 {
                values[[i, j]] = i as f64 - 10.0;
            }
        }
        let grid = ContourGrid::from_matrix(&x, &y, &values).unwrap();
        let lines = isolines(&grid, 0.0);
        assert_eq!(lines.len(), 1);
        // Twenty cells across, so twenty-one nodes along the line.
        assert_eq!(lines[0].len(), 11);
        // The polyline is straight: every point shares the same x.
        let first = lines[0][0].0;
        assert!(lines[0].iter().all(|point| (point.0 - first).abs() < 1e-12));
    }

    #[test]
    fn a_radial_bump_gives_one_closed_ring_per_level() {
        let x = axis(-3.0, 0.25, 25);
        let y = delay_axis(-3.0, 0.25, 25);
        let mut values = Array2::zeros((25, 25));
        for i in 0..25 {
            for j in 0..25 {
                let dx = x.values()[i];
                let dy = y.values()[j];
                values[[i, j]] = (-(dx * dx + dy * dy)).exp();
            }
        }
        let grid = ContourGrid::from_matrix(&x, &y, &values).unwrap();

        for level in [0.8, 0.5, 0.2] {
            let lines = isolines(&grid, level);
            assert_eq!(lines.len(), 1, "level {level} gave {} lines", lines.len());
            let ring = &lines[0];
            // A closed ring: the walk must come back to where it started.
            let first = ring[0];
            let last = ring[ring.len() - 1];
            assert!(
                (first.0 - last.0).abs() < 1e-9 && (first.1 - last.1).abs() < 1e-9,
                "level {level} ring is not closed: {first:?} to {last:?}"
            );
            // Every point is at the radius the level implies.
            let expected = (-level.ln()).sqrt();
            for (px, py) in ring {
                let radius = (px * px + py * py).sqrt();
                assert!(
                    (radius - expected).abs() < 0.02,
                    "level {level}: radius {radius} against {expected}"
                );
            }
        }
    }

    #[test]
    fn a_missing_corner_removes_the_cells_touching_it() {
        let x = axis(0.0, 1.0, 5);
        let y = delay_axis(0.0, 1.0, 3);
        let mut values = Array2::zeros((5, 3));
        for i in 0..5 {
            for j in 0..3 {
                values[[i, j]] = i as f64 - 2.0;
            }
        }

        // The level-0 crossing lies entirely in the cells that start at i = 2.
        // Removing the sample they share therefore removes the whole line:
        // nothing is drawn through a region that was never measured.
        let mut holed = values.clone();
        holed[[2, 1]] = f64::NAN;
        let grid = ContourGrid::from_matrix(&x, &y, &holed).unwrap();
        assert!(
            isolines(&grid, 0.0).is_empty(),
            "a hole at the crossing must leave no line"
        );

        // A hole away from the crossing drops only its own cells.
        let mut edged = values.clone();
        edged[[4, 1]] = f64::NAN;
        let grid = ContourGrid::from_matrix(&x, &y, &edged).unwrap();
        let lines = isolines(&grid, 0.0);
        assert!(!lines.is_empty(), "the remaining cells should still contour");
        for line in &lines {
            for &(px, py) in line {
                assert!((px - 2.0).abs() < 1e-12, "x is {px}");
                assert!((0.0..=2.0).contains(&py), "y is {py}");
            }
        }
    }

    #[test]
    fn a_level_outside_the_data_range_contributes_nothing() {
        let x = axis(0.0, 1.0, 3);
        let y = delay_axis(0.0, 1.0, 3);
        let values = Array2::from_elem((3, 3), 1.0);
        let grid = ContourGrid::from_matrix(&x, &y, &values).unwrap();
        assert!(isolines(&grid, 5.0).is_empty());
        assert!(isolines(&grid, -5.0).is_empty());
        // A level exactly at a node value is a degenerate case, not a panic.
        let _ = isolines(&grid, 1.0);
        assert!(isolines(&grid, f64::NAN).is_empty());
    }

    #[test]
    fn a_saddle_cell_produces_two_separated_lines() {
        // Diagonal high/low corners.
        let x = axis(0.0, 1.0, 2);
        let y = delay_axis(0.0, 1.0, 2);
        let values = ndarray::array![[1.0, -2.0], [-2.0, 1.0]];
        let grid = ContourGrid::from_matrix(&x, &y, &values).unwrap();
        let lines = isolines(&grid, 0.0);
        assert_eq!(lines.len(), 2, "a saddle should give two lines: {lines:?}");
        // Neither line wanders out of the single cell.
        for line in &lines {
            for (px, py) in line {
                assert!((0.0..=1.0).contains(px) && (0.0..=1.0).contains(py));
            }
        }
    }

    #[test]
    fn a_band_ring_encloses_the_cells_inside_the_band() {
        let x = axis(0.0, 1.0, 3);
        let y = delay_axis(0.0, 1.0, 3);
        let values = ndarray::array![[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 0.0]];
        let grid = ContourGrid::from_matrix(&x, &y, &values).unwrap();
        let rings = band_rings(&grid, 0.5, 1.5);
        assert!(!rings.is_empty());
        // Every ring point is inside the cell that holds the high value.
        for ring in &rings {
            for (px, py) in ring {
                assert!((0.5..=1.5).contains(px), "x is {px}");
                assert!((0.5..=1.5).contains(py), "y is {py}");
            }
        }
        assert!(band_rings(&grid, 1.5, 0.5).is_empty(), "an inverted band is nothing");
    }

    #[test]
    fn a_grid_whose_shape_disagrees_with_its_axes_is_refused() {
        let x = axis(0.0, 1.0, 3);
        let y = delay_axis(0.0, 1.0, 3);
        let values = Array2::zeros((2, 2));
        assert!(ContourGrid::from_matrix(&x, &y, &values).is_err());
    }
}
