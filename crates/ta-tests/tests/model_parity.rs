//! Parity with the reference implementation's model.
//!
//! `ta_analyzer_core.multiexp_func` is the formulation every existing result
//! came from, so Spectra Studio's legacy model has to reproduce it
//! (spec §12.2). The expected values come from
//! `reference/generate_reference.py`, which evaluates the reference expression
//! with numpy.

use ta_core::{FitModelKind, FitParameter};
use ta_fitting::{model_curve, Component, Model, ModelParams};
use ta_tests::{assert_close, number, numbers, reference, value};

/// The reference's `w0`..`w12` as a parameter list.
fn parameters_from_reference() -> Vec<FitParameter> {
    let reference = reference();
    let names = [
        "w0", "w1", "w2", "w3", "w4", "w5", "w6", "w7", "w8", "w9", "w10", "w11", "w12",
    ];
    let readable = [
        "irf_fwhm",
        "amplitude_scale",
        "amplitude_1",
        "lifetime_1",
        "amplitude_2",
        "lifetime_2",
        "amplitude_3",
        "lifetime_3",
        "amplitude_4",
        "lifetime_4",
        "time_zero",
        "pre_zero_offset",
        "long_term_offset",
    ];
    names
        .iter()
        .zip(readable)
        .map(|(legacy, name)| {
            let mut parameter = FitParameter::new(name, number(&reference, &["model", "params", legacy]));
            parameter.legacy_name = Some((*legacy).to_string());
            parameter
        })
        .collect()
}

#[test]
fn the_legacy_model_reproduces_the_reference_expression() {
    let reference = reference();
    let delays = numbers(&reference, &["model", "delays"]);
    let expected = numbers(&reference, &["model", "values"]);
    let parameters = parameters_from_reference();

    let curve = model_curve(FitModelKind::LegacyGaussianCdfIrf, &parameters, &delays)
        .expect("the parameters match the model");

    assert_eq!(curve.len(), expected.len());
    for (index, (got, want)) in curve.iter().zip(&expected).enumerate() {
        assert_close(*got, *want, 1e-11);
        let _ = index;
    }
    // The sum is a second check that catches a sign or an index error that a
    // point-by-point comparison might tolerate.
    let sum: f64 = curve.iter().sum();
    assert_close(sum, number(&reference, &["model", "sum"]), 1e-11);
}

#[test]
fn the_model_parameters_read_the_reference_slots() {
    let reference = reference();
    let parameters = parameters_from_reference();
    let params = ModelParams::from_parameters(&parameters).expect("every slot is present");

    assert_close(
        params.irf_fwhm,
        number(&reference, &["model", "params", "w0"]),
        1e-12,
    );
    assert_close(
        params.time_zero,
        number(&reference, &["model", "params", "w10"]),
        1e-12,
    );
    assert_eq!(params.components.len(), 4);
    assert_close(
        params.components[0].amplitude,
        number(&reference, &["model", "params", "w2"]),
        1e-12,
    );
    assert_close(
        params.components[1].lifetime,
        number(&reference, &["model", "params", "w5"]),
        1e-12,
    );
    // The vector form keeps the reference's slot order.
    let (names, values) = params.to_vector();
    assert_eq!(names[0], "irf_fwhm");
    assert_eq!(values[0], params.irf_fwhm);
    assert_eq!(values[10], params.time_zero);
    let back = ModelParams::from_vector(&values).expect("13 values");
    assert_eq!(back, params);
}

#[test]
fn the_convolved_model_is_not_the_legacy_one_and_is_flagged_as_such() {
    // The two formulations differ, and the difference is exactly what
    // `spec.md` §12.2 warns a user about: the legacy expression multiplies by a
    // cumulative distribution instead of convolving with the response.
    let reference = reference();
    let delays = numbers(&reference, &["model", "delays"]);
    let parameters = parameters_from_reference();

    let legacy =
        model_curve(FitModelKind::LegacyGaussianCdfIrf, &parameters, &delays).unwrap();
    let convolved =
        model_curve(FitModelKind::ConvolvedExponential, &parameters, &delays).unwrap();

    let largest = legacy
        .iter()
        .zip(&convolved)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f64, f64::max);
    assert!(
        largest > 1e-3,
        "the two models should differ materially, largest difference was {largest}"
    );
    // Both rise through the same time zero and settle together.
    let settle = delays.len() - 1;
    assert_close(legacy[settle], convolved[settle], 0.2);
}

#[test]
fn a_zero_lifetime_component_is_the_reference_behaviour() {
    // The reference skips a component whose lifetime is zero; so does ours.
    let parameters = parameters_from_reference();
    let delays = [-1.0, 0.0, 1.0, 10.0];
    let mut with_c3 = parameters.clone();
    let mut with_c3_off = parameters.clone();
    // Slot w6/w7 is the third component, which the reference leaves at zero.
    assert_eq!(
        value(
            &reference(),
            &["model", "params", "w7"]
        )
        .as_f64()
        .unwrap(),
        0.0
    );
    with_c3[6].value = 0.4;
    with_c3[7].value = 8.0;
    let off = model_curve(FitModelKind::LegacyGaussianCdfIrf, &with_c3_off, &delays).unwrap();
    with_c3_off[6].value = 0.4;
    with_c3_off[7].value = 8.0;
    let on = model_curve(FitModelKind::LegacyGaussianCdfIrf, &with_c3_off, &delays).unwrap();
    assert!(
        on.iter().zip(&off).any(|(a, b)| (a - b).abs() > 1e-6),
        "switching a component on must change the curve"
    );
    let _ = with_c3;
}

#[test]
fn evaluating_the_model_by_hand_matches_the_expression_once_more() {
    // A second, independent evaluation of the same expression, so the crate's
    // own code is not the only witness.
    let params = ModelParams {
        irf_fwhm: 0.12,
        amplitude_scale: 1.0,
        components: vec![
            Component {
                amplitude: -0.45,
                lifetime: 1.6,
            },
            Component {
                amplitude: 0.30,
                lifetime: 24.0,
            },
            Component {
                amplitude: 0.0,
                lifetime: 0.0,
            },
            Component {
                amplitude: 0.0,
                lifetime: 0.0,
            },
        ],
        time_zero: 0.07,
        pre_zero: -0.004,
        long_term: 0.02,
    };
    let delays = [-0.5, 0.0, 1.0, 10.0, 100.0];
    let evaluated = ta_fitting::evaluate(Model::LegacyGaussianCdfIrf, &params, &delays);
    let sigma = 0.12 / (2.0 * (2.0 * std::f64::consts::LN_2).sqrt());
    for (index, delay) in delays.iter().enumerate() {
        let shifted = delay - 0.07;
        let cdf = ta_fitting::normal_cdf(shifted / sigma);
        let expected = -0.45 * (-shifted / 1.6).exp() * cdf
            + 0.30 * (-shifted / 24.0).exp() * cdf
            - 0.004
            + 0.02 * cdf;
        assert!(
            (evaluated[index] - expected).abs() < 1e-14,
            "at {delay}: {} against {expected}",
            evaluated[index]
        );
    }
}
