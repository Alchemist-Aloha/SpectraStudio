# TA Analyzer / Spectra Studio — autonomous UX improvement report

## Scope and method

Three bounded passes over the existing shared shell and plot surfaces: structural
workflow repair; interaction/configuration repair; scientific-display/export
verification. No replacement visual design or processing-algorithm changes.

Read the Rust workspace, `docs/spec.md`, `docs/plan.md`, the earlier
`docs/ux-report.md`, the Impeccable surface brief and Operate/critique/distill/audit/
polish guidance. Reused the incumbent `ink` tokens, compact ruled panels, GPUI Kit
inputs/checkboxes, GPUI focus, shared figure model and Plotters wrapper.

The actual release application launched with `TA_Analyzer/data/NIR800_tcorr`
(945 wavelengths × 152 delays). Initial capture used Wayland; subsequent native
GPUI runs used X11/XWayland so targeted keyboard events could be delivered.
Small-window evidence is 943×505; wide captures are 1898×1023. Compositor tiling
changed window size during some interactions; filenames alone are not size proof.
Screenshots contain only application windows, not the surrounding desktop.

## A. Findings and priorities

| Priority | Location / problem | Impact and evidence | Correction |
|---|---|---|---|
| P0 | `app.rs::save_project` did not exist; header Save required an existing path | A new experiment could not be saved through the GUI; header handler explicitly reported “no project path yet” | Native first-save dialog; Ctrl/Cmd+S |
| P0 | `render.rs` mixed plot-local and panel coordinates | At 3.25-inch publication size, the SVG omitted the baseline/rise although its CSV contained them; reproduced in the running studio and SVG | Translate drawing coordinates once, including ticks, annotations, grids and series |
| P1 | `session.rs::set_version` changed only the session version | A restored raw version was not the dataset's saved active version; regression test | Update the dataset's active version without deleting derived work |
| P1 | Session save omitted the open analysis view | Reopening lost the active dataset, kept curves and sampling; existing test even expected the kept list to disappear | Persist and validate the analysis view in existing project layout metadata |
| P1 | `project_file.rs::save_project` truncated the destination immediately | A failed write could destroy the previous project; static File::create evidence | Same-directory temporary file, flush/sync, rename; retain permissions |
| P1 | `app.rs::stacked_axes` fell back to NIR800 axes for unrelated files | An unrelated experiment could silently inherit the fixture's coordinates if dimensions matched; regression test | Only discover axes matching the matrix name or its `_tcorr` base |
| P1 | Workspace shortcuts lacked a focused root and context restriction | `F` did not open the workbench in the baseline; native key delivery works after the fix | Track workspace focus, reclaim it on plot selection, exclude Input context from analysis keys |
| P1 | Compare had no route to export its displayed multi-object figure | Opening ordinary FigureStudio reconstructed the bench dataset instead; code inspection | FigureStudio entry on both comparison captions, preserving resolved display transforms |
| P1 | Fit component changes copied inactive fixed flags into newly active slots | Adding a component could leave it fixed; existing test missed activation | Apply template flags to newly enabled/disabled slots; preserve values/bounds and fixed flags of continuously active parameters |
| P1 | PDF was offered but the exporter only supported SVG/PNG | Actual PDF attempt produced no PDF; export dispatcher explicitly refuses it | Remove misleading PDF choice and state that PDF is unavailable; implementation remains backlog |
| P2 | Side panels consumed 524 px, with no folding control | Baseline small-window plot column was only 419 px; screenshot | Dataset/inspector toggles; scrollable dataset index |
| P2 | Two-component fit showed third/fourth component rows | 13 rows included four irrelevant rows marked fixed/not identifiable; screenshot | Show 9 relevant rows; retain hidden starting values |
| P2 | Publication split-axis tick labels collided / approached page edge | Observed after correcting drawing offsets | Reserve right-label space and thin overlapping ticks at actual drawing size |
| P2 | Time-zero plate used the latest correction anywhere in the dataset | Restoring raw could still display another version's correction; regression test | Read only the displayed version's ancestry; remove the unimplemented “drag a line” instruction |

