//! M1 exit criteria (docs/plan.md): a project saves and reopens with its
//! derived-data graph, its selections and its saved figures intact, using the
//! repository's real fixture data.

use ta_core::{
    kinetic_trace, Axis, AxisRequest, AxisScale, CropSpec, FigureModel, PlotSeries, PreZeroRange,
    ProcessingOp, Project, SeriesSource, TaDataset, TraceSelection, VersionId,
};
use ta_tests::{nir800_matrix_with_axes, repo_root};

fn project_path(name: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("spectra-studio-{}-{name}", std::process::id()));
    path
}

/// A chain of three versions over the real fixture: raw, background corrected,
/// then cropped to the notebook's wavelength window.
struct Chain {
    project: Project,
    dataset: ta_core::DatasetId,
    raw: VersionId,
    corrected: VersionId,
    cropped: VersionId,
    selection: TraceSelection,
}

fn build_chain() -> Chain {
    let mut project = Project::new("NIR800 study");
    let dataset = nir800_matrix_with_axes().expect("the fixture imports");
    let id = project.add_dataset(dataset);
    let raw = project.dataset(id).unwrap().raw_version().id;

    // Background subtraction over the first five delay points, using the
    // reference implementation's own default.
    let op = ProcessingOp::BackgroundPreZero(ta_core::PreZeroBackground {
        range: PreZeroRange::FirstPoints { count: 5 },
    });
    let (corrected, corrected_unit) = {
        let dataset = project.dataset(id).unwrap();
        let data = dataset.matrix(raw).unwrap();
        let corrected = ta_processing::background_pre_zero(data, dataset.delay(), PreZeroRange::FirstPoints { count: 5 })
            .unwrap();
        (corrected, dataset.unit(raw).unwrap())
    };
    let (wavelength, delay) = {
        let dataset = project.dataset(id).unwrap();
        let (wavelength, delay) = dataset.axes(raw).unwrap();
        (wavelength.clone(), delay.clone())
    };
    let corrected = project
        .dataset_mut(id)
        .unwrap()
        .add_derived(
            corrected,
            corrected_unit,
            wavelength,
            delay,
            "Background corrected",
            op,
            vec![raw],
        )
        .unwrap();

    // Crop to 450–750 nm: this is the case that changes the grid, so the
    // version has to carry its own axes.
    let crop = CropSpec {
        wavelength: Some((450.0, 750.0)),
        delay: None,
    };
    let cropped = {
        let dataset = project.dataset(id).unwrap();
        let data = dataset.matrix(corrected).unwrap();
        let (wavelength, delay) = dataset.axes(corrected).unwrap();
        ta_processing::crop_matrix(data, wavelength, delay, &crop).unwrap()
    };
    let cropped = project
        .dataset_mut(id)
        .unwrap()
        .add_derived(
            cropped.data,
            corrected_unit,
            cropped.wavelength,
            cropped.delay,
            "Cropped 450–750 nm",
            ProcessingOp::Crop(crop),
            vec![corrected],
        )
        .unwrap();

    // A saved trace and a figure that holds it.
    let selection = TraceSelection::new(id, cropped, AxisRequest::nearest(550.0));
    let mut figure = FigureModel::single_panel("Kinetics at 550 nm");
    figure.panels[0]
        .series
        .push(PlotSeries::new("550 nm", SeriesSource::Trace(selection.clone())));
    figure.panels[0].x.scale = AxisScale::SymmetricLogarithmic { linear_width: 1.0 };
    figure.panels[0].x.label = "Delay".to_string();
    figure.panels[0].y.label = "ΔOD".to_string();

    let dataset = project.dataset_mut(id).unwrap();
    dataset.traces.push(selection.clone());
    dataset.figures.push(figure);

    Chain {
        project,
        dataset: id,
        raw,
        corrected,
        cropped,
        selection,
    }
}

