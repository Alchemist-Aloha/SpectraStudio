use std::fmt;

/// Errors raised by the scientific data layer.
///
/// The variants name the coordinate system or index that failed, because a
/// silent wrong-array operation is the failure mode this crate exists to
/// prevent.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("axis {axis} is empty")]
    EmptyAxis { axis: &'static str },

    #[error("axis {axis} holds a non-finite value {value} at index {index}")]
    NonFiniteCoordinate {
        axis: &'static str,
        index: usize,
        value: f64,
    },

    #[error(
        "axis {axis} holds the duplicate coordinate {value} at indices {first} and {second}"
    )]
    DuplicateCoordinate {
        axis: &'static str,
        value: f64,
        first: usize,
        second: usize,
    },

    #[error("matrix is {rows}x{cols} but the axes describe {wavelengths}x{delays}")]
    ShapeMismatch {
        rows: usize,
        cols: usize,
        wavelengths: usize,
        delays: usize,
    },

    #[error("matrix is {rows}x{cols}, which is not wavelength-major")]
    NotWavelengthMajor { rows: usize, cols: usize },

    #[error(
        "coordinate {value} lies outside the sampled range {min}..={max} of axis {axis}"
    )]
    OutOfRange {
        axis: &'static str,
        value: f64,
        min: f64,
        max: f64,
    },

    #[error("axis {axis} has no sampled point to average near {center}")]
    EmptyWindow { axis: &'static str, center: f64 },

    #[error("no matrix version {version} in dataset {dataset}")]
    UnknownVersion { dataset: String, version: String },

    #[error("no dataset {0} in this project")]
    UnknownDataset(String),

    #[error("matrix version {version} references block {block}, which does not exist")]
    UnknownBlock { version: String, block: usize },

    #[error(
        "dataset {dataset} has no numbers loaded for version {version}; the project needs \
         relinking to its source files"
    )]
    MissingMatrixData { dataset: String, version: String },

    #[error("processing operation {op} needs {expected} input versions, got {got}")]
    WrongInputCount {
        op: &'static str,
        expected: usize,
        got: usize,
    },

    #[error("a matrix version cannot take itself as input")]
    SelfReferentialVersion,

    #[error("interpolation needs at least {needed} points on axis {axis}, got {got}")]
    NotEnoughPoints {
        axis: &'static str,
        needed: usize,
        got: usize,
    },

    #[error("averaging needs at least two input versions, got {got}")]
    NotEnoughInputs { got: usize },

    #[error("input versions do not share wavelength axis {axis}")]
    AxisMismatch { axis: &'static str },

    #[error("input versions have different shapes: {first:?} against {second:?}")]
    InputShapeMismatch {
        first: (usize, usize),
        second: (usize, usize),
    },

    #[error("no numeric values on line {line} of {path}")]
    EmptyRecord { path: String, line: usize },

    #[error("{path} line {line} field {field} is not a number: {text:?}")]
    MalformedNumber {
        path: String,
        line: usize,
        field: usize,
        text: String,
    },

    #[error("{path} line {line} holds {got} fields, expected {expected}")]
    RecordWidth {
        path: String,
        line: usize,
        got: usize,
        expected: usize,
    },

    #[error("cannot read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("malformed project file {path}: {detail}")]
    MalformedProject { path: String, detail: String },

    #[error("project schema {found} is newer than this build understands ({supported})")]
    ProjectTooNew { found: u32, supported: u32 },

    #[error("{0}")]
    Invalid(String),
}

impl CoreError {
    pub fn invalid(message: impl fmt::Display) -> Self {
        CoreError::Invalid(message.to_string())
    }
}

pub type Result<T> = std::result::Result<T, CoreError>;
