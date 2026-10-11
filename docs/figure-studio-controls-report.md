# FigureStudio control and refresh pass

## Changes

- Shared actions and choices invalidate the workspace scene; studio switches do the same. Preset changes synchronize existing input fields without replaying programmatic updates as user edits. Axis ranges use round-trip-safe numeric text.
- Preview rebuilding preserves page zoom and pan. Zoom in/out now apply magnification in the correct direction; Fit resets the page. Zoom and layer-order boundary actions are disabled.
- Hiding every curve leaves an empty plot instead of silently restoring the source curves. Layer ordering updates both vector order and z-order.
- Style presets update type, axis/legend sizes and series widths while retaining physical page geometry. Custom size selection retains the existing style. Marker selection enables a drawable marker mode; marker-only modes disable irrelevant line/dash controls. Color swatches are keyboard-operable buttons.
- Axis reversal now participates in rendering, hit testing, zoom and pan. Axis-specific grid flags and mirrored spines/ticks are rendered. Reference lines can specify just x or just y; Mark selection uses the opened plate's physical coordinate/orientation.
- Contour previews and exports use the edited contour specification while sharing the original matrix. Level-count edits discard overriding explicit levels. Added the offered magma and grayscale maps. Nonlinear heatmap sampling uses the same physical-coordinate map as its axes, and scale changes invalidate the raster cache.
- Font rendering follows the configured family with generic serif/sans fallback when a named font is unavailable.
- Automatic ranges use the same fitting helper as export; contour automatic ranges retain the physical matrix axes. SVG/PNG export uses the displayed panel and requested format. Visible-range export allows intentional cropping rather than failing the clipping audit. Header Export in studio exports the current edited figure instead of reopening/resetting it; elsewhere it opens the selected plate/comparison.

## Verification

- `cargo test --workspace`: **421 passed across 22 suites**, including 11 added regressions for refresh, zoom/pan preservation, visibility, layering, styles/markers/kinds/colors/dashes, contours, reference coordinates, programmatic input updates, axis reversal, pixel rendering, nonlinear heatmaps and automatic/cropped SVG/PNG export.
- Warning-free release build and clean `git diff --check`.
- Native Linux GPUI/XWayland smoke at 1440 × 1000 with the real NIR800 matrix: Zoom in showed 120%; cm conversion updated width/height; Presentation updated base/tick/legend fields to 14/13/13 and retained 120% zoom; hiding the curve cleared the preview; square markers, blue color and thicker line visibly refreshed it; Fit restored 100%.
- Evidence: `.impeccable/review/studio-controls/confirm-*.png`; logs: `.build/studio-workspace-tests.log`, `.build/studio-build.log`, `.build/studio-confirm.log` (no runtime diagnostics). XWayland's captured content was offset 14 pixels from input coordinates; the initial missed automation clicks were corrected before judging behavior.
- Original fixture SHA-256 remains `e056f95653d101a4b0759b56dbf165fdea8824c869cf12560bb695ede0823134`. Raw matrix equality is also asserted in the control regression. No fitting/processing algorithms or raw matrices were changed.

## Limits

This is a control/refresh repair, not a redesign. Export format, DPI and visible-range options affect their output, not the plotted scientific values. Code paths were reviewed and representative native controls exercised; not every option was clicked manually. Windows/macOS, high-DPI, screen-reader and exhaustive small-window/error-state testing were not performed. PDF remains unsupported; broad pre-existing formatting debt was not reformatted. Large marker glyphs/dense legends can still crowd plots, and named fonts depend on installed font availability.
