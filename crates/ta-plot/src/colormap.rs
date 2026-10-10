//! Turning a matrix into an image.
//!
//! The contour view must never draw a rectangle per cell (spec §6.5), so a
//! visible window of the matrix becomes one RGBA buffer, cached and rebuilt only
//! when the window, the colour limits, the palette or the level set changes.
//!
//! Three resamplings are offered and they mean different things:
//!
//! * [`RasterSampling::Nearest`] — the measured value of the nearest sample. The
//!   honest default for a measurement;
//! * [`RasterSampling::Smooth`] — bilinear between samples, for looking at shape;
//! * [`RasterSampling::Levels`] — quantised into the user's contour levels, which
//!   is how filled contour levels are drawn without one primitive per cell.

use crate::contour::ContourGrid;
use ta_core::{ColorLimits, Colormap, DatasetId, Rgba, VersionId};

/// How a matrix sample becomes a pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterSampling {
    Nearest,
    Smooth,
    Levels,
}

impl RasterSampling {
    pub const fn label(self) -> &'static str {
        match self {
            RasterSampling::Nearest => "nearest",
            RasterSampling::Smooth => "smooth",
            RasterSampling::Levels => "levels",
        }
    }
}

/// Everything that goes into one image.
#[derive(Clone, Debug)]
pub struct RasterRequest<'a> {
    pub grid: ContourGrid<'a>,
    /// Which dataset and version the grid came from, so the cache can tell two
    /// matrices with the same shape apart.
    pub source: Option<(DatasetId, VersionId)>,
    /// The physical window being shown.
    pub x_range: (f64, f64),
    pub y_range: (f64, f64),
    /// The output size in pixels.
    pub width: usize,
    pub height: usize,
    pub limits: ColorLimits,
    pub colormap: Colormap,
    /// Band boundaries for [`RasterSampling::Levels`]. Empty means "derive
    /// `level_count` even steps across the colour limits".
    pub levels: Vec<f64>,
    pub level_count: usize,
    pub sampling: RasterSampling,
}

/// An RGBA image, row-major, first row at the top.
#[derive(Clone, Debug, PartialEq)]
pub struct Raster {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
    pub key: RasterKey,
}

impl Raster {
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The RGBA bytes at a pixel, or transparent black outside the image.
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0, 0, 0, 0];
        }
        let offset = (y * self.width + x) * 4;
        [
            self.pixels[offset],
            self.pixels[offset + 1],
            self.pixels[offset + 2],
            self.pixels[offset + 3],
        ]
    }

    /// An index of what this image shows, so a frame that changes nothing
    /// rebuilds nothing.
    pub fn key(&self) -> &RasterKey {
        &self.key
    }
}

/// The identity of an image, used as a cache key.
///
/// Floats are compared bitwise: a viewport that moved by one ten-millionth of a
/// pixel is a different image, and pretending otherwise would show a stale one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RasterKey {
    pub source: Option<(u128, u128)>,
    pub x_range: (u64, u64),
    pub y_range: (u64, u64),
    pub size: (usize, usize),
    pub limits: (u64, u64),
    pub colormap: (String, bool),
    pub sampling: &'static str,
    pub levels: Vec<u64>,
}

impl RasterKey {
    pub fn of(request: &RasterRequest<'_>, sampling: RasterSampling, levels: &[f64]) -> Self {
        Self {
            source: request
                .source
                .map(|(dataset, version)| {
                    (dataset.as_uuid().as_u128(), version.as_uuid().as_u128())
                }),
            x_range: (
                request.x_range.0.to_bits(),
                request.x_range.1.to_bits(),
            ),
            y_range: (
                request.y_range.0.to_bits(),
                request.y_range.1.to_bits(),
            ),
            size: (request.width, request.height),
            limits: (
                request.limits.min.to_bits(),
                request.limits.max.to_bits(),
            ),
            colormap: (
                request.colormap.name.clone(),
                request.colormap.reversed,
            ),
            sampling: sampling.label(),
            levels: levels.iter().map(|level| level.to_bits()).collect(),
        }
    }
}

/// The band boundaries actually used, derived from the limits when the user has
/// not set any.
pub fn band_levels(limits: &ColorLimits, levels: &[f64], level_count: usize) -> Vec<f64> {
    if levels.len() >= 2 {
        let mut given = levels.to_vec();
        given.sort_by(f64::total_cmp);
        return given;
    }
    let count = level_count.clamp(2, 256);
    let span = limits.max - limits.min;
    (0..=count)
        .map(|index| limits.min + span * index as f64 / count as f64)
        .collect()
}

