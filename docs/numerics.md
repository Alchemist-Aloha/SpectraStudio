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
  `self.mat_array[pt, pt2 + 2, 0]`, an index that runs off the end of the delay
  axis near the end of the array.
* **Rust:** `ta_processing::burn` will take the sampled wavelength and reference
  delay the interface gives it, compute the same per-delay factor from the
  first and last experiment, and apply it from the first delay column onwards.
  The index is checked before use.
* **Why:** the intent is clear from the surrounding code, and the intent is what
  a port owes the user. Reproducing an arithmetic error that cannot run is not
  fidelity.
* **Proved by:** `ta-processing` tests at the point this lands (milestone M4);
  until then the operation is absent from the interface rather than present and
  wrong.

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
