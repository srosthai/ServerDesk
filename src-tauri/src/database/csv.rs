//! Streaming CSV import and export for table data and read-only queries.
//!
//! The codec is deliberately independent of either database driver. It streams
//! characters from a local file, understands quoted fields with embedded
//! delimiters and newlines, preserves full values, and reports the one-based
//! row and column where malformed input fails. NULL is distinct from an empty
//! string: an unquoted empty field can be imported as SQL NULL while a quoted
//! empty field stays an empty string.
//!
//! Exports are written to a sibling partial file and renamed only after every
//! row has been written and flushed. A cancelled or failed export removes the
//! partial file, so an existing destination is never replaced by an incomplete
//! export. No connection detail, credential, or row value is written to logs or
//! progress events.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use crate::{core::AppError, storage::DatabaseEngine};

use super::{
    errors::DatabaseError,
    rows::{RowQueryCancellation, RowQueryControl},
    service::DatabaseConnectionRegistry,
    traits::TableRef,
};

/// Event emitted while a CSV export or import is running.
pub const DATABASE_CSV_TRANSFER_EVENT: &str = "database-csv-transfer-event";
/// Default bound for one CSV transfer.
pub const DEFAULT_CSV_TIMEOUT: Duration = Duration::from_secs(300);
/// Hard maximum bound for one CSV transfer.
pub const MAX_CSV_TIMEOUT: Duration = Duration::from_secs(3_600);
/// Default number of rows shown in an import preview.
pub const DEFAULT_CSV_PREVIEW_ROWS: u32 = 20;
/// Hard maximum number of rows shown in an import preview.
pub const MAX_CSV_PREVIEW_ROWS: u32 = 100;
/// Maximum characters accepted in one CSV field. A larger field is rejected
/// explicitly; it is never truncated.
pub const MAX_CSV_FIELD_CHARACTERS: usize = 16_777_216;
/// Maximum columns accepted in one CSV file or table export.
pub const MAX_CSV_COLUMNS: usize = 1_024;
/// Maximum warnings kept in one response. The response says when it was cut.
pub const MAX_CSV_REPORTED_ISSUES: usize = 100;
/// Maximum length of a caller-generated transfer id.
pub const MAX_CSV_REQUEST_ID_LENGTH: usize = 128;

const MAX_LOCAL_PATH_CHARACTERS: usize = 4_096;
const MAX_NULL_LITERAL_CHARACTERS: usize = 256;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(120);

/// The field separator used by a CSV file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CsvDelimiter {
    #[default]
    Comma,
    Semicolon,
    Tab,
    Pipe,
}

impl CsvDelimiter {
    pub const fn character(self) -> char {
        match self {
            Self::Comma => ',',
            Self::Semicolon => ';',
            Self::Tab => '\t',
            Self::Pipe => '|',
        }
    }
}

/// Character encoding used to read or write a CSV file. Unsupported
/// characters are rejected instead of replaced.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CsvEncoding {
    #[default]
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Latin1,
}

impl CsvEncoding {
    const fn writes_bom(self) -> bool {
        matches!(self, Self::Utf8Bom | Self::Utf16Le | Self::Utf16Be)
    }
}

/// How an imported field becomes SQL NULL.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum CsvNullMode {
    /// An unquoted empty field is NULL. A quoted empty field (`""`) stays an
    /// empty string. This is the default because it round-trips the writer.
    #[default]
    UnquotedEmpty,
    /// A field whose text exactly matches the literal is NULL. Other fields,
    /// including an empty field, are values.
    Literal(String),
    /// Every field is a value, including an empty field.
    Never,
}

impl CsvNullMode {
    pub fn is_null(&self, field: &CsvField) -> bool {
        match self {
            Self::UnquotedEmpty => !field.quoted && field.value.is_empty(),
            Self::Literal(literal) => field.value == *literal,
            Self::Never => false,
        }
    }
}

/// One row of source data as it leaves the parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvField {
    pub value: String,
    pub quoted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvRecord {
    /// One-based physical CSV record number. Blank lines count as records even
    /// when they are skipped.
    pub record_number: u64,
    pub fields: Vec<CsvField>,
    /// True for a line that contains no characters before the record
    /// separator. For a one-column file this is a legitimate NULL/empty row;
    /// for a wider file it is a blank separator line.
    pub blank: bool,
}

/// Row and column context for a warning or a skipped row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvWarning {
    pub row_number: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column_number: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column_name: Option<String>,
    pub message: String,
}

/// Options that describe how to parse an import file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvImportOptions {
    #[serde(default)]
    pub delimiter: CsvDelimiter,
    #[serde(default)]
    pub encoding: CsvEncoding,
    pub has_header: bool,
    #[serde(default)]
    pub null_mode: CsvNullMode,
}

/// One source-column to target-column mapping. A source field can be ignored
/// by omitting it from the mapping list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvColumnMapping {
    pub source_index: u32,
    pub target_column: super::dialect::Identifier,
}

/// A table or one read-only SQL statement as the export source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CsvExportSource {
    Table { table: TableRef },
    Query { sql: String },
}

/// One export request. The destination is an absolute local path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvExportRequest {
    pub source: CsvExportSource,
    pub destination_path: String,
    #[serde(default)]
    pub delimiter: CsvDelimiter,
    #[serde(default)]
    pub encoding: CsvEncoding,
    #[serde(default = "default_true")]
    pub include_header: bool,
    #[serde(default)]
    pub overwrite: bool,
}

const fn default_true() -> bool {
    true
}

/// One import request. The source is an absolute local path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvImportRequest {
    pub table: TableRef,
    pub source_path: String,
    pub options: CsvImportOptions,
    pub mappings: Vec<CsvColumnMapping>,
}

/// Shared terminal status for import and export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CsvTransferStatus {
    Running,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CsvTransferDirection {
    Export,
    Import,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CsvTransferPhase {
    Preparing,
    Transferring,
    Finalizing,
}

/// Progress emitted while a transfer runs. It never contains row values,
/// connection details, or credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvTransferProgress {
    pub transfer_id: String,
    pub direction: CsvTransferDirection,
    pub status: CsvTransferStatus,
    pub phase: CsvTransferPhase,
    pub rows_processed: u64,
    pub rows_written: u64,
    pub rows_skipped: u64,
    pub total_rows: Option<u64>,
    pub bytes_processed: Option<u64>,
    pub total_bytes: Option<u64>,
    pub warnings: Vec<String>,
    pub may_be_incomplete: bool,
    pub emitted_at: i64,
}

/// Sink used by the command layer to forward progress to the UI.
pub trait CsvTransferEventSink: Send + Sync {
    fn emit(&self, progress: &CsvTransferProgress);
}

/// Summary returned by an adapter after writing one CSV export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvExportSummary {
    pub columns: Vec<String>,
    pub rows: u64,
}

/// Summary returned by an adapter after importing rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvImportSummary {
    pub table: TableRef,
    pub engine: DatabaseEngine,
    pub rows_read: u64,
    pub rows_imported: u64,
    pub rows_skipped: u64,
    pub warnings: Vec<CsvWarning>,
    pub warnings_truncated: bool,
}

/// Receives a header and then full, untruncated values for every exported row.
pub trait CsvExportSink: Send {
    fn write_header(&mut self, columns: &[String]) -> Result<(), DatabaseError>;
    fn write_row(&mut self, values: &[Option<String>]) -> Result<(), DatabaseError>;
}

/// Receives progress from an adapter while it imports rows.
pub trait CsvImportObserver: Send {
    fn row_imported(&mut self, row_number: u64);
    fn row_skipped(&mut self, warning: &CsvWarning);
}