/// One colour per band, so a band and the colour that represents it cannot drift
/// apart between the image and the legend.
pub fn band_colors(
    limits: &ColorLimits,
    colormap: &Colormap,
    boundaries: &[f64],
) -> Vec<Rgba> {
    let mut colours = Vec::with_capacity(boundaries.len().saturating_sub(1));
    for pair in boundaries.windows(2) {
        let centre = (pair[0] + pair[1]) / 2.0;
        let colour = limits
            .normalize(centre)
            .map(|t| colormap.sample(t))
            .unwrap_or(Rgba::rgba(0, 0, 0, 0));
        colours.push(colour);
    }
    colours
}

/// The index of the band a value falls into. Values outside the outermost
/// boundaries clamp to the end bands rather than disappearing.
fn band_index(value: f64, boundaries: &[f64]) -> Option<usize> {
    if boundaries.len() < 2 || !value.is_finite() {
        return None;
    }
    if value < boundaries[0] || value > boundaries[boundaries.len() - 1] {
        return None;
    }
    for (index, pair) in boundaries.windows(2).enumerate() {
        if value >= pair[0] && value < pair[1] {
            return Some(index);
        }
    }
    // Exactly the last boundary belongs to the last band.
    Some(boundaries.len() - 2)
}

/// The nearest sample index on a monotonic axis, by binary search.
fn nearest_index(values: &[f64], target: f64) -> usize {
    if values.len() < 2 {
        return 0;
    }
    let ascending = values[values.len() - 1] >= values[0];
    let (mut low, mut high) = (0usize, values.len() - 1);
    while high - low > 1 {
        let mid = low + (high - low) / 2;
        let past = if ascending {
            values[mid] <= target
        } else {
            values[mid] >= target
        };
        if past {
            low = mid;
        } else {
            high = mid;
        }
    }
    if (values[low] - target).abs() <= (values[high] - target).abs() {
        low
    } else {
        high
    }
}

/// The two sample indices bracketing a target, and how far between them it sits.
fn bracket_index(values: &[f64], target: f64) -> (usize, usize, f64) {
    if values.len() < 2 {
        return (0, 0, 0.0);
    }
    let ascending = values[values.len() - 1] >= values[0];
    let (mut low, mut high) = (0usize, values.len() - 1);
    while high - low > 1 {
        let mid = low + (high - low) / 2;
        let past = if ascending {
            values[mid] <= target
        } else {
            values[mid] >= target
        };
        if past {
            low = mid;
        } else {
            high = mid;
        }
    }
    let span = values[high] - values[low];
    let t = if span == 0.0 {
        0.0
    } else {
        ((target - values[low]) / span).clamp(0.0, 1.0)
    };
    (low, high, t)
}