## B. Implemented changes

- `crates/ta-gui/src/app.rs`, `main.rs`: working save dialog/shortcut, contextual
  keyboard bindings and focus, panel controls, project-path startup restoration,
  accurate selected-plot tracking, matching-axis discovery. Header Export opens
  the existing editor instead of silently writing a fixed kinetics filename.
- `crates/ta-gui/src/session.rs`: active-version synchronization; save/reopen of
  the bench, linked selections, overlays, sampling, comparison index and current
  edited figure. Invalid saved selections/figure references are rejected before
  replacing open work. Old projects without view metadata still load.
- `crates/ta-gui/src/compare.rs`, `views.rs`: comparison → FigureStudio with
  dataset/version references, colors, normalization, offsets and coordinate
  shifts retained. Actual normalization determines units; zero-reference failures
  keep ΔOD, and mixed outcomes are labelled partly normalized.
- `crates/ta-gui/src/fit_panel.rs`, `widgets.rs`: correct component activation,
  inactive-row disclosure, and scene invalidation after choice controls change.
- `crates/ta-gui/src/plates.rs`: correction provenance follows the active ancestry;
  removed the unused parallel export helper. Its caption/export assertions now
  exercise the actual studio path in `studio.rs`.
- `crates/ta-io/src/project_file.rs`: atomic project replacement and permission
  preservation, with replacement/failure tests.
- `crates/ta-plot/src/render.rs`: shared panel-offset correction and publication
  tick spacing, with bitmap and label-bound regression checks.
- Cargo changes reuse workspace `serde`, `serde_json`, `uuid`; no new third-party
  package or rendering/UI framework was introduced.

## C. Verification

- `cargo test --workspace`: **409 passed, 22 suites**, including real NIR800
  numerical parity, project/processing round trips, plotting interaction tests,
  new view restoration, version provenance, component activation, normalized
  comparison transfer and pixel-placement regressions.
- `cargo build -p ta-gui --release`: passes without warnings.
- `git diff --check`: passes.
- `cargo fmt --check`: **fails**. Repository-wide formatting debt predates this
  pass (including untouched `ta-core` files); no broad reformat was mixed into
  the UX patch. Formatting is not claimed clean. Log: `.build/ux-format.log`.

Native workflow evidence:

| Workflow | What was verified | Limit |
|---|---|---|
| Import → contour → kinetics → spectra | Actual fixture loaded; native Left/B/Up/C events changed/kept selections and linked plots | Import used startup path, not a complete arbitrary-axis file-picker flow |
| Background preview → apply → undo | Numerical processing/derived-version integration tests and active-version restore tests | No correction workbench exists; GUI workflow cannot be completed |
| Chirp drawing → correction | Core correction tests; raw restoration hides unrelated curves | No on-contour editing/apply UI exists |
| Trace → fit → results | Native `F` entry; actual two-component startup fit and residuals; τ₁=25.858319 ps, τ₂=425.944733 ps, R²=0.99977, matching the earlier report | Background cancellation and full parameter-edit delivery not desktop-tested |
| Compare → publication export | Two independent objects opened; actual comparison FigureStudio generated SVG + two 152-point CSV blocks | Second object is a renamed copy of NIR800, not a second measured experiment; scaled independent datasets/normalization covered in tests |
| Figure styling → export | Actual SVG inspected as XML and rendered image; pre-zero baseline/rise and labels present, tick spacing corrected | SVG/CSV verified; PDF cannot be generated. Default legend can overlap data and may need user repositioning |
| Save → close → reopen | Native Ctrl+S opened the native dialog; project saved, keys changed selection, Ctrl+S resaved; process closed and a fresh process reopened the `.taproject` | Zoom/splitter sizes and an unfinished fit configuration are not restored |
| Glotaran handoff | Actual ASCII and NetCDF files generated; existing parser/parity tests pass | No Glotaran-results/DAS viewer UI |

