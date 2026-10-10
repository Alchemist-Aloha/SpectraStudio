# Product

<!-- impeccable:product-schema 1 -->

## Platform

adaptive

## Stack
Rust workspace with a GPUI + GPUI Kit native frontend and Plotters as the scientific plotting engine behind a custom interactive wrapper. Crates: `ta-core`, `ta-io`, `ta-processing`, `ta-fitting`, `ta-glotaran`, `ta-plot`, `ta-plot-gpui`, `ta-gui`, `ta-tests`. Numerical and support stack: `ndarray`, `faer` and/or `nalgebra`, `argmin` or a dedicated least-squares solver, `rayon`, `serde`, `uuid`, `csv`, `image`. Scientific modules must stay testable without initializing GPUI, and Python must not be required at runtime.

## Users
Transient absorption spectroscopy users generally, not one lab: graduate students, postdocs, group PIs, and facility or support scientists who acquire and analyse ultrafast TA data. They work at a workstation beside the spectrometer or at an office machine, on Linux, Windows, or macOS. Their recurring job is to turn a measured ΔA matrix into corrected data they trust, fits they can inspect, and figures they can publish. Global analysis is not part of that initial job.

## Product Purpose
A native desktop application for ultrafast transient absorption spectroscopy that combines the interactive workflow users already know from Igor Pro and the existing TA_Analyzer Python code with a coherent GUI and publication-quality figure generation. It exists so that the whole analysis chain — import, correction, extraction, fitting, stitching, comparison, Glotaran handoff, and figure export — happens in one application without Python. Success means every analysis and visualization capability of the reference implementation is available natively, on any of the three desktop platforms, without silently altering scientific data.

## Positioning
The product mechanism a neighbouring tool could not truthfully copy is the combination of three things at once: a TA-aware analysis pipeline where every processing step is a node in a reversible, reproducible derived-data graph over immutable raw matrices; one figure model shared by every interactive plot and the publication editor, so a published figure is reproducible from its saved configuration and can export its own numerical data; and a scientific coordinate system where physical wavelength and delay coordinates, not display pixels or array indices, are authoritative everywhere from hit testing to export.

## Operating Context
- Analysis of existing measurements. Real-time instrument acquisition is out of scope.
- Imported material: TA matrices with separate wavelength and time axis files, streak-camera data, existing Glotaran result directories.
- The Python TA_Analyzer repository is the behavioural migration reference; real experimental fixtures live in `TA_Analyzer/data/` (NIR800 raw matrix and axes, `NIR800_tcorr`, `NIR800_line` correction points, Glotaran `.ascii`, DAS, summary and traces files).
- Downstream handoff to Glotaran legacy `.ascii` and pyglotaran/xarray input.
- Publication conventions in use: JACS-style presets, 3.25 in single column, 7.0 in double column, 8 pt default scientific font size.
- `.taproject` projects are saved and reopened, with portable (data included) and lightweight (source-referenced, relinkable) variants.
- Representative benchmark dataset: approximately 2048 wavelengths by 1000 delay points.
- Terminology in use: ΔA/ΔOD, time zero, chirp, IRF, DAS, trace, spectrum, `FitWorkbench`, `FigureStudio`.

## Capabilities and Constraints
Confirmed capabilities: matrix import and dataset objects; interactive 2D contour with linked kinetic and spectral plots; background correction (pre-zero interval and external reference); manual and automatic time-zero/chirp correction; kinetic trace and transient spectrum extraction with averaging windows; one- to four-exponential fitting with a Gaussian-CDF IRF, including the legacy formulation preserved as a named compatibility model; experiment averaging and photodegradation correction; VIS/NIR matrix stitching; multi-object comparison workspace; legacy and pyglotaran-compatible export plus Glotaran result import and DAS visualization; a publication figure editor reachable from every plot; universal CSV/text numerical export.

Constraints: matrices are `[wavelength, time]` in `f64`; raw matrices are immutable and processing results are derived versions that record operation, parameters, and inputs; global multiexponential analysis is excluded from the first release but its data structures, import/export, and architecture must permit adding it without redesign; the Python code is a reference for behaviour, not an oracle — known defects listed in `docs/spec.md` §2.1 must be corrected deliberately and documented with tests; no dependency on a Python runtime; visual output is never a substitute for saved data; no ranking, scoring, or reporting of datasets beyond what the user asks for.

## Brand Commitments
The name Spectra Studio is settled. No logo, wordmark, voice guide, or institutional or journal identity material is on hand, and none may be invented. The repository's MIT classifier belongs to the Python TA_Analyzer package, not to Spectra Studio, whose licensing and distribution terms are undecided.

## Evidence on Hand
- `docs/spec.md` — implementation specification v1.0, 26 sections, the authoritative product and architecture record.
- `TA_Analyzer/` — working Python reference implementation: `ta_analyzer_core.py` (~3,100 lines), `correction_line.py`, a PySide6/pyqtgraph GUI under `gui/`, an example Jupyter notebook, JACS matplotlib style helpers, a tutorial, and a published GitHub Pages documentation site.
- `TA_Analyzer/data/` — real NIR800 experimental fixtures and existing Glotaran outputs, usable as parity test data.

Absent, and not to be fabricated: user testimonials, named customers or labs, measured performance benchmarks, pricing, licensing or deployment claims, and screenshots of interfaces that do not exist yet. The project has no commits yet (empty `main`), and no Rust crate has been scaffolded.

## Product Principles
1. Scientific data is never silently altered. Raw matrices stay immutable, every derived version is traceable to its operation and parameters, and incompatible axes are never combined without saying so.
2. Reversibility and reproducibility govern every operation and every figure: preview, apply, undo, redo, restore original — and any exported figure or dataset must be reproducible from its saved configuration and underlying data.
3. The Python implementation defines expected behaviour, not perfect behaviour. Port it faithfully, fix its documented defects deliberately, and prove each numerical change with a test against the original.
4. Build complete vertical slices: a backend capability lands with its interface and its tests, rather than every screen before its supporting functionality.
5. Native first — minimal runtime dependencies, OS-appropriate behaviour on Linux, Windows, and macOS, and no Python requirement for normal operation.
