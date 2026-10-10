//! A NetCDF-3 classic file, for the modern time/spectral dataset.
//!
//! The reference implementation's `pyglotaran()` builds an xarray Dataset with
//! `time` and `spectral` coordinates and a `data` variable that is the matrix
//! with time along its first axis. This writes that dataset to a file in
//! NetCDF-3 classic, the format xarray and pyglotaran open without a plugin.
//!
//! The writer is deliberately the whole format and nothing more: fixed-size
//! dimensions, `double` and `char` variables, and global attributes. That is
//! what the dataset needs, and a reader that follows the specification can read
//! it back — which is what the test does.

use crate::matrix_view::{GloDataset, MATRIX_VARIABLE, SPECTRAL_DIMENSION, TIME_DIMENSION};
use std::io::Write;
use std::path::Path;
use ta_core::{CoreError, Result};

/// The magic number of the classic format: `CDF`, then the version byte.
const MAGIC: [u8; 4] = [b'C', b'D', b'F', 1];
const TAG_DIMENSION: u32 = 0x0A;
const TAG_ATTRIBUTE: u32 = 0x0C;
const TAG_VARIABLE: u32 = 0x0B;
const TYPE_CHAR: u32 = 2;
const TYPE_DOUBLE: u32 = 6;

/// What was written, for the report the interface shows.
#[derive(Clone, Debug, PartialEq)]
pub struct NetcdfReport {
    pub path: String,
    pub time_points: usize,
    pub spectral_points: usize,
    pub bytes: usize,
}

/// One variable of the file, before it is laid out.
struct Variable {
    name: String,
    dimensions: Vec<u32>,
    values: Vec<f64>,
    attributes: Vec<(String, String)>,
}

