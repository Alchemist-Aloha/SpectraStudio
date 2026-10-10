# UX review and improvement pass

Scope: the four workspaces of Spectra Studio (Figure, Compare, FigureStudio,
FitWorkbench) plus the shared shell — header, plate index, captions, inspector,
footer. Reviewed against the running release binary with the repository's own
NIR800 fixture, by reading frames rather than source.

Date of this pass: one pass over the shell and all four workspaces. Ground rules
followed: no new UI layers, no placeholder controls, scientific processing and
export untouched.

---

## A. Initial assessment

Highest-friction findings, in the order they were worked. "Evidence" says how
each was established — a frame read from the running application, a test, or
both.

### A1. The entry points to the two most valuable workspaces were unlabelled

The four plate captions ended with three small underlined words: `studio`,
`data`, `glotaran`. Nothing in the interface said what "studio" was. A new user
could not act on any of them.

- **Impact:** blocks the entire figure-production workflow (Workflow E) and the
  data-export workflow (§18) for anyone who has not read the code.
- **Evidence:** baseline frame
  (`.impeccable/review/ux/baseline-figure.png`).
- **Cause:** the controls were named after the internal modules (`open_studio`,
  `export_plate_data`, `export_glotaran`) rather than after what they do.

### A2. No keyboard, and no way to walk a selection without the pointer

Walking five wavelengths meant five pointer clicks at five plot positions, each
a different distance from the inspector. Keeping a curve was a click on the
plot plus a click on "Keep trace". Nothing in the interface mentioned a key, so
the keyboard was not merely missing — it was unknown.

- **Impact:** the most repeated action in every session (choose a wavelength,
  look, keep it) is pointer-only. Matters most on a laptop, where the plot area
  is smallest and pointer travel the longest.
- **Evidence:** `SHORTCUTS` did not exist; the baseline frame shows no key hint
  anywhere.

### A3. "Keep trace" was silent, and keeping the linked curve drew it twice

`save_current_trace` changed the curve list without a word in the footer.
Separately, a coordinate that was both kept *and* linked was drawn twice: the
plate drew the kept list and then the linked curve, even when they were the same
measured sample.

- **Impact:** no feedback for an action that changes what a figure contains;
  a figure with a visibly duplicated trace.
- **Evidence:** test
  `the_keep_keys_add_the_selection_and_say_so` failed on the old code;
  `Session::kinetics` built `kept + linked` with no deduplication.

### A4. "Reset view" reset one of the four workspaces

The action resized the four figure plates only. A panned comparison pane, a
zoomed workbench plate or a panned studio preview stayed panned, so "reset"
meant different things in different layouts.

- **Impact:** a user who has lost the data in a comparison pane has no action
  that brings it back.
- **Evidence:** the handler iterated `self.plates` alone.

### A5. The inspector showed controls that could not change anything

With `Reading = nearest`, the averaging window fields (`5.00 nm ±`, `3
channels`) were still offered, and the rarely-used "(d) time-zero plate" switch
sat in the middle of the panel as bare text with no label.

- **Impact:** cognitive load on the panel a user reads most; a control that
  silently does nothing is worse than an absent one.
- **Evidence:** baseline frame, inspector column.

### A6. The inspector's content was taller than the window and did not scroll

Adding anything to the panel clipped its last rows below the window edge.

- **Impact:** unreachable content in the primary panel, on any window under
  about 1000 px tall.
- **Evidence:** after-frame with the key list clipped at the bottom edge.

### A7. The comparison toolbar's "Add selection" was a guess

The button said "Add selection"; what it adds is *the bench dataset's current
coordinate*, which is not visible in the toolbar. The user had to remember what
was selected in another workspace.

- **Impact:** Workflow D's first step requires the wrong mental model — the
  toolbar describes application state it does not show.
- **Evidence:** `compare_toolbar` in the baseline.

### A8. The workbench's parameter table always showed five columns

Start values, bounds and fixed flags are the setup of a fit; the fitted value
and its error are the result. Both were always present, so the results — what a
reader returns for — were interleaved with controls.

