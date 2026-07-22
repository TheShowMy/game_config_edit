use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use game_config_edit::csv_document::CsvDocument;
use game_config_edit::diagnostics::analyze_table;
use game_config_edit::workspace::scan_workspace;

const LARGE_FILE_LIMIT: usize = 100 * 1024 * 1024;
const LARGE_RECORD_LIMIT: usize = 500_000;
const WORKSPACE_FILE_COUNT: usize = 2_000;
const WORKSPACE_FILE_SIZE: u64 = 1024 * 1024;

fn benchmark_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("p6-benchmark")
}

fn create_large_csv(path: &Path) {
    if fs::metadata(path).is_ok_and(|metadata| metadata.len() >= LARGE_FILE_LIMIT as u64) {
        return;
    }
    let mut bytes = Vec::with_capacity(LARGE_FILE_LIMIT + 256);
    bytes.extend_from_slice(b"Identifier,Display name,Metadata,Notes\nid,name,meta,notes\n");
    let payload = "x".repeat(160);
    for row in 1..=LARGE_RECORD_LIMIT {
        writeln!(
            bytes,
            "{row},Hero {row},\"{{\"\"hp\"\":{},\"\"enabled\"\":true}}\",{payload}",
            row % 10_000
        )
        .unwrap();
        if bytes.len() >= LARGE_FILE_LIMIT {
            break;
        }
    }
    fs::write(path, bytes).unwrap();
}

#[test]
#[ignore = "P6 Release benchmark; generates a 100 MiB CSV"]
fn p6_large_file_parse_analysis_and_edit() {
    let root = benchmark_root();
    fs::create_dir_all(&root).unwrap();
    let path = root.join("large.csv");
    create_large_csv(&path);

    let started = Instant::now();
    let bytes = fs::read(&path).unwrap();
    let read_elapsed = started.elapsed();
    let started = Instant::now();
    let mut document = CsvDocument::from_bytes(&path, &bytes, Some(b',')).unwrap();
    let parse_elapsed = started.elapsed();
    let started = Instant::now();
    let analyses = analyze_table(document.records.as_ref(), 2);
    let analysis_elapsed = started.elapsed();
    let started = Instant::now();
    document.replace_cell(2, 1, "Edited hero").unwrap();
    let edit_elapsed = started.elapsed();

    println!(
        "P6 large file: bytes={}, records={}, read_ms={}, parse_ms={}, analysis_ms={}, edit_ms={}, columns={}",
        bytes.len(),
        document.records.len().saturating_sub(2),
        read_elapsed.as_millis(),
        parse_elapsed.as_millis(),
        analysis_elapsed.as_millis(),
        edit_elapsed.as_millis(),
        analyses.len(),
    );
    assert!(bytes.len() >= LARGE_FILE_LIMIT);
    assert!(document.records.len().saturating_sub(2) <= LARGE_RECORD_LIMIT);
    assert!(parse_elapsed < Duration::from_secs(3));
    assert!(edit_elapsed < Duration::from_millis(100));
}

#[test]
#[ignore = "P6 Release benchmark; creates 2,000 sparse CSV files"]
fn p6_workspace_scan_and_filter() {
    let root = benchmark_root().join("workspace-2000");
    fs::create_dir_all(&root).unwrap();
    for index in 0..WORKSPACE_FILE_COUNT {
        let path = root.join(format!("config-{index:04}.csv"));
        if !path.exists() {
            let mut file = File::create(path).unwrap();
            file.write_all(b"Description\nid\n").unwrap();
            file.set_len(WORKSPACE_FILE_SIZE).unwrap();
        }
    }

    let started = Instant::now();
    let snapshot = scan_workspace(&root);
    let scan_elapsed = started.elapsed();
    let started = Instant::now();
    let filtered = snapshot
        .files
        .iter()
        .filter(|entry| {
            entry
                .relative_path
                .to_string_lossy()
                .to_lowercase()
                .contains("1999")
        })
        .count();
    let filter_elapsed = started.elapsed();
    let logical_bytes = snapshot
        .files
        .iter()
        .map(|entry| fs::metadata(&entry.absolute_path).unwrap().len())
        .sum::<u64>();

    println!(
        "P6 workspace: files={}, logical_bytes={}, scan_ms={}, filter_us={}",
        snapshot.files.len(),
        logical_bytes,
        scan_elapsed.as_millis(),
        filter_elapsed.as_micros(),
    );
    assert_eq!(snapshot.files.len(), WORKSPACE_FILE_COUNT);
    assert_eq!(
        logical_bytes,
        WORKSPACE_FILE_COUNT as u64 * WORKSPACE_FILE_SIZE
    );
    assert_eq!(filtered, 1);
    assert!(scan_elapsed < Duration::from_secs(3));
    assert!(filter_elapsed < Duration::from_millis(100));
}
