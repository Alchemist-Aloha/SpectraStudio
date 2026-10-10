# Surface: the single-dataset workspace

`crates/ta-gui` — the application shell and its first surface: import a TA
matrix, read the contour, select a wavelength and a delay, see the linked
kinetics and spectra, and export a figure.

- **Scope:** the primary analysis workspace.
- **Visitor mode:** Operate. The visitor is a spectroscopy researcher at a
  workstation, doing a job, not being persuaded.
- **Platform:** native desktop (GPUI), one product across Linux, Windows and
  macOS. No mobile surface, so the iOS and Android references do not apply;
  platform conventions here mean the toolkit's standard desktop controls,
  keyboard access, and the OS window behaviours.
- **Audience and job:** see PRODUCT.md. They import measured ΔA matrices,
  correct them, extract traces and spectra, fit, and publish.
- **Constraints from PRODUCT.md:** the raw matrix is immutable; every derived
  version names its operation and its inputs; physical coordinates are
  authoritative; a figure is reproducible from its saved configuration.

## Direction contract

**THESIS.** Every panel is a journal figure panel with a ranked caption
beneath it, and the analysis history is that caption growing as the work
proceeds. It refuses the category default: chrome-boxed panel headers over a
grey shell, where nothing on screen says which matrix version and which
sampling produced the numbers you are reading.

**OWN-WORLD.** Paper white ground (#ffffff), ink #101526, hairline rules
(#c9ced9 over #e2e6ee) instead of borders and shadows, one vermilion #b02a37
for structure and the current selection, amber #b8791f for a value pinned at a
bound, and the diverging cool (#3b4cc0) / warm (#b40426) pair reserved for the
sign of the data. Density: many ruled rows at 10–13 px, tabular figures
everywhere. No cards, no rounded containers, no coloured edge bars.

**STORY.** A postdoc imports a measured matrix, sees the contour ruled and
captioned, clicks 550 nm and 1 ps, watches the kinetics and spectra fill in,
reads in each caption exactly which version and which sampling produced that
curve, and exports a figure whose numbers leave with it.

**FIRST VIEWPORT.** A header of rules carrying the product name, the project
title and the actions; a 198 px plate index on the left listing versions,
traces and spectra as ruled rows; the centre figure block — (a) the contour,
then (b) kinetics and (c) spectra side by side, then (d) the time-zero curve —
each panel captioned beneath with its ranked caption; a 300 px right column of
caption-and-control pairs; a footer of tabular coordinates and the clickable
address `dataset ▸ version ▸ selection`.

**FORM.** The ranked-caption figure page; rank 1 of my grounded list; seed key
`9eb0a6e1`.

**FINISH.** unreviewed and undocumented is unfinished; this build ends with the
finish review, the verdict, DESIGN.md, and every shipping raster carrying its
provenance

## Open decisions

- Whether the plate index and the inspector are dockable panels (spec §3.1 wants
  dockable, resizable panels) or fixed columns with a drag splitter. Fixed
  columns first, dockable when the layout persistence lands.
- The exact noise-floor rule behind "drawn hollow": a threshold multiplier on
  the pre-zero standard deviation, to be fixed when M4 computes it.