/// Streaming source used by adapters. The source owns the file handle and
/// returns one row at a time, so an import never loads the file into memory.
pub trait CsvImportSource: Send {
    fn headers(&self) -> &[String];
    fn next_row(&mut self) -> Result<Option<CsvSourceRow>, DatabaseError>;
    fn take_issues(&mut self) -> Vec<CsvWarning>;
    fn issues_truncated(&self) -> bool;
    fn rows_read(&self) -> u64;
    fn skipped_rows(&self) -> u64;
    fn bytes_read(&self) -> u64;
}

/// One parsed data row with its physical record number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvSourceRow {
    pub record_number: u64,
    pub fields: Vec<CsvField>,
}

/// Result of one completed or cancelled export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvExportResult {
    pub status: CsvTransferStatus,
    pub engine: DatabaseEngine,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<TableRef>,
    pub destination_path: String,
    pub rows_exported: u64,
    pub columns: Vec<String>,
    pub bytes_written: u64,
    pub encoding: CsvEncoding,
    pub delimiter: CsvDelimiter,
    pub include_header: bool,
    pub warnings: Vec<String>,
    pub may_be_incomplete: bool,
    pub values_truncated: bool,
    pub duration_ms: u64,
}

/// Result of one completed or cancelled import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvImportResult {
    pub status: CsvTransferStatus,
    pub engine: DatabaseEngine,
    pub table: TableRef,
    pub rows_read: u64,
    pub rows_imported: u64,
    pub rows_skipped: u64,
    pub warnings: Vec<CsvWarning>,
    pub warnings_truncated: bool,
    pub transaction: super::row_writes::RowTransactionState,
    pub may_be_incomplete: bool,
    pub duration_ms: u64,
}

