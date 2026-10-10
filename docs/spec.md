# Spectra Studio — Technical Specification

**Project:** Spectra Studio  
**Architecture:** Rust-native desktop application  
**Frontend:** GPUI + GPUI Kit  
**Plotting:** Plotters with a custom interactive GPUI wrapper  
**Scientific backend:** Rust  
**Reference implementation:** https://github.com/Alchemist-Aloha/TA_Analyzer  
**Target:** Linux-first, with macOS and Windows support  
**Status:** Implementation specification, v1.1 — Glotaran support is input
generation only (§16); reading Glotaran results is out of scope (§16.6).

---

## 1. Project objective

Develop a native desktop application for ultrafast transient absorption spectroscopy that combines the interactive scientific analysis workflow of Igor Pro with a modern, responsive GUI and publication-quality figure generation.

The application must reproduce all relevant features of the existing TA_Analyzer Python implementation while providing a more coherent, object-oriented user experience.

The principal workflows are:

1. Import and manage multiple transient absorption datasets.
2. Visualize two-dimensional transient absorption matrices.
3. Perform background and time-zero corrections.
4. Extract transient spectra and kinetic traces.
5. Fit kinetic traces using multiexponential models.
6. Average experiments, compare datasets, and stitch visible/NIR data.
7. Prepare input for Glotaran and pyglotaran.
8. Generate publication-quality graphics and numerical exports.

The application is intended for interactive analysis of existing measurements, not real-time instrument acquisition.

### 1.1 Scope

The first complete release must include every functional analysis and visualization capability in the existing Python code.

Global analysis optimization is excluded from the initial release. However, its data structures, import/export facilities, and application architecture must permit implementing it subsequently without a major redesign.

### 1.2 Design principles

- Native Rust implementation with minimal runtime dependencies.
- Python is not required for normal application operation.
- Scientific data must never be silently altered.
- Original experimental files remain unchanged.
- Processing operations are reversible and reproducible.
- GUI interactions must be responsive and predictable.
- Scientific coordinate systems are authoritative; display pixel coordinates are not.
- All plots must support both exploration and publication output.
- A plot must be reproducible from its saved figure configuration.
- The interface prioritizes experimental analysis rather than dashboard-style visualization.

---

## 2. Existing Python functionality migration

The Python repository is the behavioral reference, but not necessarily the definitive implementation of every numerical algorithm.

| Existing Python functionality | Rust module | Required behavior |
|---|---|---|
| `tamatrix_importer` | `ta-io` | Load TA matrices and axes |
| `load_single`, `load_spectra` | `ta-io` | Single and replicate dataset loading |
| `mat_avg` | `ta-processing` | Experiment averaging |
| `auto_bgcorr` | `ta-processing` | Pre-time-zero baseline correction |
| `load_spectra.mat_sub` | `ta-processing` | Background reference subtraction |
| `correct_burn` | `ta-processing` | Photodegradation correction |
| `correction_line.py` | `ta-processing` + GUI | Interactive time-zero curve drawing |
| `auto_tcorr` | `ta-processing` | Wavelength-dependent time-zero correction |
| `fit_correlation`, `auto_tcorr_fit` | `ta-fitting` | Automatic chirp estimation and correction |
| `auto_taspectra`, `get_spectrum` | `ta-core` | Spectral extraction |
| `auto_takinetics`, `get_kinetic` | `ta-core` | Kinetic extraction |
| `multiexp_func`, `params_init` | `ta-fitting` | Multiexponential model and parameters |
| `fit_kinetic`, `plot_fit` | `ta-fitting` + GUI | Kinetic fitting and visualization |
| `compare_traces` | `ta-core` + GUI | Multi-dataset trace comparisons |
| `merge_glotaran` | `ta-processing` + `ta-io` | VIS/NIR dataset merging |
| `glotaran`, `pyglotaran` | `ta-glotaran` | Legacy and modern data export |
| `load_glotaran`, `batch_load_glotaran` | — | Deliberately not ported; see §16.6 |
| `glotaran_output`, `plot_glotaran_output_kinetics` | — | Deliberately not ported; see §16.6 |
| `new_split_axes`, `plot_split_axes` | `ta-plot` | Combined linear/logarithmic time plotting |
| `load_streak_camera_data` | `ta-io` | Streak-camera format handling |
| `gaussian`, `fit_and_plot_gaussian` | `ta-fitting` | Gaussian fitting |
| JACS plotting style helpers | `ta-plot` | Publication presets |
| Matrix/axis/spectra/kinetics save helpers | `ta-io` | Data export |

The new implementation must also include a genuine in-memory matrix-stitching operation. The existing `merge_glotaran` routine primarily merges exported representations rather than providing a complete graphical stitching workflow.

### 2.1 Migration quality requirements

Do not reproduce known limitations in the Python code blindly.

Investigate and correct:

- Implicit assumptions about sorted wavelengths or time points.
- Nearest-index selection near dataset boundaries.
- Logarithms applied to negative delay values.
- Background and spectral extraction methods with missing/default indices.
- Matrix dimensionality and transposition problems.
- Inconsistent single-dataset versus averaged-dataset handling.
- Zero-time interpolation boundary behavior.
- The existing chirp-fit auxiliary function and parameter definitions.
- Fitting parameter identifiability and inconsistent use of weights.

Any intentional numerical change must be documented and tested against the original implementation.

---

## 3. Technology stack

### 3.1 Application layer

Use GPUI as the native frontend framework, with GPUI Kit for reusable interface components.

