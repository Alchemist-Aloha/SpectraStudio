//! The raster cache behind the contour plate.
//!
//! A frame that changed nothing rebuilds nothing: the colour-mapped image is
//! keyed on every input that affects it, and the key is computed before any
//! pixels are made.

use gpui_kit::RenderImage;
use std::sync::Arc;
use ta_core::Rgba as InkColor;
use ta_plot::colormap::{band_levels, rasterise, RasterKey, RasterRequest};
use ta_plot::render::HeatmapLayer;

/// The image in the two forms the two backends need.
pub struct HeatmapData {
    /// RGB, for the recorder that builds an exported figure.
    pub layer: HeatmapLayer,
    /// BGRA, for the screen.
    pub image: Arc<RenderImage>,
    /// The image's size, so a caller can place it without asking the atlas.
    pub size: (u32, u32),
}

impl HeatmapData {
    /// Composites an RGBA raster over a background colour and swizzles it for
    /// GPUI, which takes BGRA.
    pub fn from_raster(pixels: &[u8], width: usize, height: usize, background: InkColor) -> Self {
        let layer = HeatmapLayer::from_rgba_over(pixels, width, height, background);
        let mut bgra = Vec::with_capacity(width * height * 4);
        for chunk in pixels.chunks_exact(4) {
            bgra.push(chunk[2]);
            bgra.push(chunk[1]);
            bgra.push(chunk[0]);
            bgra.push(chunk[3]);
        }
        let buffer = image::ImageBuffer::from_raw(width as u32, height as u32, bgra)
            .expect("the buffer is exactly the raster's size");
        let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
        Self {
            layer,
            image,
            size: (width as u32, height as u32),
        }
    }

    pub fn byte_len(&self) -> usize {
        self.layer.rgb.len() + self.size.0 as usize * self.size.1 as usize * 4
    }
}

/// The key a request would be cached under.
pub fn request_key(request: &RasterRequest<'_>) -> RasterKey {
    let boundaries = match request.sampling {
        ta_plot::RasterSampling::Levels => {
            band_levels(&request.limits, &request.levels, request.level_count)
        }
        _ => Vec::new(),
    };
    RasterKey::of(request, request.sampling, &boundaries)
}

/// Builds the image for a request, for callers that manage the cache key
/// themselves.
pub fn build_heatmap(
    request: &RasterRequest<'_>,
    background: InkColor,
) -> Arc<HeatmapData> {
    let raster = rasterise(request);
    Arc::new(HeatmapData::from_raster(
        &raster.pixels,
        raster.width,
        raster.height,
        background,
    ))
}

/// Holds the last image and the key that produced it.
#[derive(Default)]
pub struct HeatmapCache {
    key: Option<RasterKey>,
    data: Option<Arc<HeatmapData>>,
}

impl HeatmapCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the cache already holds this exact image.
    pub fn matches(&self, key: &RasterKey) -> bool {
        self.key.as_ref() == Some(key) && self.data.is_some()
    }

    /// Stores an image under its key.
    pub fn store(&mut self, key: RasterKey, data: Arc<HeatmapData>) {
        self.key = Some(key);
        self.data = Some(data);
    }

    /// The image currently held, if any.
    pub fn current(&self) -> Option<Arc<HeatmapData>> {
        self.data.clone()
    }

    /// The image for this request, rebuilt only when something visible changed.
    pub fn get(&mut self, request: &RasterRequest<'_>, background: InkColor) -> Arc<HeatmapData> {
        let boundaries = match request.sampling {
            ta_plot::RasterSampling::Levels => {
                band_levels(&request.limits, &request.levels, request.level_count)
            }
            _ => Vec::new(),
        };
        let key = RasterKey::of(request, request.sampling, &boundaries);
        if self.key.as_ref() == Some(&key) {
            if let Some(data) = &self.data {
                return data.clone();
            }
        }
        let raster = rasterise(request);
        let data = Arc::new(HeatmapData::from_raster(
            &raster.pixels,
            raster.width,
            raster.height,
            background,
        ));
        self.key = Some(key);
        self.data = Some(data.clone());
        data
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_none()
    }

    /// Drops the image, for a dataset that was closed.
    pub fn forget(&mut self) {
        self.key = None;
        self.data = None;
    }

    pub fn byte_len(&self) -> usize {
        self.data.as_ref().map(|data| data.byte_len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;
    use ta_core::{AutoRange, Axis, AxisKind, AxisUnit, ColorLimits, Colormap};
    use ta_plot::{ContourGrid, RasterSampling};

    fn axis(count: usize, step: f64) -> Axis {
        Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "w",
            (0..count).map(|i| i as f64 * step).collect(),
        )
        .unwrap()
    }

    #[test]
    fn the_same_request_reuses_the_same_image() {
        let x = axis(4, 1.0);
        let y = axis(4, 1.0);
        let values = Array2::from_elem((4, 4), 0.5);
        let grid = ContourGrid::from_matrix(&x, &y, &values).unwrap();
        let request = RasterRequest {
            grid,
            source: None,
            x_range: (0.0, 3.0),
            y_range: (0.0, 3.0),
            width: 8,
            height: 8,
            limits: ColorLimits {
                min: 0.0,
                max: 1.0,
                symmetric_about_zero: false,
                auto: AutoRange::Manual,
            },
            colormap: Colormap::preset("grey").unwrap(),
            levels: Vec::new(),
            level_count: 4,
            sampling: RasterSampling::Nearest,
        };

        let mut cache = HeatmapCache::new();
        assert!(cache.is_empty());
        let first = cache.get(&request, InkColor::rgb(255, 255, 255));
        let second = cache.get(&request, InkColor::rgb(255, 255, 255));
        assert!(Arc::ptr_eq(&first, &second), "the cache rebuilt an identical image");
        assert!(cache.byte_len() > 0);

        // A moved window is a different image.
        let moved = RasterRequest {
            x_range: (0.0, 2.0),
            ..request
        };
        let third = cache.get(&moved, InkColor::rgb(255, 255, 255));
        assert!(!Arc::ptr_eq(&first, &third));
        cache.forget();
        assert!(cache.is_empty());
    }
}
