//! The `.taproject` file: a JSON manifest plus a binary data layer.
//!
//! Matrices do not go into the JSON. They are appended as little-endian `f64`
//! payloads after the manifest, so a project stays readable, diffable and
//! small, and a 2000×1000 matrix costs what it costs rather than ten times that
//! in decimal text.
//!
//! shortcut: portable projects only; source-referenced (lightweight) projects
//! arrive with the processing pipeline, so a relinked project can re-derive its
//! versions instead of shipping them.

use std::io::Write;
use std::path::Path;
use ta_core::{CoreError, DatasetId, Project, PROJECT_SCHEMA};

/// File signature. The trailing digits are the container version, not the
/// manifest schema, so either can move without ambiguity.
const MAGIC: &[u8; 8] = b"TAPROJ01";

/// What a load produced.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedProject {
    pub project: Project,
    /// Datasets whose matrices could not be read back, so the interface can
    /// offer to relink them instead of showing zeros.
    pub datasets_without_data: Vec<DatasetId>,
}

fn malformed(path: &Path, detail: impl std::fmt::Display) -> CoreError {
    CoreError::MalformedProject {
        path: path.display().to_string(),
        detail: detail.to_string(),
    }
}

/// Writes a project, matrices included.
pub fn save_project(project: &Project, path: &Path) -> ta_core::Result<()> {
    project.validate()?;
    let manifest = serde_json::to_vec(project)
        .map_err(|error| CoreError::invalid(format!("cannot serialise the project: {error}")))?;

    // Write beside the destination so a failed save never truncates the last good project.
    let temporary = path.with_file_name(format!(".taproject-{}.tmp", uuid::Uuid::new_v4()));
    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)
        .map_err(|source| CoreError::Io {
            path: path.display().to_string(),
            source,
        })?;
    let mut writer = std::io::BufWriter::new(file);

    let write = |writer: &mut std::io::BufWriter<std::fs::File>| -> std::io::Result<()> {
        writer.write_all(MAGIC)?;
        writer.write_all(&PROJECT_SCHEMA.to_le_bytes())?;
        writer.write_all(&(manifest.len() as u64).to_le_bytes())?;
        writer.write_all(&manifest)?;
        for dataset in &project.datasets {
            for block in &dataset.blocks {
                let (rows, cols) = block.shape();
                writer.write_all(&(rows as u64).to_le_bytes())?;
                writer.write_all(&(cols as u64).to_le_bytes())?;
                // Row-major, matching the wavelength-major convention.
                for row in 0..rows {
                    for column in 0..cols {
                        writer.write_all(&block.data[[row, column]].to_le_bytes())?;
                    }
                }
            }
        }
        Ok(())
    };

    let result = (|| {
        if let Ok(metadata) = std::fs::metadata(path) {
            writer.get_ref().set_permissions(metadata.permissions())?;
        }
        write(&mut writer)?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        drop(writer);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|source| CoreError::Io {
        path: path.display().to_string(),
        source,
    })
}