Required UI systems:

- Dockable and resizable panels.
- File tree and hierarchical item selection.
- Tabbed workspaces.
- Dialogs, context menus, and popovers.
- Editable data tables.
- Property inspectors.
- Keyboard shortcuts and commands.
- Persistent workspace layouts.
- Theme support.
- Background task progress and cancellation.

Use a consistent GPUI Kit design system rather than maintaining a second custom UI framework.

### 3.2 Plotting layer

Use upstream Plotters as the primary scientific plotting engine.

Develop an independent crate, `ta-plot-gpui`, to provide native GPUI interaction.

Investigate `plotters-gpui` and `gpui-plot` for existing backend and interaction implementations. Reuse stable components where practical, but avoid requiring a permanently maintained Plotters fork.

Keep Plotters definitions independent of GPUI application state.

### 3.3 Numerical libraries

Preferred Rust ecosystem:

| Purpose | Preferred technologies |
|---|---|
| Multidimensional numerical arrays | `ndarray` |
| Linear algebra | `faer` and/or `nalgebra` |
| Optimization | `argmin` or a dedicated least-squares solver |
| Parallel processing | `rayon` |
| Serialization | `serde`, `serde_json` |
| Stable IDs | `uuid` |
| CSV handling | `csv` |
| Image buffers | `image` |
| SVG output | Plotters SVG backend |
| PDF conversion | A compatible SVG-to-PDF library |
| Native file dialogs | GPUI Kit/native dialog support |

Evaluate compatibility, solver capabilities, and maintained versions before final dependency selection.

Do not assume that `argmin` provides every algorithm or uncertainty estimate offered by Python's `lmfit`. Implement additional numerical logic where necessary.

---

## 4. Data architecture

### 4.1 Dataset objects

Every imported experimental dataset must be represented by an independent `TaDataset` object.

The object must contain:

- Unique object ID.
- User-editable name.
- Original file paths.
- Experimental metadata.
- Wavelength array.
- Time-delay array.
- Two-dimensional TA matrix.
- Matrix units.
- Source data format.
- Processing history.
- Named derived matrix versions.
- Saved trace and spectrum selections.
- Fitting sessions and results.
- Plot configurations.
- Export history or references.

Use the canonical internal matrix convention:

**Matrix shape: [wavelength, time]**

The matrix entry at row `i`, column `j` is the transient absorption signal measured at wavelength `wavelength[i]` and delay `time[j]`.

Preserve floating-point experimental values in `f64` unless profiling demonstrates a justified alternative. A reduced-precision image representation may be cached independently.

### 4.2 Raw and processed matrices

Every imported object must keep its original matrix immutable.

Represent processing results as a directed acyclic graph of derived datasets.

For example:

Raw → Background corrected → Time-zero corrected → Cropped → Stitched

Each processing node records its operation, parameters, input versions, and resulting data version.

Every operation must support preview, apply, undo, redo, and restore original.

Processing parameter changes should invalidate dependent cached results without overwriting their original source data.

### 4.3 Object independence

A dataset may have multiple extracted traces, spectra, fitted results, and plot configurations.

These are distinct saved objects referencing the relevant dataset version.

A comparison workspace must reference existing datasets rather than copying their entire matrices.

### 4.4 Project persistence

Use a native project format, provisionally `.taproject`.

The project must save:

- Dataset definitions and source references.
- Processing operations and parameters.
- Selected active matrix versions.
- Plot configurations and linked selections.
- Fit models and results.
- Comparison layouts.
- Publication figure specifications.
- UI panel arrangement.

Use versioned serialization with migration support.

Large experimental matrices should be stored in a separate binary data layer or referenced externally, rather than embedded directly into large JSON structures.

Support portable projects with included data and lightweight projects referencing source files. Warn when referenced files are missing and allow relinking.

Autosave should protect project metadata without modifying experimental source files.

---

## 5. Main application layout

### 5.1 Single-dataset workspace

The default application layout consists of:

**Left panel — Dataset explorer**

A hierarchical file and object tree.

**Center — Analysis workspace**

A large TA contour plot, with linked kinetic and spectral plots below.

**Right panel — Context inspector**

Parameters and controls for the currently selected object, processing operation, or plot.

**Top toolbar**

File import, save project, processing operations, fit controls, layout selection, undo/redo, and export.

**Bottom status area**

Dataset information, pointer coordinates, processing status, and numerical notifications.

### 5.2 Dataset explorer hierarchy

Example:

Project
- Sample A — 480 nm excitation
  - Raw matrix
  - Background corrected
  - Time-zero corrected
  - Kinetic traces
    - 550 nm
    - 620 nm
  - Spectra
    - 0.5 ps
    - 10 ps
    - 100 ps
  - Fits
    - 550 nm — 3 exponential
  - Figures
- Sample B — 400 nm excitation
  - Raw matrix
  - Background corrected
  - Selected traces
- VIS + NIR stitched
  - Merged matrix
  - Selected spectra
- Comparisons
  - Sample A vs Sample B
- Exported figures

Operations:

- Single-click selects an object.
- Double-click opens the corresponding workspace or plot.
- Right-click opens context-specific actions.
- Drag an object into a comparison workspace.
- Rename, duplicate, group, hide, and remove objects.
- Indicate whether an object has unapplied changes.
- Preserve stable object references when names change.

Removal from the project must not delete source files without an explicit separate operation.

### 5.3 Main plot arrangement

The default center workspace must use a resizable three-plot layout.

**Top: 2D TA contour**

