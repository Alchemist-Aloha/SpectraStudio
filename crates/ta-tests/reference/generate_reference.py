#!/usr/bin/env python3
"""Generates the Python reference fixture used by `ta-tests`.

The reference implementation in `TA_Analyzer/` is the behavioural standard
(spec §2, §22.1). Rather than committing a copy of the matrices, this script
records what the Python code computes from the repository's own fixture files:
shape, axis endpoints, a checksum, and a sample of individual values. The Rust
tests then have to reproduce those numbers.

Run from anywhere:

    python3 crates/ta-tests/reference/generate_reference.py

It reads and writes only files inside this repository.
"""

from __future__ import annotations

import hashlib
import json
import platform
from pathlib import Path

import numpy as np

REPO = Path(__file__).resolve().parents[3]
DATA = REPO / "TA_Analyzer" / "data"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()[:16]


def stats(array: np.ndarray) -> dict:
    flat = array.ravel()
    finite = flat[np.isfinite(flat)]
    return {
        "rows": int(array.shape[0]),
        "cols": int(array.shape[1]),
        "sum": float(finite.sum()),
        "sum_sq": float((finite**2).sum()),
        "min": float(finite.min()),
        "max": float(finite.max()),
        "finite": int(finite.size),
        "missing": int(flat.size - finite.size),
    }


def samples(array: np.ndarray, coords: list[tuple[int, int]]) -> list[dict]:
    return [
        {"row": row, "col": col, "value": float(array[row, col])}
        for row, col in coords
    ]


def read_ta_sheet(path: Path) -> tuple[np.ndarray, np.ndarray, np.ndarray, dict]:
    """The reference implementation's own sheet reader (`tamatrix_importer`).

    Its `import_data_silent` drops the final wavelength row through an off-by-one
    in `max_rows = endrow - startrow`. That is recorded here as
    `reference_reader_shape` so the divergence stays visible in the fixture
    instead of being silently forgotten.
    """
    raw = np.loadtxt(path)
    delay_count = raw.shape[1] - 2
    tatime = raw[:delay_count, 0]
    tawavelength = raw[:, 1]
    tamatrix = raw[:, 2:]

    # The reference reader as written: startnm=0, endnm=2000.
    firstcol = raw[:, 1]
    startrow = int(np.argmin(firstcol))
    endrow = int(np.argmax(firstcol))
    reference_reader_shape = list(
        np.loadtxt(path, skiprows=startrow, max_rows=endrow - startrow)[:, 2:].shape
    )
    return tawavelength, tatime, tamatrix, {
        "reference_reader_shape": reference_reader_shape
    }


