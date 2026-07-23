use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::csv_document::{CsvDocument, CsvDocumentError, CsvEncoding, DelimiterSource};
use crate::settings::{DEFAULT_HEADER_ROWS, MAX_HEADER_ROWS, MIN_HEADER_ROWS};

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

#[derive(Clone, Debug, Eq, PartialEq)]
struct CellEdit {
    row_index: usize,
    column_index: usize,
    before: String,
    after: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DocumentView {
    #[default]
    Table,
    Text,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextParseIssue {
    pub message: String,
    pub count: usize,
}

#[derive(Clone, Debug)]
pub struct DocumentSession {
    pub document: CsvDocument,
    pub header_rows: usize,
    text_override: Option<String>,
    view: DocumentView,
    text_parse_issue: Option<TextParseIssue>,
    undo_stack: Vec<CellEdit>,
    redo_stack: Vec<CellEdit>,
    saved_hash: blake3::Hash,
    current_hash: blake3::Hash,
    delimiter_override: Option<u8>,
    gb18030_conversion_allowed: bool,
}

#[derive(Debug, Error)]
pub enum DocumentSessionError {
    #[error(transparent)]
    Csv(#[from] CsvDocumentError),
    #[error("{path} changed on disk after it was opened")]
    ExternalModification { path: PathBuf },
    #[error("failed to save {path}: {source}")]
    Save {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("header rows must be between {MIN_HEADER_ROWS} and {MAX_HEADER_ROWS}, got {0}")]
    InvalidHeaderRows(usize),
    #[error("{path} uses GB18030 and must be converted to UTF-8 before editing")]
    Gb18030RequiresConversion { path: PathBuf },
}

impl DocumentSession {
    pub fn open(path: &Path, delimiter_override: Option<u8>) -> Result<Self, DocumentSessionError> {
        Self::open_with_options(path, delimiter_override, DEFAULT_HEADER_ROWS)
    }

    pub fn open_with_options(
        path: &Path,
        delimiter_override: Option<u8>,
        header_rows: usize,
    ) -> Result<Self, DocumentSessionError> {
        Self::open_with_conversion(path, delimiter_override, header_rows, false)
    }

    pub fn open_with_conversion(
        path: &Path,
        delimiter_override: Option<u8>,
        header_rows: usize,
        convert_gb18030: bool,
    ) -> Result<Self, DocumentSessionError> {
        validate_header_rows(header_rows)?;
        let bytes = fs::read(path).map_err(|source| CsvDocumentError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_bytes(
            path,
            bytes,
            delimiter_override,
            header_rows,
            convert_gb18030,
        )
    }

    pub fn from_loaded_document(
        mut document: CsvDocument,
        header_rows: usize,
        convert_gb18030: bool,
    ) -> Result<Self, DocumentSessionError> {
        validate_header_rows(header_rows)?;
        let saved_hash = document.source_hash();
        if document.encoding == CsvEncoding::Gb18030 {
            if !convert_gb18030 {
                return Err(DocumentSessionError::Gb18030RequiresConversion {
                    path: document.path.clone(),
                });
            }
            document.convert_to_utf8();
        }
        let current_hash = blake3::hash(&document.to_bytes());
        let delimiter_override =
            (document.delimiter_source == DelimiterSource::Manual).then_some(document.delimiter);
        Ok(Self {
            document,
            header_rows,
            text_override: None,
            view: DocumentView::Table,
            text_parse_issue: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            saved_hash,
            current_hash,
            delimiter_override,
            gb18030_conversion_allowed: convert_gb18030,
        })
    }

    fn from_bytes(
        path: &Path,
        bytes: Vec<u8>,
        delimiter_override: Option<u8>,
        header_rows: usize,
        convert_gb18030: bool,
    ) -> Result<Self, DocumentSessionError> {
        let saved_hash = blake3::hash(&bytes);
        let (mut document, text_override, view, text_parse_issue) =
            match CsvDocument::from_bytes(path, &bytes, delimiter_override) {
                Ok(document) => (document, None, DocumentView::Table, None),
                Err(error @ CsvDocumentError::Parse { .. }) => {
                    let error_count = error.parse_error_count().unwrap_or(1);
                    let (text, encoding) = CsvDocument::decode_bytes(path, &bytes)?;
                    if encoding == CsvEncoding::Gb18030 && !convert_gb18030 {
                        return Err(DocumentSessionError::Gb18030RequiresConversion {
                            path: path.to_path_buf(),
                        });
                    }
                    let placeholder_bytes = if encoding == CsvEncoding::Utf8Bom {
                        UTF8_BOM
                    } else {
                        &[]
                    };
                    let document =
                        CsvDocument::from_bytes(path, placeholder_bytes, delimiter_override)
                            .expect("an empty UTF-8 CSV is always parseable");
                    (
                        document,
                        Some(text),
                        DocumentView::Text,
                        Some(TextParseIssue {
                            message: error.to_string(),
                            count: error_count,
                        }),
                    )
                }
                Err(error) => return Err(error.into()),
            };
        if document.encoding == CsvEncoding::Gb18030 {
            if !convert_gb18030 {
                return Err(DocumentSessionError::Gb18030RequiresConversion {
                    path: path.to_path_buf(),
                });
            }
            document.convert_to_utf8();
        }
        let current_hash = if let Some(text) = text_override.as_deref() {
            hash_text(document.has_bom, text)
        } else {
            blake3::hash(&document.to_bytes())
        };
        Ok(Self {
            document,
            header_rows,
            text_override,
            view,
            text_parse_issue,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            saved_hash,
            current_hash,
            delimiter_override,
            gb18030_conversion_allowed: convert_gb18030,
        })
    }

    pub fn view(&self) -> DocumentView {
        self.view
    }

    pub fn text(&self) -> &str {
        self.text_override
            .as_deref()
            .unwrap_or(&self.document.raw_text)
    }

    pub fn text_parse_issue(&self) -> Option<&TextParseIssue> {
        self.text_parse_issue.as_ref()
    }

    pub fn show_text(&mut self) {
        self.view = DocumentView::Text;
    }

    pub fn set_text(&mut self, text: String) -> bool {
        if self.text() == text {
            return false;
        }
        self.current_hash = hash_text(self.document.has_bom, &text);
        self.text_override = Some(text);
        self.text_parse_issue = None;
        true
    }

    pub fn text_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.text().len() + UTF8_BOM.len());
        if self.document.has_bom {
            bytes.extend_from_slice(UTF8_BOM);
        }
        bytes.extend_from_slice(self.text().as_bytes());
        bytes
    }

    pub fn text_hash(&self) -> blake3::Hash {
        self.current_hash
    }

    pub fn accept_parsed_text(&mut self, parsed: CsvDocument) -> bool {
        if hash_text(parsed.has_bom, &parsed.raw_text) != self.current_hash {
            return false;
        }
        if parsed.has_bom != self.document.has_bom || parsed.raw_text != self.document.raw_text {
            self.undo_stack.clear();
            self.redo_stack.clear();
        }
        self.document = parsed;
        self.text_override = None;
        self.text_parse_issue = None;
        true
    }

    pub fn show_parsed_table(&mut self, parsed: CsvDocument) -> bool {
        if !self.accept_parsed_text(parsed) {
            return false;
        }
        self.view = DocumentView::Table;
        true
    }

    pub fn reject_parsed_text(&mut self, message: String, count: usize) {
        self.view = DocumentView::Text;
        self.text_parse_issue = Some(TextParseIssue { message, count });
    }

    pub fn show_table(&mut self) -> Result<(), TextParseIssue> {
        let bytes = self.text_bytes();
        match CsvDocument::from_bytes(&self.document.path, &bytes, self.delimiter_override) {
            Ok(parsed) => {
                let accepted = self.accept_parsed_text(parsed);
                debug_assert!(accepted, "parsed text must match the session buffer");
                self.view = DocumentView::Table;
                Ok(())
            }
            Err(error) => {
                let count = error.parse_error_count().unwrap_or(1);
                let issue = TextParseIssue {
                    message: error.to_string(),
                    count,
                };
                self.text_parse_issue = Some(issue.clone());
                self.view = DocumentView::Text;
                Err(issue)
            }
        }
    }

    pub fn validate_text(&mut self) -> Result<(), TextParseIssue> {
        let view = self.view;
        let result = self.show_table();
        self.view = view;
        result
    }

    pub fn set_header_rows(&mut self, header_rows: usize) -> Result<(), DocumentSessionError> {
        validate_header_rows(header_rows)?;
        self.header_rows = header_rows;
        Ok(())
    }

    pub fn set_delimiter(&mut self, delimiter: u8) -> Result<(), DocumentSessionError> {
        let bytes = self.text_bytes();
        let reparsed = CsvDocument::from_bytes(&self.document.path, &bytes, Some(delimiter))?;
        self.document = reparsed;
        self.text_override = None;
        self.current_hash = hash_text(self.document.has_bom, &self.document.raw_text);
        self.text_parse_issue = None;
        self.delimiter_override = Some(delimiter);
        self.undo_stack.clear();
        self.redo_stack.clear();
        Ok(())
    }

    pub fn edit_cell(
        &mut self,
        row_index: usize,
        column_index: usize,
        value: String,
    ) -> Result<bool, DocumentSessionError> {
        let before = self
            .document
            .records
            .get(row_index)
            .and_then(|record| record.get(column_index))
            .cloned()
            .ok_or(CsvDocumentError::CellOutOfBounds {
                row_index,
                column_index,
            })?;
        if before == value {
            return Ok(false);
        }

        self.document
            .replace_cell(row_index, column_index, &value)?;
        self.text_override = None;
        self.current_hash = hash_text(self.document.has_bom, &self.document.raw_text);
        self.undo_stack.push(CellEdit {
            row_index,
            column_index,
            before,
            after: value,
        });
        self.redo_stack.clear();
        Ok(true)
    }

    pub fn undo(&mut self) -> Result<bool, DocumentSessionError> {
        let Some(edit) = self.undo_stack.pop() else {
            return Ok(false);
        };
        self.document
            .replace_cell(edit.row_index, edit.column_index, &edit.before)?;
        self.text_override = None;
        self.current_hash = hash_text(self.document.has_bom, &self.document.raw_text);
        self.redo_stack.push(edit);
        Ok(true)
    }

    pub fn redo(&mut self) -> Result<bool, DocumentSessionError> {
        let Some(edit) = self.redo_stack.pop() else {
            return Ok(false);
        };
        self.document
            .replace_cell(edit.row_index, edit.column_index, &edit.after)?;
        self.text_override = None;
        self.current_hash = hash_text(self.document.has_bom, &self.document.raw_text);
        self.undo_stack.push(edit);
        Ok(true)
    }

    pub fn is_dirty(&self) -> bool {
        self.current_hash != self.saved_hash
    }

    pub fn saved_hash(&self) -> blake3::Hash {
        self.saved_hash
    }

    pub fn delimiter_override(&self) -> Option<u8> {
        self.delimiter_override
    }

    pub fn gb18030_conversion_allowed(&self) -> bool {
        self.gb18030_conversion_allowed
    }

    pub fn save(&mut self, overwrite_external_changes: bool) -> Result<(), DocumentSessionError> {
        let path = self.document.path.clone();
        if !overwrite_external_changes {
            let disk_bytes = fs::read(&path)
                .map_err(|_| DocumentSessionError::ExternalModification { path: path.clone() })?;
            if blake3::hash(&disk_bytes) != self.saved_hash {
                return Err(DocumentSessionError::ExternalModification { path });
            }
        }

        let bytes = self.text_bytes();
        atomic_write(&self.document.path, &bytes)?;
        self.saved_hash = blake3::hash(&bytes);
        self.current_hash = self.saved_hash;
        Ok(())
    }

    pub fn reload(&mut self) -> Result<(), DocumentSessionError> {
        let path = self.document.path.clone();
        let bytes = fs::read(&path).map_err(|source| CsvDocumentError::Read {
            path: path.clone(),
            source,
        })?;
        let previous_view = self.view;
        let mut replacement = Self::from_bytes(
            &path,
            bytes,
            self.delimiter_override,
            self.header_rows,
            self.gb18030_conversion_allowed,
        )?;
        if replacement.text_parse_issue.is_none() {
            replacement.view = previous_view;
        }
        *self = replacement;
        Ok(())
    }
}

fn hash_text(has_bom: bool, text: &str) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    if has_bom {
        hasher.update(UTF8_BOM);
    }
    hasher.update(text.as_bytes());
    hasher.finalize()
}

fn validate_header_rows(header_rows: usize) -> Result<(), DocumentSessionError> {
    if (MIN_HEADER_ROWS..=MAX_HEADER_ROWS).contains(&header_rows) {
        Ok(())
    } else {
        Err(DocumentSessionError::InvalidHeaderRows(header_rows))
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), DocumentSessionError> {
    let parent = path.parent().ok_or_else(|| DocumentSessionError::Save {
        path: path.to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "file path has no parent directory",
        ),
    })?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|source| DocumentSessionError::Save {
            path: path.to_path_buf(),
            source,
        })?;
    if let Ok(metadata) = fs::metadata(path) {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())
            .map_err(|source| DocumentSessionError::Save {
                path: path.to_path_buf(),
                source,
            })?;
    }
    temporary
        .write_all(bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|source| DocumentSessionError::Save {
            path: path.to_path_buf(),
            source,
        })?;
    temporary
        .persist(path)
        .map_err(|error| DocumentSessionError::Save {
            path: path.to_path_buf(),
            source: error.error,
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoding_rs::GB18030;

    fn create_session(content: &[u8]) -> (tempfile::TempDir, PathBuf, DocumentSession) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("heroes.csv");
        fs::write(&path, content).unwrap();
        let session = DocumentSession::open(&path, Some(b',')).unwrap();
        (directory, path, session)
    }

    fn gb18030_bytes(content: &str) -> Vec<u8> {
        let (bytes, _, had_errors) = GB18030.encode(content);
        assert!(!had_errors);
        bytes.into_owned()
    }

    #[test]
    fn gb18030_requires_explicit_conversion_before_editing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("heroes.csv");
        let bytes = gb18030_bytes("id,name\r\n1,亚瑟\r\n");
        fs::write(&path, &bytes).unwrap();

        let error = DocumentSession::open_with_options(&path, Some(b','), 1).unwrap_err();

        assert!(matches!(
            error,
            DocumentSessionError::Gb18030RequiresConversion { .. }
        ));
        assert_eq!(fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn converted_gb18030_session_is_dirty_and_saves_as_utf8_without_bom() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("heroes.csv");
        let original = gb18030_bytes("id,name\r\n1,亚瑟\r\n");
        fs::write(&path, &original).unwrap();

        let mut session =
            DocumentSession::open_with_conversion(&path, Some(b','), 1, true).unwrap();

        assert_eq!(session.document.encoding, CsvEncoding::Utf8);
        assert_eq!(
            session.document.line_ending,
            crate::csv_document::LineEnding::CrLf
        );
        assert_eq!(session.saved_hash(), blake3::hash(&original));
        assert!(session.is_dirty());

        session.save(false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), "id,name\r\n1,亚瑟\r\n".as_bytes());
        assert!(!session.is_dirty());
    }

    #[test]
    fn converted_gb18030_session_detects_external_modification() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("heroes.csv");
        fs::write(&path, gb18030_bytes("id,name\n1,亚瑟\n")).unwrap();
        let mut session =
            DocumentSession::open_with_conversion(&path, Some(b','), 1, true).unwrap();
        fs::write(&path, gb18030_bytes("id,name\n1,梅林\n")).unwrap();

        let error = session.save(false).unwrap_err();

        assert!(matches!(
            error,
            DocumentSessionError::ExternalModification { .. }
        ));
        assert!(session.is_dirty());
    }

    #[test]
    fn loaded_gb18030_preview_reuses_original_bytes_as_the_editing_baseline() {
        let original = gb18030_bytes("id,name\n1,亚瑟\n");
        let document =
            CsvDocument::from_bytes(Path::new("heroes.csv"), &original, Some(b',')).unwrap();

        let session = DocumentSession::from_loaded_document(document, 1, true).unwrap();

        assert_eq!(session.saved_hash(), blake3::hash(&original));
        assert_eq!(session.document.encoding, CsvEncoding::Utf8);
        assert!(session.is_dirty());
    }

    #[test]
    fn edit_undo_and_redo_track_dirty_state() {
        let (_directory, _path, mut session) = create_session(b"id,name\n1,Arthur\n");

        assert!(session.edit_cell(1, 1, "Merlin".to_owned()).unwrap());
        assert!(session.is_dirty());
        assert!(session.undo().unwrap());
        assert!(!session.is_dirty());
        assert!(session.redo().unwrap());
        assert!(session.is_dirty());
    }

    #[test]
    fn no_op_edit_does_not_create_history() {
        let (_directory, _path, mut session) = create_session(b"id,name\n1,Arthur\n");

        assert!(!session.edit_cell(1, 1, "Arthur".to_owned()).unwrap());
        assert!(!session.undo().unwrap());
        assert!(!session.is_dirty());
    }

    #[test]
    fn save_updates_disk_and_clears_dirty_state() {
        let (_directory, path, mut session) = create_session(b"id,name\n1,Arthur\n");
        session.edit_cell(1, 1, "Merlin".to_owned()).unwrap();

        session.save(false).unwrap();

        assert_eq!(fs::read_to_string(path).unwrap(), "id,name\n1,Merlin\n");
        assert!(!session.is_dirty());
    }

    #[test]
    fn save_detects_external_modification() {
        let (_directory, path, mut session) = create_session(b"id,name\n1,Arthur\n");
        session.edit_cell(1, 1, "Merlin".to_owned()).unwrap();
        fs::write(&path, b"id,name\n1,Lancelot\n").unwrap();

        let error = session.save(false).unwrap_err();

        assert!(matches!(
            error,
            DocumentSessionError::ExternalModification { .. }
        ));
        assert_eq!(fs::read_to_string(path).unwrap(), "id,name\n1,Lancelot\n");
        assert!(session.is_dirty());
    }

    #[test]
    fn forced_save_overwrites_external_modification() {
        let (_directory, path, mut session) = create_session(b"id,name\n1,Arthur\n");
        session.edit_cell(1, 1, "Merlin".to_owned()).unwrap();
        fs::write(&path, b"id,name\n1,Lancelot\n").unwrap();

        session.save(true).unwrap();

        assert_eq!(fs::read_to_string(path).unwrap(), "id,name\n1,Merlin\n");
        assert!(!session.is_dirty());
    }

    #[test]
    fn exposes_the_disk_baseline_without_marking_the_session_dirty() {
        let (_directory, path, session) = create_session(b"id,name\n1,Arthur\n");

        assert_eq!(session.saved_hash(), blake3::hash(&fs::read(path).unwrap()));
        assert_eq!(session.delimiter_override(), Some(b','));
        assert!(!session.is_dirty());
    }

    #[test]
    fn reload_discards_local_edits_and_history() {
        let (_directory, path, mut session) = create_session(b"id,name\n1,Arthur\n");
        session.edit_cell(1, 1, "Merlin".to_owned()).unwrap();
        fs::write(&path, b"id,name\n1,Lancelot\n").unwrap();

        session.reload().unwrap();

        assert_eq!(session.document.records[1][1], "Lancelot");
        assert!(!session.is_dirty());
        assert!(!session.undo().unwrap());
    }

    #[test]
    fn reload_preserves_manual_delimiter_selection() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("heroes.csv");
        fs::write(&path, b"id|name\n1|Arthur\n").unwrap();
        let mut session = DocumentSession::open(&path, Some(b'|')).unwrap();
        fs::write(&path, b"id|name\n1|Merlin\n").unwrap();

        session.reload().unwrap();

        assert_eq!(session.document.delimiter, b'|');
        assert_eq!(session.document.records[1][1], "Merlin");
    }

    #[test]
    fn changing_header_rows_does_not_mark_the_document_dirty() {
        let (_directory, _path, mut session) =
            create_session(b"description,name\nid,name\n1,Arthur\n");

        session.set_header_rows(3).unwrap();

        assert_eq!(session.header_rows, 3);
        assert!(!session.is_dirty());
        assert!(matches!(
            session.set_header_rows(0),
            Err(DocumentSessionError::InvalidHeaderRows(0))
        ));
    }

    #[test]
    fn changing_delimiter_reparses_without_changing_document_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("heroes.csv");
        fs::write(&path, b"id|name\n1|Arthur\n").unwrap();
        let mut session = DocumentSession::open(&path, Some(b',')).unwrap();
        let original = session.document.to_bytes();

        session.set_delimiter(b'|').unwrap();

        assert_eq!(session.document.records[1], vec!["1", "Arthur"]);
        assert_eq!(session.document.to_bytes(), original);
        assert!(!session.is_dirty());
    }

    #[test]
    fn failed_delimiter_change_preserves_the_previous_parse() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("heroes.csv");
        fs::write(&path, b"id|name\n1|Arthur,extra\n").unwrap();
        let mut session = DocumentSession::open(&path, Some(b'|')).unwrap();
        let previous = session.document.clone();

        assert!(session.set_delimiter(b',').is_err());

        assert_eq!(session.document, previous);
        assert_eq!(session.document.delimiter, b'|');
    }

    #[test]
    fn text_edits_share_dirty_state_and_preserve_utf8_bom() {
        let (_directory, path, mut session) =
            create_session(b"\xEF\xBB\xBFid,name\r\n1,Arthur\r\n");

        session.show_text();
        assert!(session.set_text("id,name\r\n1,Merlin\r\n".to_owned()));

        assert_eq!(session.view(), DocumentView::Text);
        assert!(session.is_dirty());
        session.save(false).unwrap();
        assert_eq!(
            fs::read(path).unwrap(),
            b"\xEF\xBB\xBFid,name\r\n1,Merlin\r\n"
        );
        assert!(!session.is_dirty());
    }

    #[test]
    fn invalid_text_keeps_the_last_valid_table_snapshot() {
        let (_directory, _path, mut session) = create_session(b"id,name\n1,Arthur\n");
        let previous = session.document.clone();
        session.show_text();
        session.set_text("id,name\n1\n".to_owned());

        let issue = session.show_table().unwrap_err();

        assert_eq!(session.view(), DocumentView::Text);
        assert_eq!(session.document, previous);
        assert_eq!(issue.count, 1);
        assert!(session.text_parse_issue().is_some());
    }

    #[test]
    fn invalid_text_reports_all_record_width_errors() {
        let (_directory, _path, mut session) = create_session(b"id,name\n1,Arthur\n");
        session.show_text();
        session.set_text("id,name\n1\n2,Merlin,extra\n".to_owned());

        let issue = session.show_table().unwrap_err();

        assert_eq!(issue.count, 2);
        assert!(issue.message.contains("record 2"));
    }

    #[test]
    fn valid_text_replaces_the_table_snapshot() {
        let (_directory, _path, mut session) = create_session(b"id,name\n1,Arthur\n");
        session.show_text();
        session.set_text("id,name\n1,Merlin\n2,Morgana\n".to_owned());

        session.show_table().unwrap();

        assert_eq!(session.view(), DocumentView::Table);
        assert_eq!(session.document.records.len(), 3);
        assert_eq!(session.document.records[1][1], "Merlin");
        assert!(session.text_parse_issue().is_none());
    }

    #[test]
    fn opening_and_reloading_invalid_csv_enters_text_error_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("broken.csv");
        fs::write(&path, b"id,name\n1\n").unwrap();

        let mut session = DocumentSession::open(&path, Some(b',')).unwrap();
        assert_eq!(session.view(), DocumentView::Text);
        assert!(session.text_parse_issue().is_some());
        assert!(!session.is_dirty());

        fs::write(&path, b"id,name\n1,Merlin\n").unwrap();
        session.reload().unwrap();
        assert_eq!(session.view(), DocumentView::Text);
        assert!(session.text_parse_issue().is_none());
        assert_eq!(session.document.records[1][1], "Merlin");
    }

    #[test]
    fn table_edits_are_immediately_visible_in_the_text_buffer() {
        let (_directory, _path, mut session) = create_session(b"id,name\n1,Arthur\n");

        assert!(session.text_override.is_none());

        session.edit_cell(1, 1, "Merlin".to_owned()).unwrap();

        assert_eq!(session.text(), "id,name\n1,Merlin\n");
        assert!(session.text_override.is_none());
    }
}