Occupies approximately 55–65% of the available plotting area.

**Bottom left: kinetic traces**

Time versus ΔA.

**Bottom right: transient spectra**

Wavelength versus ΔA.

All three plots must be linked interactively.

Users can maximize any plot, detach it to another window, or switch to alternate layouts.

### 5.4 Context inspector

The right inspector changes based on the active tool.

Contexts include:

- Dataset information.
- Background correction.
- Time-zero correction.
- Trace selection.
- Spectrum selection.
- Stitching.
- Fit configuration.
- Plot style.
- Figure export.

Use progressive disclosure: common operations remain directly accessible, while specialized settings appear in expandable sections.

Avoid large parameter tables occupying the default workspace.

---

## 6. Interactive scientific plotting wrapper

### 6.1 Objective

Create a reusable `InteractivePlot` abstraction that converts Plotters from a primarily rendering-oriented library into an interactive native GPUI component.

The wrapper must support XY plots, scatter plots, heatmaps, contours, and multi-panel figures.

It must not contain TA-specific processing logic.

### 6.2 Architecture

Implement four layers:

**Figure model**

Serializable description of plotted datasets, series, axes, styling, annotations, and layout.

**Coordinate and layout engine**

Maps physical scientific coordinates to plot coordinates, calculates axes and ticks, and establishes clipping and margins.

**Rendering backend**

Uses Plotters to render vector primitives and handles heatmap rasterization.

**Interaction controller**

Translates GPUI pointer/keyboard events into viewport changes, selections, and figure-edit commands.

Plotters should not directly own persistent application data.

### 6.3 Required interactions

All plots must support:

- Mouse-wheel zoom centered on the pointer.
- Click-and-drag panning.
- Box zoom.
- Fit-to-data.
- Reset viewport.
- X-only and Y-only zoom.
- Aspect locking where relevant.
- Crosshair and coordinate tooltip.
- Nearest-data-point selection.
- Line and series selection.
- Range selection.
- Rectangular ROI selection.
- Context menus.
- Copy plot and export actions.
- Undo/redo for editing operations.

Implement keyboard shortcuts for common commands.

### 6.4 Scientific coordinate system

Implement shared axis transforms for:

- Linear coordinates.
- Logarithmic coordinates.
- Symmetric-logarithmic coordinates.
- Piecewise linear/logarithmic coordinates.
- Reversed axes.
- Custom calibrated physical coordinates.

Transient absorption delay points are frequently nonuniform and include negative time.

Do not map the time dimension as a uniformly spaced index unless the user explicitly selects an index-based view.

Coordinate transforms must be reversible and shared between display, mouse hit testing, ROI selection, and export.

### 6.5 Plot rendering

For XY curves, use Plotters vector drawing primitives.

For large TA heatmaps, use an image-backed layer with a cached color-mapped bitmap. Do not generate millions of individual rectangular drawing primitives.

Implement viewport-aware resampling for nonuniform coordinates.

During drag interactions, reuse cached content where possible. Recalculate only when the visible data or color mapping changes.

Support true contour isolines and filled contour levels independently of the heatmap display.

Contour extraction can use a marching-squares implementation with appropriate handling of masked values.

### 6.6 Reusable API

The internal application API should include:

- `InteractivePlot`
- `InteractiveHeatmap`
- `InteractiveLinePlot`
- `FigureModel`
- `AxisModel`
- `PlotSeries`
- `PlotViewport`
- `PlotSelection`
- `PlotAnnotation`
- `PlotInteractionController`
- `PublicationStyle`

All plot widgets share the same data-binding and figure-export interfaces.

### 6.7 Linking plots

Build an application-level selection bus.

Selecting a wavelength on the contour updates the kinetic trace. Selecting a delay updates the spectrum.

Selecting a trace or spectrum from the corresponding pane updates its marker on the contour.

Support multiple persistent cursor/selection lines with independent names and colors.

Prevent circular update events between linked views.

---

## 7. TA contour viewer

### 7.1 Required features

Display a heatmap or filled contour of ΔA as a function of wavelength and delay.

Default axes:

- X: wavelength in nm.
- Y: delay in ps.
- Color: ΔA or ΔOD.

Controls:

- Change matrix version.
- Adjust wavelength range.
- Adjust delay range.
- Set color limits.
- Select colormap.
- Set colorbar range manually.
- Use symmetric color limits around zero.
- Show contour lines.
- Edit contour levels.
- Invert colormap.
- Apply percentile-based automatic color scaling.
- Inspect exact values.
- Display processing overlays.

Support color-map presets suitable for signed TA data, including diverging palettes.

### 7.2 Contour interactions

Clicking on a point updates both selected time and wavelength.

Vertical cursor lines identify kinetic extraction wavelengths.

Horizontal cursor lines identify spectral extraction delays.

Multiple cursors can coexist.

Allow rectangular ROI selection for averaging, inspection, and subsequent analysis.

### 7.3 Colorbar

Colorbar controls must include:

- Minimum and maximum.
- Symmetric range.
- Automatic range.
- Number of ticks.
- Display units.
- Palette reversal.
- Publication styling.

Adjusting color limits must not change the underlying data.

---

## 8. Background subtraction

Implement a processing module with a dedicated inspector.

### 8.1 Pre-time-zero subtraction

Calculate an average reference spectrum over a selected negative-delay interval and subtract it from every delay column.

The default mode reproduces the existing Python approach using the first N time points.

Also support selecting an explicit time interval.

