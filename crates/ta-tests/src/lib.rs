//! Reference-data tests: the fixtures, the paths, and the comparison helpers
//! that `tests/` uses.
//!
//! The numbers these tests check were produced by the Python reference
//! implementation; see `reference/generate_reference.py`.

use std::path::{Path, PathBuf};
use ta_core::Result;

/// The repository root, found from this crate's own location, so the tests work
/// whatever directory cargo is run from.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/ta-tests lives two levels below the repository root")
        .to_path_buf()
}

/// A file from the reference implementation's data directory.
pub fn fixture(name: &str) -> PathBuf {
    repo_root().join("TA_Analyzer").join("data").join(name)
}

/// The generated Python reference, parsed.
pub fn reference() -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("reference")
        .join("nir800_reference.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).expect("the reference fixture is valid JSON")
}

/// Walks a path of object keys and array indices.
pub fn value<'a>(root: &'a serde_json::Value, path: &[&str]) -> &'a serde_json::Value {
    let mut current = root;
    for step in path {
        current = match step.parse::<usize>() {
            Ok(index) => current
                .get(index)
                .unwrap_or_else(|| panic!("no index {index} in the reference at {path:?}")),
            Err(_) => current
                .get(*step)
                .unwrap_or_else(|| panic!("no key {step} in the reference at {path:?}")),
        };
    }
    current
}

pub fn number(root: &serde_json::Value, path: &[&str]) -> f64 {
    value(root, path)
        .as_f64()
        .unwrap_or_else(|| panic!("{path:?} is not a number"))
}

pub fn integer(root: &serde_json::Value, path: &[&str]) -> i64 {
    value(root, path)
        .as_i64()
        .unwrap_or_else(|| panic!("{path:?} is not an integer"))
}

pub fn numbers(root: &serde_json::Value, path: &[&str]) -> Vec<f64> {
    value(root, path)
        .as_array()
        .unwrap_or_else(|| panic!("{path:?} is not an array"))
        .iter()
        .map(|item| item.as_f64().expect("a number"))
        .collect()
}

/// Relative comparison: exact agreement for zeros, and a relative tolerance
/// everywhere else, because summation order differs between numpy and Rust.
#[track_caller]
pub fn assert_close(actual: f64, expected: f64, tolerance: f64) {
    if expected == 0.0 {
        assert!(
            actual.abs() <= tolerance,
            "expected {expected}, got {actual}"
        );
        return;
    }
    let relative = (actual - expected).abs() / expected.abs();
    assert!(
        relative <= tolerance,
        "expected {expected}, got {actual} (relative difference {relative:.3e}, \
         tolerance {tolerance:.3e})"
    );
}

#[track_caller]
pub fn assert_slice_close(actual: &[f64], expected: &[f64], tolerance: f64) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "length mismatch: {actual:?} against {expected:?}"
    );
    for (index, (a, b)) in actual.iter().zip(expected).enumerate() {
        if b == &0.0 {
            assert!(a.abs() <= tolerance, "index {index}: expected 0, got {a}");
        } else {
            let relative = (a - b).abs() / b.abs();
            assert!(
                relative <= tolerance,
                "index {index}: expected {b}, got {a} (relative difference {relative:.3e})"
            );
        }
    }
}

/// Shape, endpoints, checksum and missing-value count of a matrix, in the same
/// form the Python reference records.
pub fn matrix_stats(data: &ndarray::Array2<f64>) -> (usize, usize, f64, f64, f64, f64, usize, usize) {
    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut finite = 0usize;
    let mut missing = 0usize;
    for value in data.iter() {
        if value.is_finite() {
            finite += 1;
            sum += value;
            sum_sq += value * value;
            min = min.min(*value);
            max = max.max(*value);
        } else {
            missing += 1;
        }
    }
    (data.nrows(), data.ncols(), sum, sum_sq, min, max, finite, missing)
}

/// Imports the repository's own sheet fixture.
pub fn nir800_sheet() -> Result<ta_core::TaDataset> {
    ta_io::import_ta_sheet(&fixture("NIR800"), ta_io::Units::default())
}

/// Imports the notebook's cropped, corrected matrix with its two axis files.
pub fn nir800_matrix_with_axes() -> Result<ta_core::TaDataset> {
    ta_io::import_matrix_with_axes(
        "NIR800_tcorr",
        &fixture("NIR800_tcorr"),
        &fixture("NIR800_tawavelength"),
        &fixture("NIR800_tatime"),
        ta_io::Units::default(),
    )
}
