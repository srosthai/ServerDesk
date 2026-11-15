//! Deterministic CSV codec, preview, and round-trip tests.
//!
//! These tests use the in-memory adapter for both engines and local temporary
//! files. They cover quoted fields, Unicode, NULL versus empty string,
//! malformed row context, encodings, large streaming files, full-value
//! round-trips, and partial-file cleanup without a live database.

use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Cursor, Write},
    path::Path,
    time::Duration,
};

use serverdesk_lib::{
    database::{
        ColumnIdentity, CsvColumnMapping, CsvDelimiter, CsvEncoding, CsvExportSink,
        CsvExportSource, CsvImportObserver, CsvImportOptions, CsvImportRequest, CsvImportSource,
        CsvImportSourceFile, CsvNullMode, CsvTempFile, CsvWarning, CsvWriter, DatabaseAdapter,
        DatabaseConnectRequest, DatabasePassword, FakeDatabaseAdapter, FakeRowTable, MetadataValue,
        RowColumn, RowIdentity, RowQueryCancellation, RowQueryControl, TableRef, csv::CsvParser,
    },
    storage::DatabaseEngine,
};

fn identifier(value: &str) -> serverdesk_lib::database::Identifier {
    serverdesk_lib::database::Identifier::new(value).expect("identifier")
}

fn column(name: &str, data_type: &str, nullable: bool, primary: bool) -> RowColumn {
    RowColumn::new(
        identifier(name),
        data_type.to_string(),
        nullable,
        1,
        primary,
        ColumnIdentity::None,
        false,
    )
}

fn table(engine: DatabaseEngine) -> TableRef {
    match engine {
        DatabaseEngine::Mysql => TableRef::new(Some("app"), Some("app"), "users").expect("table"),
        DatabaseEngine::Postgresql => {
            TableRef::new(Some("app"), Some("public"), "users").expect("table")
        }
    }
}

fn fixture(engine: DatabaseEngine, rows: Vec<Vec<Option<String>>>) -> FakeRowTable {
    FakeRowTable::new(
        table(engine),
        vec![
            column("id", "bigint", false, true),
            column("name", "text", false, false),
            column("note", "text", true, false),
            column("empty_value", "text", true, false),
            column("long_value", "text", true, false),
        ],
        RowIdentity::primary_key(vec![identifier("id")]),
        MetadataValue::available(rows.len() as u64),
        rows,
    )
}

fn options() -> CsvImportOptions {
    CsvImportOptions {
        delimiter: CsvDelimiter::Comma,
        encoding: CsvEncoding::Utf8,
        has_header: true,
        null_mode: CsvNullMode::UnquotedEmpty,
    }
}

fn mappings() -> Vec<CsvColumnMapping> {
    [
        (0, "id"),
        (1, "name"),
        (2, "note"),
        (3, "empty_value"),
        (4, "long_value"),
    ]
    .into_iter()
    .map(|(source_index, target)| CsvColumnMapping {
        source_index,
        target_column: identifier(target),
    })
    .collect()
}

async fn connected(engine: DatabaseEngine, rows: Vec<Vec<Option<String>>>) -> FakeDatabaseAdapter {
    let mut adapter =
        FakeDatabaseAdapter::new(engine, Some("16.2")).with_row_table(fixture(engine, rows));
    adapter
        .connect(DatabaseConnectRequest {
            host: "127.0.0.1".to_string(),
            port: 5432,
            socket: None,
            database: Some("app".to_string()),
            username: "app".to_string(),
            password: DatabasePassword::new("fixture-password"),
            read_only: false,
            ssl_mode: None,
            connect_timeout: Duration::from_secs(5),
        })
        .await
        .expect("connect");
    adapter
}

struct TestSink {
    writer: CsvWriter<BufWriter<File>>,
    columns: Vec<String>,
    rows: u64,
}

impl TestSink {
    fn new(path: &Path, encoding: CsvEncoding) -> Self {
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .expect("create export");
        Self {
            writer: CsvWriter::new(BufWriter::new(file), CsvDelimiter::Comma, encoding)
                .expect("writer"),
            columns: Vec::new(),
            rows: 0,
        }
    }
}

impl CsvExportSink for TestSink {
    fn write_header(
        &mut self,
        columns: &[String],
    ) -> Result<(), serverdesk_lib::database::DatabaseError> {
        self.columns = columns.to_vec();
        self.writer.write_header(columns)
    }