Show the chosen baseline region on the contour and a plot of the extracted background spectrum.

### 8.2 External reference subtraction

Allow another TA dataset to be used as the reference background.

Support:

- Multiplicative subtraction factor.
- Optional scale normalization.
- Wavelength matching.
- Time-axis matching.
- Interpolation if coordinates differ.
- Preview of the resulting matrix.

Never silently subtract arrays with incompatible axes.

### 8.3 Operation controls

Provide Preview, Apply, Reset, and Cancel actions.

The output becomes a derived dataset version. The original remains available.

---

## 9. Time-zero and chirp correction

### 9.1 Manual correction

Implement a correction editing mode directly on the contour plot.

The user can:

- Zoom into the time-zero region.
- Place control points along the signal onset.
- Drag individual points.
- Insert or delete points.
- Undo point edits.
- Display a connected correction curve.
- Fit or interpolate the correction curve.
- Preview the corrected contour.
- Apply the correction.
- Save and load correction-point files.

Control points must use physical wavelength/time coordinates.

Display both measured points and the interpolated correction curve.

### 9.2 Numerical correction

For each wavelength, determine the time-zero offset from the correction curve and resample the kinetic trace onto the common corrected time grid.

The initial implementation must reproduce the existing Python interpolation sign convention and numerical behavior.

Offer selectable interpolation methods:

- Linear, default.
- Cubic or shape-preserving interpolation where numerically appropriate.

Boundary behavior must be explicit: clamp, mask, or extrapolate. Avoid silent extrapolation artifacts.

Preserve original time and wavelength axes.

### 9.3 Automatic time-zero estimation

Port the existing cross-correlation fitting workflow.

Fit selected kinetic traces across wavelength to estimate time zero, reject poor or low-signal fits, and construct a wavelength-dependent correction curve.

The automatic result must open in the same correction editor used for manual processing.

Users must be able to inspect and adjust the curve before applying it.

Automatic correction must not overwrite manual correction data.

### 9.4 Validation

The corrected matrix must preserve shape and wavelength coordinates.

Verify that an artificial known wavelength-dependent chirp is recovered within the interpolation tolerance.

Compare the corrected matrix against the Python implementation on repository test data.

---

## 10. Kinetic trace extraction

### 10.1 Selection

Users can enter one or more wavelengths or select them directly from the contour.

Provide:

- Nearest sampled wavelength.
- Interpolated wavelength.
- Averaged spectral interval.
- Averaged N-channel window.

Display the actual sampled or averaged wavelength range in plot labels.

### 10.2 Plot features

- Multiple overlaid traces.
- Independent visibility.
- Individual colors and styles.
- Linear, log, symlog, and split linear/log time axes.
- Time-range selection.
- Normalization options.
- Zero-reference line.
- Fit overlay.
- Residual overlay.
- Cursor measurement.
- Export data and figure.

Preserve negative delays on supported axis types.

### 10.3 Saved selections

Each selected kinetic trace becomes a reusable plot-series definition referencing a dataset and data version.

Changing dataset processing invalidates the cached extracted values and triggers a recalculation.

---

## 11. Transient spectrum extraction

### 11.1 Selection

Allow selection of one or more delay points by text entry or direct contour interaction.

Selection modes:

- Nearest measured delay.
- Interpolated delay.
- Averaged time window.
- Averaged N-time-point window.

### 11.2 Plot features

- Multiple spectra on one axis.
- Per-spectrum color, style, and visibility.
- Wavelength cropping.
- Signal normalization.
- Zero-reference line.
- Spectral comparison.
- Legend customization.
- Publication styling.

Each legend entry should initially show the actual sampled delay and its unit.

### 11.3 Interactive linking

Dragging a horizontal delay-selection line on the contour must update the associated transient spectrum.

Continuous movement should feel responsive without unnecessary full-matrix recalculation.

---

## 12. Multiexponential fitting module

### 12.1 Module organization

Develop a standalone, reusable fitting component named `FitWorkbench`.

It must be accessible from:

- Single-dataset kinetic traces.
- Multi-object kinetic comparison.
- Imported kinetic files.
- Future global-analysis workflows.

The fitting module must not require the contour viewer.

### 12.2 Fit models

Initially implement the existing Python multiexponential model with one to four exponential components.

The legacy model includes:

- Exponential amplitudes.
- Exponential lifetimes.
- Gaussian-CDF instrument-response onset.
- IRF width.
- Time-zero offset.
- Pre-zero baseline offset.
- Long-term step-like offset.

Preserve the existing model as a specifically named compatibility model.

The current Python expression multiplies exponential terms by a Gaussian cumulative distribution. This is not mathematically identical to exact Gaussian convolution of causal exponential decays.

Therefore, expose the legacy formulation explicitly. A physically convolved exponential/IRF model may be implemented as an additional selectable model, with independent validation.

### 12.3 Fitting interface

Provide a reusable dockable panel or independent window.

Layout:

**Left:** model selection and parameters.

**Center:** experimental data and fitted curve.

**Bottom:** residuals.

**Right:** fitting results and statistics.

Parameters must support:

- Initial value.
- Lower and upper bounds.
- Fixed/free state.
- Units.
- Reuse of values from previous fits.

### 12.4 Fitting controls

- Select 1–4 exponential components.
- Set IRF width.
- Set time-zero.
- Configure baseline and long-lived offset.
- Choose fit range.
- Exclude selected time intervals.
- Apply point weights.
- Choose wavelength averaging range.
- Select supported optimizer.
- Start, cancel, and rerun fitting.
- Save parameter presets.
- Apply previous fit values to another trace.

