//! Plot description and rendering.
//!
//! Three things live here, and the split matters:
//!
//! * [`scale`] and [`viewport`] own the map between a physical coordinate and a
//!   pixel. Display, mouse hit testing, selection and export all ask them, so
//!   they cannot disagree;
//! * [`contour`] and [`colormap`] turn a matrix into lines and an image;
//! * [`render`] draws a panel through Plotters.

pub mod colormap;
pub mod contour;
pub mod scale;
pub mod viewport;

pub use colormap::{Raster, RasterKey, RasterRequest, RasterSampling};
pub use contour::{band_rings, isolines, ContourGrid};
pub use scale::Scale;
pub use viewport::PlotViewport;
