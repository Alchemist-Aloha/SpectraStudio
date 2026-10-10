//! A bounded Levenberg–Marquardt least-squares solver.
//!
//! Written here rather than taken from a general optimiser for one reason the
//! task demands: the fit has to report whether a parameter is identifiable. A
//! solver that returns numbers without a covariance cannot say that, and
//! inventing standard errors is worse than reporting none (`spec.md` §12.5).
//!
//! The algorithm is the scaled trust-region form: the damping is applied to the
//! diagonal of `JᵀJ`, so a lifetime spanning 0.01 to 5000 ps and an amplitude
//! spanning −0.5 to 0.5 are stepped on comparable terms.

use crate::Result;

/// What the solver is allowed to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolverOptions {
    pub max_iterations: usize,
    /// Stop when the residual sum of squares cannot be improved by this much,
    /// relatively.
    pub function_tolerance: f64,
    /// Stop when a step is this small, relatively.
    pub step_tolerance: f64,
    pub initial_damping: f64,
}

impl Default for SolverOptions {
    fn default() -> Self {
        Self {
            max_iterations: 200,
            function_tolerance: 1e-10,
            step_tolerance: 1e-10,
            initial_damping: 1e-3,
        }
    }
}

/// A parameter's bounds and whether it moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParameterSpec {
    pub start: f64,
    pub lower: f64,
    pub upper: f64,
    /// A fixed parameter is held at `start` and carries no uncertainty.
    pub fixed: bool,
}

impl ParameterSpec {
    pub fn free(start: f64) -> Self {
        Self {
            start,
            lower: f64::NEG_INFINITY,
            upper: f64::INFINITY,
            fixed: false,
        }
    }

    pub fn bounded(start: f64, lower: f64, upper: f64) -> Self {
        Self {
            start,
            lower,
            upper,
            fixed: false,
        }
    }

    pub fn fixed(start: f64) -> Self {
        Self {
            start,
            lower: f64::NEG_INFINITY,
            upper: f64::INFINITY,
            fixed: true,
        }
    }

    fn clamp(&self, value: f64) -> f64 {
        value.clamp(self.lower, self.upper)
    }
}

/// What the solver found.
#[derive(Clone, Debug, PartialEq)]
pub struct Solution {
    /// A value for every parameter, in the order given.
    pub parameters: Vec<f64>,
    /// Sum of squared residuals at the solution.
    pub sse: f64,
    pub iterations: usize,
    pub converged: bool,
    pub message: String,
    /// `(JᵀJ)⁻¹`, scaled by the residual variance, or `None` when the solver
    /// could not invert it — which is the honest answer for a model whose
    /// parameters trade off against each other.
    pub covariance: Option<Vec<Vec<f64>>>,
    /// The residuals at the solution.
    pub residuals: Vec<f64>,
}

/// The standard error of each free parameter, `None` where there is no
/// covariance.
pub fn standard_errors(solution: &Solution, specs: &[ParameterSpec]) -> Vec<Option<f64>> {
    let Some(covariance) = &solution.covariance else {
        return specs.iter().map(|_| None).collect();
    };
    let mut free = 0usize;
    specs
        .iter()
        .map(|spec| {
            if spec.fixed {
                return None;
            }
            let value = covariance[free][free];
            free += 1;
            if value.is_finite() && value >= 0.0 {
                Some(value.sqrt())
            } else {
                None
            }
        })
        .collect()
}

pub(crate) fn cholesky_solve(matrix: &[Vec<f64>], rhs: &[f64]) -> Option<Vec<f64>> {
    let n = rhs.len();
    let mut lower = vec![vec![0.0f64; n]; n];
    for row in 0..n {
        for column in 0..=row {
            let mut sum = matrix[row][column];
            for k in 0..column {
                sum -= lower[row][k] * lower[column][k];
            }
            if row == column {
                // A non-positive pivot means the matrix is not positive
                // definite: the parameters are not separable.
                if sum <= 1e-300 {
                    return None;
                }
                lower[row][column] = sum.sqrt();
            } else {
                lower[row][column] = sum / lower[column][column];
            }
        }
    }
    let mut intermediate = vec![0.0f64; n];
    for row in 0..n {
        let mut sum = rhs[row];
        for k in 0..row {
            sum -= lower[row][k] * intermediate[k];
        }
        intermediate[row] = sum / lower[row][row];
    }
    let mut solution = vec![0.0f64; n];
    for row in (0..n).rev() {
        let mut sum = intermediate[row];
        for k in (row + 1)..n {
            sum -= lower[k][row] * solution[k];
        }
        solution[row] = sum / lower[row][row];
    }
    solution.iter().all(|value| value.is_finite()).then_some(solution)
}