Fit jobs must run outside the GPUI UI thread.

### 12.5 Fit results

Display:

- Lifetime and amplitude estimates.
- Parameter standard errors where identifiable.
- Parameter bounds.
- Reduced chi-square.
- Sum of squared residuals.
- R², where useful.
- Correlation diagnostics.
- Parameter covariance/correlation matrix when available.
- Solver convergence status.
- Residual plot.

Do not invent standard errors when the solver cannot estimate them reliably.

### 12.6 Fit visualization

Support the existing linear/log split-axis convention.

Show experimental points and fitted curves using independently configurable styles.

Support multiple fitted wavelengths in a shared figure and multiple retained fit sessions.

### 12.7 Data storage

Store fitted parameters, model type, optimizer settings, bounds, weighting masks, residuals, predicted values, covariance information, and source dataset version.

A fit must be reproducible after reopening the project.

---

## 13. Experiment averaging and photodegradation correction

### 13.1 Multiple experiment averaging

Allow loading repeated measurements as a dataset group.

The user selects experiments to include or exclude.

Support:

- Mean matrix.
- Standard deviation matrix.
- Standard error matrix.
- Individual experiment visualization.
- Average spectrum or kinetic trace.
- Comparison with individual repetitions.

Validate coordinate compatibility before averaging.

### 13.2 Photodegradation correction

Port the existing sample-burning correction workflow.

Provide an interactive reference wavelength selection and display the correction factor across experiment progression.

The existing heuristic correction should remain available for compatibility, but be labeled accordingly.

Users must inspect the estimated correction before applying it.

Support comparison of corrected and uncorrected matrices.

---

## 14. Visible/NIR matrix stitching

### 14.1 Stitching workspace

Provide an independent stitching workspace that accepts two datasets:

- Visible-region TA matrix.
- NIR-region TA matrix.

The interface displays the original matrices, selected spectral overlap, and proposed stitched result.

### 14.2 Requirements

- User-selectable VIS maximum wavelength.
- User-selectable NIR minimum wavelength.
- Detect overlapping spectral regions.
- Detect gaps.
- Validate delay-axis compatibility.
- Support explicit time interpolation if necessary.
- Optional scale matching in the overlap region.
- Optional smooth blending.
- Preview stitched spectra at multiple delay points.

For overlapping regions, support a configurable blend weighting between the visible and NIR signals.

Never automatically force intensity matching without making the operation visible to the user.

### 14.3 Output

Create a new derived TA dataset with an explicit wavelength axis, time axis, and merged matrix.

Preserve references to the input datasets and all stitching parameters.

The result must behave like any other dataset in the application.

It must support further processing, fitting, contour plotting, and Glotaran export.

---

## 15. Multi-object comparison workspace

### 15.1 Purpose

Provide a separate analysis layout specifically for comparing multiple TA datasets.

This workspace must be independent of the single-dataset workspace.

### 15.2 Default layout

**Left comparison pane:** transient spectra.

**Right comparison pane:** kinetic traces.

The panes are independently resizable and can be maximized or detached into separate native windows.

A compact dataset-selection toolbar appears above them.

### 15.3 Dataset selection

Users may select any combination of loaded objects.

Each plotted series references:

- Dataset ID.
- Matrix version.
- Wavelength or delay selection.
- Extraction method.
- Display label.
- Plot style.

For example, one comparison can display kinetics from three different samples, each at a different wavelength.

### 15.4 Comparison tools

Provide:

- Overlay and individual visibility.
- Normalize to positive maximum.
- Normalize to absolute maximum.
- Normalize by a chosen reference point.
- Manual scaling.
- Vertical offset.
- Time offset for display.
- Common and independent axes.
- Linked cursors.
- User-editable legend labels.

Traces from datasets with different time axes must retain their native coordinates for overlay plotting.

Interpolation is required only when performing mathematical operations that need common sampling.

### 15.5 Comparison persistence

Users can save named comparison workspaces.

Comparison settings must survive application restart and project reload.

Each comparison plot must support the publication editor and numerical data export.

---

## 16. Glotaran integration

### 16.1 Scope

The application prepares and writes Glotaran input. It does not read Glotaran
results, and it does not perform global analysis.

Glotaran is a *destination*: a matrix is corrected here, exported in a format
Glotaran reads, and fitted there. Reading its output back would mean maintaining
a second result format, a second parameter vocabulary and a second set of
uncertainty conventions for a question the application already answers with its
own kinetic fitting (§12). Anything a result file could show is therefore either
visible here already, or exported as numbers (§18) and inspected where the
analysis was run.

### 16.2 Legacy Glotaran export

Support the existing time-explicit ASCII format.

Export:

- Dataset identification.
- Time points.
- Wavelength points.
- Matrix values in the required orientation.
- The header lines, the separator and the numeric formatting the format
  expects.

The export offers the processed matrix version currently on the bench, and names
in the file which version it wrote.

Validation is by re-import: the file the exporter writes must be readable by
this application's own importer, yielding the same matrix and the same axes it
was written from.

### 16.3 pyglotaran export

Support the modern time/spectral dataset convention used by pyglotaran.

The internal representation maps to:

- `time`
- `spectral`
- `data`

where `data` is the matrix with time along its first axis, which is the
transpose of the internal wavelength-major storage.

Support a compatible serialized dataset format, with NetCDF as a candidate, and
verify interoperability with the targeted pyglotaran version before the release
is called finished.