#[test]
fn a_processing_chain_survives_save_and_reopen() {
    let chain = build_chain();
    chain.project.validate().expect("the chain is consistent before saving");

    let path = project_path("chain.taproject");
    ta_io::save_project(&chain.project, &path).unwrap();
    let loaded = ta_io::load_project(&path).unwrap().project;

    loaded.validate().expect("the reopened project is consistent");

    let before = chain.project.dataset(chain.dataset).unwrap();
    let after = loaded.dataset(chain.dataset).unwrap();

    assert_eq!(after.name, before.name);
    assert_eq!(after.versions.len(), 3);
    assert_eq!(
        after.versions.iter().map(|v| v.kind).collect::<Vec<_>>(),
        before.versions.iter().map(|v| v.kind).collect::<Vec<_>>()
    );

    // The graph edges survived, in order.
    let corrected = after.version(chain.corrected).unwrap();
    assert_eq!(corrected.inputs, vec![chain.raw]);
    let cropped = after.version(chain.cropped).unwrap();
    assert_eq!(cropped.inputs, vec![chain.corrected]);
    assert_eq!(after.ancestry(chain.cropped), vec![chain.raw, chain.corrected, chain.cropped]);

    // The operation and its parameters survived intact.
    match &cropped.op {
        Some(ProcessingOp::Crop(spec)) => assert_eq!(spec.wavelength, Some((450.0, 750.0))),
        other => panic!("expected a crop, got {other:?}"),
    }
    match &corrected.op {
        Some(ProcessingOp::BackgroundPreZero(background)) => {
            assert_eq!(background.range, PreZeroRange::FirstPoints { count: 5 })
        }
        other => panic!("expected a pre-zero background, got {other:?}"),
    }

    // Matrices are bit-identical through the binary layer.
    for version in [chain.raw, chain.corrected, chain.cropped] {
        let bit_before = before.matrix(version).unwrap();
        let bit_after = after.matrix(version).unwrap();
        assert_eq!(bit_before.dim(), bit_after.dim());
        assert!(
            bit_before
                .iter()
                .zip(bit_after.iter())
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "version {version} changed through the round trip"
        );
    }

    // The cropped version's own grid is what came back, not the raw one.
    let (wavelength, _) = after.axes(chain.cropped).unwrap();
    // Cropping selects the samples inside the interval, so the first kept
    // wavelength is the first one at or above 450 nm, not 450 itself.
    assert!(wavelength.first() >= 450.0);
    assert!(wavelength.first() < 451.0);
    assert!(wavelength.last() <= 750.0);
    assert!(wavelength.len() < after.axes(chain.raw).unwrap().0.len());

    // The saved trace still resolves, and to the same numbers.
    let trace_before = kinetic_trace(before, &chain.selection).unwrap();
    let trace_after = kinetic_trace(after, &chain.selection).unwrap();
    assert_eq!(trace_before.signal.len(), trace_after.signal.len());
    assert!(trace_before
        .signal
        .iter()
        .zip(&trace_after.signal)
        .all(|(a, b)| a.to_bits() == b.to_bits()));

    // And the figure still points at something that exists.
    let figure = &after.figures[0];
    assert_eq!(figure.title, "Kinetics at 550 nm");
    assert_eq!(
        figure.panels[0].x.scale,
        AxisScale::SymmetricLogarithmic { linear_width: 1.0 }
    );
    assert_eq!(figure.dataset_references(), vec![chain.dataset]);

    std::fs::remove_file(&path).ok();
}

#[test]
fn removing_a_derived_version_removes_what_depended_on_it_and_survives_saving() {
    let mut chain = build_chain();
    let dataset = chain.project.dataset_mut(chain.dataset).unwrap();
    let removed = dataset.remove_version(chain.corrected).unwrap();
    assert_eq!(removed.versions.len(), 2);
    assert_eq!(removed.pruned_traces, 1);
    assert_eq!(removed.pruned_series, 1);
    // The trace pointed at the cropped version, which is gone with it.
    assert!(dataset.traces.is_empty());
    assert_eq!(dataset.active_version, chain.raw);

    chain.project.validate().unwrap();
    let path = project_path("removed.taproject");
    ta_io::save_project(&chain.project, &path).unwrap();
    let loaded = ta_io::load_project(&path).unwrap().project;
    let after = loaded.dataset(chain.dataset).unwrap();
    assert_eq!(after.versions.len(), 1);
    assert!(after.version(chain.raw).is_ok());
    std::fs::remove_file(&path).ok();
}

#[test]
fn a_project_does_not_need_its_source_files_once_it_is_saved() {
    let chain = build_chain();
    assert!(
        !chain
            .project
            .dataset(chain.dataset)
            .unwrap()
            .source_paths
            .is_empty(),
        "an imported dataset records where it came from"
    );

    // Record a source that does not exist: saving and reopening must still work,
    // because the matrices travel inside the project.
    let mut project = chain.project.clone();
    project
        .dataset_mut(chain.dataset)
        .unwrap()
        .source_paths
        .push("/nonexistent/moved-file".to_string());

    let path = project_path("moved-sources.taproject");
    ta_io::save_project(&project, &path).unwrap();
    let loaded = ta_io::load_project(&path).unwrap();
    assert!(loaded.datasets_without_data.is_empty());
    let after = loaded.project.dataset(chain.dataset).unwrap();
    assert!(after.matrix(chain.raw).is_ok());
    std::fs::remove_file(&path).ok();
}

#[test]
fn the_expanded_dataset_is_the_full_cropped_grid() {
    // Sanity check that the chain above is built on real numbers, not on a
    // degenerate crop: 450-750 nm of a 400-800 nm dataset.
    let chain = build_chain();
    let dataset = chain.project.dataset(chain.dataset).unwrap();
    let (wavelength, delay) = dataset.axes(chain.cropped).unwrap();
    assert!(wavelength.len() > 500, "expected a substantial window");
    assert_eq!(delay.len(), 152);
    let data = dataset.matrix(chain.cropped).unwrap();
    assert_eq!(data.dim(), (wavelength.len(), delay.len()));
    assert!(data.iter().any(|value| *value != 0.0));
}

#[test]
fn the_fixture_files_are_present_where_the_tests_expect_them() {
    let root = repo_root();
    assert!(root.join("docs/spec.md").exists());
    for name in ["NIR800", "NIR800_tcorr", "NIR800_tatime", "NIR800_tawavelength"] {
        assert!(
            root.join("TA_Analyzer").join("data").join(name).exists(),
            "missing fixture {name}"
        );
    }
    let _: Option<&TaDataset> = None;
    let _: Option<Axis> = None;
}