Source-fixture SHA-256 remained
`e056f95653d101a4b0759b56dbf165fdea8824c869cf12560bb695ede0823134`.
Scientific processing algorithms were untouched. The deliberate coordinate change
is drawing-origin translation, not wavelength/delay conversion or matrix edits.

Screenshots: `.impeccable/review/ux-autonomous/` — `baseline-small.png`,
`baseline-x11.png`, `export-svg.png` (before), `final-comparison-svg.png`,
`final-comparison-studio.png`, `final-fit.png`, `save-dialog.png`,
`confirm-key-fit.png`, `final-saved-figure.png`, `final-reopened-figure.png`.
Test project: `.build/final-ux-smoke.taproject`; exports: `.build/ux-fixtures/`.

## D. Measured impact

Counts describe the implemented interaction model, not a timed usability study.

| Task | Before | After |
|---|---|---|
| Save a new project | Blocked | Save/Ctrl+S → filename → confirm; one native dialog |
| Reopen the tested analysis selection | Kept list lost | 576.73956 nm, 0 ps and kept wavelength restored; native before/after captures |
| Open the fit from the focused workspace | Advertised F had no focused target | One native F event, verified |
| Two-component parameter table | 13 rows | 9 rows; 4 irrelevant rows removed |
| Transfer a comparison into publication editor | No direct path | One contextual action; transformations/reference provenance retained |
| Free plotting width | Fixed 524 px of side panels | Two toggles can reclaim those 524 px; no modal |
| Publication kinetic SVG | Baseline/rise clipped | Full selected trace drawn; numerical CSV remains 152 points |

At 943 px width, the default plot **column** is 419 px (44.4% of window width);
with both side panels folded it can use the full width. This is not a measured
plot-area fraction: axes, captions and toolbars still occupy space. No interaction
latency, large-matrix responsiveness, time-to-result or failure-frequency benchmark
was collected; no performance improvement is claimed.

## E. Remaining backlog / stop boundary

1. **P0 — Processing GUI:** implement background/reference preview/apply/discard,
   manual chirp editor, corrected/original comparison and explicit undo/redo.
   Engines exist; the researcher-facing workflow does not.
2. **P0 — Advanced operations:** stitching, experiment averaging/photodegradation,
   and Glotaran result/DAS viewing lack complete UI entry points.
3. **P0 for PDF workflow — PDF backend:** implement and visually validate native
   PDF export. SVG is the working vector alternative; no PDF button is promised.
4. **P1 — Unsaved-work recovery:** confirmation before replacing/closing a dirty
   project, autosave, Save As, and export overwrite protection.
5. **P1 — Complete view persistence:** remember selections per dataset when
   switching benches; preserve zoom/splitters and fit drafts/selected results.
6. **P1 — Small-window workbench/studio:** fit controls and table still compress
   plots at 943×505; long captions and comparison controls can clip. Folding is
   implemented only for the single-dataset view, not every inspector.
7. **P1 — Input/error robustness:** strict multi-value/bounds parsing with local
   feedback; desktop validation of fit cancellation and selection changes during
   running jobs; expose advanced solver settings without dominating common setup.
8. **P1 — Accessibility/themes:** full tab order and focus rings for custom word
   actions; screen-reader verification, high-DPI scaling and dark theme. Current
   palette remains the pinned light scientific-paper design.
9. **P2 — Export/layout quality:** automatic legend placement to avoid data,
   contour colorbar robustness, explicit export destination/overwrite feedback.
10. **P2 — Release checks:** repository formatting, Windows/macOS, large datasets,
    UI responsiveness/memory measurements and lightweight project relinking.

Stopped after the bounded evidence-driven fixes and confirmation runs. This is a
working improvement patch, **not certification that every requested workflow is
complete**. Missing application capabilities are explicitly listed rather than
substituted with placeholder controls. Historical product/plan metadata remains
stale; Impeccable configuration/design-document repair was not mixed into this task.