### 16.4 Version selection

Either export writes the version that is on the bench, and the exported file
records it: the dataset name, the version's label, and the operation that
produced it. A user comparing two exports can tell which was which without
opening the project.

### 16.5 Export paths

Both formats are written beside the project, or beside the imported file, and
both are available from the contour plate's own actions, so preparing input does
not require opening another workspace.

### 16.6 Deliberately not ported

The reference implementation's result-side features are out of scope, and are
listed here so that the omission is a decision rather than a gap:

- `load_glotaran` and `batch_load_glotaran`: reading a Glotaran result file or a
  folder of them.
- `glotaran_output`: decay-associated spectra and reconstructed traces.
- `plot_glotaran_output_kinetics`: measured-against-fitted kinetic overlays from
  a Glotaran run.

The decay-associated spectra and the fitted curves of *this* application's own
fits remain available: a fit's record holds its parameters, its prediction and
its residuals (§12.7), and both a fit and an arbitrary curve can be drawn in any
figure through `SeriesSource::Fit` and `SeriesSource::Curve` (§17.4).

### 16.7 Future expansion

Reserve interfaces for global-analysis models, parameter sets, fit jobs and
results, so a future global analysis could write into the same structures.

Do not implement an incomplete global-analysis solver merely to satisfy the
interface.

---

## 17. Publication-quality figure editor

### 17.1 Objective

Create a standalone `FigureStudio` module.

It must be possible to launch the module from **every interactive plot** in the application.

This includes:

- TA contours.
- Kinetic plots.
- Transient spectra.
- Comparison plots.
- Fitting results.
- Residual plots.
- Time-zero correction diagnostics.
- Stitching previews.

### 17.2 Figure editor UI

Open a separate GPUI window or dockable editor panel.

Recommended layout:

**Left:** figure structure and selected series.

**Center:** live publication figure preview.

**Right:** figure and element properties.

**Bottom:** export actions.

The preview must share the same saved figure model as the original interactive plot.

### 17.3 Figure dimensions

Provide physical dimensions with units:

- Inches.
- Centimeters.
- Millimeters.
- Pixels for raster export.

Include presets for common journal single-column and double-column figures.

Use the existing JACS presets as initial examples:

- Single-column width: 3.25 inches.
- Double-column width: 7.0 inches.
- Default scientific font size: 8 pt.

Allow freely editable dimensions and automatic figure-height adjustment.

### 17.4 Per-series controls

Each curve must provide:

- Include/exclude.
- Legend label.
- Line color.
- Line thickness in points.
- Solid, dashed, dotted, or custom dash style.
- Marker type.
- Marker size.
- Marker outline.
- Marker fill.
- Transparency.
- Plotting order.

Plot styles should be editable by clicking the corresponding plot object or choosing it from the figure tree.

### 17.5 Axis controls

Provide:

- Axis label.
- Label font and size.
- Tick-label font and size.
- Minimum and maximum.
- Linear/log/symlog scale.
- Major/minor tick spacing.
- Automatic or manual ticks.
- Tick direction.
- Tick length and width.
- Axis spine thickness.
- Axis visibility.
- Mirror top/right axes.
- Grid visibility.
- Major/minor grid styles.
- Scientific notation.
- Number precision.
- Unit formatting.
- Reversed axis direction.

Mirrored axes should support top/right spines and ticks without necessarily duplicating axis labels.

### 17.6 Legend controls

- Show/hide.
- Custom text per series.
- Position.
- Number of columns.
- Font and size.
- Line sample width.
- Spacing.
- Border.
- Background.
- Transparency.

Allow dragging legends within the figure preview.

### 17.7 Text and annotations

Support:

- Plain text annotations.
- Superscripts and subscripts.
- Greek symbols.
- Mathematical expressions where supported.
- Arrows.
- Rectangles.
- Lines.
- Markers.
- Panel labels.
- Reference lines.

Text positions should be saved in explicit plot or figure coordinate systems.

### 17.8 Multi-panel figures

Users must be able to assemble multiple saved plots into one publication figure.

Support:

- Horizontal and vertical arrangements.
- Arbitrary grid layouts.
- Shared axes.
- Synchronized axis limits.
- Adjustable spacing.
- Panel labels such as A, B, C.
- Consistent fonts and line weights.
- Optional common legends and colorbars.

### 17.9 Contour publication controls

In addition to ordinary plot settings:

- Colormap.
- Color limits.
- Number and values of contour levels.
- Colorbar title and position.
- Colorbar tick formatting.
- Display/omit crosshair lines.
- Display/omit correction curves.
- Rasterization resolution.

### 17.10 Export formats

**Default: SVG**

Additional options:

- PDF.
- PNG.
- TIFF, if supported by the selected image library.
- JPEG for convenience, not as the preferred scientific format.

For SVG/PDF, maintain vector axes, text, curves, and annotations whenever possible.

Large heatmap layers may be raster-embedded at a specified effective resolution.

Allow 300, 600, and 1200 DPI raster presets.

A high DPI value must not be presented as meaningful for purely vector geometry.

### 17.11 Export consistency

Exported figures must use the same axis limits, selected curves, annotations, colors, and styling as the editor preview.

The export implementation must share coordinate and layout definitions with the interactive renderer.

Perform clipping, font-layout, and bounding-box checks before exporting.

Avoid cropped labels, missing legend entries, and unexpected margins.

### 17.12 Figure styles

Provide reusable presets:

- General scientific.
- ACS/JACS-style.
- Nature-style.
- IEEE-style.
- Presentation.
- User-defined presets.

