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
pub mod commands;
pub mod contour;
pub mod export;
pub mod record;
pub mod render;
pub mod scale;
pub mod ticks;
pub mod viewport;

pub use colormap::{Raster, RasterKey, RasterRequest, RasterSampling};
pub use contour::{band_rings, isolines, ContourGrid};
pub use export::{
    export, export_png, export_svg, figure_pixels, figure_points, ExportPanel,
};
pub use render::{
    draw_panel,
    dash_polyline, render_panel, HeatmapLayer, PanelLayout, PlottedSeries, Rect,
};
pub use commands::{CommandBuffer, DrawCommand, StrokeStyle, TextAnchor};
pub use record::{record_panel, CommandRecorder};
pub use scale::Scale;
pub use viewport::PlotViewport;