    fn write_row(
        &mut self,
        values: &[Option<String>],
    ) -> Result<(), serverdesk_lib::database::DatabaseError> {
        let borrowed = values.iter().map(Option::as_deref).collect::<Vec<_>>();
        self.writer.write_record(&borrowed)?;
        self.rows = self.rows.saturating_add(1);
        Ok(())
    }
}

struct TestObserver;

impl CsvImportObserver for TestObserver {
    fn row_imported(&mut self, _row_number: u64) {}

    fn row_skipped(&mut self, _warning: &CsvWarning) {}
}

#[test]
fn quoted_fields_unicode_and_malformed_input_keep_context() {
    let mut parser = CsvParser::new(
        Cursor::new("name,note\n\"Smith, Ann\",\"line one\nline two\"\nZoë 🚀,\"\"\n".as_bytes()),
        CsvDelimiter::Comma,
        CsvEncoding::Utf8,
    );
    let header = parser.next_record().expect("header").expect("record");
    assert_eq!(header.fields.len(), 2);
    let first = parser.next_record().expect("first").expect("record");
    assert_eq!(first.fields[0].value, "Smith, Ann");
    assert_eq!(first.fields[1].value, "line one\nline two");
    let second = parser.next_record().expect("second").expect("record");
    assert_eq!(second.fields[0].value, "Zoë 🚀");
    assert!(second.fields[1].quoted);
    assert!(second.fields[1].value.is_empty());

    let mut malformed = CsvParser::new(
        Cursor::new("name,note\nAnn,\"unterminated\n".as_bytes()),
        CsvDelimiter::Comma,
        CsvEncoding::Utf8,
    );
    malformed.next_record().expect("header");
    let error = malformed.next_record().expect_err("unterminated quote");
    assert!(error.message.contains("row 2"));
    assert!(error.message.contains("column 2"));

    let mut stray = CsvParser::new(
        Cursor::new("name,note\nAnn,no\"te\n".as_bytes()),
        CsvDelimiter::Comma,
        CsvEncoding::Utf8,
    );
    stray.next_record().expect("header");
    let error = stray.next_record().expect_err("stray quote");
    assert!(error.message.contains("row 2"));
    assert!(error.message.contains("column 2"));
}

#[test]
fn preview_distinguishes_null_from_empty_and_reports_ragged_rows() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("import.csv");
    std::fs::write(&path, "name,note\nunquoted,\nquoted,\"\"\nmissing-field\n").expect("fixture");
    let preview = serverdesk_lib::database::preview_csv_import(
        path.to_str().expect("path"),
        &options(),
        Some(10),
    )
    .expect("preview");
    assert_eq!(preview.headers, ["name", "note"]);
    assert_eq!(preview.rows.len(), 2);
    assert!(preview.rows[0].fields[1].is_null);
    assert!(!preview.rows[1].fields[1].is_null);
    assert!(preview.rows[1].fields[1].quoted);
    assert_eq!(preview.warnings.len(), 1);
    assert_eq!(preview.warnings[0].row_number, 4);
    assert_eq!(preview.warnings[0].column_number, Some(3));
    assert!(preview.warnings[0].message.contains("header defines 2"));
    assert_eq!(preview.sampled_rows, 3);
}

#[test]
fn utf16_and_latin1_files_are_decoded_without_loss() {
    let directory = tempfile::tempdir().expect("temp dir");
    for (encoding, text) in [
        (CsvEncoding::Utf16Le, "name\nZoë 🚀\n"),
        (CsvEncoding::Utf16Be, "name\nZoë 🚀\n"),
        (CsvEncoding::Latin1, "name\ncafé\n"),
    ] {
        let path = directory.path().join(format!("{encoding:?}.csv"));
        {
            let file = File::create(&path).expect("create");
            let mut writer = CsvWriter::new(BufWriter::new(file), CsvDelimiter::Comma, encoding)
                .expect("writer");
            writer.write_header(&["name".to_string()]).expect("header");
            writer
                .write_record(&[Some(text.lines().nth(1).expect("value"))])
                .expect("row");
            writer.flush().expect("flush");
        }
        let mut source = CsvImportSourceFile::open(
            &path,
            &CsvImportOptions {
                encoding,
                ..options()
            },
        )
        .expect("source");
        let row = source.next_row().expect("row").expect("record");
        assert_eq!(row.fields[0].value, text.lines().nth(1).expect("value"));
    }
}