- **Impact:** the most numerically dense table in the application is harder to
  scan than it needs to be.
- **Evidence:** the workbench frame; each row is `start | bounds | fixed |
  fitted ± error`.

### Non-findings, checked and left alone

- **Plot interactions** are already consistent across all four workspaces: the
  same wheel zoom, drag pan, drag region, crosshair with margin read-outs, and
  the same "a cursor stands a tick label down" rule. No change made.
- **Data-first layout:** the contour, kinetics and spectra occupy the large
  majority of the window in every layout. No change made.
- **Scientific integrity:** a display transform (scale, offset, shift,
  normalization) is recorded per series and named in the comparison caption;
  processing is done by adding a version, and every caption names the version
  and operation. No confusion found between display and data.
- **Colour-only identification:** every curve carries a legend label naming its
  sampled coordinate, so colour is never the only cue.

---

## B. Changes implemented

All in `crates/ta-gui`. No scientific code, coordinate transformation or export
behaviour was touched.

| # | Finding | Change | Where |
|---|---|---|---|
| 1 | A1 | Caption controls renamed to `FigureStudio`, `Export data`, `Glotaran input` | `app.rs` (`plate_block`) |
| 2 | A2 | Keyboard model: `←`/`→` step the wavelength, `↑`/`↓` the delay by one **measured sample**; `B`/`C` keep the current curve; `F` opens the fit; `0` resets every view | `app.rs` (`bind_shortcuts`, `handle_shortcut`, `step_linked`, `keep_current`), `main.rs` |
| 3 | A2 | The keys are listed in the inspector, in three compact lines | `app.rs` (`shortcut_list`) |
| 4 | A3 | Keeping reports what it did ("kept on (b) — 3 curve(s)"), and a curve that is both kept and linked is drawn once, keeping the accent | `app.rs`, `session.rs` (`kinetics`, `spectra`, `kinetics_records`, `spectra_records`) |
| 5 | A4 | `Reset all views` resets the figure plates, both comparison panes, the workbench plates and the studio preview | `app.rs` (`reset_all_views`) |
| 6 | A5 | The averaging window fields appear only when the reading averages over one | `app.rs` (`inspector`) |
| 7 | A5 | The fourth plate's switch is a labelled row: `(d) time-zero plate` | `app.rs` |
| 8 | A6 | The inspector scrolls | `app.rs` |
| 9 | A7 | The comparison toolbar names what it will add ("Add its trace to the comparison") and states what the bench holds ("the bench has NIR800_tcorr at 577.16 nm") | `compare.rs` (`compare_source_label`) |
| 10 | A8 | The workbench's table has a switch in its header, "edit start values and bounds"; the fitted value and its error always show | `views.rs`, `fit_panel.rs` |

Two defects the work also found and fixed, because the new tests reached code
that had no test:

- `Session::kinetics` / `spectra` (and their `_records` twins) appended the
  linked curve unconditionally, so a kept-and-linked sample was drawn twice.
- The old "Keep trace" path and the new key took different code paths; both now
  go through `save_current_trace` / `save_current_spectrum`.

---

## C. Verification

**Tests.** 393 pass across the workspace, zero warnings. The new ones:

- `the_arrow_keys_walk_the_two_axes_one_measured_sample_at_a_time` — a step is
  one measured sample on the fixture's 50 nm grid, the two axes move
  independently, and a step past either end stops there instead of wrapping.
- `the_keep_keys_add_the_selection_and_say_so` — keeping adds once, a second
  press is harmless rather than an error, and the footer says so.
- `one_action_puts_every_view_back` — a panned figure plate, a comparison pane
  and the workbench all return to their own ranges.

**Frames read from the running release binary** (`.impeccable/review/ux/`):

- `baseline-figure.png` → `after-figure.png`: the renames, the window fields
  gone, the labelled plate switch, `Reset all views`, the key list.
- `after-inspector.png`: the panel now fits, keys included.
- `after-compare-toolbar.png`: "the bench has NIR800_tcorr at 577.16 nm · Add
  its trace to the comparison".