These are configurable templates, not claims of automatic compliance with every journal's current artwork requirements.

Save user presets independently of projects.

---

## 18. Universal numerical data export

Every interactive plot must provide a standard **Export Data** action alongside Export Figure.

For line plots:

- Export the displayed X coordinates.
- Export the full-resolution underlying Y values.
- Include series names, units, and data provenance.
- Support CSV and tab-delimited text.

For spectra and kinetics:

- Export selected experimental coordinates.
- Include requested and actual sampled wavelength/time.
- Include averaging/interpolation information.
- Support multiple series in one export.

For TA contours:

- Export matrix values.
- Export wavelength and delay axes.
- Support separate axis files and combined matrix formats.
- Preserve NaN/masked values explicitly.

For fitting:

- Export measured data.
- Export model prediction.
- Export residual.
- Export parameter table.
- Export model metadata.

For comparisons:

- Support common-axis tables where appropriate.
- Otherwise export per-series coordinate columns without silently interpolating.

Provide a choice between exporting the full selected data or only values within the visible plotted range.

Displayed decimation must never reduce the scientific resolution of an ordinary data export.

All numerical files must be self-describing and reproducible.

---

## 19. Application crate architecture

Organize the Rust workspace into separate crates or equivalent clearly isolated modules.

**`ta-core`**

Scientific dataset types, metadata, object identity, unit definitions, coordinate handling, derived-data graph.

**`ta-io`**

Experimental data import, text/CSV export, streak-camera import, native project storage.

**`ta-processing`**

Background correction, chirp correction, averaging, photodegradation correction, stitching, resampling, masking.

**`ta-fitting`**

Gaussian fitting, multiexponential models, optimization, fit parameter structures, uncertainty reporting.

**`ta-glotaran`**

Legacy and pyglotaran-compatible input generation: the dataset view that both
writers consume, the time-explicit ASCII writer and the NetCDF writer. Export
only; see §16.1.

**`ta-plot`**

Plot series, figure model, scientific coordinate transforms, style definitions, Plotters render orchestration, SVG/PDF export.

**`ta-plot-gpui`**

Interactive GPUI plot wrapper, event handling, hit testing, selection, zoom/pan, rendering and caching integration.

**`ta-gui`**

Application entry point, dataset explorer, workspaces, GPUI Kit integration, inspectors, dialogs, FigureStudio, FitWorkbench.

**`ta-tests`**

Reference-data regression tests, numerical correctness tests, import/export tests, UI integration tests, and performance benchmarks.

Maintain clear boundaries so the scientific backend can be tested without initializing GPUI.

---

## 20. State management and background tasks

The application must distinguish:

- Persistent scientific data state.
- Processing operation state.
- Plot configuration state.
- Transient interaction state.
- UI layout state.

Long-running fitting, parsing, stitching, and export tasks must run asynchronously from the UI's perspective using background worker execution.

Provide task cancellation and progress information.

A cancelled operation must not leave partially applied changes.

Use version identifiers to prevent a stale background result from overwriting newer user changes.

Avoid copying complete TA matrices on every cursor or viewport update.

Cache rendered images and extracted curves using source-version and configuration keys.

---

## 21. Performance requirements

Use a representative benchmark dataset containing approximately 2,048 wavelengths and 1,000 delay points.

Initial performance objectives:

| Operation | Target |
|---|---|
| Hover and crosshair movement | Responsive at display refresh rates |
| Pan/zoom with cached imagery | Smooth interaction |
| Extract one kinetic trace | Under 50 ms |
| Extract one transient spectrum | Under 50 ms |
| Recalculate heatmap colormap | Under 200 ms |
| Load typical text TA dataset | Under 3 seconds on reference hardware |
| Background correction | Under 250 ms |
| Basic 1–4 component fit | No UI blocking |
| SVG figure export | No UI blocking |

These are engineering acceptance targets, not guaranteed performance claims.

Benchmark on representative Linux hardware and document test conditions.

Loading and analysis operations must provide progress or visible feedback when necessary.

---

## 22. Scientific correctness and testing

### 22.1 Python parity tests

Use the existing repository data files as reference fixtures.

Generate reference outputs from the Python implementation for:

- Data import.
- Background subtraction.
- Time-zero correction.
- Spectral extraction.
- Kinetic extraction.
- Multiexponential model evaluation.
- Fitting.
- Experiment averaging.
- Glotaran export.

Compare Rust outputs against Python with operation-appropriate tolerances.

Where an original algorithm is corrected, keep a reference test demonstrating the difference.

### 22.2 Synthetic tests

Create synthetic TA datasets with known parameters:

- Known baseline offset.
- Known exponential lifetimes.
- Known time-zero shift.
- Wavelength-dependent chirp.
- Overlapping visible/NIR coverage.
- Different time grids.
- Missing or masked values.
- Known Gaussian IRF.

Verify parameter recovery and correction accuracy.

### 22.3 Coordinate correctness

Test ascending and descending spectral axes, uneven delay sampling, negative delays, logarithmic scale boundaries, duplicate coordinates, and NaN data.

Prevent silent data reordering.

### 22.4 Plot export tests

Compare screen and exported plots for axis ranges, labels, legend entries, clipping, and series visibility.

Use image-based regression testing where possible.

Validate that SVG files open correctly in standard vector editors and that PDF dimensions match the requested physical size.

### 22.5 UI integration tests

Test complete user workflows:

- Import dataset.
- Apply correction.
- Select a wavelength.
- Select a delay.
- Fit a trace.
- Compare multiple objects.
- Export a publication figure.
- Save and reopen the project.

Include keyboard and mouse-driven interaction tests.

---

## 23. Implementation phases

### Phase 1 — Foundation

- Establish Rust workspace and GPUI Kit application.
- Create canonical dataset and project models.
- Implement native file import.
- Build the dataset explorer.
- Display initial TA contour and 1D plots.
- Implement save/load of project metadata.

**Exit condition:** Multiple TA files can be imported, inspected, and restored after application restart.

### Phase 2 — Interactive plotting engine

- Integrate Plotters.
- Build `InteractivePlot` and `InteractiveHeatmap`.
- Implement physical coordinate mapping.
- Implement zoom, pan, crosshair, and hit testing.
- Link the contour to kinetics and spectra.
- Introduce consistent plot styling and SVG output.

**Exit condition:** Interactive exploration is functional and scientifically coordinate-correct without Python.

### Phase 3 — Processing

- Background subtraction.
- Manual chirp correction.
- Automatic time-zero correction.
- Experiment averaging.
- Reference subtraction.
- Photodegradation correction.
- VIS/NIR stitching.

**Exit condition:** Core processing passes Python parity and synthetic-data tests.

### Phase 4 — Kinetic fitting

- Implement multiexponential model.
- Implement solver and fitting constraints.
- Build reusable FitWorkbench.
- Implement residual plots.
- Persist fit results and presets.
- Add fit diagnostics and export.

**Exit condition:** Representative Python fit workflows can be reproduced in the GUI.

### Phase 5 — Comparison and advanced visualization

- Multi-object comparison workspace.
- Dataset-specific selections.
- Per-trace and per-spectrum styles.
- Normalization and alignment options.
- Split-axis plotting.

**Exit condition:** Multiple datasets can be compared and plotted without changing or duplicating the underlying data.

### Phase 6 — FigureStudio

- Build publication figure editor.
- Implement full styling controls.
- Implement multi-panel figure composition.
- SVG/PDF/raster export.
- Universal plot data export.
- Reusable figure presets.

**Exit condition:** Every plot can be converted into a publication-quality figure and exported with its underlying numerical data.

### Phase 7 — Glotaran input and remaining parity

- Legacy and pyglotaran-compatible export.
- Streak-camera data import.
- Remaining Gaussian and helper functionality.
- Complete feature-parity audit that names every Python entry point and where
  it now lives, including the result-side features that are deliberately not
  ported (§16.6).

**Exit condition:** Every workflow the application claims — import, processing,
extraction, fitting, comparison, stitching, figure production and Glotaran input
generation — is available without invoking Python.

### Phase 8 — Polish and release

- UI consistency review.
- Usability testing.
- Performance profiling.
- Memory optimization.
- File and data integrity validation.
- Linux packaging.
- Cross-platform build testing.
- Documentation and sample project.
- Automated regression testing.

---

## 24. Acceptance criteria

The project is considered feature-complete when all of the following are true:

1. A user can import a TA matrix and its physical coordinate axes as one independent dataset object.
2. Multiple datasets can coexist without affecting one another.
3. The application displays a scientifically correct interactive contour.
4. Clicking the contour extracts kinetics and spectra into linked plots.
5. Background correction is interactive and non-destructive.
6. Time-zero correction supports editing points directly on the contour.
7. Automatic time-zero estimation and correction are implemented.
8. One- through four-exponential fitting is supported and its numerical results are inspectable.
9. Experimental datasets can be averaged and corrected for photodegradation.
10. Visible and NIR matrices can be stitched into a new dataset.
11. Multiple dataset objects can be compared in dedicated spectral and kinetic panes.
12. Legacy Glotaran and pyglotaran-compatible data preparation are supported.
13. Existing Glotaran results can be imported and visualized.
14. Every displayed scientific plot can be opened in FigureStudio.
15. FigureStudio supports professional typography, trace styling, axis configuration, legends, annotations, and physical figure dimensions.
16. SVG is the default output, with PDF and appropriate raster formats available.
17. Every figure can export its underlying numerical data as CSV or text.
18. Projects preserve corrections, selected traces, fitting results, comparison workspaces, and figure styles.
19. All scientific backend operations are testable without GPUI.
20. The complete application works without a Python runtime.

---

## 25. Future development

The architecture should permit later addition of:

- Global multiexponential analysis.
- Target analysis with kinetic models.
- Species-associated spectra.
- Decay-associated spectra calculated natively.
- SVD and dimensionality reduction.
- IRF-aware global fitting.
- Spectral smoothing and denoising.
- Batch processing pipelines.
- Scientific plugin support.
- Python scripting interoperability.
- Native acquisition instrument interfaces.

These extensions must not delay completion of the initial feature-parity release.

---

## 26. Development instructions

Implementation should prioritize scientific correctness, modularity, and an intuitive workflow.

Do not attempt to replace Plotters with an entirely custom plotting renderer unless specific performance or functionality requirements cannot be met.

Do not mix numerical processing logic into GPUI view code.

Do not treat GUI screenshots as scientific data.

Do not save only a rendered image when a reproducible figure specification and underlying data can be preserved.

Use the existing Python code as the primary migration reference. Port actual behavior, validate it, and document numerical improvements.

Build complete, testable vertical slices rather than implementing every UI screen before its backend functionality exists.

The ultimate result should function as an extensible native scientific analysis environment, not merely a graphical wrapper around the existing notebook.