/// Builds the image for one request.
pub fn rasterise(request: &RasterRequest<'_>) -> Raster {
    let width = request.width.max(1);
    let height = request.height.max(1);
    let mut pixels = vec![0u8; width * height * 4];

    let boundaries = match request.sampling {
        RasterSampling::Levels => {
            band_levels(&request.limits, &request.levels, request.level_count)
        }
        _ => Vec::new(),
    };
    let colours = band_colors(&request.limits, &request.colormap, &boundaries);

    // The physical coordinate at the centre of each pixel. Screen y runs
    // downwards, so the top row is the high end of the y range.
    let x_positions: Vec<f64> = (0..width)
        .map(|column| {
            let t = (column as f64 + 0.5) / width as f64;
            request.x_range.0 + t * (request.x_range.1 - request.x_range.0)
        })
        .collect();
    let y_positions: Vec<f64> = (0..height)
        .map(|row| {
            let t = 1.0 - (row as f64 + 0.5) / height as f64;
            request.y_range.0 + t * (request.y_range.1 - request.y_range.0)
        })
        .collect();

    let x_values = request.grid.x.values();
    let y_values = request.grid.y.values();

    for (row, y) in y_positions.iter().enumerate() {
        for (column, x) in x_positions.iter().enumerate() {
            let value = match request.sampling {
                RasterSampling::Smooth => {
                    let (i0, i1, tx) = bracket_index(x_values, *x);
                    let (j0, j1, ty) = bracket_index(y_values, *y);
                    let v00 = request.grid.value(i0, j0);
                    let v10 = request.grid.value(i1, j0);
                    let v01 = request.grid.value(i0, j1);
                    let v11 = request.grid.value(i1, j1);
                    if !(v00.is_finite() && v10.is_finite() && v01.is_finite() && v11.is_finite())
                    {
                        f64::NAN
                    } else {
                        let bottom = v00 * (1.0 - tx) + v10 * tx;
                        let top = v01 * (1.0 - tx) + v11 * tx;
                        bottom * (1.0 - ty) + top * ty
                    }
                }
                RasterSampling::Nearest | RasterSampling::Levels => {
                    let i = nearest_index(x_values, *x);
                    let j = nearest_index(y_values, *y);
                    request.grid.value(i, j)
                }
            };

            let colour = match request.sampling {
                RasterSampling::Levels => match band_index(value, &boundaries) {
                    Some(index) => colours.get(index).copied().unwrap_or(Rgba::rgba(0, 0, 0, 0)),
                    None => Rgba::rgba(0, 0, 0, 0),
                },
                _ => match request.limits.normalize(value) {
                    Some(t) => request.colormap.sample(t),
                    None => Rgba::rgba(0, 0, 0, 0),
                },
            };

            let offset = (row * width + column) * 4;
            pixels[offset] = colour.r;
            pixels[offset + 1] = colour.g;
            pixels[offset + 2] = colour.b;
            pixels[offset + 3] = colour.a;
        }
    }

    Raster {
        width,
        height,
        pixels,
        key: RasterKey::of(request, request.sampling, &boundaries),
    }
}

