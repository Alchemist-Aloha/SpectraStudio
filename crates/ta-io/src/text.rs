//! Text parsing shared by every importer.

use std::path::Path;
use ta_core::{CoreError, Result};

/// One line that held numbers, with the line number it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct NumericLine {
    pub line: usize,
    pub values: Vec<f64>,
}

/// Reads a file as UTF-8, reporting the path when it fails.
pub fn read_to_string(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|source| CoreError::Io {
        path: path.display().to_string(),
        source,
    })
}

/// Parses every line that holds numbers.
///
/// Blank lines are skipped, and `#` or `%` starts a comment. A field that is
/// not a number is an error naming its line and column, because a silently
/// dropped column is how a transposed matrix slips through.
pub fn parse_numeric_lines(text: &str, path: &str) -> Result<Vec<NumericLine>> {
    let mut lines = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let body = match raw.find(|c| c == '#' || c == '%') {
            Some(position) => &raw[..position],
            None => raw,
        };
        let fields: Vec<&str> = body.split_whitespace().collect();
        if fields.is_empty() {
            continue;
        }
        let mut values = Vec::with_capacity(fields.len());
        for (column, field) in fields.iter().enumerate() {
            match field.parse::<f64>() {
                Ok(value) => values.push(value),
                Err(_) => {
                    return Err(CoreError::MalformedNumber {
                        path: path.to_string(),
                        line: number,
                        field: column,
                        text: (*field).to_string(),
                    })
                }
            }
        }
        lines.push(NumericLine {
            line: number,
            values,
        });
    }
    Ok(lines)
}

/// The width every row shares, or an error naming the row that breaks it.
pub fn require_uniform_width(lines: &[NumericLine], path: &str) -> Result<usize> {
    let Some(first) = lines.first() else {
        return Err(CoreError::invalid(format!("{path} holds no numbers")));
    };
    let width = first.values.len();
    for line in &lines[1..] {
        if line.values.len() != width {
            return Err(CoreError::RecordWidth {
                path: path.to_string(),
                line: line.line,
                got: line.values.len(),
                expected: width,
            });
        }
    }
    Ok(width)
}

/// Formats a number for export, keeping missing values recognisable rather
/// than writing them as zero.
pub fn format_number(value: f64, precision: usize) -> String {
    if value.is_nan() {
        "nan".to_string()
    } else if value.is_infinite() {
        if value > 0.0 {
            "inf".to_string()
        } else {
            "-inf".to_string()
        }
    } else {
        format!("{value:.precision$}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_blank_lines_do_not_shift_line_numbers() {
        let text = "# header\n\n1 2\n% note\n3 4\n";
        let lines = parse_numeric_lines(text, "t").unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].line, 3);
        assert_eq!(lines[1].line, 5);
    }

    #[test]
    fn scientific_notation_and_unsigned_values_parse() {
        let lines = parse_numeric_lines("-1.0e-3\t+2\n.5 5.\n", "t").unwrap();
        assert_eq!(lines[0].values, vec![-0.001, 2.0]);
        assert_eq!(lines[1].values, vec![0.5, 5.0]);
    }

    #[test]
    fn a_ragged_row_names_itself() {
        let lines = parse_numeric_lines("1 2 3\n4 5\n", "t").unwrap();
        match require_uniform_width(&lines, "t").unwrap_err() {
            CoreError::RecordWidth { line, got, expected, .. } => {
                assert_eq!((line, got, expected), (2, 2, 3));
            }
            other => panic!("expected RecordWidth, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_file_is_an_error_not_an_empty_matrix() {
        assert!(require_uniform_width(&[], "t").is_err());
    }

    #[test]
    fn missing_values_format_as_nan_rather_than_zero() {
        assert_eq!(format_number(f64::NAN, 3), "nan");
        assert_eq!(format_number(f64::INFINITY, 3), "inf");
        assert_eq!(format_number(1.5, 3), "1.500");
    }
}
