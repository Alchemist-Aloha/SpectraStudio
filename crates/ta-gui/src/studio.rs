//! FigureStudio (§17) and the numerical export that goes with it (§18).
//!
//! The studio edits the *same* `FigureModel` the interactive plate produced, so
//! the preview and the exported file cannot disagree: one panel description,
//! one renderer, one set of coordinates. What the studio adds is the physical
//! size, the per-series and per-axis controls, the legend and annotation
//! controls, and the export settings.

use crate::app::{ink, Workspace};
use crate::plates::{self, PlateContent};
use crate::widgets::{self, FieldId};
use gpui_kit::{div, px, IntoElement, Styled};
use std::path::PathBuf;
use ta_core::{
    AnnotationKind, AnnotationSpec, AxisScale, DashPattern, ExportFormat, FigureModel,
    JournalPreset, LegendPosition, LengthUnit, MarkerShape, PublicationStyle, Rgba, SeriesKind,
    SeriesStyle,
};

/// A figure style the user can apply to any figure. The presets of §17.12.
pub fn style_presets() -> Vec<(String, PublicationStyle)> {
    let base = PublicationStyle::default();
    vec![
        (
            "General scientific".to_string(),
            PublicationStyle {
                font_family: "Helvetica".to_string(),
                ..base.clone()
            },
        ),
        (
            "ACS / JACS".to_string(),
            PublicationStyle {
                font_family: "Helvetica".to_string(),
                base_font_size: 8.0,
                axis_title_size: 8.0,
                tick_label_size: 8.0,
                legend_font_size: 7.5,
                series_width: 1.5,
                axis_line_width: 0.75,
                tick_length: 3.0,
                tick_direction: ta_core::TickDirection::Out,
                frame: true,
                grid: false,
                preset: JournalPreset::JacsSingleColumn,
            },
        ),
        (
            "Nature".to_string(),
            PublicationStyle {
                font_family: "Arial".to_string(),
                base_font_size: 7.0,
                axis_title_size: 7.0,
                tick_label_size: 7.0,
                legend_font_size: 7.0,
                series_width: 1.0,
                axis_line_width: 0.5,
                tick_length: 2.5,
                tick_direction: ta_core::TickDirection::Out,
                frame: false,
                grid: false,
                preset: JournalPreset::Custom,
            },
        ),
        (
            "IEEE".to_string(),
            PublicationStyle {
                font_family: "Times New Roman".to_string(),
                base_font_size: 8.0,
                series_width: 1.5,
                grid: true,
                preset: JournalPreset::Custom,
                ..base.clone()
            },
        ),
        (
            "Presentation".to_string(),
            PublicationStyle {
                base_font_size: 14.0,
                axis_title_size: 16.0,
                tick_label_size: 13.0,
                legend_font_size: 13.0,
                series_width: 3.0,
                axis_line_width: 1.5,
                tick_length: 6.0,
                grid: true,
                preset: JournalPreset::Custom,
                ..base.clone()
            },
        ),
    ]
}

/// The size presets of §17.3, with the raster scales of §17.10.
pub const SIZE_PRESETS: [JournalPreset; 3] = [
    JournalPreset::JacsSingleColumn,
    JournalPreset::JacsDoubleColumn,
    JournalPreset::Custom,
];

pub fn preset_label(preset: JournalPreset) -> &'static str {
    match preset {
        JournalPreset::JacsSingleColumn => "single column 3.25 in",
        JournalPreset::JacsDoubleColumn => "double column 7 in",
        JournalPreset::Custom => "custom",
    }
}