fn invert(matrix: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let n = matrix.len();
    let mut inverse = vec![vec![0.0f64; n]; n];
    for column in 0..n {
        let mut rhs = vec![0.0f64; n];
        rhs[column] = 1.0;
        let solution = cholesky_solve(matrix, &rhs)?;
        for row in 0..n {
            inverse[row][column] = solution[row];
        }
    }
    Some(inverse)
}

/// Minimises the sum of squared residuals of `residual` over the free
/// parameters.
///
/// The residual function is called with the full parameter vector, so the caller
/// keeps the model in its own terms.
pub fn least_squares(
    specs: &[ParameterSpec],
    options: &SolverOptions,
    residual: impl Fn(&[f64]) -> Vec<f64>,
) -> Result<Solution> {
    let n = specs.len();
    if n == 0 {
        return Err(crate::FitError::Invalid(
            "a fit needs at least one parameter".to_string(),
        ));
    }
    let free: Vec<usize> = specs
        .iter()
        .enumerate()
        .filter(|(_, spec)| !spec.fixed)
        .map(|(index, _)| index)
        .collect();

    let mut parameters: Vec<f64> = specs
        .iter()
        .map(|spec| spec.clamp(spec.start))
        .collect();
    let mut residuals = residual(&parameters);
    if residuals.is_empty() {
        return Err(crate::FitError::Invalid(
            "there is nothing to fit: the delay range is empty".to_string(),
        ));
    }
    let mut sse: f64 = residuals.iter().map(|value| value * value).sum();

    if free.is_empty() {
        return Ok(Solution {
            parameters,
            sse,
            iterations: 0,
            converged: false,
            message: "every parameter is fixed, so there is nothing to solve".to_string(),
            covariance: None,
            residuals,
        });
    }

    let m = residuals.len();
    let mut damping = options.initial_damping;
    let mut iterations = 0usize;
    let mut message = String::from("iteration limit reached");
    // Whether the solver reached a minimum, as opposed to running out of
    // iterations or failing to factorise. A step that cannot improve the fit is
    // a minimum however it was found.
    let mut converged = false;

    while iterations < options.max_iterations {
        iterations += 1;

        // The Jacobian of the residuals, by central differences inside the
        // bounds and one-sided at them.
        let mut jacobian = vec![vec![0.0f64; free.len()]; m];
        for (column, index) in free.iter().enumerate() {
            let spec = specs[*index];
            let current = parameters[*index];
            let step = (1e-6 * current.abs()).max(1e-8);
            let forward = spec.clamp(current + step);
            let backward = spec.clamp(current - step);
            let span = forward - backward;
            if span == 0.0 {
                continue;
            }
            let mut probe = parameters.clone();
            probe[*index] = forward;
            let plus = residual(&probe);
            probe[*index] = backward;
            let minus = residual(&probe);
            for row in 0..m {
                jacobian[row][column] = (plus[row] - minus[row]) / span;
            }
        }

        // `JᵀJ` and `Jᵀr`.
        let mut normal = vec![vec![0.0f64; free.len()]; free.len()];
        let mut gradient = vec![0.0f64; free.len()];
        for row in 0..m {
            for a in 0..free.len() {
                gradient[a] += jacobian[row][a] * residuals[row];
                for b in 0..=a {
                    normal[a][b] += jacobian[row][a] * jacobian[row][b];
                }
            }
        }
        for a in 0..free.len() {
            for b in 0..a {
                normal[b][a] = normal[a][b];
            }
        }

        // The scaled damping: the diagonal is the parameter's own sensitivity.
        let mut scaled = normal.clone();
        for a in 0..free.len() {
            let diagonal = normal[a][a].max(1e-12);
            scaled[a][a] = diagonal * (1.0 + damping);
        }

        let Some(step_vector) = cholesky_solve(&scaled, &gradient)
            .map(|solution| solution.into_iter().map(|value| -value).collect::<Vec<_>>())
        else {
            // The damping did not rescue the factorisation: raise it and try
            // again, and give up when that stops helping.
            damping *= 10.0;
            if damping > 1e12 {
                message = "the parameters could not be separated".to_string();
                break;
            }
            iterations -= 1;
            continue;
        };

        let mut candidate = parameters.clone();
        for (column, index) in free.iter().enumerate() {
            candidate[*index] = specs[*index].clamp(parameters[*index] + step_vector[column]);
        }
        let candidate_residuals = residual(&candidate);
        let candidate_sse: f64 = candidate_residuals
            .iter()
            .map(|value| value * value)
            .sum();

        let relative_step = step_vector
            .iter()
            .zip(free.iter())
            .map(|(step, index)| {
                let scale = parameters[*index].abs().max(1e-12);
                (step / scale).abs()
            })
            .fold(0.0f64, f64::max);

        if candidate_sse < sse {
            let improvement = (sse - candidate_sse) / sse.max(f64::MIN_POSITIVE);
            parameters = candidate;
            residuals = candidate_residuals;
            sse = candidate_sse;
            damping = (damping * 0.3).max(1e-12);
            if improvement < options.function_tolerance || relative_step < options.step_tolerance {
                message = format!(
                    "converged after {iterations} iterations: the sum of squares is {sse:.6e}"
                );
                converged = true;
                break;
            }
        } else {
            damping *= 10.0;
            if damping > 1e12 {
                // No step the damping allows improves the fit, which is a
                // minimum: the parameters cannot be moved to a better place.
                message = format!(
                    "converged at a minimum no step improves: the sum of squares is {sse:.6e}"
                );
                converged = true;
                break;
            }
        }
    }

    // The covariance needs the Jacobian at the solution, so it is taken once
    // more rather than reused from the last trial step.
    let covariance = {
        let mut jacobian = vec![vec![0.0f64; free.len()]; m];
        for (column, index) in free.iter().enumerate() {
            let spec = specs[*index];
            let current = parameters[*index];
            let step = (1e-6 * current.abs()).max(1e-8);
            let forward = spec.clamp(current + step);
            let backward = spec.clamp(current - step);
            let span = forward - backward;
            if span == 0.0 {
                continue;
            }
            let mut probe = parameters.clone();
            probe[*index] = forward;
            let plus = residual(&probe);
            probe[*index] = backward;
            let minus = residual(&probe);
            for row in 0..m {
                jacobian[row][column] = (plus[row] - minus[row]) / span;
            }
        }
        let mut normal = vec![vec![0.0f64; free.len()]; free.len()];
        for row in 0..m {
            for a in 0..free.len() {
                for b in 0..=a {
                    normal[a][b] += jacobian[row][a] * jacobian[row][b];
                }
            }
        }
        for a in 0..free.len() {
            for b in 0..a {
                normal[b][a] = normal[a][b];
            }
        }
        let degrees = m.saturating_sub(free.len());
        if degrees == 0 {
            None
        } else {
            let variance = sse / degrees as f64;
            invert(&normal).map(|mut inverse| {
                for row in inverse.iter_mut() {
                    for value in row.iter_mut() {
                        *value *= variance;
                    }
                }
                inverse
            })
        }
    };

    Ok(Solution {
        parameters,
        sse,
        iterations,
        converged,
        message,
        covariance,
        residuals,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_straight_line_is_recovered_exactly() {
        // y = 2x + 1, no noise.
        let data: Vec<(f64, f64)> = (0..20).map(|i| (i as f64 * 0.5, i as f64 + 1.0)).collect();
        let specs = [ParameterSpec::free(0.0), ParameterSpec::free(0.0)];
        let solution = least_squares(&specs, &SolverOptions::default(), |p| {
            data.iter().map(|(x, y)| p[0] * x + p[1] - y).collect()
        })
        .unwrap();
        assert!(solution.converged, "{}", solution.message);
        assert!((solution.parameters[0] - 2.0).abs() < 1e-6);
        assert!((solution.parameters[1] - 1.0).abs() < 1e-6);
        let errors = standard_errors(&solution, &specs);
        assert!(errors[0].unwrap() < 1e-3);
    }

    #[test]
    fn a_bounded_parameter_stops_at_its_bound_and_says_so() {
        // The best unbounded slope is 5, but the bound allows only 2.
        let data: Vec<(f64, f64)> = (0..10).map(|i| (i as f64, 5.0 * i as f64)).collect();
        let specs = [ParameterSpec::bounded(0.0, -1.0, 2.0)];
        let solution = least_squares(&specs, &SolverOptions::default(), |p| {
            data.iter().map(|(x, y)| p[0] * x - y).collect()
        })
        .unwrap();
        assert!((solution.parameters[0] - 2.0).abs() < 1e-9);
        // The solver still reports a covariance here — it is the *fit record*
        // that withholds a standard error for a parameter on a bound, because
        // only the record knows the bound.
        assert!(solution.covariance.is_some());
    }

    #[test]
    fn a_fixed_parameter_is_held_and_carries_no_uncertainty() {
        let data: Vec<(f64, f64)> = (0..12).map(|i| (i as f64, 3.0 * i as f64 + 7.0)).collect();
        let specs = [
            ParameterSpec::free(1.0),
            ParameterSpec::fixed(7.0),
        ];
        let solution = least_squares(&specs, &SolverOptions::default(), |p| {
            data.iter().map(|(x, y)| p[0] * x + p[1] - y).collect()
        })
        .unwrap();
        assert!((solution.parameters[1] - 7.0).abs() < 1e-12);
        assert!((solution.parameters[0] - 3.0).abs() < 1e-6);
        let errors = standard_errors(&solution, &specs);
        assert_eq!(errors[1], None);
        assert!(errors[0].is_some());
    }

    #[test]
    fn parameters_that_trade_off_report_no_covariance_rather_than_a_number() {
        // Two columns that are identical: the coefficients are not separable,
        // so there is no honest standard error.
        let data: Vec<(f64, f64)> = (0..10).map(|i| (i as f64, 4.0 * i as f64)).collect();
        let specs = [ParameterSpec::free(1.0), ParameterSpec::free(1.0)];
        let solution = least_squares(&specs, &SolverOptions::default(), |p| {
            data.iter()
                .map(|(x, y)| (p[0] + p[1]) * x - y)
                .collect()
        })
        .unwrap();
        let errors = standard_errors(&solution, &specs);
        assert!(
            errors.iter().all(|error| error.is_none()),
            "a degenerate model must not report standard errors: {errors:?}"
        );
    }

    #[test]
    fn a_nonlinear_curve_is_recovered_from_a_rough_start() {
        // y = a exp(-x / tau) with a = 2.5 and tau = 1.7.
        let data: Vec<(f64, f64)> = (0..40)
            .map(|i| {
                let x = i as f64 * 0.2;
                (x, 2.5 * (-x / 1.7).exp())
            })
            .collect();
        let specs = [
            ParameterSpec::bounded(1.0, 0.0, 10.0),
            ParameterSpec::bounded(1.0, 0.01, 100.0),
        ];
        let solution = least_squares(&specs, &SolverOptions::default(), |p| {
            data.iter()
                .map(|(x, y)| p[0] * (-x / p[1]).exp() - y)
                .collect()
        })
        .unwrap();
        assert!(solution.converged, "{}", solution.message);
        assert!((solution.parameters[0] - 2.5).abs() < 1e-4, "{:?}", solution.parameters);
        assert!((solution.parameters[1] - 1.7).abs() < 1e-4, "{:?}", solution.parameters);
    }

    #[test]
    fn every_parameter_fixed_is_reported_rather_than_looped_over() {
        let specs = [ParameterSpec::fixed(1.0)];
        let solution = least_squares(&specs, &SolverOptions::default(), |p| {
            vec![p[0] - 2.0]
        })
        .unwrap();
        assert!(!solution.converged);
        assert!(solution.message.contains("nothing to solve"));
        assert_eq!(solution.iterations, 0);
    }

    #[test]
    fn an_empty_residual_is_an_error_not_a_zero_fit() {
        let specs = [ParameterSpec::free(1.0)];
        assert!(least_squares(&specs, &SolverOptions::default(), |_| Vec::new()).is_err());
        assert!(least_squares(&[], &SolverOptions::default(), |_| vec![1.0]).is_err());
    }

    #[test]
    fn the_solver_stops_at_its_iteration_limit_without_claiming_success() {
        let options = SolverOptions {
            max_iterations: 1,
            ..SolverOptions::default()
        };
        let data: Vec<(f64, f64)> = (0..40)
            .map(|i| {
                let x = i as f64 * 0.2;
                (x, 2.5 * (-x / 1.7).exp())
            })
            .collect();
        let specs = [
            ParameterSpec::bounded(1.0, 0.0, 10.0),
            ParameterSpec::bounded(1.0, 0.01, 100.0),
        ];
        let solution = least_squares(&specs, &options, |p| {
            data.iter()
                .map(|(x, y)| p[0] * (-x / p[1]).exp() - y)
                .collect()
        })
        .unwrap();
        assert!(!solution.converged);
        assert_eq!(solution.iterations, 1);
    }
}