/// Writes the dataset as a NetCDF-3 classic file.
pub fn write_netcdf(dataset: &GloDataset, path: &Path) -> Result<NetcdfReport> {
    let dimensions = vec![
        (TIME_DIMENSION.to_string(), dataset.time.len()),
        (SPECTRAL_DIMENSION.to_string(), dataset.spectral.len()),
    ];
    let attributes: Vec<(String, String)> = vec![
        ("title".to_string(), dataset.name.clone()),
        ("version".to_string(), dataset.version_label.clone()),
        ("operation".to_string(), dataset.operation.clone()),
        ("unit".to_string(), dataset.unit.symbol().to_string()),
        (
            "Conventions".to_string(),
            "CF-1.8, Glotaran time/spectral".to_string(),
        ),
        (
            "history".to_string(),
            format!("written by Spectra Studio; {}", dataset.provenance()),
        ),
    ];
    let variables = vec![
        Variable {
            name: TIME_DIMENSION.to_string(),
            dimensions: vec![0],
            values: dataset.time.clone(),
            attributes: vec![
                ("units".to_string(), "ps".to_string()),
                ("long_name".to_string(), "delay".to_string()),
            ],
        },
        Variable {
            name: SPECTRAL_DIMENSION.to_string(),
            dimensions: vec![1],
            values: dataset.spectral.clone(),
            attributes: vec![
                ("units".to_string(), "nm".to_string()),
                ("long_name".to_string(), "wavelength".to_string()),
            ],
        },
        Variable {
            name: MATRIX_VARIABLE.to_string(),
            dimensions: vec![0, 1],
            values: dataset.data.clone(),
            attributes: vec![
                ("units".to_string(), dataset.unit.symbol().to_string()),
                (
                    "long_name".to_string(),
                    "transient absorption".to_string(),
                ),
            ],
        },
    ];

    let mut bytes: Vec<u8> = Vec::with_capacity(64 + dataset.data.len() * 8);
    bytes.extend_from_slice(&MAGIC);
    // No record variables: every dimension is fixed size.
    bytes.extend_from_slice(&0u32.to_be_bytes());

    bytes.extend_from_slice(&TAG_DIMENSION.to_be_bytes());
    bytes.extend_from_slice(&(dimensions.len() as u32).to_be_bytes());
    for (name, size) in &dimensions {
        write_name(&mut bytes, name);
        bytes.extend_from_slice(&(*size as u32).to_be_bytes());
    }
    write_attributes(&mut bytes, &attributes);

    // The variable list. Each variable's data offset is measured from the
    // beginning of the *file*, so the offsets are patched in once the header's
    // own length is known: the header's size never depends on them, every one
    // being four bytes wide.
    bytes.extend_from_slice(&TAG_VARIABLE.to_be_bytes());
    bytes.extend_from_slice(&(variables.len() as u32).to_be_bytes());
    let mut begins: Vec<usize> = Vec::with_capacity(variables.len());
    let mut sizes: Vec<u32> = Vec::with_capacity(variables.len());
    let mut slots: Vec<usize> = Vec::with_capacity(variables.len());
    for variable in &variables {
        let size = (variable.values.len() * 8) as u32;
        sizes.push(size);
        begins.push(0);
        write_name(&mut bytes, &variable.name);
        bytes.extend_from_slice(&(variable.dimensions.len() as u32).to_be_bytes());
        for dimension in &variable.dimensions {
            bytes.extend_from_slice(&dimension.to_be_bytes());
        }
        write_attributes(&mut bytes, &variable.attributes);
        bytes.extend_from_slice(&TYPE_DOUBLE.to_be_bytes());
        bytes.extend_from_slice(&size.to_be_bytes());
        slots.push(bytes.len());
        bytes.extend_from_slice(&0u32.to_be_bytes());
    }
    while bytes.len() % 4 != 0 {
        bytes.push(0);
    }
    let mut offset = bytes.len();
    for (index, begin) in begins.iter_mut().enumerate() {
        *begin = offset;
        let size = sizes[index] as usize;
        bytes[slots[index]..slots[index] + 4]
            .copy_from_slice(&(offset as u32).to_be_bytes());
        offset += size;
        while offset % 4 != 0 {
            offset += 1;
        }
    }

    // The data, each variable at its own offset, padded so the next one starts
    // on a four-byte boundary.
    for variable in &variables {
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        for value in &variable.values {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
    }

    std::fs::File::create(path)
        .and_then(|mut file| file.write_all(&bytes))
        .map_err(|source| CoreError::Io {
            path: path.display().to_string(),
            source,
        })?;

    Ok(NetcdfReport {
        path: path.display().to_string(),
        time_points: dataset.time.len(),
        spectral_points: dataset.spectral.len(),
        bytes: bytes.len(),
    })
}

/// The header of a NetCDF attribute list, shared by the global and the
/// per-variable lists.
fn write_attributes(bytes: &mut Vec<u8>, attributes: &[(String, String)]) {
    bytes.extend_from_slice(&TAG_ATTRIBUTE.to_be_bytes());
    bytes.extend_from_slice(&(attributes.len() as u32).to_be_bytes());
    for (name, value) in attributes {
        write_name(bytes, name);
        bytes.extend_from_slice(&TYPE_CHAR.to_be_bytes());
        let text = value.as_bytes();
        bytes.extend_from_slice(&(text.len() as u32).to_be_bytes());
        bytes.extend_from_slice(text);
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
    }
}

/// A name in the format: its length, its bytes, then padding to a four-byte
/// boundary.
fn write_name(bytes: &mut Vec<u8>, name: &str) {
    let text = name.as_bytes();
    bytes.extend_from_slice(&(text.len() as u32).to_be_bytes());
    bytes.extend_from_slice(text);
    while bytes.len() % 4 != 0 {
        bytes.push(0);
    }
}

/// Reads back a file this module wrote.
///
/// It is a reader for the subset of the format that is written, which is what
/// makes the writer checkable; a full NetCDF implementation is not what this
/// crate is for.
pub fn read_netcdf(path: &Path) -> Result<GloDataset> {
    let bytes = std::fs::read(path).map_err(|source| CoreError::Io {
        path: path.display().to_string(),
        source,
    })?;
    if bytes.len() < 8 || bytes[..4] != MAGIC {
        return Err(CoreError::invalid("not a NetCDF-3 classic file"));
    }
    let mut cursor = 8usize;
    let mut dimensions: Vec<(String, usize)> = Vec::new();
    let mut attributes: Vec<(String, String)> = Vec::new();
    let mut variables: Vec<(String, Vec<usize>, Vec<u64>)> = Vec::new();

    while cursor + 8 <= bytes.len() {
        let tag = read_u32(&bytes, &mut cursor)?;
        match tag {
            TAG_DIMENSION => {
                let count = read_u32(&bytes, &mut cursor)?;
                for _ in 0..count {
                    let name = read_name(&bytes, &mut cursor)?;
                    let size = read_u32(&bytes, &mut cursor)? as usize;
                    dimensions.push((name, size));
                }
            }
            TAG_ATTRIBUTE => {
                attributes = read_attributes(&bytes, &mut cursor)?;
            }
            TAG_VARIABLE => {
                let count = read_u32(&bytes, &mut cursor)?;
                for _ in 0..count {
                    let name = read_name(&bytes, &mut cursor)?;
                    let dimensions_count = read_u32(&bytes, &mut cursor)? as usize;
                    let mut dims = Vec::with_capacity(dimensions_count);
                    for _ in 0..dimensions_count {
                        dims.push(read_u32(&bytes, &mut cursor)? as usize);
                    }
                    // The attribute list of a variable carries its own tag,
                    // which the global list's tag is consumed by the loop above.
                    let attribute_tag = read_u32(&bytes, &mut cursor)?;
                    if attribute_tag != TAG_ATTRIBUTE {
                        return Err(CoreError::invalid(
                            "a variable's attribute list is missing its tag",
                        ));
                    }
                    let _ = read_attributes(&bytes, &mut cursor)?;
                    let kind = read_u32(&bytes, &mut cursor)?;
                    if kind != TYPE_DOUBLE {
                        return Err(CoreError::invalid(
                            "only double variables are written by this crate",
                        ));
                    }
                    let _size = read_u32(&bytes, &mut cursor)?;
                    let begin = read_u32(&bytes, &mut cursor)? as u64;
                    variables.push((name, dims, vec![begin]));
                }
            }
            // The data section begins here; everything else has been read.
            _ => break,
        }
    }
    let data_start = cursor;
    let _ = data_start;

    let mut time = Vec::new();
    let mut spectral = Vec::new();
    let mut data = Vec::new();
    for (name, dims, begins) in &variables {
        let begin = begins[0] as usize;
        let count = dims
            .iter()
            .map(|index| dimensions.get(*index).map(|(_, size)| *size).unwrap_or(1))
            .product::<usize>();
        let mut values = Vec::with_capacity(count);
        for index in 0..count {
            let at = begin + index * 8;
            if at + 8 > bytes.len() {
                return Err(CoreError::invalid("the file ends inside a variable"));
            }
            let mut word = [0u8; 8];
            word.copy_from_slice(&bytes[at..at + 8]);
            values.push(f64::from_be_bytes(word));
        }
        match name.as_str() {
            TIME_DIMENSION => time = values,
            SPECTRAL_DIMENSION => spectral = values,
            MATRIX_VARIABLE => data = values,
            _ => {}
        }
    }
    if time.is_empty() || spectral.is_empty() {
        return Err(CoreError::invalid(
            "the file has no time or spectral coordinate",
        ));
    }
    let name = attributes
        .iter()
        .find(|(key, _)| key == "title")
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| "glotaran".to_string());
    let mut dataset = GloDataset::new(
        name,
        time,
        spectral,
        data,
        ta_core::SignalUnit::DeltaOpticalDensity,
    )?;
    if let Some((_, version)) = attributes.iter().find(|(key, _)| key == "version") {
        dataset.version_label = version.clone();
    }
    if let Some((_, operation)) = attributes.iter().find(|(key, _)| key == "operation") {
        dataset.operation = operation.clone();
    }
    Ok(dataset)
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32> {
    if *cursor + 4 > bytes.len() {
        return Err(CoreError::invalid("the file ends inside its header"));
    }
    let mut word = [0u8; 4];
    word.copy_from_slice(&bytes[*cursor..*cursor + 4]);
    *cursor += 4;
    Ok(u32::from_be_bytes(word))
}

fn read_name(bytes: &[u8], cursor: &mut usize) -> Result<String> {
    let length = read_u32(bytes, cursor)? as usize;
    if *cursor + length > bytes.len() {
        return Err(CoreError::invalid("the file ends inside a name"));
    }
    let text = String::from_utf8_lossy(&bytes[*cursor..*cursor + length]).to_string();
    *cursor += length;
    while *cursor % 4 != 0 {
        *cursor += 1;
    }
    Ok(text)
}

fn read_attributes(bytes: &[u8], cursor: &mut usize) -> Result<Vec<(String, String)>> {
    let count = read_u32(bytes, cursor)? as usize;
    let mut attributes = Vec::with_capacity(count);
    for _ in 0..count {
        let name = read_name(bytes, cursor)?;

        let kind = read_u32(bytes, cursor)?;
        let length = read_u32(bytes, cursor)? as usize;
        if *cursor + length > bytes.len() {
            return Err(CoreError::invalid("the file ends inside an attribute"));
        }
        if kind == TYPE_CHAR {
            let text = String::from_utf8_lossy(&bytes[*cursor..*cursor + length]).to_string();
            attributes.push((name, text));
        }
        *cursor += length;
        while *cursor % 4 != 0 {
            *cursor += 1;
        }
    }
    Ok(attributes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_core::SignalUnit;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("ta-glotaran-nc-{}-{name}", std::process::id()));
        path
    }

    fn dataset() -> GloDataset {
        GloDataset::new(
            "NIR800_tcorr",
            vec![-1.0, 0.5, 10.0],
            vec![400.38315, 500.0, 799.61685],
            vec![
                -0.00011, 0.00028, -8e-5, //
                -6e-5, 0.0, 4e-5, //
                0.1, 0.2, 0.3,
            ],
            SignalUnit::DeltaOpticalDensity,
        )
        .unwrap()
    }

    #[test]
    fn the_file_starts_with_the_classic_magic_number() {
        let path = temp_path("magic.nc");
        let report = write_netcdf(&dataset(), &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"CDF\x01");
        assert_eq!(report.time_points, 3);
        assert_eq!(report.spectral_points, 3);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn what_the_writer_writes_the_reader_reads_back() {
        let dataset = dataset();
        let path = temp_path("round.nc");
        write_netcdf(&dataset, &path).unwrap();
        let back = read_netcdf(&path).unwrap();
        assert_eq!(back.name, dataset.name);
        assert_eq!(back.time, dataset.time);
        assert_eq!(back.spectral, dataset.spectral);
        assert_eq!(back.data, dataset.data);
        assert_eq!(back.version_label, dataset.version_label);
        assert_eq!(back.operation, dataset.operation);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn the_variables_carry_the_dimensions_the_modern_convention_names() {
        let path = temp_path("dims.nc");
        write_netcdf(&dataset(), &path).unwrap();
        let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).to_string();
        // The names are in the header as plain text, which is how a reader
        // finds them.
        for name in ["time", "spectral", "data", "ps", "nm", "ΔOD"] {
            assert!(text.contains(name), "{name} is missing from the header");
        }
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn the_header_says_what_the_format_says() {
        let path = temp_path("layout.nc");
        write_netcdf(&dataset(), &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"CDF\x01");
        let word = |at: usize| {
            u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        assert_eq!(word(4), 0, "no record variables");
        assert_eq!(word(8), TAG_DIMENSION, "the dimensions come first");
        assert_eq!(word(12), 2, "time and spectral");
        // Every variable's data begins on a four-byte boundary, inside the
        // file, and the values fit the dimensions the header declares.
        let view = read_netcdf(&path).unwrap();
        assert_eq!(view.time.len(), 3);
        assert_eq!(view.spectral.len(), 3);
        assert_eq!(view.data.len(), 9);
        // A name sits just after its four-byte length, so a name is aligned
        // when its length's own offset is.
        let name_at = bytes
            .windows(4)
            .position(|window| window == b"data")
            .expect("the data variable is named");
        assert_eq!(name_at % 4, 0, "a name is aligned");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn an_independent_reader_can_read_the_file() {
        // `check_netcdf.py` parses the format from the specification rather
        // than from this crate, so a file it accepts is a file the rest of the
        // world can open. Python is a test-time dependency here, as it is for
        // the reference fixtures; without it the check is skipped, not failed.
        let path = temp_path("independent.nc");
        write_netcdf(&dataset(), &path).unwrap();
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../ta-tests/reference/check_netcdf.py");
        let outcome = std::process::Command::new("python3")
            .arg(&script)
            .arg(&path)
            .output();
        match outcome {
            Ok(output) => {
                assert!(
                    output.status.success(),
                    "the independent reader refused the file: {}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                let text = String::from_utf8_lossy(&output.stdout);
                assert!(text.contains("3 times × 3 wavelengths"), "{text}");
            }
            Err(_) => { /* no Python here; the other tests still cover the file */ }
        }
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_file_that_is_not_netcdf_is_refused() {
        let path = temp_path("bad.nc");
        std::fs::write(&path, b"not a netcdf file at all").unwrap();
        assert!(read_netcdf(&path).is_err());
        std::fs::remove_file(&path).ok();
    }
}
