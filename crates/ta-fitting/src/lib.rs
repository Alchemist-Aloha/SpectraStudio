//! Kinetic fitting: the models, the solver, and the record a fit leaves.
//!
//! Two things here are deliberately more than a wrapper around an optimiser:
//!
//! * the models are written out rather than generated, because the reference
//!   implementation's formulation is *not* an exact convolution and both are
//!   offered under their own names (`spec.md` §12.2);
//! * the solver reports a covariance, so a parameter it cannot identify gets no
//!   standard error instead of an invented one (§12.5).

pub mod chirp;
pub mod fit;
pub mod model;
pub mod solver;

#[derive(Debug, thiserror::Error)]
pub enum FitError {
    #[error("the parameter {0} is missing from the model")]
    MissingParameter(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Solver(String),
}

pub type Result<T> = std::result::Result<T, FitError>;

pub use chirp::{
    estimate_time_zero_curve, AutoTimeZero, AutoTimeZeroOptions, AutoTimeZeroPoint,
};
pub use fit::{estimate_time_zero, fit_trace, model_curve, FitRequest, TimeZeroEstimate};
pub use model::{erf, erfc, evaluate, normal_cdf, Component, Model, ModelParams};
pub use solver::{least_squares, standard_errors, ParameterSpec, Solution, SolverOptions};
