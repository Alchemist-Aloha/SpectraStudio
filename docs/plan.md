# Spectra Studio — implementation plan

Source of truth for behaviour is [`spec.md`](spec.md) (v1.0) plus [`../PRODUCT.md`](../PRODUCT.md).
This file adds only what the spec does not: the order of work, the exit criteria that
prove each step, and the two deliberate architectural decisions listed at the bottom.

## Shape of the work

Nine milestones. Each one is a complete vertical slice: data structure, algorithm,
interface, and the test that proves it. Nothing later is allowed to require a
redesign of something earlier.

**MVP = M0–M3.** After M3 a scientist can import a real TA matrix, see a
scientifically correct interactive contour, click it, get linked kinetics and
spectra, and export a figure. Everything after M3 adds capability in front of
that spine, or replaces a placeholder with the real engine.

| # | Milestone | Spec | Crates | Exit criteria |
|---|---|---|---|---|
| M0 | Workspace and toolchain | §19 | all | `cargo test --workspace` green; every crate in §19 exists with a boundary that compiles without GPUI. |
| M1 | Data layer and persistence | §4, §22.1 | ta-core, ta-io, ta-tests | The real NIR800 fixture imports into `TaDataset`; shape, axes and matrix values verified against the Python reference output; project saves and reopens with bit-identical matrices; a derived version round-trips through the DAG. |
| M2 | Interactive plotting engine | §6, §7 | ta-plot, ta-plot-gpui | `FigureModel` renders to SVG via Plotters and to a live GPUI element; pointer-anchored zoom, pan, box zoom, crosshair, nearest-point hit test, ROI selection work against synthetic data; heatmap paints from a cached bitmap, not per-cell quads; export ranges equal screen ranges. |
| M3 | Application shell, explorer, linked views | §5, §6.7, §10, §11 | ta-gui (+ all) | Dockable panel layout persists across restarts; dataset tree shows the §5.2 hierarchy; selecting a wavelength/delay on the contour updates the kinetic and spectral panes and vice versa without feedback loops; extraction honours nearest/interpolated/averaged modes; 2048×1000 dataset stays interactive. **MVP ends here.** |
| M4 | Processing | §8, §9, §13, §14 | ta-processing, ta-gui | Pre-zero and external-reference background subtraction; manual chirp editing on the contour; automatic time-zero estimation landing in the same editor; averaging; photodegradation correction; VIS/NIR stitching produces a real stitched in-memory matrix. Each op: preview, apply, undo, reset; each op tested against synthetic data with known truth, and against Python where a reference exists. |
| M5 | Kinetic fitting | §12 | ta-fitting, ta-gui | `FitWorkbench` is usable from a trace, a comparison, and an imported kinetic file, and needs no contour. One to four exponentials, Gaussian-CDF IRF, legacy model preserved by name, converged fits with standard errors only where identifiable, residuals, presets, fits persisted in the project. Verified against Python `fit_kinetic` on the fixture. |
| M6 | Comparison and advanced visualization | §15, §10.2, §16.5 | ta-gui, ta-plot | Multi-object comparison workspace referencing datasets without copying matrices; per-series style, normalization, alignment; split linear/logarithmic axes; Glotaran DAS and fitted-trace overlays. |
| M7 | FigureStudio and universal data export | §17, §18 | ta-plot, ta-gui | Every interactive plot opens in FigureStudio on the same saved figure model; physical dimensions with unit conversion and journal presets; per-series, axis, legend, annotation and multi-panel controls; SVG default with PDF and raster; every figure exports its underlying numbers as CSV or text. |
| M8 | Glotaran integration and parity audit | §16, §2 | ta-glotaran, ta-tests | Legacy `.ascii` and pyglotaran-compatible export; Glotaran result import (single and batch); streak-camera import; Gaussian helpers; a written feature-parity audit naming every Python feature and where it now lives. |
| M9 | Polish and release | §21, §23.8, §24 | all | Performance targets of §21 measured and recorded; memory profiled; Linux package built and launched; macOS/Windows build checked; sample project and user documentation; §24 acceptance list walked item by item with evidence. |

## Ordering rules

- A milestone is done when its exit criteria run, not when its code is written.
- Numerical behaviour follows the Python implementation; every intentional
  divergence is named in a test and in `docs/numerics.md`.
- Placeholders are allowed only where the interface they sit behind already
  exists, and each carries a `shortcut:` comment naming what replaces it.

## Deliberate deviations from the spec

1. **Where the figure and fit *data* types live.** §19 gives the figure model to
   `ta-plot` and fit results to `ta-fitting`, while §4.1 requires a dataset to
   contain its plot configurations and fitting results. Those two cannot both
   hold without a dependency cycle. Resolution: `ta-core` owns the serializable
   data (`FigureModel`, `AxisModel`, `PlotSeries`, `FitResult`), `ta-plot` owns
   transforms and Plotters rendering for it, `ta-fitting` owns the model
   evaluation and solver. No cycle, and §4.1 is satisfied literally.
2. **One drawing description, two backends.** Rather than forking Plotters to
   reach the screen, a Plotters `DrawingBackend` records primitives into a
   command buffer. SVG, PNG and PDF export replay it through Plotters' own
   backends; `ta-plot-gpui` replays it through GPUI paths, and heatmaps through a
   cached bitmap. Screen and paper therefore cannot drift apart.