#[test]
fn large_files_stream_without_collecting_every_row() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("large.csv");
    {
        let file = File::create(&path).expect("create");
        let mut writer =
            CsvWriter::new(BufWriter::new(file), CsvDelimiter::Comma, CsvEncoding::Utf8)
                .expect("writer");
        writer
            .write_header(&["id".to_string(), "name".to_string()])
            .expect("header");
        for index in 0..20_000u32 {
            writer
                .write_record(&[Some(&index.to_string()), Some(&format!("name-{index}"))])
                .expect("row");
        }
        writer.flush().expect("flush");
    }
    let mut source = CsvImportSourceFile::open(&path, &options()).expect("source");
    let mut count = 0u64;
    let mut last = None;
    while let Some(row) = source.next_row().expect("row") {
        count += 1;
        last = Some(row.fields[1].value.clone());
    }
    assert_eq!(count, 20_000);
    assert_eq!(last.as_deref(), Some("name-19999"));
    assert_eq!(source.rows_read(), 20_000);
}

#[tokio::test]
async fn round_trip_preserves_full_values_on_both_engines() {
    let directory = tempfile::tempdir().expect("temp dir");
    let long_value = "x".repeat(5_000);
    let original = vec![
        vec![
            Some("1".to_string()),
            Some("Smith, Ann \"A\"".to_string()),
            Some("line one\nline two 🚀".to_string()),
            Some(String::new()),
            Some(long_value.clone()),
        ],
        vec![
            Some("2".to_string()),
            Some("Zoë".to_string()),
            None,
            Some(String::new()),
            None,
        ],
    ];

    for engine in [DatabaseEngine::Mysql, DatabaseEngine::Postgresql] {
        let path = directory.path().join(format!("{engine:?}.csv"));
        let mut source_adapter = connected(engine, original.clone()).await;
        let mut sink = TestSink::new(&path, CsvEncoding::Utf8);
        let exported = source_adapter
            .export_csv(
                &CsvExportSource::Table {
                    table: table(engine),
                },
                &mut sink,
                &RowQueryControl::default(),
            )
            .await
            .expect("export");
        assert_eq!(exported.rows, 2);
        assert_eq!(exported.columns.len(), 5);
        sink.writer.flush().expect("flush");

        let mut target_adapter = connected(engine, Vec::new()).await;
        let mut import_source = CsvImportSourceFile::open(&path, &options()).expect("source");
        let summary = target_adapter
            .import_csv(
                &CsvImportRequest {
                    table: table(engine),
                    source_path: path.to_string_lossy().to_string(),
                    options: options(),
                    mappings: mappings(),
                },
                &mut import_source,
                &mut TestObserver,
                &RowQueryControl::default(),
            )
            .await
            .expect("import");
        assert_eq!(summary.rows_imported, 2);
        assert_eq!(summary.rows_skipped, 0);
        assert_eq!(
            target_adapter.row_table_rows(&table(engine)).expect("rows"),
            original.as_slice()
        );
    }
}

#[tokio::test]
async fn query_export_streams_rows_and_rejects_writes() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("query.csv");
    let mut adapter = connected(DatabaseEngine::Postgresql, Vec::new()).await;
    let mut sink = TestSink::new(&path, CsvEncoding::Utf8);
    let summary = adapter
        .export_csv(
            &CsvExportSource::Query {
                sql: "SELECT many".to_string(),
            },
            &mut sink,
            &RowQueryControl::default(),
        )
        .await
        .expect("query export");
    assert_eq!(summary.columns, ["value"]);
    assert_eq!(summary.rows, 6);
    sink.writer.flush().expect("flush");

    let error = adapter
        .export_csv(
            &CsvExportSource::Query {
                sql: "DELETE FROM users".to_string(),
            },
            &mut sink,
            &RowQueryControl::default(),
        )
        .await
        .expect_err("write rejected");
    assert_eq!(
        error.kind,
        serverdesk_lib::database::DatabaseErrorKind::Validation
    );
}