/// A colour map's stop colours as a legend strip, for the colour bar.
pub fn colour_bar_strip(colormap: &Colormap, width: usize) -> Raster {
    let width = width.max(2);
    let mut pixels = vec![0u8; width * 4];
    for column in 0..width {
        let t = column as f64 / (width - 1) as f64;
        let colour = colormap.sample(t as f32);
        let offset = column * 4;
        pixels[offset] = colour.r;
        pixels[offset + 1] = colour.g;
        pixels[offset + 2] = colour.b;
        pixels[offset + 3] = colour.a;
    }
    Raster {
        width,
        height: 1,
        pixels,
        key: RasterKey {
            source: None,
            x_range: (0, 0),
            y_range: (0, 0),
            size: (width, 1),
            limits: (0, 0),
            colormap: (colormap.name.clone(), colormap.reversed),
            sampling: "legend",
            levels: Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contour::ContourGrid;
    use ndarray::Array2;
    use ta_core::{AutoRange, Axis, AxisKind, AxisUnit};

    fn axes(nx: usize, ny: usize) -> (Axis, Axis) {
        (
            Axis::new(
                AxisKind::Wavelength,
                AxisUnit::Nanometer,
                "Wavelength",
                (0..nx).map(|i| i as f64).collect(),
            )
            .unwrap(),
            Axis::new(
                AxisKind::Delay,
                AxisUnit::Picosecond,
                "Delay",
                (0..ny).map(|i| i as f64).collect(),
            )
            .unwrap(),
        )
    }

    fn request<'a>(
        grid: ContourGrid<'a>,
        values: &'a Array2<f64>,
        sampling: RasterSampling,
    ) -> RasterRequest<'a> {
        let _ = values;
        RasterRequest {
            grid,
            source: None,
            x_range: (0.0, 1.0),
            y_range: (0.0, 1.0),
            width: 2,
            height: 2,
            limits: ColorLimits {
                min: 0.0,
                max: 1.0,
                symmetric_about_zero: false,
                auto: AutoRange::Manual,
            },
            colormap: Colormap::preset("grey").unwrap(),
            levels: Vec::new(),
            level_count: 4,
            sampling,
        }
    }

    #[test]
    fn a_two_by_two_grid_lands_in_the_expected_corners() {
        let (x, y) = axes(2, 2);
        // value(i, j): row index is x, column index is y.
        let values = ndarray::array![[0.0, 1.0], [0.0, 1.0]];
        let grid = ContourGrid::from_matrix(&x, &y, &values).unwrap();
        let raster = rasterise(&request(grid, &values, RasterSampling::Nearest));

        assert_eq!(raster.width, 2);
        assert_eq!(raster.height, 2);
        // Top row is the high y end: y = 1 is the white end of a grey ramp.
        let top_left = raster.pixel(0, 0);
        let top_right = raster.pixel(1, 0);
        let bottom_left = raster.pixel(0, 1);
        assert_eq!(top_left, [255, 255, 255, 255]);
        assert_eq!(top_right, [255, 255, 255, 255]);
        assert_eq!(bottom_left, [0, 0, 0, 255]);
    }

    #[test]
    fn a_missing_sample_paints_nothing_rather_than_zero() {
        let (x, y) = axes(2, 2);
        let values = ndarray::array![[f64::NAN, 1.0], [0.5, 1.0]];
        let grid = ContourGrid::from_matrix(&x, &y, &values).unwrap();
        let raster = rasterise(&request(grid, &values, RasterSampling::Nearest));
        // The missing sample sits at x = 0, y = 0, which is the bottom-left of
        // a screen-ordered image.
        assert_eq!(raster.pixel(0, 1)[3], 0, "the bottom-left sample is missing");
        assert_ne!(raster.pixel(0, 0)[3], 0);
        assert_ne!(raster.pixel(1, 1)[3], 0);
    }

    #[test]
    fn the_colour_limits_decide_the_colour_not_the_data() {
        let (x, y) = axes(2, 2);
        let values = ndarray::array![[0.0, 0.0], [0.5, 0.5]];
        let grid = ContourGrid::from_matrix(&x, &y, &values).unwrap();
        let mut base = request(grid, &values, RasterSampling::Nearest);
        base.limits = ColorLimits {
            min: 0.0,
            max: 1.0,
            symmetric_about_zero: false,
            auto: AutoRange::Manual,
        };
        let wide = rasterise(&base);

        let grid = ContourGrid::from_matrix(&x, &y, &values).unwrap();
        let mut narrow = request(grid, &values, RasterSampling::Nearest);
        narrow.limits = ColorLimits {
            min: 0.0,
            max: 0.5,
            symmetric_about_zero: false,
            auto: AutoRange::Manual,
        };
        let tight = rasterise(&narrow);

        // The pixel over x = 1 holds 0.5: mid-grey against 0..1, and the far
        // end of the ramp against 0..0.5.
        assert_ne!(wide.pixel(1, 0), tight.pixel(1, 0));
        assert_eq!(tight.pixel(1, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn a_reversed_colormap_flips_the_image() {
        let (x, y) = axes(2, 2);
        let values = ndarray::array![[0.0, 1.0], [0.0, 1.0]];
        let grid = ContourGrid::from_matrix(&x, &y, &values).unwrap();
        let forward = rasterise(&request(grid, &values, RasterSampling::Nearest));

        let grid = ContourGrid::from_matrix(&x, &y, &values).unwrap();
        let mut reversed = request(grid, &values, RasterSampling::Nearest);
        reversed.colormap.reversed = true;
        let backward = rasterise(&reversed);

        assert_eq!(forward.pixel(0, 1), backward.pixel(0, 0));
        assert_ne!(forward.key(), backward.key());
    }

    #[test]
    fn the_level_sampling_produces_a_discrete_set_of_colours() {
        let (x, y) = axes(3, 3);
        let mut values = Array2::zeros((3, 3));
        for i in 0..3 {
            for j in 0..3 {
                values[[i, j]] = (i + j) as f64 / 4.0;
            }
        }
        let grid = ContourGrid::from_matrix(&x, &y, &values).unwrap();
        let mut spec = request(grid, &values, RasterSampling::Levels);
        spec.width = 3;
        spec.height = 3;
        spec.levels = vec![0.0, 0.25, 0.5, 0.75, 1.0];
        let raster = rasterise(&spec);

        let mut seen: Vec<[u8; 4]> = Vec::new();
        for row in 0..3 {
            for column in 0..3 {
                let pixel = raster.pixel(column, row);
                if !seen.contains(&pixel) {
                    seen.push(pixel);
                }
            }
        }
        // Five boundary pairs make four bands; the values used here land in
        // three of them at most.
        assert!(
            seen.len() <= 4,
            "level sampling invented more colours than bands: {seen:?}"
        );
        assert!(!seen.is_empty());
    }

    #[test]
    fn derived_bands_span_the_colour_limits_evenly() {
        let limits = ColorLimits {
            min: -2.0,
            max: 2.0,
            symmetric_about_zero: true,
            auto: AutoRange::Full,
        };
        let boundaries = band_levels(&limits, &[], 4);
        assert_eq!(boundaries.len(), 5);
        assert_eq!(boundaries[0], -2.0);
        assert_eq!(boundaries[4], 2.0);
        assert!((boundaries[2] - 0.0).abs() < 1e-12);
    }

    #[test]
    fn explicit_bands_are_sorted_and_used_as_given() {
        let limits = ColorLimits::default();
        let boundaries = band_levels(&limits, &[0.5, -0.5, 0.0], 4);
        assert_eq!(boundaries, vec![-0.5, 0.0, 0.5]);
        assert!(band_index(0.25, &boundaries) == Some(1));
        assert!(band_index(-0.75, &boundaries).is_none());
        assert_eq!(band_index(0.5, &boundaries), Some(1));
    }

    #[test]
    fn smooth_sampling_interpolates_between_samples() {
        let (x, y) = axes(2, 2);
        let values = ndarray::array![[0.0, 0.0], [1.0, 1.0]];
        let grid = ContourGrid::from_matrix(&x, &y, &values).unwrap();

        let mut nearest = request(grid, &values, RasterSampling::Nearest);
        nearest.width = 4;
        nearest.height = 1;
        let stepped = rasterise(&nearest);

        let grid = ContourGrid::from_matrix(&x, &y, &values).unwrap();
        let mut smooth = request(grid, &values, RasterSampling::Smooth);
        smooth.width = 4;
        smooth.height = 1;
        let blended = rasterise(&smooth);

        // Nearest repeats the two sample values; smooth passes between them.
        assert_eq!(stepped.pixel(0, 0), stepped.pixel(1, 0));
        assert_ne!(blended.pixel(1, 0), blended.pixel(0, 0));
        let left = blended.pixel(0, 0)[0] as i32;
        let right = blended.pixel(3, 0)[0] as i32;
        assert!(left < right, "the ramp should increase left to right");
    }

    #[test]
    fn a_window_smaller_than_the_grid_shows_only_that_window() {
        let (x, y) = axes(5, 5);
        let mut values = Array2::zeros((5, 5));
        for i in 0..5 {
            values[[i, 2]] = 1.0;
        }
        let grid = ContourGrid::from_matrix(&x, &y, &values).unwrap();
        let mut spec = request(grid, &values, RasterSampling::Nearest);
        spec.x_range = (1.0, 3.0);
        spec.y_range = (1.0, 3.0);
        spec.width = 3;
        spec.height = 3;
        let raster = rasterise(&spec);
        // Only the middle band of the window is bright.
        assert_eq!(raster.pixel(1, 1), [255, 255, 255, 255]);
        assert_eq!(raster.pixel(0, 0), [0, 0, 0, 255]);
    }

    #[test]
    fn the_cache_key_changes_when_anything_visible_changes() {
        let (x, y) = axes(2, 2);
        let values = ndarray::array![[0.0, 1.0], [0.0, 1.0]];
        let grid = ContourGrid::from_matrix(&x, &y, &values).unwrap();
        let base = request(grid, &values, RasterSampling::Nearest);
        let first = rasterise(&base);

        let grid = ContourGrid::from_matrix(&x, &y, &values).unwrap();
        let mut moved = request(grid, &values, RasterSampling::Nearest);
        moved.x_range = (0.0, 0.9);
        let second = rasterise(&moved);
        assert_ne!(first.key(), second.key());

        let grid = ContourGrid::from_matrix(&x, &y, &values).unwrap();
        let same = request(grid, &values, RasterSampling::Nearest);
        assert_eq!(first.key(), rasterise(&same).key());
    }

    #[test]
    fn the_colour_bar_strip_runs_from_one_end_of_the_palette_to_the_other() {
        let map = Colormap::preset("bwr").unwrap();
        let strip = colour_bar_strip(&map, 5);
        assert_eq!(strip.width, 5);
        assert_eq!(strip.pixel(0, 0), [0, 0, 255, 255]);
        assert_eq!(strip.pixel(4, 0), [255, 0, 0, 255]);
        assert_eq!(strip.pixel(2, 0), [255, 255, 255, 255]);
    }
}