def main() -> None:
    sheet_path = DATA / "NIR800"
    matrix_path = DATA / "NIR800_tcorr"
    wavelength_path = DATA / "NIR800_tawavelength"
    delay_path = DATA / "NIR800_tatime"
    line_path = DATA / "NIR800_line"

    tawavelength, tatime, tamatrix, sheet_extra = read_ta_sheet(sheet_path)

    tcorr = np.loadtxt(matrix_path)
    corr_wavelength = np.loadtxt(wavelength_path)
    corr_delay = np.loadtxt(delay_path)
    correction_line = np.loadtxt(line_path)

    # A trace taken at 550 nm, nearest sample, over every delay.
    trace_index = int(np.argmin(np.abs(tawavelength - 550.0)))
    trace = tamatrix[trace_index, :]

    # A trace on the cropped matrix, which is what the notebook fits.
    corr_trace_index = int(np.argmin(np.abs(corr_wavelength - 550.0)))
    corr_trace = tcorr[corr_trace_index, :]

    # A spectrum at 1 ps, the delay the reference implementation singles out
    # with `get_1ps`.
    spectrum_index = int(np.argmin(np.abs(corr_delay - 1.0)))
    spectrum = tcorr[:, spectrum_index]

    # An averaged band, 600 ± 5 nm, on the cropped matrix.
    band = np.abs(corr_wavelength - 600.0) <= 5.0
    band_trace = tcorr[band, :].mean(axis=0)

    # Linear interpolation at 550.5 nm on the cropped matrix.
    lower = int(np.searchsorted(corr_wavelength, 550.5) - 1)
    upper = lower + 1
    fraction = (550.5 - corr_wavelength[lower]) / (
        corr_wavelength[upper] - corr_wavelength[lower]
    )
    interpolated = (1 - fraction) * tcorr[lower, :] + fraction * tcorr[upper, :]

    reference = {
        "generated_by": "crates/ta-tests/reference/generate_reference.py",
        "python": platform.python_version(),
        "numpy": np.__version__,
        "sources": {
            "sheet": {
                "path": "TA_Analyzer/data/NIR800",
                "sha256_16": digest(sheet_path),
            },
            "matrix": {
                "path": "TA_Analyzer/data/NIR800_tcorr",
                "sha256_16": digest(matrix_path),
            },
            "wavelength": {
                "path": "TA_Analyzer/data/NIR800_tawavelength",
                "sha256_16": digest(wavelength_path),
            },
            "delay": {
                "path": "TA_Analyzer/data/NIR800_tatime",
                "sha256_16": digest(delay_path),
            },
            "correction_line": {
                "path": "TA_Analyzer/data/NIR800_line",
                "sha256_16": digest(line_path),
            },
        },
        "sheet": {
            "wavelength_len": int(tawavelength.size),
            "wavelength_first": float(tawavelength[0]),
            "wavelength_last": float(tawavelength[-1]),
            "delay_len": int(tatime.size),
            "delay_first": float(tatime[0]),
            "delay_last": float(tatime[-1]),
            "matrix": stats(tamatrix),
            "samples": samples(tamatrix, [(0, 0), (1, 1), (500, 100), (1339, 151)]),
            **sheet_extra,
        },
        "matrix_with_axes": {
            "wavelength_first": float(corr_wavelength[0]),
            "wavelength_last": float(corr_wavelength[-1]),
            "delay_first": float(corr_delay[0]),
            "delay_last": float(corr_delay[-1]),
            "matrix": stats(tcorr),
            "samples": samples(tcorr, [(0, 0), (944, 151), (400, 20)]),
        },
        "extraction": {
            "trace_wavelength_nm": 550.0,
            "trace_sample_index": trace_index,
            "trace_sample_wavelength_nm": float(tawavelength[trace_index]),
            "trace_sum": float(np.nansum(trace)),
            "trace_first": float(trace[0]),
            "trace_last": float(trace[-1]),
            "trace_values": [float(v) for v in trace[::50]],
            "spectrum_delay_ps": 1.0,
            "spectrum_sample_index": spectrum_index,
            "spectrum_sample_delay_ps": float(corr_delay[spectrum_index]),
            "spectrum_sum": float(np.nansum(spectrum)),
            "spectrum_values": [float(v) for v in spectrum[::100]],
            "band_centre_nm": 600.0,
            "band_half_width_nm": 5.0,
            "band_channels": int(band.sum()),
            "band_sum": float(np.nansum(band_trace)),
            "band_half_width_scaled": None,
            "band_values": [float(v) for v in band_trace[::200]],
            "interpolated_wavelength_nm": 550.5,
            "interpolated_lower_index": lower,
            "interpolated_upper_index": upper,
            "interpolated_fraction": float(fraction),
            "interpolated_first": float(interpolated[0]),
            "interpolated_sum": float(np.nansum(interpolated)),
        },
        "pre_zero_background": {
            "first_points": 5,
            "reference": [
                float(v) for v in tcorr[:, :5].mean(axis=1)[::100]
            ],
            "corrected_first_column_first_values": [
                float(v) for v in (tcorr[:, 5:] - tcorr[:, :5].mean(axis=1)[:, None])[0, :5]
            ],
        },
        "correction_line": {
            "points": [
                {"wavelength": float(w), "time_zero": float(t)}
                for w, t in correction_line
            ],
        },
        "cropped_trace_sum": float(np.nansum(corr_trace)),
        "cropped_trace_index": corr_trace_index,
    }

    out = Path(__file__).resolve().parent / "nir800_reference.json"
    out.write_text(json.dumps(reference, indent=2) + "\n")
    print(f"wrote {out.relative_to(REPO)}")


if __name__ == "__main__":
    main()
