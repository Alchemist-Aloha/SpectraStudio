//! The colour-mapped matrix behind the contour plate, and its cache.
//!
//! The image is asked for *after* the plate's layout is known, so it is built at
//! exactly the size of the plot rectangle. Building it earlier — at the sample
//! count of the visible window — paints a small image at its own pixel size,
//! which Plotters cannot scale, and the field then covers only part of the
//! plate.

use ndarray::Array2;
use std::rc::Rc;
use std::sync::Arc;
use ta_core::{Axis, ContourSpec, DatasetId, Rgba as InkColor, VersionId};
use ta_plot::colormap::{band_levels, rasterise, RasterKey, RasterRequest};
use ta_plot::render::HeatmapLayer;
use ta_plot::{ContourGrid, RasterSampling};

/// Everything needed to make one plate's image.
pub struct HeatmapSource {
    pub wavelength: Axis,
    pub delay: Axis,
    /// Shared rather than borrowed, so the plate can build its image during
    /// prepaint, when the dataset is only reachable through the session.
    pub matrix: Arc<Array2<f64>>,
    pub spec: ContourSpec,
    pub sampling: RasterSampling,
    pub background: InkColor,
    pub source: Option<(DatasetId, VersionId)>,
}

impl HeatmapSource {
    /// The image for one window, at the plot rectangle's own size.
    pub fn layer(
        &self,
        cache: &mut HeatmapCache,
        pixels: (u32, u32),
        x_range: (f64, f64),
        y_range: (f64, f64),
    ) -> Option<Arc<HeatmapLayer>> {
        let Ok(grid) = ContourGrid::from_matrix(&self.wavelength, &self.delay, &self.matrix) else {
            return None;
        };
        // A plate can be small, and a zero-sized image is not an image.
        let width = (pixels.0.max(2).min(4096)) as usize;
        let height = (pixels.1.max(2).min(4096)) as usize;
        let request = RasterRequest {
            grid,
            source: self.source,
            x_range,
            y_range,
            width,
            height,
            limits: self.spec.limits.clone(),
            colormap: self.spec.colormap.clone(),
            levels: self.spec.levels.clone(),
            level_count: self.spec.level_count,
            sampling: self.sampling,
        };
        Some(cache.layer(&request, self.background))
    }
}

/// The key a request would be cached under.
pub fn request_key(request: &RasterRequest<'_>) -> RasterKey {
    let boundaries = match request.sampling {
        RasterSampling::Levels => {
            band_levels(&request.limits, &request.levels, request.level_count)
        }
        _ => Vec::new(),
    };
    RasterKey::of(request, request.sampling, &boundaries)
}

/// Holds the last image and the key that produced it.
///
/// Shared through an `Rc` because the plate builds its image while painting,
/// when the workspace that owns the cache is only borrowed immutably.
#[derive(Default)]
pub struct HeatmapCache {
    key: Option<RasterKey>,
    layer: Option<Arc<HeatmapLayer>>,
}

impl HeatmapCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn matches(&self, key: &RasterKey) -> bool {
        self.key.as_ref() == Some(key) && self.layer.is_some()
    }

    pub fn current(&self) -> Option<Arc<HeatmapLayer>> {
        self.layer.clone()
    }

    /// What the held image was built from.
    pub fn key(&self) -> Option<&RasterKey> {
        self.key.as_ref()
    }

    /// The image for this request, rebuilt only when something visible changed.
    pub fn layer(&mut self, request: &RasterRequest<'_>, background: InkColor) -> Arc<HeatmapLayer> {
        let key = request_key(request);
        if self.key.as_ref() == Some(&key) {
            if let Some(layer) = &self.layer {
                return layer.clone();
            }
        }
        let raster = rasterise(request);
        let layer = Arc::new(HeatmapLayer::from_rgba_over(
            &raster.pixels,
            raster.width,
            raster.height,
            background,
        ));
        self.key = Some(key);
        self.layer = Some(layer.clone());
        layer
    }

    pub fn is_empty(&self) -> bool {
        self.layer.is_none()
    }

    /// Drops the image, for a dataset that was closed.
    pub fn forget(&mut self) {
        self.key = None;
        self.layer = None;
    }

    pub fn byte_len(&self) -> usize {
        self.layer
            .as_ref()
            .map(|layer| layer.rgb.len())
            .unwrap_or(0)
    }
}

/// A cache shared by every plate.
pub type SharedHeatmapCache = Rc<std::cell::RefCell<HeatmapCache>>;

#[cfg(test)]
mod tests {
    use super::*;
    use ta_core::{AutoRange, AxisKind, AxisUnit, ColorLimits, Colormap};

    fn source() -> HeatmapSource {
        let wavelength = Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "w",
            (0..8).map(|i| 500.0 + i as f64).collect(),
        )
        .unwrap();
        let delay = Axis::new(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            "t",
            (0..4).map(|i| i as f64).collect(),
        )
        .unwrap();
        HeatmapSource {
            wavelength,
            delay,
            matrix: Arc::new(Array2::from_elem((8, 4), 0.5)),
            spec: ContourSpec {
                limits: ColorLimits {
                    min: 0.0,
                    max: 1.0,
                    symmetric_about_zero: false,
                    auto: AutoRange::Manual,
                },
                colormap: Colormap::preset("grey").unwrap(),
                ..ContourSpec::default()
            },
            sampling: RasterSampling::Nearest,
            background: InkColor::rgb(255, 255, 255),
            source: None,
        }
    }

    #[test]
    fn the_image_is_exactly_the_size_the_plate_asked_for() {
        // This is the defect: an image at the sample count is painted at its own
        // pixel size, so the field covers only part of the plate.
        let mut cache = HeatmapCache::new();
        let layer = source()
            .layer(&mut cache, (640, 480), (500.0, 507.0), (0.0, 3.0))
            .expect("an image");
        assert_eq!((layer.width, layer.height), (640, 480));
    }

    #[test]
    fn a_tiny_plate_still_gets_a_drawable_image() {
        let mut cache = HeatmapCache::new();
        let layer = source()
            .layer(&mut cache, (0, 1), (500.0, 507.0), (0.0, 3.0))
            .expect("an image");
        assert!(layer.width >= 2 && layer.height >= 2);
    }

    #[test]
    fn the_same_request_reuses_the_same_image() {
        let mut cache = HeatmapCache::new();
        assert!(cache.is_empty());
        let first = source()
            .layer(&mut cache, (64, 32), (500.0, 507.0), (0.0, 3.0))
            .expect("an image");
        let second = source()
            .layer(&mut cache, (64, 32), (500.0, 507.0), (0.0, 3.0))
            .expect("an image");
        assert!(Arc::ptr_eq(&first, &second), "the cache rebuilt an identical image");
        assert!(cache.byte_len() > 0);

        // A resized plate is a different image.
        let third = source()
            .layer(&mut cache, (128, 64), (500.0, 507.0), (0.0, 3.0))
            .expect("an image");
        assert!(!Arc::ptr_eq(&first, &third));
        cache.forget();
        assert!(cache.is_empty());
    }
}
