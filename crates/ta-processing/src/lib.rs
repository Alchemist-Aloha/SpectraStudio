//! Matrix processing: baseline subtraction, cropping, averaging.
//!
//! Every function here takes a matrix and returns a *new* one. Nothing mutates
//! its input, which is what lets a raw measurement stay the floor of the
//! derived-data graph.
//!
//! Where a calculation differs from the reference implementation, the
//! difference is named in `docs/numerics.md` and proved by a test.

pub mod average;
pub mod background;
pub mod crop;

pub use average::{average_matrices, subtract_matrices};
pub use background::{
    background_pre_zero, pre_zero_columns, reference_columns, subtract_reference,
};
pub use crop::{crop_matrix, Cropped};