#[tokio::test]
async fn cancelled_or_malformed_imports_leave_the_fixture_unchanged() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("import.csv");
    std::fs::write(&path, "id,name\n1,Ann\n2,Bob\n").expect("fixture");
    let original = vec![vec![Some("9".to_string()), Some("existing".to_string())]];
    let mut adapter = connected(DatabaseEngine::Mysql, original.clone()).await;
    let cancellation = RowQueryCancellation::default();
    cancellation.cancel();
    let mut source = CsvImportSourceFile::open(&path, &options()).expect("source");
    let error = adapter
        .import_csv(
            &CsvImportRequest {
                table: table(DatabaseEngine::Mysql),
                source_path: path.to_string_lossy().to_string(),
                options: options(),
                mappings: vec![
                    CsvColumnMapping {
                        source_index: 0,
                        target_column: identifier("id"),
                    },
                    CsvColumnMapping {
                        source_index: 1,
                        target_column: identifier("name"),
                    },
                ],
            },
            &mut source,
            &mut TestObserver,
            &RowQueryControl {
                timeout: Duration::from_secs(5),
                cancellation: Some(cancellation),
            },
        )
        .await
        .expect_err("cancelled import");
    assert_eq!(
        error.kind,
        serverdesk_lib::database::DatabaseErrorKind::Cancelled
    );
    assert_eq!(
        adapter
            .row_table_rows(&table(DatabaseEngine::Mysql))
            .expect("rows"),
        original.as_slice()
    );

    let malformed_path = directory.path().join("malformed.csv");
    std::fs::write(&malformed_path, "id,name\n1,\"unterminated\n").expect("fixture");
    let mut source = CsvImportSourceFile::open(&malformed_path, &options()).expect("source");
    let error = adapter
        .import_csv(
            &CsvImportRequest {
                table: table(DatabaseEngine::Mysql),
                source_path: malformed_path.to_string_lossy().to_string(),
                options: options(),
                mappings: vec![
                    CsvColumnMapping {
                        source_index: 0,
                        target_column: identifier("id"),
                    },
                    CsvColumnMapping {
                        source_index: 1,
                        target_column: identifier("name"),
                    },
                ],
            },
            &mut source,
            &mut TestObserver,
            &RowQueryControl::default(),
        )
        .await
        .expect_err("malformed import");
    assert!(error.message.contains("row 2"));
    assert_eq!(
        adapter
            .row_table_rows(&table(DatabaseEngine::Mysql))
            .expect("rows"),
        original.as_slice()
    );
}

#[test]
fn one_column_null_and_empty_string_round_trip() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("one-column.csv");
    {
        let file = File::create(&path).expect("create");
        let mut writer =
            CsvWriter::new(BufWriter::new(file), CsvDelimiter::Comma, CsvEncoding::Utf8)
                .expect("writer");
        writer.write_header(&["value".to_string()]).expect("header");
        writer.write_record(&[None]).expect("null");
        writer.write_record(&[Some("")]).expect("empty");
        writer.write_record(&[Some("text")]).expect("text");
        writer.flush().expect("flush");
    }
    let mut source = CsvImportSourceFile::open(&path, &options()).expect("source");
    let null_row = source.next_row().expect("row").expect("null");
    assert!(null_row.fields[0].value.is_empty());
    assert!(!null_row.fields[0].quoted);
    let empty_row = source.next_row().expect("row").expect("empty");
    assert!(empty_row.fields[0].quoted);
    let text_row = source.next_row().expect("row").expect("text");
    assert_eq!(text_row.fields[0].value, "text");
    assert!(source.next_row().expect("eof").is_none());
    assert_eq!(source.rows_read(), 3);
}

#[test]
fn partial_files_are_removed_and_disarmed_only_after_success() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("partial.csv");
    {
        let mut file = File::create(&path).expect("create");
        file.write_all(b"partial").expect("write");
    }
    {
        let guard = CsvTempFile::new(path.clone());
        assert!(guard.path().exists());
    }
    assert!(!path.exists());

    let path = directory.path().join("complete.csv");
    File::create(&path).expect("create");
    let mut guard = CsvTempFile::new(path.clone());
    guard.disarm();
    drop(guard);
    assert!(path.exists());
}

#[test]
fn a_value_longer_than_the_row_browser_bound_is_exported_in_full() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("long.csv");
    let long_value = "y".repeat(5_000);
    let mut bytes = Vec::new();
    {
        let mut writer =
            CsvWriter::new(&mut bytes, CsvDelimiter::Comma, CsvEncoding::Utf8).expect("writer");
        writer
            .write_record(&[Some(long_value.as_str())])
            .expect("row");
        writer.flush().expect("flush");
    }
    std::fs::write(&path, bytes).expect("write");
    let mut parser = CsvParser::new(
        File::open(&path).expect("open"),
        CsvDelimiter::Comma,
        CsvEncoding::Utf8,
    );
    let record = parser.next_record().expect("record").expect("row");
    assert_eq!(record.fields[0].value.len(), 5_000);
}