/// Preview returned before an import. Values are full; only the row count is
/// bounded, and `has_more` says when more data exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvImportPreview {
    pub headers: Vec<String>,
    pub rows: Vec<CsvPreviewRow>,
    pub warnings: Vec<CsvWarning>,
    pub warnings_truncated: bool,
    pub sampled_rows: u64,
    pub has_more: bool,
    pub delimiter: CsvDelimiter,
    pub encoding: CsvEncoding,
    pub has_header: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvPreviewRow {
    pub row_number: u64,
    pub fields: Vec<CsvPreviewField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvPreviewField {
    pub value: String,
    pub quoted: bool,
    pub is_null: bool,
}

/// Resolves an optional timeout in milliseconds with the CSV hard bound.
pub fn csv_transfer_timeout(timeout_ms: Option<u64>) -> Duration {
    let milliseconds = timeout_ms
        .unwrap_or(DEFAULT_CSV_TIMEOUT.as_millis() as u64)
        .clamp(1_000, MAX_CSV_TIMEOUT.as_millis() as u64);
    Duration::from_millis(milliseconds)
}

/// Validates a caller-generated transfer id. It is only an in-memory
/// cancellation key and is never sent to a database or written to a file.
pub fn validate_csv_request_id(value: &str) -> Result<(), DatabaseError> {
    if value.is_empty() || value.len() > MAX_CSV_REQUEST_ID_LENGTH {
        return Err(DatabaseError::validation(format!(
            "A CSV transfer id must contain between 1 and {MAX_CSV_REQUEST_ID_LENGTH} characters."
        )));
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(DatabaseError::validation(
            "A CSV transfer id may contain only letters, digits, hyphens, underscores, and dots.",
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Character decoding
// ---------------------------------------------------------------------------

struct DecodedCharReader<R> {
    inner: BufReader<R>,
    encoding: CsvEncoding,
    first: bool,
    pending: VecDeque<u8>,
    bytes_read: u64,
}

impl<R: Read> DecodedCharReader<R> {
    fn new(inner: R, encoding: CsvEncoding) -> Self {
        Self {
            inner: BufReader::new(inner),
            encoding,
            first: true,
            pending: VecDeque::new(),
            bytes_read: 0,
        }
    }

    fn read_byte(&mut self) -> Result<Option<u8>, DatabaseError> {
        if let Some(byte) = self.pending.pop_front() {
            self.bytes_read += 1;
            return Ok(Some(byte));
        }
        let mut buffer = [0u8; 1];
        match self.inner.read(&mut buffer) {
            Ok(0) => Ok(None),
            Ok(_) => {
                self.bytes_read += 1;
                Ok(Some(buffer[0]))
            }
            Err(_) => Err(DatabaseError::validation("The CSV file could not be read.")),
        }
    }

    fn push_back(&mut self, bytes: &[u8]) {
        for byte in bytes.iter().rev() {
            self.pending.push_front(*byte);
            self.bytes_read = self.bytes_read.saturating_sub(1);
        }
    }

    fn read_char(&mut self) -> Result<Option<char>, DatabaseError> {
        match self.encoding {
            CsvEncoding::Utf8 | CsvEncoding::Utf8Bom => self.read_utf8(),
            CsvEncoding::Utf16Le | CsvEncoding::Utf16Be => self.read_utf16(),
            CsvEncoding::Latin1 => Ok(self.read_byte()?.map(char::from)),
        }
    }

    fn skip_utf8_bom(&mut self) -> Result<(), DatabaseError> {
        let Some(first) = self.read_byte()? else {
            return Ok(());
        };
        if first != 0xEF {
            self.push_back(&[first]);
            return Ok(());
        }
        let second = self.read_byte()?;
        let third = if second.is_some() {
            self.read_byte()?
        } else {
            None
        };
        match (second, third) {
            (Some(0xBB), Some(0xBF)) => Ok(()),
            (Some(second), Some(third)) => {
                self.push_back(&[first, second, third]);
                Ok(())
            }
            (Some(second), None) => {
                self.push_back(&[first, second]);
                Ok(())
            }
            (None, _) => {
                self.push_back(&[first]);
                Ok(())
            }
        }
    }

    fn read_utf8(&mut self) -> Result<Option<char>, DatabaseError> {
        if self.first {
            self.first = false;
            self.skip_utf8_bom()?;
        }
        let Some(first) = self.read_byte()? else {
            return Ok(None);
        };
        if first < 0x80 {
            return Ok(Some(char::from(first)));
        }
        let (width, mut code) = match first {
            0xC2..=0xDF => (2, u32::from(first & 0x1F)),
            0xE0..=0xEF => (3, u32::from(first & 0x0F)),
            0xF0..=0xF4 => (4, u32::from(first & 0x07)),
            _ => {
                return Err(DatabaseError::validation(
                    "The CSV file contains an invalid UTF-8 byte sequence.",
                ));
            }
        };
        for _ in 1..width {
            let Some(byte) = self.read_byte()? else {
                return Err(DatabaseError::validation(
                    "The CSV file ends in the middle of a UTF-8 character.",
                ));
            };
            if byte & 0xC0 != 0x80 {
                return Err(DatabaseError::validation(
                    "The CSV file contains an invalid UTF-8 continuation byte.",
                ));
            }
            code = (code << 6) | u32::from(byte & 0x3F);
        }
        let minimum = match width {
            2 => 0x80,
            3 => 0x800,
            4 => 0x10000,
            _ => 0,
        };
        if code < minimum {
            return Err(DatabaseError::validation(
                "The CSV file contains an overlong UTF-8 character.",
            ));
        }
        char::from_u32(code).map(Some).ok_or_else(|| {
            DatabaseError::validation("The CSV file contains an invalid UTF-8 character.")
        })
    }

    fn read_utf16_unit(&mut self) -> Result<Option<u16>, DatabaseError> {
        let Some(first) = self.read_byte()? else {
            return Ok(None);
        };
        let Some(second) = self.read_byte()? else {
            return Err(DatabaseError::validation(
                "The CSV file ends in the middle of a UTF-16 code unit.",
            ));
        };
        Ok(Some(match self.encoding {
            CsvEncoding::Utf16Le => u16::from_le_bytes([first, second]),
            CsvEncoding::Utf16Be => u16::from_be_bytes([first, second]),
            _ => unreachable!("only UTF-16 reaches this decoder"),
        }))
    }

    fn read_utf16(&mut self) -> Result<Option<char>, DatabaseError> {
        let Some(first) = self.read_utf16_unit()? else {
            return Ok(None);
        };
        if self.first {
            self.first = false;
            let expected = match self.encoding {
                CsvEncoding::Utf16Le => 0xFEFF,
                CsvEncoding::Utf16Be => 0xFEFF,
                _ => unreachable!("only UTF-16 reaches this decoder"),
            };
            if first == expected {
                return self.read_utf16();
            }
            if first == 0xFFFE {
                return Err(DatabaseError::validation(
                    "The CSV file has a UTF-16 byte-order mark that does not match the selected encoding.",
                ));
            }
        }

        if (0xD800..=0xDBFF).contains(&first) {
            let Some(second) = self.read_utf16_unit()? else {
                return Err(DatabaseError::validation(
                    "The CSV file ends in the middle of a UTF-16 surrogate pair.",
                ));
            };
            if !(0xDC00..=0xDFFF).contains(&second) {
                return Err(DatabaseError::validation(
                    "The CSV file contains an invalid UTF-16 surrogate pair.",
                ));
            }
            let code = 0x10000 + ((u32::from(first) - 0xD800) << 10) + (u32::from(second) - 0xDC00);
            return char::from_u32(code).map(Some).ok_or_else(|| {
                DatabaseError::validation("The CSV file contains an invalid UTF-16 character.")
            });
        }
        if (0xDC00..=0xDFFF).contains(&first) {
            return Err(DatabaseError::validation(
                "The CSV file contains an unpaired UTF-16 surrogate.",
            ));
        }
        char::from_u32(u32::from(first)).map(Some).ok_or_else(|| {
            DatabaseError::validation("The CSV file contains an invalid UTF-16 character.")
        })
    }
}

// ---------------------------------------------------------------------------
// Parser and writer
// ---------------------------------------------------------------------------

enum FieldTerminator {
    Delimiter,
    Newline,
    EndOfFile,
}

pub struct CsvParser<R> {
    reader: DecodedCharReader<R>,
    delimiter: char,
    record_number: u64,
    pending: Option<char>,
}

impl<R: Read> CsvParser<R> {
    pub fn new(reader: R, delimiter: CsvDelimiter, encoding: CsvEncoding) -> Self {
        Self {
            reader: DecodedCharReader::new(reader, encoding),
            delimiter: delimiter.character(),
            record_number: 0,
            pending: None,
        }
    }

    fn read_char(&mut self) -> Result<Option<char>, DatabaseError> {
        if let Some(character) = self.pending.take() {
            return Ok(Some(character));
        }
        self.reader.read_char()
    }

    fn push_back(&mut self, character: char) {
        self.pending = Some(character);
    }

    fn error(&self, column: u32, message: &str) -> DatabaseError {
        DatabaseError::validation(format!(
            "CSV row {}, column {column}: {message}",
            self.record_number
        ))
    }

    /// Reads the next physical record. A blank line is returned as a record
    /// with one empty unquoted field; the import source decides whether that is
    /// a separator line or a one-column NULL row.
    pub fn next_record(&mut self) -> Result<Option<CsvRecord>, DatabaseError> {
        let Some(first) = self.read_char()? else {
            return Ok(None);
        };
        self.record_number += 1;
        if first == '\n' || first == '\r' {
            if first == '\r' {
                let next = self.read_char()?;
                if next != Some('\n')
                    && let Some(next) = next
                {
                    self.push_back(next);
                }
            }
            return Ok(Some(CsvRecord {
                record_number: self.record_number,
                fields: vec![CsvField {
                    value: String::new(),
                    quoted: false,
                }],
                blank: true,
            }));
        }

        let mut fields = Vec::new();
        let mut column = 1u32;
        let mut character = first;
        loop {
            let field = if character == '"' {
                self.parse_quoted(column)?
            } else {
                self.parse_unquoted(character, column)?
            };
            fields.push(field.0);
            match field.1 {
                FieldTerminator::Delimiter => {
                    column += 1;
                    match self.read_char()? {
                        Some(next) => character = next,
                        None => {
                            fields.push(CsvField {
                                value: String::new(),
                                quoted: false,
                            });
                            break;
                        }
                    }
                }
                FieldTerminator::Newline | FieldTerminator::EndOfFile => break,
            }
        }

        if fields.len() > MAX_CSV_COLUMNS {
            return Err(DatabaseError::validation(format!(
                "CSV row {} has more than {MAX_CSV_COLUMNS} columns.",
                self.record_number
            )));
        }
        Ok(Some(CsvRecord {
            record_number: self.record_number,
            fields,
            blank: false,
        }))
    }

    fn parse_unquoted(
        &mut self,
        first: char,
        column: u32,
    ) -> Result<(CsvField, FieldTerminator), DatabaseError> {
        let mut value = String::new();
        let mut character_count = 0usize;
        let mut character = first;
        loop {
            match character {
                '\n' => {
                    return Ok((
                        CsvField {
                            value,
                            quoted: false,
                        },
                        FieldTerminator::Newline,
                    ));
                }
                '\r' => {
                    let next = self.read_char()?;
                    if next != Some('\n')
                        && let Some(next) = next
                    {
                        self.push_back(next);
                    }
                    return Ok((
                        CsvField {
                            value,
                            quoted: false,
                        },
                        FieldTerminator::Newline,
                    ));
                }
                value_character if value_character == self.delimiter => {
                    return Ok((
                        CsvField {
                            value,
                            quoted: false,
                        },
                        FieldTerminator::Delimiter,
                    ));
                }
                '"' => {
                    return Err(self.error(
                        column,
                        "a quote must be the first character of a quoted field",
                    ));
                }
                '\0' => {
                    return Err(self.error(column, "NUL characters are not accepted in CSV fields"));
                }
                other => {
                    if character_count >= MAX_CSV_FIELD_CHARACTERS {
                        return Err(self.error(
                            column,
                            &format!(
                                "the field exceeds the {MAX_CSV_FIELD_CHARACTERS}-character limit"
                            ),
                        ));
                    }
                    character_count += 1;
                    value.push(other);
                }
            }

            let Some(next) = self.read_char()? else {
                return Ok((
                    CsvField {
                        value,
                        quoted: false,
                    },
                    FieldTerminator::EndOfFile,
                ));
            };
            character = next;
        }
    }

    fn parse_quoted(&mut self, column: u32) -> Result<(CsvField, FieldTerminator), DatabaseError> {
        let mut value = String::new();
        let mut character_count = 0usize;
        loop {
            let Some(character) = self.read_char()? else {
                return Err(self.error(
                    column,
                    "a quoted field is not closed before the end of the file",
                ));
            };
            match character {
                '"' => {
                    let Some(next) = self.read_char()? else {
                        return Ok((
                            CsvField {
                                value,
                                quoted: true,
                            },
                            FieldTerminator::EndOfFile,
                        ));
                    };
                    match next {
                        '"' => {
                            if character_count >= MAX_CSV_FIELD_CHARACTERS {
                                return Err(self.error(
                                    column,
                                    &format!(
                                        "the field exceeds the {MAX_CSV_FIELD_CHARACTERS}-character limit"
                                    ),
                                ));
                            }
                            character_count += 1;
                            value.push('"');
                        }
                        '\n' => {
                            return Ok((
                                CsvField {
                                    value,
                                    quoted: true,
                                },
                                FieldTerminator::Newline,
                            ));
                        }
                        '\r' => {
                            let after = self.read_char()?;
                            if after != Some('\n')
                                && let Some(after) = after
                            {
                                self.push_back(after);
                            }
                            return Ok((
                                CsvField {
                                    value,
                                    quoted: true,
                                },
                                FieldTerminator::Newline,
                            ));
                        }
                        other if other == self.delimiter => {
                            return Ok((
                                CsvField {
                                    value,
                                    quoted: true,
                                },
                                FieldTerminator::Delimiter,
                            ));
                        }
                        _ => {
                            return Err(self.error(column, "unexpected text after a closing quote"));
                        }
                    }
                }
                '\0' => {
                    return Err(self.error(column, "NUL characters are not accepted in CSV fields"));
                }
                other => {
                    if character_count >= MAX_CSV_FIELD_CHARACTERS {
                        return Err(self.error(
                            column,
                            &format!(
                                "the field exceeds the {MAX_CSV_FIELD_CHARACTERS}-character limit"
                            ),
                        ));
                    }
                    character_count += 1;
                    value.push(other);
                }
            }
        }
    }
}

/// Streaming CSV writer. `None` is an unquoted empty field; `Some("")` is a
/// quoted empty field, so NULL and an empty string survive a round trip.
pub struct CsvWriter<W: Write> {
    writer: W,
    delimiter: char,
    encoding: CsvEncoding,
    bytes_written: u64,
    rows_written: u64,
}

impl<W: Write> CsvWriter<W> {
    pub fn new(
        mut writer: W,
        delimiter: CsvDelimiter,
        encoding: CsvEncoding,
    ) -> Result<Self, DatabaseError> {
        let mut bytes_written = 0u64;
        if encoding.writes_bom() {
            let bom: &[u8] = match encoding {
                CsvEncoding::Utf8Bom => &[0xEF, 0xBB, 0xBF],
                CsvEncoding::Utf16Le => &[0xFF, 0xFE],
                CsvEncoding::Utf16Be => &[0xFE, 0xFF],
                _ => &[],
            };
            writer.write_all(bom).map_err(|_| {
                DatabaseError::internal("The CSV destination could not be written.")
            })?;
            bytes_written = u64::try_from(bom.len()).unwrap_or(0);
        }
        Ok(Self {
            writer,
            delimiter: delimiter.character(),
            encoding,
            bytes_written,
            rows_written: 0,
        })
    }

    pub fn write_header(&mut self, columns: &[String]) -> Result<(), DatabaseError> {
        let values = columns
            .iter()
            .map(|column| Some(column.as_str()))
            .collect::<Vec<_>>();
        self.write_record(&values)
    }

    pub fn write_record(&mut self, fields: &[Option<&str>]) -> Result<(), DatabaseError> {
        let mut line = String::new();
        for (position, field) in fields.iter().enumerate() {
            if position > 0 {
                line.push(self.delimiter);
            }
            match field {
                None => {}
                Some(value) => {
                    let must_quote = value.is_empty()
                        || value.contains(self.delimiter)
                        || value.contains('"')
                        || value.contains('\n')
                        || value.contains('\r');
                    if must_quote {
                        line.push('"');
                        for character in value.chars() {
                            if character == '"' {
                                line.push('"');
                            }
                            line.push(character);
                        }
                        line.push('"');
                    } else {
                        line.push_str(value);
                    }
                }
            }
        }
        line.push('\n');
        self.write_encoded(&line)?;
        self.rows_written += 1;
        Ok(())
    }

    fn write_encoded(&mut self, text: &str) -> Result<(), DatabaseError> {
        let bytes = match self.encoding {
            CsvEncoding::Utf8 | CsvEncoding::Utf8Bom => text.as_bytes().to_vec(),
            CsvEncoding::Utf16Le | CsvEncoding::Utf16Be => {
                let mut bytes = Vec::with_capacity(text.len() * 2);
                for unit in text.encode_utf16() {
                    match self.encoding {
                        CsvEncoding::Utf16Le => bytes.extend_from_slice(&unit.to_le_bytes()),
                        CsvEncoding::Utf16Be => bytes.extend_from_slice(&unit.to_be_bytes()),
                        _ => unreachable!("matched UTF-16 above"),
                    }
                }
                bytes
            }
            CsvEncoding::Latin1 => {
                let mut bytes = Vec::with_capacity(text.len());
                for character in text.chars() {
                    let value = u32::from(character);
                    if value > 0xFF {
                        return Err(DatabaseError::validation(
                            "A value cannot be written with the Latin-1 encoding. Choose UTF-8 instead.",
                        ));
                    }
                    bytes.push(value as u8);
                }
                bytes
            }
        };
        self.writer
            .write_all(&bytes)
            .map_err(|_| DatabaseError::internal("The CSV destination could not be written."))?;
        self.bytes_written = self
            .bytes_written
            .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), DatabaseError> {
        self.writer
            .flush()
            .map_err(|_| DatabaseError::internal("The CSV destination could not be written."))
    }

    pub const fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    pub const fn rows_written(&self) -> u64 {
        self.rows_written
    }
}

// ---------------------------------------------------------------------------
// File source, sink, and temporary-file guard
// ---------------------------------------------------------------------------

pub struct CsvImportSourceFile {
    parser: CsvParser<File>,
    headers: Vec<String>,
    pending: Option<CsvRecord>,
    expected_fields: usize,
    issues: Vec<CsvWarning>,
    issues_truncated: bool,
    rows_read: u64,
    skipped_rows: u64,
    bytes_read: u64,
}

impl CsvImportSourceFile {
    pub fn open(path: &Path, options: &CsvImportOptions) -> Result<Self, DatabaseError> {
        let file = File::open(path).map_err(|_| {
            DatabaseError::validation("The CSV source file does not exist or cannot be read.")
        })?;
        let mut parser = CsvParser::new(file, options.delimiter, options.encoding);
        let first = loop {
            let Some(record) = parser.next_record()? else {
                return Err(DatabaseError::validation("The CSV file is empty."));
            };
            if options.has_header && record.blank {
                continue;
            }
            break record;
        };
        if first.fields.is_empty() {
            return Err(DatabaseError::validation(
                "The CSV file does not contain any columns.",
            ));
        }
        if first.fields.len() > MAX_CSV_COLUMNS {
            return Err(DatabaseError::validation(format!(
                "The CSV file has more than {MAX_CSV_COLUMNS} columns."
            )));
        }

        let mut issues = Vec::new();
        let (headers, pending) = if options.has_header {
            let headers = first
                .fields
                .iter()
                .map(|field| field.value.clone())
                .collect::<Vec<_>>();
            (headers, None)
        } else {
            let headers = (1..=first.fields.len())
                .map(|index| format!("Column {index}"))
                .collect::<Vec<_>>();
            (headers, Some(first))
        };
        let expected_fields = headers.len();
        let mut seen = HashSet::new();
        let mut issues_truncated = false;
        for (index, header) in headers.iter().enumerate() {
            if !seen.insert(header.as_str()) {
                push_warning(
                    &mut issues,
                    &mut issues_truncated,
                    CsvWarning {
                        row_number: 1,
                        column_number: Some(u32::try_from(index + 1).unwrap_or(u32::MAX)),
                        column_name: Some(header.clone()),
                        message: "The same header appears more than once. Map columns by position to avoid ambiguity.".to_string(),
                    },
                );
            }
        }

        Ok(Self {
            parser,
            headers,
            pending,
            expected_fields,
            issues,
            issues_truncated,
            rows_read: 0,
            skipped_rows: 0,
            bytes_read: 0,
        })
    }

    pub fn has_more(&mut self) -> Result<bool, DatabaseError> {
        loop {
            if self.pending.is_some() {
                return Ok(true);
            }
            let Some(record) = self.parser.next_record()? else {
                return Ok(false);
            };
            if record.blank && self.expected_fields != 1 {
                continue;
            }
            self.pending = Some(record);
            return Ok(true);
        }
    }

    fn record_to_row(&mut self, record: CsvRecord) -> Option<CsvSourceRow> {
        if record.blank && self.expected_fields != 1 {
            return None;
        }
        self.rows_read += 1;
        if record.fields.len() != self.expected_fields {
            self.skipped_rows += 1;
            push_warning(
                &mut self.issues,
                &mut self.issues_truncated,
                CsvWarning {
                    row_number: record.record_number,
                    column_number: Some(
                        u32::try_from(self.expected_fields.saturating_add(1)).unwrap_or(u32::MAX),
                    ),
                    column_name: None,
                    message: format!(
                        "Row {} has {} field(s); the header defines {}. The row was skipped.",
                        record.record_number,
                        record.fields.len(),
                        self.expected_fields
                    ),
                },
            );
            return None;
        }
        if record.fields.iter().any(|field| field.value.contains('\0')) {
            self.skipped_rows += 1;
            let column = record
                .fields
                .iter()
                .position(|field| field.value.contains('\0'))
                .map(|index| u32::try_from(index + 1).unwrap_or(u32::MAX));
            push_warning(
                &mut self.issues,
                &mut self.issues_truncated,
                CsvWarning {
                    row_number: record.record_number,
                    column_number: column,
                    column_name: column.and_then(|column| {
                        self.headers
                            .get(usize::try_from(column).ok()?.saturating_sub(1))
                            .cloned()
                    }),
                    message: format!(
                        "Row {} contains a NUL character and was skipped.",
                        record.record_number
                    ),
                },
            );
            return None;
        }
        Some(CsvSourceRow {
            record_number: record.record_number,
            fields: record.fields,
        })
    }
}

impl CsvImportSource for CsvImportSourceFile {
    fn headers(&self) -> &[String] {
        &self.headers
    }

    fn next_row(&mut self) -> Result<Option<CsvSourceRow>, DatabaseError> {
        loop {
            let record = match self.pending.take() {
                Some(record) => record,
                None => match self.parser.next_record()? {
                    Some(record) => record,
                    None => {
                        self.bytes_read = self.parser.reader.bytes_read;
                        return Ok(None);
                    }
                },
            };
            if let Some(row) = self.record_to_row(record) {
                self.bytes_read = self.parser.reader.bytes_read;
                return Ok(Some(row));
            }
        }
    }

    fn take_issues(&mut self) -> Vec<CsvWarning> {
        std::mem::take(&mut self.issues)
    }

    fn issues_truncated(&self) -> bool {
        self.issues_truncated
    }

    fn rows_read(&self) -> u64 {
        self.rows_read
    }

    fn skipped_rows(&self) -> u64 {
        self.skipped_rows
    }

    fn bytes_read(&self) -> u64 {
        self.bytes_read
    }
}

/// Removes a partial destination unless it is explicitly disarmed after a
/// successful rename.
pub struct CsvTempFile {
    path: PathBuf,
    armed: bool,
}

impl CsvTempFile {
    pub fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CsvTempFile {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

struct CsvFileExportSink {
    writer: CsvWriter<BufWriter<File>>,
    columns: Vec<String>,
    rows: u64,
    include_header: bool,
    tracker: CsvProgressTracker,
}

impl CsvFileExportSink {
    fn new(
        file: File,
        delimiter: CsvDelimiter,
        encoding: CsvEncoding,
        include_header: bool,
        tracker: CsvProgressTracker,
    ) -> Result<Self, DatabaseError> {
        Ok(Self {
            writer: CsvWriter::new(BufWriter::new(file), delimiter, encoding)?,
            columns: Vec::new(),
            rows: 0,
            include_header,
            tracker,
        })
    }

    fn finish(mut self) -> Result<(u64, u64, Vec<String>), DatabaseError> {
        self.writer.flush()?;
        Ok((self.writer.bytes_written(), self.rows, self.columns))
    }
}

impl CsvExportSink for CsvFileExportSink {
    fn write_header(&mut self, columns: &[String]) -> Result<(), DatabaseError> {
        if columns.len() > MAX_CSV_COLUMNS {
            return Err(DatabaseError::validation(format!(
                "A CSV export cannot contain more than {MAX_CSV_COLUMNS} columns."
            )));
        }
        self.columns = columns.to_vec();
        if self.include_header {
            self.writer.write_header(columns)?;
        }
        Ok(())
    }

    fn write_row(&mut self, values: &[Option<String>]) -> Result<(), DatabaseError> {
        if values.len() != self.columns.len() {
            return Err(DatabaseError::internal(
                "The CSV row did not match the exported columns.",
            ));
        }
        let borrowed = values.iter().map(Option::as_deref).collect::<Vec<_>>();
        self.writer.write_record(&borrowed)?;
        self.rows = self.rows.saturating_add(1);
        self.tracker
            .row_written(self.rows, self.writer.bytes_written());
        Ok(())
    }
}

struct CsvFileImportObserver {
    tracker: CsvProgressTracker,
}

impl CsvImportObserver for CsvFileImportObserver {
    fn row_imported(&mut self, _row_number: u64) {
        self.tracker.row_imported();
    }

    fn row_skipped(&mut self, warning: &CsvWarning) {
        self.tracker.row_skipped(&warning.message);
    }
}

// ---------------------------------------------------------------------------
// Progress tracking
// ---------------------------------------------------------------------------

struct CsvProgressState {
    rows_processed: u64,
    rows_written: u64,
    rows_skipped: u64,
    total_bytes: Option<u64>,
    bytes_processed: Option<u64>,
    warnings: Vec<String>,
    last_emit: Instant,
}

#[derive(Clone)]
struct CsvProgressTracker {
    inner: Arc<Mutex<CsvProgressState>>,
    events: Arc<dyn CsvTransferEventSink>,
    transfer_id: String,
    direction: CsvTransferDirection,
}

impl CsvProgressTracker {
    fn new(
        transfer_id: String,
        direction: CsvTransferDirection,
        events: Arc<dyn CsvTransferEventSink>,
        total_bytes: Option<u64>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(CsvProgressState {
                rows_processed: 0,
                rows_written: 0,
                rows_skipped: 0,
                total_bytes,
                bytes_processed: None,
                warnings: Vec::new(),
                last_emit: Instant::now()
                    .checked_sub(PROGRESS_INTERVAL)
                    .unwrap_or_else(Instant::now),
            })),
            events,
            transfer_id,
            direction,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CsvProgressState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn row_written(&self, rows: u64, bytes: u64) {
        let mut state = self.lock();
        state.rows_written = rows;
        state.rows_processed = rows.saturating_add(state.rows_skipped);
        state.bytes_processed = Some(bytes);
        self.emit_locked(&mut state, false);
    }

    fn row_imported(&self) {
        let mut state = self.lock();
        state.rows_written = state.rows_written.saturating_add(1);
        state.rows_processed = state.rows_written.saturating_add(state.rows_skipped);
        self.emit_locked(&mut state, false);
    }

    fn row_skipped(&self, message: &str) {
        let mut state = self.lock();
        state.rows_skipped = state.rows_skipped.saturating_add(1);
        state.rows_processed = state.rows_written.saturating_add(state.rows_skipped);
        if state.warnings.len() < 20 {
            state.warnings.push(message.to_string());
        }
        self.emit_locked(&mut state, false);
    }

    fn set_counts(&self, rows_processed: u64, rows_written: u64, rows_skipped: u64) {
        let mut state = self.lock();
        state.rows_processed = rows_processed;
        state.rows_written = rows_written;
        state.rows_skipped = rows_skipped;
    }

    fn set_bytes(&self, bytes: u64) {
        let mut state = self.lock();
        state.bytes_processed = Some(bytes);
    }

    fn finish(
        &self,
        status: CsvTransferStatus,
        phase: CsvTransferPhase,
        may_be_incomplete: bool,
        warning: Option<String>,
    ) {
        let mut state = self.lock();
        if let Some(warning) = warning
            && state.warnings.len() < 20
        {
            state.warnings.push(warning);
        }
        self.emit_locked_with(&mut state, status, phase, may_be_incomplete);
    }

    fn emit_locked(&self, state: &mut CsvProgressState, _force: bool) {
        self.emit_locked_with(
            state,
            CsvTransferStatus::Running,
            CsvTransferPhase::Transferring,
            false,
        );
    }

    fn emit_locked_with(
        &self,
        state: &mut CsvProgressState,
        status: CsvTransferStatus,
        phase: CsvTransferPhase,
        may_be_incomplete: bool,
    ) {
        let should_emit =
            status != CsvTransferStatus::Running || state.last_emit.elapsed() >= PROGRESS_INTERVAL;
        if !should_emit {
            return;
        }
        state.last_emit = Instant::now();
        self.events.emit(&CsvTransferProgress {
            transfer_id: self.transfer_id.clone(),
            direction: self.direction,
            status,
            phase,
            rows_processed: state.rows_processed,
            rows_written: state.rows_written,
            rows_skipped: state.rows_skipped,
            total_rows: None,
            bytes_processed: state.bytes_processed,
            total_bytes: state.total_bytes,
            warnings: state.warnings.clone(),
            may_be_incomplete,
            emitted_at: now_unix_seconds(),
        });
    }

    fn snapshot(&self) -> (u64, u64, u64, Vec<String>) {
        let state = self.lock();
        (
            state.rows_processed,
            state.rows_written,
            state.rows_skipped,
            state.warnings.clone(),
        )
    }
}

fn now_unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Cancellation registry
// ---------------------------------------------------------------------------

type CancellationEntries = Arc<Mutex<HashMap<String, Arc<RowQueryCancellation>>>>;

/// Tracks cancellation handles for in-flight CSV transfers.
#[derive(Debug, Clone, Default)]
pub struct CsvTransferRegistry {
    entries: CancellationEntries,
}

impl CsvTransferRegistry {
    pub fn begin(&self, request_id: &str) -> Result<CsvTransferGuard, DatabaseError> {
        validate_csv_request_id(request_id)?;
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if entries.contains_key(request_id) {
            return Err(DatabaseError::validation(
                "A CSV transfer with this request id is already running.",
            ));
        }
        let cancellation = Arc::new(RowQueryCancellation::default());
        entries.insert(request_id.to_string(), Arc::clone(&cancellation));
        Ok(CsvTransferGuard {
            request_id: request_id.to_string(),
            cancellation,
            entries: Arc::clone(&self.entries),
        })
    }

    pub fn cancel(&self, request_id: &str) -> bool {
        if validate_csv_request_id(request_id).is_err() {
            return false;
        }
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match entries.get(request_id) {
            Some(cancellation) => {
                cancellation.cancel();
                true
            }
            None => false,
        }
    }

    pub fn active_count(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

/// Removes a cancellation entry when the command finishes.
#[derive(Debug)]
pub struct CsvTransferGuard {
    request_id: String,
    cancellation: Arc<RowQueryCancellation>,
    entries: CancellationEntries,
}

impl CsvTransferGuard {
    pub fn cancellation(&self) -> RowQueryCancellation {
        RowQueryCancellation::clone(&self.cancellation)
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }
}

impl Drop for CsvTransferGuard {
    fn drop(&mut self) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(current) = entries.get(&self.request_id)
            && Arc::ptr_eq(current, &self.cancellation)
        {
            entries.remove(&self.request_id);
        }
    }
}

// ---------------------------------------------------------------------------
// Preview and service
// ---------------------------------------------------------------------------

/// Parses a bounded preview without importing anything. Full field values are
/// returned; only the number of rows is bounded.
pub fn preview_csv_import(
    source_path: &str,
    options: &CsvImportOptions,
    limit: Option<u32>,
) -> Result<CsvImportPreview, DatabaseError> {
    validate_local_path(source_path, "CSV source")?;
    validate_import_options(options)?;
    let limit = limit.unwrap_or(DEFAULT_CSV_PREVIEW_ROWS);
    if !(1..=MAX_CSV_PREVIEW_ROWS).contains(&limit) {
        return Err(DatabaseError::validation(format!(
            "A CSV preview limit must be between 1 and {MAX_CSV_PREVIEW_ROWS} rows."
        )));
    }

    let mut source = CsvImportSourceFile::open(Path::new(source_path), options)?;
    let headers = source.headers().to_vec();
    let mut rows = Vec::new();
    for _ in 0..limit {
        let Some(row) = source.next_row()? else {
            break;
        };
        rows.push(CsvPreviewRow {
            row_number: row.record_number,
            fields: row
                .fields
                .iter()
                .map(|field| CsvPreviewField {
                    value: field.value.clone(),
                    quoted: field.quoted,
                    is_null: options.null_mode.is_null(field),
                })
                .collect(),
        });
    }
    let has_more = source.has_more()?;
    let mut warnings = source.take_issues();
    let warnings_truncated = source.issues_truncated();
    warnings.truncate(MAX_CSV_REPORTED_ISSUES);
    Ok(CsvImportPreview {
        headers,
        rows,
        warnings,
        warnings_truncated,
        sampled_rows: source.rows_read(),
        has_more,
        delimiter: options.delimiter,
        encoding: options.encoding,
        has_header: options.has_header,
    })
}

/// Stateless CSV transfer service over the live connection registry.
pub struct DatabaseCsvService<'a> {
    connections: &'a DatabaseConnectionRegistry,
}

impl<'a> DatabaseCsvService<'a> {
    pub fn new(connections: &'a DatabaseConnectionRegistry) -> Self {
        Self { connections }
    }

    pub async fn export(
        &self,
        profile_id: &str,
        transfer_id: &str,
        request: &CsvExportRequest,
        control: &RowQueryControl,
        events: Arc<dyn CsvTransferEventSink>,
    ) -> Result<CsvExportResult, AppError> {
        validate_export_request(request)?;
        let started = Instant::now();
        let destination = PathBuf::from(&request.destination_path);
        let parent = destination.parent().ok_or_else(|| {
            AppError::validation("The CSV destination must have a parent directory.")
        })?;
        if !parent.is_dir() {
            return Err(AppError::validation(
                "The CSV destination directory does not exist.",
            ));
        }
        if destination.exists() {
            if destination.is_dir() {
                return Err(AppError::validation(
                    "The CSV destination is a directory, not a file.",
                ));
            }
            if !request.overwrite {
                return Err(AppError::conflict(
                    "The CSV destination already exists. Choose another path or allow overwrite.",
                ));
            }
        }

        let temp_path = parent.join(format!(".serverdesk-csv-{}.partial", Uuid::new_v4()));
        let mut open_options = OpenOptions::new();
        open_options.write(true).create_new(true);
        #[cfg(unix)]
        open_options.mode(0o600);
        let file = open_options
            .open(&temp_path)
            .map_err(|_| AppError::internal("The CSV destination could not be created."))?;
        let mut temp = CsvTempFile::new(temp_path.clone());
        let tracker = CsvProgressTracker::new(
            transfer_id.to_string(),
            CsvTransferDirection::Export,
            Arc::clone(&events),
            None,
        );
        tracker.finish(
            CsvTransferStatus::Running,
            CsvTransferPhase::Preparing,
            false,
            None,
        );
        let mut sink = CsvFileExportSink::new(
            file,
            request.delimiter,
            request.encoding,
            request.include_header,
            tracker.clone(),
        )
        .map_err(AppError::from)?;

        let adapter = self.connections.adapter(profile_id).await?;
        let mut adapter = adapter.lock().await;
        let engine = adapter.engine();
        let outcome = adapter
            .export_csv(&request.source, &mut sink, control)
            .await;

        match outcome {
            Ok(summary) => {
                let (bytes_written, rows, columns) = sink.finish().map_err(AppError::from)?;
                tracker.set_counts(rows, rows, 0);
                tracker.set_bytes(bytes_written);
                if std::fs::rename(&temp_path, &destination).is_err() {
                    return Err(AppError::internal(
                        "The completed CSV file could not be moved to its destination. The partial file was removed.",
                    ));
                }
                temp.disarm();
                tracker.finish(
                    CsvTransferStatus::Completed,
                    CsvTransferPhase::Finalizing,
                    false,
                    None,
                );
                Ok(CsvExportResult {
                    status: CsvTransferStatus::Completed,
                    engine,
                    table: match &request.source {
                        CsvExportSource::Table { table } => Some(table.clone()),
                        CsvExportSource::Query { .. } => None,
                    },
                    destination_path: request.destination_path.clone(),
                    rows_exported: rows,
                    columns: if columns.is_empty() {
                        summary.columns
                    } else {
                        columns
                    },
                    bytes_written,
                    encoding: request.encoding,
                    delimiter: request.delimiter,
                    include_header: request.include_header,
                    warnings: Vec::new(),
                    may_be_incomplete: false,
                    values_truncated: false,
                    duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                })
            }
            Err(error) if error.kind == super::errors::DatabaseErrorKind::Cancelled => {
                let (_processed, _written, _skipped, mut warnings) = tracker.snapshot();
                drop(sink);
                drop(temp);
                tracker.finish(
                    CsvTransferStatus::Cancelled,
                    CsvTransferPhase::Finalizing,
                    false,
                    Some(
                        "The partial CSV file was removed; the destination was not changed."
                            .to_string(),
                    ),
                );
                warnings.push(
                    "The partial CSV file was removed; the destination was not changed."
                        .to_string(),
                );
                Ok(CsvExportResult {
                    status: CsvTransferStatus::Cancelled,
                    engine,
                    table: match &request.source {
                        CsvExportSource::Table { table } => Some(table.clone()),
                        CsvExportSource::Query { .. } => None,
                    },
                    destination_path: request.destination_path.clone(),
                    rows_exported: 0,
                    columns: Vec::new(),
                    bytes_written: 0,
                    encoding: request.encoding,
                    delimiter: request.delimiter,
                    include_header: request.include_header,
                    warnings,
                    may_be_incomplete: false,
                    values_truncated: false,
                    duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                })
            }
            Err(error) => {
                drop(sink);
                drop(temp);
                tracker.finish(
                    CsvTransferStatus::Failed,
                    CsvTransferPhase::Finalizing,
                    false,
                    Some(
                        "The partial CSV file was removed; the destination was not changed."
                            .to_string(),
                    ),
                );
                Err(append_error_details(
                    AppError::from(error),
                    "The partial CSV file was removed; the destination was not changed.",
                ))
            }
        }
    }

    pub async fn import(
        &self,
        profile_id: &str,
        transfer_id: &str,
        request: &CsvImportRequest,
        control: &RowQueryControl,
        events: Arc<dyn CsvTransferEventSink>,
    ) -> Result<CsvImportResult, AppError> {
        validate_import_request(request)?;
        let started = Instant::now();
        let path = Path::new(&request.source_path);
        let total_bytes = std::fs::metadata(path).ok().map(|metadata| metadata.len());
        let mut source = CsvImportSourceFile::open(path, &request.options)?;
        validate_mapping_shape(request, source.headers())?;
        let tracker = CsvProgressTracker::new(
            transfer_id.to_string(),
            CsvTransferDirection::Import,
            Arc::clone(&events),
            total_bytes,
        );
        tracker.finish(
            CsvTransferStatus::Running,
            CsvTransferPhase::Preparing,
            false,
            None,
        );
        let mut observer = CsvFileImportObserver {
            tracker: tracker.clone(),
        };
        let adapter = self.connections.adapter(profile_id).await?;
        let mut adapter = adapter.lock().await;
        let engine = adapter.engine();
        let outcome = adapter
            .import_csv(request, &mut source, &mut observer, control)
            .await;

        match outcome {
            Ok(summary) => {
                tracker.set_counts(
                    summary.rows_read,
                    summary.rows_imported,
                    summary.rows_skipped,
                );
                tracker.set_bytes(source.bytes_read());
                tracker.finish(
                    CsvTransferStatus::Completed,
                    CsvTransferPhase::Finalizing,
                    false,
                    None,
                );
                Ok(CsvImportResult {
                    status: CsvTransferStatus::Completed,
                    engine,
                    table: summary.table,
                    rows_read: summary.rows_read,
                    rows_imported: summary.rows_imported,
                    rows_skipped: summary.rows_skipped,
                    warnings: summary.warnings,
                    warnings_truncated: summary.warnings_truncated,
                    transaction: super::row_writes::RowTransactionState::Committed,
                    may_be_incomplete: false,
                    duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                })
            }
            Err(error) if error.kind == super::errors::DatabaseErrorKind::Cancelled => {
                let (_processed, written, skipped, warnings) = tracker.snapshot();
                let rows_read = source.rows_read();
                let rows_skipped = source.skipped_rows().saturating_add(skipped);
                tracker.set_counts(rows_read, written, rows_skipped);
                tracker.finish(
                    CsvTransferStatus::Cancelled,
                    CsvTransferPhase::Finalizing,
                    false,
                    Some(
                        "Cancellation was requested; the transaction was rolled back and no rows were imported."
                            .to_string(),
                    ),
                );
                Ok(CsvImportResult {
                    status: CsvTransferStatus::Cancelled,
                    engine,
                    table: request.table.clone(),
                    rows_read,
                    rows_imported: 0,
                    rows_skipped,
                    warnings: warnings
                        .into_iter()
                        .map(|message| CsvWarning {
                            row_number: 0,
                            column_number: None,
                            column_name: None,
                            message,
                        })
                        .collect(),
                    warnings_truncated: false,
                    transaction: super::row_writes::RowTransactionState::RolledBack,
                    may_be_incomplete: false,
                    duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                })
            }
            Err(error) => {
                let (_processed, written, skipped, _warnings) = tracker.snapshot();
                tracker.set_counts(
                    source.rows_read(),
                    written,
                    source.skipped_rows().saturating_add(skipped),
                );
                tracker.finish(
                    CsvTransferStatus::Failed,
                    CsvTransferPhase::Finalizing,
                    false,
                    Some("No rows were imported.".to_string()),
                );
                Err(append_error_details(
                    AppError::from(error),
                    "No rows were imported.",
                ))
            }
        }
    }
}

fn append_error_details(mut error: AppError, suffix: &str) -> AppError {
    error.details = Some(match error.details {
        Some(details) if !details.trim().is_empty() => format!("{details} {suffix}"),
        _ => suffix.to_string(),
    });
    error
}

fn push_warning(issues: &mut Vec<CsvWarning>, truncated: &mut bool, warning: CsvWarning) {
    if issues.len() < MAX_CSV_REPORTED_ISSUES {
        issues.push(warning);
    } else {
        *truncated = true;
    }
}

fn validate_local_path(path: &str, label: &str) -> Result<(), DatabaseError> {
    if path.trim().is_empty() {
        return Err(DatabaseError::validation(format!(
            "Enter an absolute {label} path."
        )));
    }
    if path.chars().count() > MAX_LOCAL_PATH_CHARACTERS {
        return Err(DatabaseError::validation(format!(
            "The {label} path is too long."
        )));
    }
    if path.contains('\0') {
        return Err(DatabaseError::validation(format!(
            "The {label} path cannot contain NUL characters."
        )));
    }
    if !Path::new(path).is_absolute() {
        return Err(DatabaseError::validation(format!(
            "The {label} path must be absolute."
        )));
    }
    Ok(())
}

fn validate_import_options(options: &CsvImportOptions) -> Result<(), DatabaseError> {
    if let CsvNullMode::Literal(literal) = &options.null_mode {
        if literal.chars().count() > MAX_NULL_LITERAL_CHARACTERS {
            return Err(DatabaseError::validation(format!(
                "A NULL literal cannot exceed {MAX_NULL_LITERAL_CHARACTERS} characters."
            )));
        }
        if literal.contains('\0') {
            return Err(DatabaseError::validation(
                "A NULL literal cannot contain NUL characters.",
            ));
        }
    }
    Ok(())
}

fn validate_export_request(request: &CsvExportRequest) -> Result<(), AppError> {
    validate_local_path(&request.destination_path, "CSV destination").map_err(AppError::from)?;
    if let CsvExportSource::Query { sql } = &request.source {
        super::queries::QueryRequest::try_new(sql.clone(), None).map_err(AppError::from)?;
    }
    Ok(())
}

fn validate_import_request(request: &CsvImportRequest) -> Result<(), AppError> {
    validate_local_path(&request.source_path, "CSV source").map_err(AppError::from)?;
    validate_import_options(&request.options).map_err(AppError::from)?;
    Ok(())
}

fn validate_mapping_shape(
    request: &CsvImportRequest,
    headers: &[String],
) -> Result<(), DatabaseError> {
    if request.mappings.is_empty() {
        return Err(DatabaseError::validation(
            "Map at least one source column to a destination column.",
        ));
    }
    if request.mappings.len() > MAX_CSV_COLUMNS {
        return Err(DatabaseError::validation(format!(
            "An import cannot map more than {MAX_CSV_COLUMNS} columns."
        )));
    }
    let mut source_indices = HashSet::new();
    let mut target_columns = HashSet::new();
    for mapping in &request.mappings {
        let source_index = usize::try_from(mapping.source_index).unwrap_or(usize::MAX);
        if source_index >= headers.len() {
            return Err(DatabaseError::validation(format!(
                "Source column {} does not exist in the CSV header.",
                mapping.source_index
            )));
        }
        if !source_indices.insert(mapping.source_index) {
            return Err(DatabaseError::validation(format!(
                "Source column {} is mapped more than once.",
                mapping.source_index
            )));
        }
        if !target_columns.insert(mapping.target_column.as_str()) {
            return Err(DatabaseError::validation(format!(
                "Destination column {} is mapped more than once.",
                mapping.target_column
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn parse(text: &str, delimiter: CsvDelimiter) -> Vec<CsvRecord> {
        let mut parser = CsvParser::new(Cursor::new(text.as_bytes()), delimiter, CsvEncoding::Utf8);
        let mut records = Vec::new();
        while let Some(record) = parser.next_record().expect("record") {
            records.push(record);
        }
        records
    }

    #[test]
    fn quoted_fields_preserve_delimiters_quotes_and_newlines() {
        let records = parse(
            "name,note,empty\r\n\"Smith, Ann\",\"line one\nline two\",\"\"\r\n",
            CsvDelimiter::Comma,
        );
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].fields[0].value, "Smith, Ann");
        assert_eq!(records[1].fields[1].value, "line one\nline two");
        assert!(records[1].fields[2].quoted);
        assert!(records[1].fields[2].value.is_empty());
    }

    #[test]
    fn quoted_quotes_are_unescaped() {
        let records = parse("\"he said \"\"hi\"\"\"\n", CsvDelimiter::Comma);
        assert_eq!(records[0].fields[0].value, "he said \"hi\"");
    }

    #[test]
    fn unicode_survives_utf8_and_utf16() {
        let text = "name\nZoë 🚀\n";
        assert_eq!(
            parse(text, CsvDelimiter::Comma)[1].fields[0].value,
            "Zoë 🚀"
        );

        let mut bytes = vec![0xFF, 0xFE];
        for unit in "name\nZoë 🚀\n".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let mut parser = CsvParser::new(
            Cursor::new(bytes),
            CsvDelimiter::Comma,
            CsvEncoding::Utf16Le,
        );
        let mut records = Vec::new();
        while let Some(record) = parser.next_record().expect("record") {
            records.push(record);
        }
        assert_eq!(records[1].fields[0].value, "Zoë 🚀");
    }

    #[test]
    fn malformed_input_reports_row_and_column() {
        let mut parser = CsvParser::new(
            Cursor::new(b"\"unterminated\n".to_vec()),
            CsvDelimiter::Comma,
            CsvEncoding::Utf8,
        );
        let error = parser.next_record().expect_err("unterminated quote");
        assert!(error.message.contains("row 1"));
        assert!(error.message.contains("column 1"));
    }

    #[test]
    fn unquoted_quote_is_malformed_with_context() {
        let mut parser = CsvParser::new(
            Cursor::new(b"a,b\"c\n".to_vec()),
            CsvDelimiter::Comma,
            CsvEncoding::Utf8,
        );
        let error = parser.next_record().expect_err("stray quote");
        assert!(error.message.contains("row 1"));
        assert!(error.message.contains("column 2"));
    }

    #[test]
    fn writer_and_reader_round_trip_null_and_empty_string() {
        let mut bytes = Vec::new();
        {
            let mut writer =
                CsvWriter::new(&mut bytes, CsvDelimiter::Comma, CsvEncoding::Utf8).expect("writer");
            writer
                .write_header(&["a".to_string(), "b".to_string()])
                .expect("header");
            writer
                .write_record(&[None, Some("")])
                .expect("null and empty");
            writer
                .write_record(&[Some("x,y"), Some("line\nbreak")])
                .expect("quoted");
            writer.flush().expect("flush");
        }
        let mut parser = CsvParser::new(Cursor::new(bytes), CsvDelimiter::Comma, CsvEncoding::Utf8);
        let header = parser.next_record().expect("header").expect("record");
        assert_eq!(header.fields[0].value, "a");
        let row = parser.next_record().expect("row").expect("record");
        assert!(!row.fields[0].quoted);
        assert!(row.fields[1].quoted);
        assert!(row.fields[1].value.is_empty());
        let quoted = parser.next_record().expect("row").expect("record");
        assert_eq!(quoted.fields[0].value, "x,y");
        assert_eq!(quoted.fields[1].value, "line\nbreak");
    }

    #[test]
    fn latin1_rejects_unrepresentable_characters() {
        let mut bytes = Vec::new();
        let mut writer =
            CsvWriter::new(&mut bytes, CsvDelimiter::Comma, CsvEncoding::Latin1).expect("writer");
        assert!(writer.write_record(&[Some("café")]).is_ok());
        assert!(writer.write_record(&[Some("🚀")]).is_err());
    }

    #[test]
    fn transfer_ids_are_bounded_ascii() {
        assert!(validate_csv_request_id("export-1").is_ok());
        assert!(validate_csv_request_id("bad id").is_err());
        assert!(validate_csv_request_id("").is_err());
    }
}