/// Reads a project and validates every cross-reference before handing it back.
pub fn load_project(path: &Path) -> ta_core::Result<LoadedProject> {
    let bytes = std::fs::read(path).map_err(|source| CoreError::Io {
        path: path.display().to_string(),
        source,
    })?;

    if bytes.len() < 20 {
        return Err(malformed(path, "file is too short to be a project"));
    }
    if &bytes[0..8] != MAGIC {
        return Err(malformed(
            path,
            "not a Spectra Studio project (bad file signature)",
        ));
    }
    let container = u32::from_le_bytes(bytes[8..12].try_into().expect("four bytes"));
    if container > PROJECT_SCHEMA {
        return Err(CoreError::ProjectTooNew {
            found: container,
            supported: PROJECT_SCHEMA,
        });
    }
    let manifest_len =
        u64::from_le_bytes(bytes[12..20].try_into().expect("eight bytes")) as usize;
    let manifest_end = 20usize
        .checked_add(manifest_len)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| malformed(path, "manifest length runs past the end of the file"))?;

    let mut project: Project = serde_json::from_slice(&bytes[20..manifest_end])
        .map_err(|error| malformed(path, format!("cannot read the manifest: {error}")))?;

    // Walk the payload in the same order it was written.
    let mut cursor = manifest_end;
    let mut datasets_without_data = Vec::new();
    for dataset in &mut project.datasets {
        let mut needs_data = false;
        for block in &mut dataset.blocks {
            let rows = block.rows();
            let cols = block.cols();
            if rows == 0 || cols == 0 {
                continue;
            }
            if cursor + 16 > bytes.len() {
                needs_data = true;
                break;
            }
            let stored_rows =
                u64::from_le_bytes(bytes[cursor..cursor + 8].try_into().expect("eight bytes"))
                    as usize;
            let stored_cols =
                u64::from_le_bytes(bytes[cursor + 8..cursor + 16].try_into().expect("eight bytes"))
                    as usize;
            cursor += 16;
            if stored_rows != rows || stored_cols != cols {
                return Err(malformed(
                    path,
                    format!(
                        "payload shape {stored_rows}x{stored_cols} does not match the axes' \
                         {rows}x{cols}"
                    ),
                ));
            }
            let values = rows * cols;
            let needed = values * 8;
            if cursor + needed > bytes.len() {
                return Err(malformed(path, "matrix payload is truncated"));
            }
            let mut data = ndarray::Array2::zeros((rows, cols));
            for row in 0..rows {
                for column in 0..cols {
                    let start = cursor + (row * cols + column) * 8;
                    let value = f64::from_le_bytes(
                        bytes[start..start + 8].try_into().expect("eight bytes"),
                    );
                    data[[row, column]] = value;
                }
            }
            cursor += needed;
            block.data = data;
        }
        if needs_data {
            datasets_without_data.push(dataset.id);
        }
    }

    project.validate()?;
    Ok(LoadedProject {
        project,
        datasets_without_data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;
    use ta_core::{
        Axis, AxisKind, AxisUnit, DatasetId, Project, SignalUnit, SourceFormat, TaDataset,
    };

    fn dataset(name: &str) -> TaDataset {
        let wavelength = Axis::new(
            AxisKind::Wavelength,
            AxisUnit::Nanometer,
            "Wavelength",
            vec![500.0, 510.0, 520.0],
        )
        .unwrap();
        let delay = Axis::new(
            AxisKind::Delay,
            AxisUnit::Picosecond,
            "Delay",
            vec![-1.0, 0.0],
        )
        .unwrap();
        let mut dataset = TaDataset::new_raw(
            name,
            wavelength,
            delay,
            array![[0.5, f64::NAN], [1.25, -2.75], [0.0, 1e-9]],
            SignalUnit::MilliDeltaOpticalDensity,
            SourceFormat::TextMatrix,
        )
        .unwrap();
        dataset.source_paths = vec!["/data/NIR800".to_string()];
        dataset
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("taproject-test-{}-{name}", std::process::id()));
        path
    }

    /// Compares two matrices bit for bit, which is the only honest way to check
    /// a round trip of data that is allowed to contain `NaN`.
    fn same_bits(left: &ndarray::Array2<f64>, right: &ndarray::Array2<f64>) -> bool {
        left.dim() == right.dim()
            && left
                .iter()
                .zip(right.iter())
                .all(|(a, b)| a.to_bits() == b.to_bits())
    }

    #[test]
    fn a_project_round_trips_with_bit_identical_matrices() {
        let mut project = Project::new("NIR800 study");
        project.add_dataset(dataset("NIR800"));
        project.layout = Some(serde_json::json!({"center": "contour"}));
        let path = temp_path("roundtrip.taproject");

        save_project(&project, &path).unwrap();
        let loaded = load_project(&path).unwrap();

        assert_eq!(loaded.project.name, "NIR800 study");
        assert!(loaded.datasets_without_data.is_empty());
        assert_eq!(loaded.project.datasets.len(), 1);
        let before = &project.datasets[0];
        let after = &loaded.project.datasets[0];
        assert!(same_bits(&before.blocks[0].data, &after.blocks[0].data));
        assert_eq!(before.wavelength(), after.wavelength());
        assert_eq!(before.delay(), after.delay());
        assert_eq!(before.blocks[0].unit, after.blocks[0].unit);
        assert_eq!(before.source_paths, after.source_paths);
        // NaN survives the round trip rather than turning into a number.
        assert!(after.blocks[0].data[[0, 1]].is_nan());
        assert_eq!(loaded.project.layout, project.layout);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_save_replaces_a_project_only_after_the_new_file_is_complete() {
        let mut project = Project::new("before");
        project.add_dataset(dataset("a"));
        let path = temp_path("replace.taproject");
        save_project(&project, &path).unwrap();
        let before = std::fs::read(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        project.schema = PROJECT_SCHEMA + 1;
        assert!(save_project(&project, &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        project.schema = PROJECT_SCHEMA;
        project.name = "after".into();
        save_project(&project, &path).unwrap();
        assert_eq!(load_project(&path).unwrap().project.name, "after");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // A rename failure also leaves the existing destination intact.
        let directory = temp_path("rename-failure");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("keep"), b"original").unwrap();
        assert!(save_project(&project, &directory).is_err());
        assert_eq!(std::fs::read(directory.join("keep")).unwrap(), b"original");
        std::fs::remove_dir_all(directory).unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_file_without_the_signature_is_refused() {
        let path = temp_path("bad.taproject");
        std::fs::write(&path, b"{\"name\": \"not ours\"}").unwrap();
        let error = load_project(&path).unwrap_err();
        assert!(format!("{error}").contains("signature"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_truncated_payload_is_refused_rather_than_half_loaded() {
        let mut project = Project::new("truncated");
        project.add_dataset(dataset("a"));
        let path = temp_path("truncated.taproject");
        save_project(&project, &path).unwrap();

        let mut bytes = std::fs::read(&path).unwrap();
        bytes.truncate(bytes.len() - 8);
        std::fs::write(&path, &bytes).unwrap();

        let error = load_project(&path).unwrap_err();
        assert!(format!("{error}").contains("truncated"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_manifest_from_the_future_is_refused() {
        let path = temp_path("future.taproject");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&(PROJECT_SCHEMA + 1).to_le_bytes());
        bytes.extend_from_slice(&2u64.to_le_bytes());
        bytes.extend_from_slice(b"{}");
        std::fs::write(&path, &bytes).unwrap();
        assert!(matches!(
            load_project(&path).unwrap_err(),
            CoreError::ProjectTooNew { .. }
        ));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn every_dataset_gets_a_stable_identity_through_a_save_and_load() {
        let mut project = Project::new("ids");
        let id: DatasetId = project.add_dataset(dataset("a"));
        let path = temp_path("ids.taproject");
        save_project(&project, &path).unwrap();
        let loaded = load_project(&path).unwrap();
        assert_eq!(loaded.project.datasets[0].id, id);
        std::fs::remove_file(&path).ok();
    }
}
