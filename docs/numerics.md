# Numerical behaviour and deliberate divergences

The Python implementation in `TA_Analyzer/` is the behavioural reference
(`spec.md` §2). This file records every place where Spectra Studio does
something different on purpose, why, and which test proves it. Anything not
listed here is intended to agree with the reference to floating-point
precision.

Each entry names the Python symbol, the Rust symbol, the difference, and the
test that holds the line.

## 1. The last wavelength row of a TA sheet is kept

* **Python:** `tamatrix_importer.import_data_silent` computes
  `max_rows = endrow - startrow` and then reads `data[startrow:startrow + max_rows]`,
  which drops the final wavelength row. For `data/NIR800` that is 1339 rows on a
  1340-row file.
* **Rust:** `ta_io::read_ta_sheet` keeps every row, so the imported matrix has
  1340 wavelengths.
* **Why:** an off-by-one that silently discards a measured wavelength is exactly
  the class of defect `spec.md` §2.1 asks to correct.
* **Proved by:** `ta-tests/tests/nir800_parity.rs`,
  `the_importer_keeps_the_last_wavelength_the_reference_reader_drops`, which
  asserts both numbers — the reference reader's 1339 and our 1340.

## 2. A trace keeps the first two delays

* **Python:** `load_spectra.get_traces` reads `self.tamatrix_avg[pt, 2:]` and
  `self.mat_array[pt, 2:, i]`, so the two leading delay columns never reach the
  plot or the fit.
* **Rust:** `ta_core::kinetic_trace` returns every delay column of the version
  it is given.
* **Why:** those columns are the pre-zero baseline. Dropping them silently
  removes the data a baseline correction or a pre-zero offset fit needs, and the
  user cannot tell that it happened.
* **Proved by:** `ta-tests/tests/nir800_parity.rs`,
  `the_trace_keeps_the_first_two_delays_the_reference_drops`.

## 3. Negative delays are never passed to `log`

* **Python:** `get_traces` plots `np.log(self.tatime)`. TA delay axes start at
  negative values (`-1 ps` in the fixture), so the logarithm of those points is
  `nan` and they vanish from the plot without a word.
* **Rust:** the delay axis is drawn through a scale that is defined at and below
  zero: `AxisScale::SymmetricLogarithmic { linear_width }` or
  `AxisScale::SplitLinearLog { break_at }` (`ta_core::figure::AxisScale`). The
  reference implementation's own split-axis convention
  (`new_split_axes` / `plot_split_axes`) is the model for the second.
* **Why:** a plot that silently omits its early-time data misrepresents the
  measurement, and the early times are where the interesting physics is.
* **Proved by:** `ta-core/src/axis.rs`, the scale constructors refuse a
  non-positive `break_at`/`linear_width`; and the plot layer's axis tests
  (`ta-plot`) check that a negative coordinate lands inside the linear region.

## 4. A missing sample inside an average is left out, not propagated

* **Python:** `np.mean` over a window, or `np.nansum` depending on the call
  site, so a single `NaN` either poisons the whole average or is quietly treated
  as absent depending on which numpy function the line happened to use.
* **Rust:** `ta_core::extract::weighted_over_rows`/`weighted_over_columns`,
  `ta_processing::reference_columns`, and `ta_processing::average_matrices`
  renormalise over the samples that are finite. A window with no finite sample
  at all yields `NaN`, and a per-experiment average with a `NaN` in one
  experiment is `NaN` rather than being divided by the wrong count.
* **Why:** "the average of the samples that exist" is a statement the user can
  reason about; "the average including a missing value" is not. This is the one
  place where the two implementations can differ on identical input, and only
  when the input contains a missing value.
* **Proved by:** `ta-core/src/extract.rs`
  `a_missing_sample_inside_a_window_is_left_out_rather_than_counted_as_zero`,
  `ta-processing/src/background.rs`
  `a_partly_missing_baseline_averages_what_is_there`,
  `ta-processing/src/average.rs`
  `a_missing_sample_in_one_experiment_makes_the_average_missing`.