- `after-fit.png`: the workbench with the new header switch; data, fitted curve
  and residuals unchanged, τ₁ = 25.858 ps, τ₂ = 425.945 ps, R² = 0.99977.

**Workflows exercised end to end in the running application:** import →
contour → kinetics → spectra (Figure layout); `--layout compare` with a trace;
`--layout fit --fit` with a completed two-exponential fit; `--layout studio`
with a figure, its SVG and its CSV written beside the data. The Glotaran export
writes both files and its own parser reads one back.

**Not verified, and why — stated rather than claimed:**

- **Key delivery end to end.** This display server (Hyprland/Wayland) has no
  `wtype`, `ydotool` or `xdotool` that can reach a native client, so no keystroke
  was synthesised into the window. The handlers the keys dispatch to are covered
  by the tests above, and the bindings are a plain table in `bind_shortcuts`;
  the mapping itself was not exercised through the compositor.
- **The workbench's folded table** (switch off) and every hover/focus state:
  both need pointer input, same reason.
- **`cargo fmt --check`** reports diffs in 56 files. They are pre-existing — the
  repository is formatted by hand at a wider line width and has no rustfmt
  config or CI check — so reformatting them would bury this change in noise. The
  files in this pass match the surrounding style.

---

## D. Measured impact

Counted from the interaction model, not timed: the number of discrete user
actions to reach a stated result. Pointer distance and reading time are not
claimed.

| Workflow step | Before | After |
|---|---|---|
| Look at the next wavelength's kinetics, five times | 5 plot clicks | 5 × `→` |
| Same, then keep each one | 5 clicks + 5 clicks on "Keep trace" | 5 × `→` + 5 × `B` |
| Enter the fit from the plots | pointer to the layout tab, or to the inspector | `F` |
| Put every view back after exploring | 4 resize/zoom actions by hand in three workspaces | `0` |
| Learn the keys without documentation | not possible | 3 lines in the inspector |
| Know what the comparison button will add | not possible from the toolbar | one readout |
| Keep a curve and see that it worked | silent | footer: "kept on (b) — 3 curve(s)" |
| Scan the workbench for fitted values | interleaved with start values and bounds | one switch folds the setup away |

Working notes that did **not** change: 3 clicks to import and see a contour
(unchanged), 2 to open FigureStudio (unchanged, but the control is now named),
1 to export a figure and 1 to export its numbers (unchanged), 1 action to write
both Glotaran files (unchanged).

Panel occupancy: the inspector's visible rows went from 15 to 14 in the default
state (two window controls removed, the key list added in three lines), while
the plate area is unchanged — the shell's proportions were not altered.

---

## E. Remaining backlog

Ordered by user impact. None of these were reached in this pass.

1. **Stitching and experiment averaging have no interface.** The engines are
   implemented and tested in `ta-processing`, but there is no way to start
   either from the application. This is the largest remaining gap and it is a
   missing feature rather than a usability defect.
2. **No collapsible side panels.** At 1400 × 900 the plots get about 880 px. The
   index and the inspector should fold away, which also serves the brief's
   small-window requirement. The toggles are straightforward; the plumbing for
   the splitter is not, which is why it was left for a dedicated pass.
3. **No tooltips.** The underlined word-controls change colour on hover but do
   not say what they do. A hover card on the plate controls and the fit actions
   would help most where a label must stay short.
4. **No visible focus ring or tab order.** The shortcuts work, but nothing shows
   which control has focus, so keyboard-only navigation is still awkward.
5. **Undo is implicit.** A processing step adds a version, and the version chain
   is in the plate index — but nothing says "revert to this version". The
   affordance exists (clicking a version switches to it); the wording does not.
6. **The status line shows one message.** A problem replaces the last successful
   action rather than being listed beside it.
7. **The fit's advanced numerical settings** (damping, tolerances, iteration
   limit) are still in `SolverOptions` with no panel. The brief asks that they
   stay accessible without dominating; they are currently unreachable, which is
   the safer of the two failures but still a gap.
8. **Colour-bar space is reserved but the bar is missing from the studio
   contour preview** — the layout allows for it and the label would be drawn.
   Cosmetic; found while reading frames, not fixed.
