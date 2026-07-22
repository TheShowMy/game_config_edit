use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use csv::{ReaderBuilder, StringRecord, Terminator, WriterBuilder};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";
const DELIMITER_CANDIDATES: [u8; 4] = [b',', b'\t', b';', b'|'];
const SAMPLE_RECORD_LIMIT: usize = 50;
static NEXT_ANALYSIS_VERSION: AtomicU64 = AtomicU64::new(1);

struct ParsedRecords {
    records: Vec<Vec<String>>,
    spans: Vec<std::ops::Range<usize>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DelimiterSource {
    Detected,
    Default,
    Manual,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CsvDelimiter {
    Comma,
    Tab,
    Semicolon,
    Pipe,
}

impl CsvDelimiter {
    pub const ALL: [Self; 4] = [Self::Comma, Self::Tab, Self::Semicolon, Self::Pipe];

    pub const fn byte(self) -> u8 {
        match self {
            Self::Comma => b',',
            Self::Tab => b'\t',
            Self::Semicolon => b';',
            Self::Pipe => b'|',
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Comma => "Comma",
            Self::Tab => "Tab",
            Self::Semicolon => "Semicolon",
            Self::Pipe => "Pipe",
        }
    }

    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            b',' => Some(Self::Comma),
            b'\t' => Some(Self::Tab),
            b';' => Some(Self::Semicolon),
            b'|' => Some(Self::Pipe),
            _ => None,
        }
    }