## 5. Photodegradation correction is not ported as written

* **Python:** `load_spectra.correct_burn` computes a per-delay factor and then
  applies it as `self.tamatrix_avg_burncorr[:, 2:] *= burn_correction`, where
  `burn_correction` has one element per delay while the slice has two fewer.
  The two shapes cannot broadcast, so the call as written raises. It also reads
  `self.mat_array[pt, pt2 + 2, 0]` — the third delay point past the one nearest
  1 ps — where `pt2 + 2` runs off the end of the delay axis near the end of the
  array.
* **Rust:** `ta_processing::burn::correct_burn` takes the same inputs, clamps the
  probe index instead of running off the end, and applies the factors from the
  reference delay onwards. Columns before it are left alone, because they are
  the baseline the drift is measured against and the background subtraction has
  already removed their offset.
* **Why:** the intent is clear from the surrounding code, and the intent is what
  a port owes the user. Reproducing an arithmetic error that cannot run is not
  fidelity.
* **Proved by:** `ta-processing/src/burn.rs`, which checks the factor per point,
  the clamped probe, a zero reference signal, a missing reference point, and
  experiments of different shapes.

## 7. The general amplitude is honoured

* **Python:** `multiexp_func` ends with `result *= 1`, so the parameter `w1`,
  documented as "general amplitude of the fitting", never multiplies anything.
* **Rust:** `ta_fitting::model::evaluate` multiplies by `amplitude_scale`.
* **Why:** the parameter is in the model's documented interface and in the
  parameter template the interface shows. A parameter that silently does nothing
  is worse than one that works; at its default of 1 the two agree exactly.
* **Proved by:** `ta-fitting/src/model.rs`,
  `the_amplitude_scale_multiplies_the_whole_curve`, and the parity test against
  the reference expression with `w1 = 1`.

## 8. The automatic correction curve is a polynomial in wavelength

* **Python:** `fit_correlation` fits each sampled wavelength's time zero, then
  calls `polyfit`, whose model is `polynomial_func(x, a, b, c) = a / (1e-9 +
  x²) + c` with `x` the *wavelength* and `b` unused. So the "fitted correction
  line" is an inverse-square in wavelength with one dead parameter, which is not
  the chirp shape and is the auxiliary function `spec.md` §2.1 asks to
  investigate.
* **Rust:** `ta_fitting::chirp::estimate_time_zero_curve` fits a weighted
  polynomial in wavelength — degree 1 or 2, chosen by the caller — with the
  weight of each point being the reciprocal of its own residual sum of squares,
  which is what the reference passes to its curve fit.
* **Why:** a chirp is a smooth, slowly varying delay offset; a quadratic in
  wavelength describes it and can be extrapolated over a matrix's range without
  the pole that `1/(1e-9 + λ²)` has at the origin.
* **Proved by:** `ta-fitting/src/chirp.rs`, on synthetic matrices with a known
  linear chirp, a flat time zero, wavelengths below the signal threshold, and too
  few usable points.

## 9. A fit reports what it can and no more

* **Python:** `lmfit` with the Powell method; a result is kept when
  `result.success` and `chisqr < 0.15`.
* **Rust:** a bounded Levenberg–Marquardt solver that reports convergence
  explicitly, with three deliberate differences. A step that cannot improve the
  fit counts as convergence rather than failure, because that is a minimum. A
  parameter sitting on a bound gets no standard error, because the solver had no
  room to estimate one. A parameter the solver cannot separate — a degenerate
  covariance — gets no standard error either, instead of a large invented one.
* **Why:** `spec.md` §12.5 forbids inventing standard errors, and the reference's
  fixed chi-square threshold is a property of its own data rather than a
  statement about a fit's quality.
