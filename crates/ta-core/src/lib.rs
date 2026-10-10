//! Spectra Studio's scientific data layer.
//!
//! This crate holds everything that must be true whether or not a window is
//! open: the dataset object, its physical coordinate axes, the derived-data
//! graph, extraction of traces and spectra, the figure description, and the
//! project that ties them together. It has no user-interface dependency, which
//! is what lets the numerical behaviour be tested on its own (spec §24.19).
//!
//! Two rules run through all of it:
//!
//! 1. The raw measurement is immutable. Everything a user does produces a new
//!    version that names its operation and its inputs.
//! 2. A physical coordinate, not an array index, is what a caller asks for, and
//!    every lookup reports which sampled coordinate it actually used.

pub mod axis;
pub mod dataset;
pub mod error;
pub mod extract;
pub mod figure;
pub mod fitting;
pub mod ids;
pub mod processing;
pub mod project;
pub mod selection;
pub mod units;

pub use axis::{Axis, Interpolation, Ordering, Sample, SampleMode};
pub use dataset::{
    DatasetMetadata, MatrixBlock, SourceFormat, TaDataset, VersionRemoval,
};
pub use error::{CoreError, Result};
pub use extract::{delay_column, kinetic_trace, transient_spectrum, wavelength_row, Spectrum, Trace};
pub use figure::{
    series_references_dataset, series_references_version,
    AnnotationKind, AnnotationSpec, AutoRange, AxisModel, AxisScale, ColorLimits, Colormap,
    ColorbarSpec, ContourSpec, DashPattern, ExportFormat, ExportSpec, FigureModel, FigureSize,
    JournalPreset, LegendPosition, LegendSpec, LengthUnit, MarkerShape, PanelModel, PanelRect,
    PlotSeries, PublicationStyle, Rgba, SeriesKind, SeriesSource, SeriesStyle,
    SeriesTransform, TickDirection,
};
pub use fitting::{
    legacy_parameter_template, FitModelKind, FitParameter, FitResult, FitStats, ParameterPreset,
    ResidualData, Weighting,
};
pub use ids::{
    AnnotationId, ComparisonId, DatasetId, FigureId, FitId, SeriesId, VersionId,
};
pub use processing::{
    AverageSpec, BlendMode, BoundaryMode, BurnCorrection, ControlPoint, CorrectionCurve,
    CorrectionOrigin, CropSpec, MatrixVersion, PreZeroBackground, PreZeroRange, ProcessingOp,
    ReferenceBackground, StitchDelayGrid, StitchSpec, TimeZeroCorrection, VersionKind,
};
pub use project::{
    Alignment, ComparisonWorkspace, Normalization, Project, RemovalReport, PROJECT_SCHEMA,
};
pub use selection::{AxisRequest, SpectrumSelection, TraceSelection};
pub use units::{AxisKind, AxisUnit, SignalUnit};
