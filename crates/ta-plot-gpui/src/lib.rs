//! Interactive GPUI plates for the scientific plotting engine.
//!
//! `ta-plot` owns the figure description, the coordinate transforms and the
//! renderer; this crate owns the two things that need a window: replaying a
//! recorded frame as GPUI paths, and turning pointer input into viewport
//! changes. The pointer and the renderer read the same layout, so the
//! coordinate a crosshair reports is the one that was drawn.

mod cache;
mod plot;
mod replay;

pub use cache::{build_heatmap, request_key, HeatmapCache, HeatmapData};
pub use plot::{
    DragState, Interaction, PanelScene, Plate, PlotCursor, PlotElement, PlotFrame, PlotGesture,
    PlotState,
};
pub use replay::{paint_buffer, ReplayTransform};

pub use ta_plot::commands::{CommandBuffer, DrawCommand};
pub use ta_plot::render::{HeatmapLayer, PanelLayout, PlottedSeries};
pub use ta_plot::{Scale, PlotViewport};