* **Proved by:** `ta-fitting/src/solver.rs` and `fit.rs`: a straight line
  recovered exactly, a bound respected, a degenerate model reporting no errors,
  a bounded non-linear curve recovered, the iteration limit reported as *not*
  converged, and known lifetimes recovered from a noisy synthetic trace.

## 10. The time-zero boundary is the reference's, and now selectable

* **Python:** `np.interp` clamps outside the measured delays, so the correction
  silently holds the edge value.
* **Rust:** `BoundaryMode::Clamp` is the default and reproduces that exactly
  (verified against numpy on the repository's fixture); `Mask` leaves the sample
  missing and `Extrapolate` extends the edge slope, both as explicit choices.
* **Why:** the reference's behaviour is a reasonable default and a poor silent
  one: a user correcting a chirp near the edge of their delay axis should be able
  to see that the edge was held rather than measured.

## 6. Values are not rescaled on import

* **Python:** reading is unit-agnostic; the y axis is labelled ΔOD by hand.
* **Rust:** `ta_io::Units` defaults to nanometres, picoseconds and ΔOD, matching
  that labelling, and records the assumption in the dataset's metadata rather
  than converting anything. `SignalUnit::to_delta_od` exists for when a caller
  explicitly asks for a conversion, and nothing calls it during import.
* **Why:** guessing a unit from the magnitude of the numbers would silently
  rescale real data by a factor of 1000.

## Adding an entry

A new divergence needs all five parts: the Python symbol, the Rust symbol, the
difference, the reason, and the test. A divergence without a test is a bug with
a paragraph attached.

## 11. Glotaran export writes what the reference writes

* **Python:** `glotaran()` writes the time-explicit ASCII file with
  `np.savetxt(..., fmt="%s")`, so every number is Python's own `str()` of the
  float; `pyglotaran()` builds an xarray Dataset with `time` and `spectral`
  coordinates and a `data` variable whose first axis is time.
* **Rust:** `ta-glotaran::ascii` writes the same file, and
  `ta-glotaran::format::python_float` reproduces Python's float representation
  including its plain-notation window (-4 to 16 in decimal exponents) and its
  two-digit signed exponents. `ta-glotaran::netcdf` writes the modern dataset as
  NetCDF-3 classic, which is the serialized form of what the reference builds in
  memory.
* **Why:** the file is read by another program, so its format is an interface,
  not an internal choice. A user who has been pasting the reference's file into
  Glotaran should be able to hand ours over unchanged.
* **Proved by:** `ta-tests/tests/glotaran_parity.rs` compares the exported file
  line by line against the reference's own numpy output on the repository
  fixture, including the header, and checks every float against a table of
  Python's answers for the values whose form is easy to get wrong. The NetCDF
  file is read back by an independent parser written from the format
  specification (`crates/ta-tests/reference/check_netcdf.py`), which the test
  runs when Python is available.
* **Not ported:** reading Glotaran results back. `spec.md` §16.6 records this as
  a decision: the application's own fitted curves and arbitrary curves can be
  drawn in any figure (`SeriesSource::Fit`, `SeriesSource::Curve`), so a
  result-side parser would be a second result format for a question already
  answered here.

## 12. Text the screen turns, and text the file turns

* **Python:** matplotlib draws every label itself, including the rotated ones.
* **Rust:** one drawing description, two backends. The exported SVG and PDF turn
  an axis title with a `rotate` transform, and the on-screen backend paints the
  same text through the same SVG renderer, so a caption that is vertical in the
  figure is vertical on screen — at the figure's type size, in its margin, turned
  the same way. An earlier version set such a caption upright across its anchor,
  because GPUI's shaped lines cannot be rotated; the SVG renderer can rotate
  anything, so the divergence is gone and the two backends agree.
* **Proved by:** `ta-plot-gpui`'s preview tests (the drawn type is the
  configured size at the pane's zoom, and the preview's layout is the export's
  magnified) and `ta-plot`'s, which check that the axis title stands in the left
  margin, clear of the tick labels, and that the x title is centred under the
  plot.