    pub const fn setting_value(self) -> &'static str {
        match self {
            Self::Comma => "comma",
            Self::Tab => "tab",
            Self::Semicolon => "semicolon",
            Self::Pipe => "pipe",
        }
    }

    pub fn from_setting_value(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|delimiter| delimiter.setting_value() == value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CsvDocument {
    pub path: PathBuf,
    pub raw_text: String,
    pub has_bom: bool,
    pub line_ending: LineEnding,
    pub delimiter: u8,
    pub delimiter_source: DelimiterSource,
    pub records: Arc<Vec<Vec<String>>>,
    record_spans: Arc<Vec<std::ops::Range<usize>>>,
    analysis_version: u64,
}

#[derive(Debug, Error)]
pub enum CsvDocumentError {
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not valid UTF-8 at byte {valid_up_to}")]
    InvalidUtf8 { path: PathBuf, valid_up_to: usize },
    #[error("failed to parse {path}: {message}")]
    Parse {
        path: PathBuf,
        message: String,
        error_count: usize,
    },
    #[error("cell is outside the document at record {row_index}, column {column_index}")]
    CellOutOfBounds {
        row_index: usize,
        column_index: usize,
    },
}

impl CsvDocumentError {
    pub fn parse_error_count(&self) -> Option<usize> {
        match self {
            Self::Parse { error_count, .. } => Some(*error_count),
            _ => None,
        }
    }
}

impl CsvDocument {
    pub fn open(path: &Path, delimiter_override: Option<u8>) -> Result<Self, CsvDocumentError> {
        let bytes = std::fs::read(path).map_err(|source| CsvDocumentError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_bytes(path, &bytes, delimiter_override)
    }

    pub fn from_bytes(
        path: &Path,
        bytes: &[u8],
        delimiter_override: Option<u8>,
    ) -> Result<Self, CsvDocumentError> {
        let has_bom = bytes.starts_with(UTF8_BOM);
        let content = if has_bom {
            &bytes[UTF8_BOM.len()..]
        } else {
            bytes
        };
        let raw_text = std::str::from_utf8(content)
            .map_err(|error| CsvDocumentError::InvalidUtf8 {
                path: path.to_path_buf(),
                valid_up_to: error.valid_up_to() + usize::from(has_bom) * UTF8_BOM.len(),
            })?
            .to_owned();
        let line_ending = detect_line_ending(&raw_text);
        let (delimiter, delimiter_source) = match delimiter_override {
            Some(delimiter) => (delimiter, DelimiterSource::Manual),
            None => detect_delimiter(&raw_text),
        };
        let parsed = parse_records(path, &raw_text, delimiter)?;

        Ok(Self {
            path: path.to_path_buf(),
            raw_text,
            has_bom,
            line_ending,
            delimiter,
            delimiter_source,
            records: Arc::new(parsed.records),
            record_spans: Arc::new(parsed.spans),
            analysis_version: next_analysis_version(),
        })
    }

    pub fn analysis_version(&self) -> u64 {
        self.analysis_version
    }

    pub fn dimensions(&self, header_rows: usize) -> Option<(usize, usize)> {
        if self.records.len() < header_rows {
            return None;
        }
        let columns = self.records.first().map_or(0, Vec::len);
        Some((self.records.len() - header_rows, columns))
    }

    pub fn replace_cell(
        &mut self,
        row_index: usize,
        column_index: usize,
        value: &str,
    ) -> Result<(), CsvDocumentError> {
        let record = self
            .records
            .get(row_index)
            .ok_or(CsvDocumentError::CellOutOfBounds {
                row_index,
                column_index,
            })?;
        if column_index >= record.len() {
            return Err(CsvDocumentError::CellOutOfBounds {
                row_index,
                column_index,
            });
        }
        if record[column_index] == value {
            return Ok(());
        }

        let mut updated_record = record.clone();
        updated_record[column_index] = value.to_owned();
        let span = self.record_spans[row_index].clone();
        let original_record = &self.raw_text[span.clone()];
        let had_terminator = original_record.ends_with('\n');
        let mut writer = WriterBuilder::new()
            .has_headers(false)
            .delimiter(self.delimiter)
            .terminator(match self.line_ending {
                LineEnding::Lf => Terminator::Any(b'\n'),
                LineEnding::CrLf => Terminator::CRLF,
            })
            .from_writer(Vec::new());
        writer
            .write_record(&updated_record)
            .map_err(|error| CsvDocumentError::Parse {
                path: self.path.clone(),
                message: error.to_string(),
                error_count: 1,
            })?;
        let mut replacement =
            String::from_utf8(
                writer
                    .into_inner()
                    .map_err(|error| CsvDocumentError::Parse {
                        path: self.path.clone(),
                        message: error.to_string(),
                        error_count: 1,
                    })?,
            )
            .expect("csv writer only writes UTF-8 strings");
        if !had_terminator {
            replacement.truncate(replacement.trim_end_matches(['\r', '\n']).len());
        }

        let replacement_length = replacement.len();
        let length_delta = replacement_length as isize - span.len() as isize;
        self.raw_text.replace_range(span.clone(), &replacement);
        Arc::make_mut(&mut self.records)[row_index] = updated_record;
        let spans = Arc::make_mut(&mut self.record_spans);
        spans[row_index] = span.start..span.start + replacement_length;
        if length_delta != 0 {
            for following in spans.iter_mut().skip(row_index + 1) {
                following.start = following.start.saturating_add_signed(length_delta);
                following.end = following.end.saturating_add_signed(length_delta);
            }
        }
        self.analysis_version = next_analysis_version();
        Ok(())
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.raw_text.len() + UTF8_BOM.len());
        if self.has_bom {
            bytes.extend_from_slice(UTF8_BOM);
        }
        bytes.extend_from_slice(self.raw_text.as_bytes());
        bytes
    }
}

fn parse_records(
    path: &Path,
    raw_text: &str,
    delimiter: u8,
) -> Result<ParsedRecords, CsvDocumentError> {
    let mut issues = validate_quote_syntax(raw_text, delimiter);
    let mut reader = ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(raw_text.as_bytes());
    let mut records = Vec::new();
    let mut record_spans = Vec::new();
    let mut record = StringRecord::new();
    let mut expected_width = None;
    loop {
        let has_record = match reader.read_record(&mut record) {
            Ok(has_record) => has_record,
            Err(error) => {
                let byte = error
                    .position()
                    .map_or(0, |position| position.byte() as usize);
                issues.push(ParseIssue {
                    byte,
                    message: error.to_string(),
                });
                break;
            }
        };
        if !has_record {
            break;
        }
        let start = record
            .position()
            .map_or(0, |position| position.byte() as usize);
        let start = normalize_crlf_boundary(raw_text.as_bytes(), start);
        let end = normalize_crlf_boundary(raw_text.as_bytes(), reader.position().byte() as usize);
        match expected_width {
            Some(width) if width != record.len() => issues.push(ParseIssue {
                byte: start,
                message: format!(
                    "record {} (line {}) has {} fields, expected {width}",
                    records.len() + 1,
                    record.position().map_or(1, |position| position.line()),
                    record.len(),
                ),
            }),
            None => expected_width = Some(record.len()),
            _ => {}
        }
        records.push(record.iter().map(str::to_owned).collect());
        record_spans.push(start..end);
    }
    if !issues.is_empty() {
        issues.sort_by_key(|issue| issue.byte);
        return Err(CsvDocumentError::Parse {
            path: path.to_path_buf(),
            message: issues[0].message.clone(),
            error_count: issues.len(),
        });
    }
    Ok(ParsedRecords {
        records,
        spans: record_spans,
    })
}

#[derive(Debug)]
struct ParseIssue {
    byte: usize,
    message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QuoteState {
    FieldStart,
    Unquoted,
    Quoted,
    QuoteClosed,
}

fn validate_quote_syntax(raw_text: &str, delimiter: u8) -> Vec<ParseIssue> {
    let bytes = raw_text.as_bytes();
    let mut issues = Vec::new();
    let mut state = QuoteState::FieldStart;
    let mut quoted_field_start = 0;
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        let line_break = byte == b'\n' || byte == b'\r';
        match state {
            QuoteState::FieldStart => {
                if byte == b'"' {
                    quoted_field_start = index;
                    state = QuoteState::Quoted;
                } else if byte == delimiter {
                    // The next byte starts another field.
                } else if line_break {
                    index += usize::from(byte == b'\r' && bytes.get(index + 1) == Some(&b'\n'));
                } else {
                    state = QuoteState::Unquoted;
                }
            }
            QuoteState::Unquoted => {
                if byte == b'"' {
                    issues.push(ParseIssue {
                        byte: index,
                        message: format!("unexpected quote in unquoted field at byte {index}"),
                    });
                } else if byte == delimiter {
                    state = QuoteState::FieldStart;
                } else if line_break {
                    state = QuoteState::FieldStart;
                    index += usize::from(byte == b'\r' && bytes.get(index + 1) == Some(&b'\n'));
                }
            }
            QuoteState::Quoted => {
                if byte == b'"' {
                    state = QuoteState::QuoteClosed;
                }
            }
            QuoteState::QuoteClosed => {
                if byte == b'"' {
                    state = QuoteState::Quoted;
                } else if byte == delimiter {
                    state = QuoteState::FieldStart;
                } else if line_break {
                    state = QuoteState::FieldStart;
                    index += usize::from(byte == b'\r' && bytes.get(index + 1) == Some(&b'\n'));
                } else {
                    issues.push(ParseIssue {
                        byte: index,
                        message: format!(
                            "unexpected character after closing quote at byte {index}"
                        ),
                    });
                    state = QuoteState::Unquoted;
                }
            }
        }
        index += 1;
    }

    if state == QuoteState::Quoted {
        issues.push(ParseIssue {
            byte: quoted_field_start,
            message: format!("unclosed quoted field starting at byte {quoted_field_start}"),
        });
    }
    issues
}

fn next_analysis_version() -> u64 {
    NEXT_ANALYSIS_VERSION.fetch_add(1, Ordering::Relaxed)
}

fn normalize_crlf_boundary(bytes: &[u8], position: usize) -> usize {
    if position > 0
        && bytes.get(position) == Some(&b'\n')
        && bytes.get(position - 1) == Some(&b'\r')
    {
        position + 1
    } else {
        position
    }
}

fn detect_delimiter(raw_text: &str) -> (u8, DelimiterSource) {
    let candidates = DELIMITER_CANDIDATES
        .into_iter()
        .filter_map(|delimiter| {
            delimiter_score(raw_text, delimiter).map(|score| (delimiter, score))
        })
        .collect::<Vec<_>>();

    let Some(best_score) = candidates.iter().map(|(_, score)| *score).max() else {
        return (b',', DelimiterSource::Default);
    };
    let best = candidates
        .iter()
        .filter(|(_, score)| *score == best_score)
        .map(|(delimiter, _)| *delimiter)
        .collect::<Vec<_>>();

    match best.as_slice() {
        [delimiter] => (*delimiter, DelimiterSource::Detected),
        _ => (b',', DelimiterSource::Default),
    }
}

fn delimiter_score(raw_text: &str, delimiter: u8) -> Option<(usize, usize)> {
    let mut reader = ReaderBuilder::new()
        .has_headers(false)
        .flexible(false)
        .delimiter(delimiter)
        .from_reader(raw_text.as_bytes());
    let mut width = None;
    let mut non_empty_records = 0;

    for record in reader.records().take(SAMPLE_RECORD_LIMIT) {
        let record = record.ok()?;
        if record_is_empty(&record) {
            continue;
        }
        if record.len() <= 1 {
            return None;
        }
        match width {
            Some(expected) if expected != record.len() => return None,
            None => width = Some(record.len()),
            _ => {}
        }
        non_empty_records += 1;
    }

    width.map(|width| (non_empty_records, width))
}

fn record_is_empty(record: &StringRecord) -> bool {
    record.iter().all(|value| value.is_empty())
}

fn detect_line_ending(raw_text: &str) -> LineEnding {
    if raw_text.as_bytes().windows(2).any(|pair| pair == b"\r\n") {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_utf8_bom_and_preserves_line_ending() {
        let document = CsvDocument::from_bytes(
            Path::new("heroes.csv"),
            b"\xEF\xBB\xBFid,name\r\n1,Arthur\r\n",
            None,
        )
        .unwrap();

        assert!(document.has_bom);
        assert_eq!(document.line_ending, LineEnding::CrLf);
        assert_eq!(document.delimiter, b',');
        assert_eq!(document.delimiter_source, DelimiterSource::Detected);
        assert_eq!(document.dimensions(1), Some((1, 2)));
    }

    #[test]
    fn detects_semicolon_delimiter() {
        let document =
            CsvDocument::from_bytes(Path::new("heroes.csv"), b"id;name;hp\n1;Arthur;500\n", None)
                .unwrap();

        assert_eq!(document.delimiter, b';');
        assert_eq!(document.records[1], vec!["1", "Arthur", "500"]);
    }

    #[test]
    fn ambiguous_single_column_uses_comma_without_inference() {
        let document =
            CsvDocument::from_bytes(Path::new("names.csv"), b"name\nArthur\n", None).unwrap();

        assert_eq!(document.delimiter, b',');
        assert_eq!(document.delimiter_source, DelimiterSource::Default);
    }

    #[test]
    fn manual_delimiter_overrides_detection() {
        let document =
            CsvDocument::from_bytes(Path::new("heroes.csv"), b"id|name\n1|Arthur\n", Some(b'|'))
                .unwrap();

        assert_eq!(document.delimiter, b'|');
        assert_eq!(document.delimiter_source, DelimiterSource::Manual);
        assert_eq!(document.records.len(), 2);
    }

    #[test]
    fn rejects_invalid_utf8() {
        let error =
            CsvDocument::from_bytes(Path::new("invalid.csv"), b"id\n\xFF\n", None).unwrap_err();

        assert!(matches!(
            error,
            CsvDocumentError::InvalidUtf8 { valid_up_to: 3, .. }
        ));
    }

    #[test]
    fn invalid_utf8_offset_includes_bom() {
        let error =
            CsvDocument::from_bytes(Path::new("invalid.csv"), b"\xEF\xBB\xBFid\n\xFF\n", None)
                .unwrap_err();

        assert!(matches!(
            error,
            CsvDocumentError::InvalidUtf8 { valid_up_to: 6, .. }
        ));
    }

    #[test]
    fn rejects_inconsistent_record_width() {
        let error = CsvDocument::from_bytes(
            Path::new("invalid.csv"),
            b"id,name\n1,Arthur,extra\n",
            Some(b','),
        )
        .unwrap_err();

        assert!(matches!(error, CsvDocumentError::Parse { .. }));
    }

    #[test]
    fn reports_every_inconsistent_record_width() {
        let error = CsvDocument::from_bytes(
            Path::new("invalid.csv"),
            b"id,name\n1\n2,Merlin,extra\n3,Morgana\n",
            Some(b','),
        )
        .unwrap_err();

        assert_eq!(error.parse_error_count(), Some(2));
        assert!(error.to_string().contains("record 2"));
    }

    #[test]
    fn rejects_unclosed_and_misplaced_quotes() {
        let unclosed = CsvDocument::from_bytes(
            Path::new("unclosed.csv"),
            b"id,name\n1,\"Arthur\n",
            Some(b','),
        )
        .unwrap_err();
        let misplaced = CsvDocument::from_bytes(
            Path::new("misplaced.csv"),
            b"id,name\n1,Art\"hur\n",
            Some(b','),
        )
        .unwrap_err();

        assert_eq!(unclosed.parse_error_count(), Some(1));
        assert!(unclosed.to_string().contains("unclosed quoted field"));
        assert_eq!(misplaced.parse_error_count(), Some(1));
        assert!(misplaced.to_string().contains("unexpected quote"));
    }

    #[test]
    fn replacing_a_cell_only_normalizes_its_record() {
        let mut document = CsvDocument::from_bytes(
            Path::new("heroes.csv"),
            b"id,name\r\n1,\"Arthur\"\r\n2,Merlin\r\n",
            Some(b','),
        )
        .unwrap();

        document.replace_cell(1, 1, "Art,hur").unwrap();

        assert_eq!(
            document.raw_text,
            "id,name\r\n1,\"Art,hur\"\r\n2,Merlin\r\n"
        );
        assert_eq!(document.records[1][1], "Art,hur");
    }

    #[test]
    fn replacing_the_final_record_preserves_missing_trailing_newline() {
        let mut document =
            CsvDocument::from_bytes(Path::new("heroes.csv"), b"id,name\n1,Arthur", Some(b','))
                .unwrap();

        document.replace_cell(1, 1, "Merlin").unwrap();

        assert_eq!(document.raw_text, "id,name\n1,Merlin");
    }

    #[test]
    fn replacing_multiple_records_keeps_incremental_spans_valid() {
        let mut document = CsvDocument::from_bytes(
            Path::new("heroes.csv"),
            b"id,name\n1,A\n2,B\n3,C\n",
            Some(b','),
        )
        .unwrap();
        let initial_version = document.analysis_version();

        document.replace_cell(1, 1, "Arthur the Brave").unwrap();
        document.replace_cell(3, 1, "C, the Third").unwrap();

        assert_eq!(
            document.raw_text,
            "id,name\n1,Arthur the Brave\n2,B\n3,\"C, the Third\"\n"
        );
        assert_eq!(document.records[1][1], "Arthur the Brave");
        assert_eq!(document.records[3][1], "C, the Third");
        assert!(document.analysis_version() > initial_version);
    }

    #[test]
    fn serialization_restores_utf8_bom() {
        let document = CsvDocument::from_bytes(
            Path::new("heroes.csv"),
            b"\xEF\xBB\xBFid,name\n1,Arthur\n",
            Some(b','),
        )
        .unwrap();

        assert_eq!(document.to_bytes(), b"\xEF\xBB\xBFid,name\n1,Arthur\n");
    }

    #[test]
    fn supported_delimiters_round_trip_through_ui_values() {
        for delimiter in CsvDelimiter::ALL {
            assert_eq!(CsvDelimiter::from_byte(delimiter.byte()), Some(delimiter));
            assert_eq!(
                CsvDelimiter::from_setting_value(delimiter.setting_value()),
                Some(delimiter)
            );
        }
        assert_eq!(CsvDelimiter::from_byte(b':'), None);
        assert_eq!(CsvDelimiter::from_setting_value("custom"), None);
    }
}
