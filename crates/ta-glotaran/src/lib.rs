//! Glotaran input generation.
//!
//! Glotaran is a *destination*: a matrix is corrected here, exported in a format
//! Glotaran reads, and fitted there. This crate therefore writes and never
//! parses a result; `spec.md` §16.6 records that as a decision.
//!
//! Two writers share one view of the data:
//!
//! * [`legacy`] writes the time-explicit ASCII file the Java Glotaran reads,
//!   reproducing the reference implementation's `glotaran()` byte for byte;
//! * [`netcdf`] writes the time/spectral dataset with `time`, `spectral` and
//!   `data` — the shape the reference's `pyglotaran()` builds in memory — as a
//!   NetCDF-3 classic file, which xarray and pyglotaran open directly.

pub mod ascii;
pub mod format;
pub mod matrix_view;
pub mod netcdf;

pub use ascii::{read_ascii, write_ascii, AsciiReport};
pub use format::python_float;
pub use matrix_view::{GloDataset, MATRIX_VARIABLE, SPECTRAL_DIMENSION, TIME_DIMENSION};
pub use netcdf::{read_netcdf, write_netcdf, NetcdfReport};
