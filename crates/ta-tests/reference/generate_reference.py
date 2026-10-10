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

import math

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


def norm_cdf(x: np.ndarray, scale: float = 1.0) -> np.ndarray:
    """The standard normal CDF, from the standard library's `erf`.

    `scipy.stats.norm.cdf` is what the reference implementation calls; this is
    the same function to within an ulp on this machine, and scipy is not
    installed here.
    """
    return np.array([0.5 * math.erfc(-value / (scale * math.sqrt(2))) for value in np.atleast_1d(x)])


def multiexp_func(t, w0, w1, w2, w3, w4, w5, w6, w7, w8, w9, w10, w11, w12):
    """The reference implementation's model, copied verbatim from
    `ta_analyzer_core.multiexp_func` (the module itself cannot be imported here
    because it pulls in lmfit, which is not installed). Each exponential is
    multiplied by a Gaussian CDF rather than convolved with the instrument
    response, which is why Spectra Studio keeps this formulation under its own
    name.
    """
    sigma = np.sqrt(w0**2) / (2 * np.sqrt(2 * np.log(2)))
    result = np.zeros_like(t)
    for amplitude, lifetime in ((w2, w3), (w4, w5), (w6, w7), (w8, w9)):
        if lifetime == 0:
            continue
        result += amplitude * np.exp(-(t - w10) / lifetime) * norm_cdf(
            t - w10, sigma
        )
    result += w11 + w12 * norm_cdf(t - w10, sigma)
    result *= 1
    return result


def model_reference() -> dict:
    """Values of the legacy model at the delays a fit would visit."""
    params = {
        "w0": 0.12,
        "w1": 1.0,
        "w2": -0.45,
        "w3": 1.6,
        "w4": 0.30,
        "w5": 24.0,
        "w6": 0.0,
        "w7": 0.0,
        "w8": 0.0,
        "w9": 0.0,
        "w10": 0.07,
        "w11": -0.004,
        "w12": 0.02,
    }
    delays = np.array(
        [-1.0, -0.5, 0.0, 0.05, 0.07, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0,
         24.0, 50.0, 100.0, 500.0, 1079.7752]
    )
    values = multiexp_func(delays, *params.values())
    # The stable series Spectra Studio evaluates the normal CDF with, so the
    # comparison is of the model and not of two error functions.
    cdf = norm_cdf(delays - params["w10"], sigma_of(params["w0"]))
    with_erfc = np.zeros_like(delays)
    for amplitude, lifetime in ((params["w2"], params["w3"]), (params["w4"], params["w5"])):
        with_erfc += amplitude * np.exp(-(delays - params["w10"]) / lifetime) * cdf
    with_erfc += params["w11"] + params["w12"] * cdf
    return {
        "params": params,
        "delays": [float(value) for value in delays],
        "values": [float(value) for value in values],
        "values_erfc": [float(value) for value in with_erfc],
        "sum": float(np.sum(values)),
    }


def sigma_of(fwhm: float) -> float:
    return float(np.sqrt(fwhm**2) / (2 * np.sqrt(2 * np.log(2))))


def time_zero_reference() -> dict:
    """The manual time-zero correction, exactly as the reference implements it.

    `auto_tcorr_fit` resamples each wavelength at `delay + offset` with
    `np.interp`, which clamps outside the measured delays, and keeps the
    original delay axis. The control points are the repository's own drawn
    correction line, whose wavelength range is narrower than the matrix's — so
    this fixture exercises the clamping path as well.
    """
    matrix = np.loadtxt(DATA / "NIR800_tcorr")
    wavelength = np.loadtxt(DATA / "NIR800_tawavelength")
    delay = np.loadtxt(DATA / "NIR800_tatime")
    line = np.loadtxt(DATA / "NIR800_line")

    corrected = np.zeros_like(matrix)
    for i in range(len(wavelength)):
        offset = np.interp(wavelength[i], line[:, 0], line[:, 1])
        corrected[i, :] = np.interp(delay + offset, delay, matrix[i, :])

    return {
        "curve_points": [[float(w), float(t)] for w, t in line],
        "sum": float(np.sum(corrected)),
        "min": float(np.min(corrected)),
        "max": float(np.max(corrected)),
        "samples": [
            {"row": row, "col": col, "value": float(corrected[row, col])}
            for row, col in [(0, 0), (400, 20), (552, 40), (944, 151), (700, 100)]
        ],
        "offset_at_550": float(np.interp(550.0, line[:, 0], line[:, 1])),
        "offset_below_the_curve": float(np.interp(400.0, line[:, 0], line[:, 1])),
        "offset_above_the_curve": float(np.interp(900.0, line[:, 0], line[:, 1])),
    }


def glotaran_reference() -> dict:
    """The legacy time-explicit ASCII, exactly as the reference writes it.

    `glotaran()` builds the table the Java Glotaran reads: a header naming the
    file and the number of time points, a first row of times, and one row per
    wavelength whose first column is the wavelength. Everything is written with
    `fmt="%s"`, so the numbers are Python's own `str()` of the float.
    """
    matrix = np.loadtxt(DATA / "NIR800_tcorr")
    tatime = np.loadtxt(DATA / "NIR800_tatime")
    tawavelength = np.loadtxt(DATA / "NIR800_tawavelength")

    output_matrix = np.append(tatime.reshape(1, -1), matrix, axis=0)
    output_matrix = np.append(
        np.append("", tawavelength).reshape(1, -1).T, output_matrix, axis=1
    )
    header = "NIR800_tcorr\n\nTime explicit\nintervalnr " + str(len(tatime))
    lines: list[str] = []
    for line in header.split("\n"):
        lines.append(line)
    for row in output_matrix:
        lines.append("\t".join(str(value) for value in row))
    return {
        "header_lines": header.split("\n"),
        "separator": "\t",
        "format": "str() of the float, as np.savetxt fmt=%s writes it",
        "line_count": len(lines),
        "columns": int(output_matrix.shape[1]),
        "first_rows": ["\t".join(str(v) for v in row) for row in output_matrix[:3]],
        "last_row": "\t".join(str(v) for v in output_matrix[-1]),
        "row_400": "\t".join(str(v) for v in output_matrix[1]),
        "time_row_first": str(tatime[0]),
        "time_row_last": str(tatime[-1]),
    }


def float_format_reference() -> dict:
    """How Python writes a float, which is what the ASCII export has to match.

    `np.savetxt(fmt="%s")` hands every value to `str()`, so the file's numbers
    are Python's own shortest round-tripping representation. These are the values
    whose form is easy to get wrong: the ends of the decimal notation, the
    exponent's sign and width, and negative zero.
    """
    values = [
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.1,
        -0.00011,
        0.00028,
        -0.00008,
        1e-4,
        1e-5,
        1.5e-5,
        -8e-05,
        0.000123456,
        400.38315,
        799.61685,
        1179.2252,
        1e15,
        1e16,
        1e17,
        1.23e16,
        -1.2345678901234567,
        123456.789,
        0.5,
        2.5,
        100.0,
        1e-300,
        1.7976931348623157e308,
        5e-324,
        3.141592653589793,
        6.02214076e23,
    ]
    return {str(value): repr(value) for value in values}


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
        "model": model_reference(),
        "time_zero": time_zero_reference(),
        "glotaran": glotaran_reference(),
        "float_format": float_format_reference(),
    }

    out = Path(__file__).resolve().parent / "nir800_reference.json"
    out.write_text(json.dumps(reference, indent=2) + "\n")
    print(f"wrote {out.relative_to(REPO)}")


if __name__ == "__main__":
    main()
