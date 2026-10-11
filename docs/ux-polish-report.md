# UI polish pass

Preserved the incumbent scientific-paper design: white plots, tinted shell,
hairline rules, compact type and one vermilion accent. No numerical algorithms,
matrices, plot transforms or publication settings changed in this pass.

## Changes

- `crates/ta-gui/src/main.rs`: register the installed toolkit's bundled SVG
  assets. Checked controls now show their checkmarks, including offline. Align
  native component colors and focus rings with existing `ink` tokens; retain the
  pinned light theme rather than mixing dark native controls with light panels.
- `crates/ta-gui/src/widgets.rs`, `app.rs`: word-actions and choice controls use
  the toolkit's unstyled Button, with explicit accessible names, keyboard
  activation, pointer cursors and visible focus. Shared checks use compact,
  consistent type and invalidate scenes after their state changes.
- `crates/ta-gui/src/views.rs`: navigation is keyboard-operable; panel switches
  align vertically with the tabs. FigureStudio's export bar wraps its grouped
  controls instead of clipping the last actions at 1024 px.
- `crates/ta-gui/src/app.rs`: scientific captions wrap independently of their
  export actions and grow rather than losing provenance beneath a fixed height.
- `crates/ta-gui/src/fit_panel.rs`, `views.rs`: folded editing no longer leaves
  empty start/bounds columns. Align headers, states, units and fitted values;
  keep the long fitted-heading explanation in a tooltip. Read-only column width
  is 392 px instead of 560 px, returning **168 px** to the plot column. Starting
  values and bounds remain intact when hidden.

## Evidence

One combined rendered inspection, one correction batch, one confirmation batch.
Native GPUI/XWayland, real copied NIR800 fixture (945 × 152), at **1440×900** and
**1024×720**. Original user's running window was not closed; disposable inspection
processes were cleaned up, and the updated app remains open.

- **410 tests pass across 22 suites**, including the new offline checkmark-asset
  regression test. Release build passes without warnings; `git diff --check`
  passes. Latest runtime log contains no warnings/errors.
- Tab → Enter opened the native Import dialog; Escape canceled without replacing
  the dataset. From Figure, Tab → Enter switched to Compare. Mouse navigation,
  fit edit folding and publication-editor entry were exercised.
- Native fit stayed τ₁=25.858319 ps, τ₂=425.944733 ps, R²=0.99977.
- Token contrast: body/paper **18.15:1**, muted/shell **5.24:1**, white/accent
  **6.50:1**. This is not a complete contrast audit of plot palettes/bound states.
- Before/after screenshots: `.impeccable/review/polish/`; final captures use
  `confirm-*`. Logs: `.build/polish-{tests,build,confirm,format}.log`.

## Limits

Dense spectrum legends still overcrowd very small plot rectangles when both side
panels are open. Longer provenance captions intentionally take additional height;
fold panels for more plotting space. Compact comparison series controls can still
compress long labels. No additional polishing loop was started for these broader
layout issues.

`cargo fmt --check` still fails on repository-wide formatting debt. Windows/macOS,
screen readers, comprehensive empty/error/permission states and performance were
not tested. Impeccable product metadata still misclassifies this desktop product
as adaptive/mobile; the persisted native-desktop surface brief and actual Rust UI
were used instead. Missing DESIGN.md and context drift were not repaired as a
side effect. Existing incomplete workflows remain listed in
[the autonomous UX audit](ux-autonomous-report.md).