impl Workspace {
    /// An edit to a figure's own fields, from the studio's property column.
    pub fn on_figure_field(&mut self, id: FieldId, text: &str) {
        let Some(figure) = self.session.studio.as_mut() else {
            return;
        };
        match id {
            FieldId::FigureWidth => {
                if let Some(value) = widgets::parse_number(text) {
                    figure.size.width = value.max(0.5);
                }
            }
            FieldId::FigureHeight => {
                if let Some(value) = widgets::parse_number(text) {
                    figure.size.height = value.max(0.5);
                }
            }
            FieldId::FigureFontSize => {
                if let Some(value) = widgets::parse_number(text) {
                    figure.set_type_size(value.max(4.0) as f32);
                }
            }
            FieldId::FigureTickSize => {
                if let Some(value) = widgets::parse_number(text) {
                    let size = value.clamp(4.0, 24.0) as f32;
                    // The style's size is the one the layout reserves room for,
                    // so both agree or the margin is measured for the wrong
                    // type.
                    figure.style.tick_label_size = size;
                    for panel in &mut figure.panels {
                        panel.x.tick_label_size = size;
                        panel.y.tick_label_size = size;
                    }
                }
            }
            FieldId::LegendFontSize => {
                if let Some(value) = widgets::parse_number(text) {
                    let size = value.clamp(4.0, 24.0) as f32;
                    figure.style.legend_font_size = size;
                    for panel in &mut figure.panels {
                        panel.legend.font_size = size;
                    }
                }
            }
            FieldId::AxisXRange => {
                if let Some(panel) = figure.panel_mut(0) {
                    panel.x.range = widgets::parse_range(text)
                        .map(|(low, high)| (low, if high.is_finite() { high } else { f64::MAX }));
                }
            }
            FieldId::AxisYRange => {
                if let Some(panel) = figure.panel_mut(0) {
                    panel.y.range = widgets::parse_range(text)
                        .map(|(low, high)| (low, if high.is_finite() { high } else { f64::MAX }));
                }
            }
            FieldId::ContourMin | FieldId::ContourMax => {
                let (min, max) = {
                    let contour = figure.panel(0).and_then(|panel| panel.contour.as_ref());
                    (
                        contour.map(|contour| contour.limits.min).unwrap_or(-1.0),
                        contour.map(|contour| contour.limits.max).unwrap_or(1.0),
                    )
                };
                if let Some(value) = widgets::parse_number(text) {
                    if let Some(contour) =
                        figure.panel_mut(0).and_then(|panel| panel.contour.as_mut())
                    {
                        match id {
                            FieldId::ContourMin => contour.limits.min = value,
                            _ => contour.limits.max = value,
                        }
                        // A limit set by hand stops being automatic, which is
                        // what makes the numbers in the caption truthful.
                        let _ = (min, max);
                        contour.limits.auto = ta_core::AutoRange::Manual;
                    }
                }
            }
            FieldId::ContourLevels => {
                let levels = widgets::parse_numbers(text);
                if let Some(contour) = figure.panel_mut(0).and_then(|panel| panel.contour.as_mut()) {
                    contour.levels = levels;
                }
            }
            FieldId::AnnotationText | FieldId::AnnotationX | FieldId::AnnotationY => {}
            FieldId::AxisXLabel => {
                if let Some(panel) = figure.panel_mut(0) {
                    panel.x.label = text.to_string();
                }
            }
            FieldId::AxisYLabel => {
                if let Some(panel) = figure.panel_mut(0) {
                    panel.y.label = text.to_string();
                }
            }
            FieldId::LegendLabel(index) => {
                if let Some(panel) = figure.panel_mut(0) {
                    if let Some(series) = panel.series.get(index) {
                        let id = series.id;
                        if text.trim().is_empty() {
                            panel.legend.labels.remove(&id);
                        } else {
                            panel.legend.labels.insert(id, text.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
        self.touch();
    }

    /// Opens the studio on a plate, carrying that plate's figure across.
    pub fn open_studio(&mut self, plate: crate::session::PlateId) {
        let spec = plates::contour_spec(&self.session);
        let content = plates::content(&self.session, plate, &spec);
        let mut figure = ta_core::FigureModel::single_panel(self.session.caption(plate));
        let x_label = content.panel.x.label.clone();
        let count = content.series.len();
        let mut records = content.records;
        figure.panels[0] = content.panel;
        figure.panels[0].legend.visible = count > 1;
        figure.panels[0].x.label = x_label;
        // The plate's panel carries the working screen's type sizes; a figure
        // is typed the way the figure says, so the three controls in the panel
        // and the type on screen agree from the first moment.
        figure.set_type_size(figure.style.base_font_size);
        // The records are the plate's own: a reference to a dataset and a
        // coordinate, not a copy of the numbers, so the studio edits the same
        // figure the plate was showing.
        for (index, record) in records.iter_mut().enumerate() {
            record.z = index as i32;
            record.style = content_style(plate, index, count);
            figure.panels[0].series.push(record.clone());
        }
        self.session.studio = Some(figure);
        self.layout = crate::app::Layout::Studio;
        self.current_plate = plate;
        self.session
            .status
            .info(format!("FigureStudio opened on {}", plate.letter()));
        self.touch();
    }

    /// Writes the version on the bench as Glotaran input.
    ///
    /// Both files are written, because the two Glotaran generations want
    /// different things and a user should not have to know which one their
    /// installation reads: the legacy time-explicit ASCII, and the modern
    /// time/spectral NetCDF dataset. The export names the version it wrote.
    pub fn export_glotaran(&mut self) {
        let Some(dataset) = self.session.dataset() else {
            self.session.status.problem("no dataset is open");
            return;
        };
        let Some(version) = self.session.version else {
            self.session.status.problem("no version is on the bench");
            return;
        };
        let view = match ta_glotaran::GloDataset::from_version(dataset, version) {
            Ok(view) => view,
            Err(error) => {
                self.session.status.problem(format!("{error}"));
                return;
            }
        };
        let directory = self.export_directory();
        let stem = slug(&format!("{}-glo", dataset.name));
        let ascii_path = directory.join(format!("{stem}.ascii"));
        let netcdf_path = directory.join(format!("{stem}.nc"));
        match ta_glotaran::write_ascii(&view, &ascii_path) {
            Ok(report) => {
                let netcdf = ta_glotaran::write_netcdf(&view, &netcdf_path);
                let extra = match netcdf {
                    Ok(report) => format!(" and {} (NetCDF)", report.path),
                    Err(error) => format!("; the NetCDF file failed: {error}"),
                };
                self.session.status.info(format!(
                    "Glotaran input written for {}: {} rows × {} time points to {}{extra}",
                    view.version_label, report.rows, report.columns, report.path
                ));
            }
            Err(error) => self.session.status.problem(format!("{error}")),
        }
        self.touch();
    }

    /// Opens the workbench on the current selection.
    pub fn open_fit(&mut self) {
        let _ = self.session.fit_session();
        self.layout = crate::app::Layout::Fit;
        self.session
            .status
            .info("the workbench is ready; choose a model and fit");
        self.touch();
    }

    /// Writes one plate's numbers without opening the studio: the Export Data
    /// action §18 requires beside every plot.
    pub fn export_plate_data(&mut self, plate: crate::session::PlateId) {
        let spec = plates::contour_spec(&self.session);
        let content = plates::content(&self.session, plate, &spec);
        let directory = self.export_directory();
        let stem = format!("spectra-studio-{}-data", plate.letter());
        let path = directory.join(format!("{stem}.csv"));
        let options = ta_io::ExportOptions::default();
        match plate {
            crate::session::PlateId::Contour => {
                let (Some(dataset), Some(version)) = (self.session.dataset(), self.session.version)
                else {
                    return;
                };
                let (Ok((wavelength, delay)), Ok(matrix)) =
                    (dataset.axes(version), dataset.matrix(version))
                else {
                    return;
                };
                let export = ta_io::MatrixExport {
                    wavelength,
                    delay,
                    data: matrix,
                    unit: dataset
                        .unit(version)
                        .unwrap_or(ta_core::SignalUnit::DeltaOpticalDensity),
                    rows: None,
                    columns: None,
                    notes: vec![
                        format!("{} · {}", dataset.name, self.session.caption(plate)),
                        "the full matrix, every measured wavelength and delay".to_string(),
                        "NaN marks a sample that was not measured or was masked".to_string(),
                    ],
                };
                let matrix_path = directory.join(format!("{stem}-matrix.csv"));
                match ta_io::write_matrix(&matrix_path, &export, options) {
                    Ok(()) => {
                        // §18: the axes travel with the matrix.
                        let wavelength_path = directory.join(format!("{stem}-wavelength.csv"));
                        let delay_path = directory.join(format!("{stem}-delay.csv"));
                        let _ = ta_io::write_axis(&wavelength_path, wavelength, options);
                        let _ = ta_io::write_axis(&delay_path, delay, options);
                        self.session.status.info(format!(
                            "exported {} with its axes beside it",
                            matrix_path.display()
                        ));
                    }
                    Err(error) => self.session.status.problem(format!("{error}")),
                }
            }
            _ => {
                let blocks: Vec<ta_io::SeriesBlock> = content
                    .series
                    .iter()
                    .map(|series| {
                        let points: Vec<(f64, f64)> = series
                            .segments
                            .iter()
                            .flat_map(|segment| segment.iter().copied())
                            .collect();
                        ta_io::SeriesBlock::new(
                            series.label.clone(),
                            content.panel.x.labelled(),
                            content.panel.y.labelled(),
                            points.iter().map(|(x, _)| *x).collect(),
                            points.iter().map(|(_, y)| *y).collect(),
                        )
                    })
                    .collect();
                let notes = vec![
                    format!(
                        "{} · {}",
                        self.session
                            .dataset()
                            .map(|dataset| dataset.name.clone())
                            .unwrap_or_default(),
                        self.session.caption(plate)
                    ),
                    "full measured resolution; no decimation".to_string(),
                ];
                match ta_io::write_blocks(&path, &blocks, &notes, options) {
                    Ok(written) => self
                        .session
                        .status
                        .info(format!("exported {written} points to {}", path.display())),
                    Err(error) => self.session.status.problem(format!("{error}")),
                }
            }
        }
    }

    /// The figure the studio is editing, carrying the plate's own curves.
    pub fn studio_figure(&self) -> Option<&FigureModel> {
        self.session.studio.as_ref()
    }

    /// The panel the preview draws, with its series resolved to numbers.
    pub fn studio_preview(&self) -> Option<PlateContent> {
        let figure = self.session.studio.as_ref()?;
        let panel = figure.panel(0)?;
        let curves = crate::resolve::resolve_panel(
            &self.session.project,
            panel,
            &crate::resolve::ResolveOptions {
                normalization: ta_core::Normalization::None,
                alignment: ta_core::Alignment::AsMeasured,
                visible_only: true,
            },
        );
        let mut content = PlateContent {
            panel: panel.clone(),
            series: curves.iter().map(|curve| curve.plotted()).collect(),
            records: panel.series.clone(),
        };
        // A plate with no saved series still shows what it was opened on: a
        // contour's isolines, or the curve the plate drew. The *panel* stays the
        // figure's own — it is the thing the editor's controls change, and a
        // contour has no series precisely because its field is the matrix.
        if content.series.is_empty() {
            let spec = plates::contour_spec(&self.session);
            let fresh = plates::content(&self.session, self.current_plate, &spec);
            content.series = fresh.series;
        }
        Some(content)
    }

    /// The studio's export actions: the figure, and its numbers.
    pub fn export_studio(&mut self, format: ExportFormat) {
        let Some(figure) = self.session.studio.clone() else {
            return;
        };
        let Some(content) = self.studio_preview() else {
            return;
        };
        let directory = self.export_directory();
        let stem = slug(&figure.title);
        let path = directory.join(format!("{stem}.{}", extension(format)));
        let series = content.series.clone();
        let mut panels = vec![ta_plot::export::ExportPanel::new(0, series.clone())];
        let (figure_width, figure_height) = ta_plot::export::figure_points(&figure);
        if self.current_plate == crate::session::PlateId::Contour {
            if let Some(source) = &self.contour_source {
                let x_scale = ta_plot::Scale::new(figure.panels[0].x.scale).unwrap_or_default();
                let y_scale = ta_plot::Scale::new(figure.panels[0].y.scale).unwrap_or_default();
                let layout = ta_plot::PanelLayout::compute(
                    &figure.panels[0],
                    &figure.style,
                    x_scale,
                    y_scale,
                    figure.panels[0].x.range.unwrap_or((0.0, 1.0)),
                    figure.panels[0].y.range.unwrap_or((0.0, 1.0)),
                    (figure_width, figure_height),
                );
                let (plot_width, plot_height) = layout.plot_size();
                let (x_range, y_range) = self.plate_window();
                if let Some(layer) = source.layer(
                    &mut self.raster.borrow_mut(),
                    (
                        plot_width.round().max(2.0) as u32,
                        plot_height.round().max(2.0) as u32,
                    ),
                    x_range,
                    y_range,
                ) {
                    panels = vec![ta_plot::export::ExportPanel::new(0, series)
                        .with_heatmap((*layer).clone())];
                }
            }
        }
        // §17.11: the export is checked before it is written, not after.
        match ta_plot::export::audit(&figure, &panels) {
            Ok(report) if report.is_clean() => {
                match ta_plot::export::export(&figure, panels, &path) {
                    Ok(()) => self
                        .session
                        .status
                        .info(format!("exported {}", path.display())),
                    Err(error) => self.session.status.problem(format!("{error}")),
                };
            }
            Ok(report) => {
                self.session.status.problem(format!(
                    "the export would not be clean: {}",
                    report.summary()
                ));
            }
            Err(error) => self.session.status.problem(format!("{error}")),
        }
    }

    /// Writes the numbers behind the studio's panel: one file for a curve
    /// panel, three for a contour.
    pub fn export_studio_data(&mut self, visible_only: bool) {
        let Some(content) = self.studio_preview() else {
            return;
        };
        let directory = self.export_directory();
        let figure = self.session.studio.clone();
        let stem = figure
            .as_ref()
            .map(|figure| slug(&figure.title))
            .unwrap_or_else(|| "figure".to_string());
        let options = ta_io::ExportOptions {
            delimiter: ta_io::Delimiter::Comma,
            ..Default::default()
        };

        if self.current_plate == crate::session::PlateId::Contour {
            let Some(dataset) = self.session.dataset() else {
                return;
            };
            let Some(version) = self.session.version else {
                return;
            };
            let (Ok((wavelength, delay)), Ok(matrix)) = (dataset.axes(version), dataset.matrix(version))
            else {
                return;
            };
            let (x_range, y_range) = self.plate_window();
            let rows: Vec<usize> = (0..wavelength.len())
                .filter(|row| {
                    !visible_only
                        || (wavelength.values()[*row] >= x_range.0
                            && wavelength.values()[*row] <= x_range.1)
                })
                .collect();
            let columns: Vec<usize> = (0..delay.len())
                .filter(|column| {
                    !visible_only
                        || (delay.values()[*column] >= y_range.0
                            && delay.values()[*column] <= y_range.1)
                })
                .collect();
            let rows_count = rows.len();
            let columns_count = columns.len();
            let matrix_path = directory.join(format!("{stem}-matrix.csv"));
            let export = ta_io::MatrixExport {
                wavelength,
                delay,
                data: matrix,
                unit: dataset.unit(version).unwrap_or(ta_core::SignalUnit::DeltaOpticalDensity),
                rows: Some(rows),
                columns: Some(columns),
                notes: vec![
                    figure
                        .as_ref()
                        .map(|figure| format!("figure: {}", figure.title))
                        .unwrap_or_default(),
                    format!(
                        "{} · {} · {} against {}",
                        dataset.name,
                        self.session
                            .version()
                            .map(|version| version.label.clone())
                            .unwrap_or_default(),
                        wavelength.name(),
                        delay.name()
                    ),
                    format!(
                        "{} rows × {} columns written{}",
                        if visible_only { rows_count } else { wavelength.len() },
                        if visible_only { columns_count } else { delay.len() },
                        if visible_only { " (the visible range only)" } else { "" }
                    ),
                    "NaN marks a sample that was not measured or was masked".to_string(),
                ],
            };
            match ta_io::write_matrix(&matrix_path, &export, options) {
                Ok(()) => self.session.status.info(format!(
                    "exported {} (axis files beside it)",
                    matrix_path.display()
                )),
                Err(error) => self.session.status.problem(format!("{error}")),
            }
            return;
        }

        let path = directory.join(format!("{stem}-data.csv"));
        let x_range = content.panel.x.range;
        let blocks: Vec<ta_io::SeriesBlock> = content
            .series
            .iter()
            .map(|series| {
                // §18: the full measured resolution, or the window that is on
                // screen — never a decimated version of either.
                let points: Vec<(f64, f64)> = series
                    .segments
                    .iter()
                    .flat_map(|segment| segment.iter().copied())
                    .filter(|(x, _)| match (visible_only, x_range) {
                        (true, Some((low, high))) => *x >= low && *x <= high,
                        _ => true,
                    })
                    .collect();
                ta_io::SeriesBlock::new(
                    series.label.clone(),
                    content.panel.x.labelled(),
                    content.panel.y.labelled(),
                    points.iter().map(|(x, _)| *x).collect(),
                    points.iter().map(|(_, y)| *y).collect(),
                )
            })
            .collect();
        let notes = curve_notes(&figure, &content, visible_only, &blocks);
        match ta_io::write_blocks(&path, &blocks, &notes, options) {
            Ok(written) => self.session.status.info(format!(
                "exported {} points to {}",
                written,
                path.display()
            )),
            Err(error) => self.session.status.problem(format!("{error}")),
        }
    }

    /// Where exports go: beside the project, else beside the file that was
    /// imported, else the temporary directory. A figure written next to the
    /// data it came from is the one a user can find again.
    fn export_directory(&self) -> PathBuf {
        if let Some(path) = &self.session.path {
            if let Some(parent) = path.parent() {
                return parent.to_path_buf();
            }
        }
        if let Some(source) = self
            .session
            .dataset()
            .and_then(|dataset| dataset.source_paths.first())
        {
            let path = PathBuf::from(source);
            if let Some(parent) = path.parent() {
                return parent.to_path_buf();
            }
        }
        std::env::temp_dir()
    }

    fn plate_window(&self) -> ((f64, f64), (f64, f64)) {
        state_window_of(&self.plates, 0)
    }
}

/// The style a plate's curve carries into the studio, so the editor opens on
/// the figure the plate was already showing rather than on a house default.
fn content_style(plate: crate::session::PlateId, index: usize, count: usize) -> SeriesStyle {
    use crate::session::PlateId;
    match plate {
        PlateId::Kinetics | PlateId::Spectra => {
            // The last curve is the linked selection, which the plates accent.
            let linked = index + 1 == count;
            SeriesStyle {
                color: if linked {
                    plates::accent()
                } else {
                    crate::compare::palette(index)
                },
                width: if linked { 1.5 } else { 1.0 },
                dash: if linked {
                    DashPattern::Solid
                } else {
                    DashPattern::Dashed
                },
                ..SeriesStyle::default()
            }
        }
        PlateId::TimeZero => SeriesStyle {
            color: plates::warm(),
            marker: MarkerShape::Circle,
            ..SeriesStyle::default()
        },
        PlateId::Contour => SeriesStyle::default(),
    }
}

fn state_window_of(plates: &[ta_plot_gpui::Plate], index: usize) -> ((f64, f64), (f64, f64)) {
    match plates.get(index) {
        Some(plate) => {
            let state = plate.state.borrow();
            (state.x_range, state.y_range)
        }
        None => ((0.0, 1.0), (0.0, 1.0)),
    }
}

pub fn extension(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Svg => "svg",
        ExportFormat::Png => "png",
        ExportFormat::Pdf => "pdf",
        ExportFormat::Eps => "eps",
    }
}

/// A file stem made from a caption.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for character in text.chars() {
        if character.is_ascii_alphanumeric() {
            out.push(character.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "figure".to_string()
    } else {
        trimmed.chars().take(48).collect()
    }
}

/// An annotation a user can add without leaving the studio: the reference line
/// a paper figure usually carries at the delay of interest.
pub fn reference_line(axis: ta_core::AxisKind, value: f64, label: &str) -> AnnotationSpec {
    AnnotationSpec {
        id: ta_core::AnnotationId::new(),
        kind: AnnotationKind::CursorLine,
        text: label.to_string(),
        x: Some(value),
        y: None,
        width: None,
        height: None,
        axis: Some(axis),
        color: Rgba::rgb(120, 120, 120),
        font_size: 7.0,
    }
}

/// The axis scales the studio offers, including the split convention of §12.6.
pub fn scale_choices() -> Vec<(&'static str, AxisScale)> {
    vec![
        ("linear", AxisScale::Linear),
        ("log", AxisScale::Logarithmic),
        ("symlog", AxisScale::SymmetricLogarithmic { linear_width: 1.0 }),
        ("split 1 ps", AxisScale::SplitLinearLog { break_at: 1.0 }),
    ]
}

pub fn legend_positions() -> Vec<(&'static str, LegendPosition)> {
    vec![
        ("top left", LegendPosition::TopLeft),
        ("top right", LegendPosition::TopRight),
        ("bottom left", LegendPosition::BottomLeft),
        ("bottom right", LegendPosition::BottomRight),
        ("outside right", LegendPosition::OutsideRight),
        ("outside bottom", LegendPosition::OutsideBottom),
    ]
}

pub fn units() -> Vec<(&'static str, LengthUnit)> {
    vec![
        ("in", LengthUnit::Inches),
        ("cm", LengthUnit::Centimeters),
        ("mm", LengthUnit::Millimeters),
        ("px", LengthUnit::Pixels),
    ]
}

pub fn dashes() -> Vec<(&'static str, DashPattern)> {
    vec![
        ("solid", DashPattern::Solid),
        ("dashed", DashPattern::Dashed),
        ("dotted", DashPattern::Dotted),
        ("dash-dot", DashPattern::DashDot),
    ]
}

pub fn markers() -> Vec<(&'static str, MarkerShape)> {
    vec![
        ("none", MarkerShape::None),
        ("circle", MarkerShape::Circle),
        ("square", MarkerShape::Square),
        ("triangle", MarkerShape::Triangle),
        ("diamond", MarkerShape::Diamond),
        ("cross", MarkerShape::Cross),
    ]
}

pub fn kinds() -> Vec<(&'static str, SeriesKind)> {
    vec![
        ("line", SeriesKind::Line),
        ("markers", SeriesKind::Markers),
        ("line + markers", SeriesKind::LineMarkers),
        ("scatter", SeriesKind::Scatter),
        ("step", SeriesKind::Step),
        ("bars", SeriesKind::Bars),
    ]
}

/// The provenance lines of a curve export: what the figure is, which version
/// produced it, and how the coordinates were read.
fn curve_notes(
    figure: &Option<FigureModel>,
    content: &PlateContent,
    visible_only: bool,
    blocks: &[ta_io::SeriesBlock],
) -> Vec<String> {
    let mut notes = Vec::new();
    if let Some(figure) = figure {
        notes.push(format!("figure: {}", figure.title));
        notes.push(format!(
            "size: {:.3} × {:.3} {}",
            figure.size.width,
            figure.size.height,
            figure.size.unit.symbol()
        ));
    }
    notes.push(format!(
        "axes: {} against {}",
        content.panel.y.labelled(),
        content.panel.x.labelled()
    ));
    notes.push(format!(
        "range: {}",
        if visible_only {
            "the visible window only"
        } else {
            "the full measured range"
        }
    ));
    notes.push(format!("series: {}", blocks.len()));
    for block in blocks {
        notes.push(format!("  {} — {} points", block.label, block.len()));
    }
    notes.push("a blank line separates each series; no curve was interpolated".to_string());
    notes
}

/// The colour of a series as a UI swatch.
pub fn swatch(color: Rgba, selected: bool) -> impl IntoElement {
    div()
        .w(px(12.0))
        .h(px(12.0))
        .bg(ink::hsla(color))
        .border_1()
        .border_color(if selected { ink::text() } else { ink::rule() })
}

/// The editor's own colour choices for a series: the palette, so a figure made
/// in the studio matches one made in the comparison layout.
pub fn series_colors() -> Vec<Rgba> {
    (0..6).map(crate::compare::palette).collect()
}

impl Workspace {
    /// Re-opens the studio's figure on the plate currently selected.
    pub fn reload_studio(&mut self) {
        let plate = self.current_plate;
        self.open_studio(plate);
    }

    /// Applies one of the §17.12 style presets by name.
    pub fn apply_style_preset(&mut self, name: &str) {
        let Some((_, style)) = style_presets()
            .into_iter()
            .find(|(candidate, _)| candidate == name)
        else {
            return;
        };
        if let Some(figure) = self.session.studio.as_mut() {
            let preset = style.preset;
            figure.style = style;
            // The preset sets the type; the size preset keeps the geometry.
            figure.style.preset = preset;
            for panel in &mut figure.panels {
                panel.x.title_size = figure.style.axis_title_size;
                panel.x.tick_label_size = figure.style.tick_label_size;
                panel.y.title_size = figure.style.axis_title_size;
                panel.y.tick_label_size = figure.style.tick_label_size;
                panel.legend.font_size = figure.style.legend_font_size;
            }
        }
        self.touch();
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_figure_opened_from_a_plate_is_typed_the_way_the_figure_says() {
        // The plate's panel carries the working screen's sizes. A figure is
        // typed at the figure's size, so the studio's three type controls and
        // the type on screen agree as soon as the studio opens.
        use crate::session::Session;
        use ndarray::Array2;
        use ta_core::{Axis, AxisKind, AxisUnit, SignalUnit, SourceFormat, TaDataset};

        let wavelength = Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "Wavelength",
            vec![500.0, 600.0, 700.0],
        )
        .unwrap();
        let delay = Axis::new(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            "Delay",
            vec![-1.0, 0.0, 1.0, 10.0],
        )
        .unwrap();
        let data = Array2::from_elem((3, 4), 0.01);
        let dataset = TaDataset::new_raw(
            "test",
            wavelength,
            delay,
            data,
            SignalUnit::DeltaOpticalDensity,
            SourceFormat::Synthetic,
        )
        .unwrap();
        let mut session = Session::new();
        session.open_dataset(dataset);
        let mut workspace = crate::app::Workspace::new(session);
        workspace.open_studio(crate::session::PlateId::Kinetics);

        let figure = workspace.studio_figure().expect("the studio is open");
        // The working plate's type is larger than a figure's.
        let plate_type = crate::plates::style().tick_label_size;
        assert!(plate_type > figure.style.tick_label_size);
        let panel = figure.panel(0).unwrap();
        assert_eq!(panel.x.tick_label_size, figure.style.tick_label_size);
        assert_eq!(panel.y.tick_label_size, figure.style.tick_label_size);
        assert_eq!(panel.x.title_size, figure.style.axis_title_size);
        assert_eq!(panel.y.title_size, figure.style.axis_title_size);
        assert_eq!(panel.legend.font_size, figure.style.legend_font_size);
    }

    #[test]
    fn a_caption_becomes_a_file_stem() {
        assert_eq!(slug("Kinetics at 600.00 nm"), "kinetics-at-600-00-nm");
        assert_eq!(slug(""), "figure");
        assert_eq!(slug("ΔA contour, 400–800 nm"), "a-contour-400-800-nm");
    }

    #[test]
    fn every_export_format_has_an_extension() {
        assert_eq!(extension(ExportFormat::Svg), "svg");
        assert_eq!(extension(ExportFormat::Png), "png");
        assert_eq!(extension(ExportFormat::Pdf), "pdf");
        assert_eq!(extension(ExportFormat::Eps), "eps");
    }

    #[test]
    fn the_presets_cover_the_journal_sizes_the_spec_names() {
        let single = JournalPreset::JacsSingleColumn.size();
        assert_eq!(single.width, 3.25);
        assert_eq!(single.unit, LengthUnit::Inches);
        let double = JournalPreset::JacsDoubleColumn.size();
        assert_eq!(double.width, 7.0);
        // And the type size the presets imply is eight points.
        let mut figure = FigureModel::single_panel("test");
        figure.apply_preset(JournalPreset::JacsSingleColumn);
        assert_eq!(figure.style.base_font_size, 8.0);
        assert_eq!(figure.panels[0].x.title_size, 8.0);
    }

    #[test]
    fn a_style_preset_carries_its_type_into_every_panel() {
        let styles = style_presets();
        assert_eq!(styles.len(), 5);
        let (_, nature) = styles
            .iter()
            .find(|(name, _)| name == "Nature")
            .expect("the Nature preset exists");
        assert_eq!(nature.font_family, "Arial");
        assert!(!nature.frame);
    }

    #[test]
    fn the_studio_offers_the_split_axis_the_fitting_convention_needs() {
        let scales = scale_choices();
        assert!(scales
            .iter()
            .any(|(_, scale)| matches!(scale, AxisScale::SplitLinearLog { .. })));
        assert!(scales
            .iter()
            .any(|(_, scale)| matches!(scale, AxisScale::SymmetricLogarithmic { .. })));
    }

    #[test]
    fn a_reference_line_records_the_axis_it_belongs_to() {
        let line = reference_line(ta_core::AxisKind::Delay, 1.0, "1 ps");
        assert_eq!(line.kind, AnnotationKind::CursorLine);
        assert_eq!(line.axis, Some(ta_core::AxisKind::Delay));
        assert_eq!(line.x, Some(1.0));
    }

    #[test]
    fn a_figure_sized_for_one_convention_is_typed_for_it_too() {
        let mut figure = FigureModel::single_panel("test");
        figure.apply_preset(JournalPreset::JacsDoubleColumn);
        assert_eq!(figure.size.width, 7.0);
        assert_eq!(figure.style.base_font_size, 8.0);
        // And a deliberate change of type is not undone by a later size change.
        figure.set_type_size(11.0);
        assert_eq!(figure.panels[0].x.tick_label_size, 11.0);
        assert_eq!(figure.style.legend_font_size, 11.0);
    }
}
