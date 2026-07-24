use std::fs;
use std::path::{Path, PathBuf};

use game_config_edit::csv_document::CsvDocument;
use game_config_edit::diagnostics::{CellProblemKind, TableKind, analyze_table, table_kind};

#[test]
#[ignore = "requires GCONF_CSV_FIXTURE_ROOT with the external cehua dataset"]
fn cehua_dataset_parses_and_recognizes_misc_tables() {
    let root = PathBuf::from(
        std::env::var_os("GCONF_CSV_FIXTURE_ROOT")
            .expect("GCONF_CSV_FIXTURE_ROOT must point to the cehua dataset"),
    );
    let mut files = Vec::new();
    collect_csv_files(&root, &mut files);
    files.sort();

    let mut parse_errors = Vec::new();
    let mut misc_count = 0;
    let mut invalid_declarations = 0;
    let mut duplicate_keys = 0;
    let mut type_mismatches = 0;
    let mut semantic_diagnostics = Vec::new();
    for path in &files {
        let document = match CsvDocument::open(path, None) {
            Ok(document) => document,
            Err(error) => {
                parse_errors.push(format!("{}: {error}", path.display()));
                continue;
            }
        };
        if table_kind(&document.records) != TableKind::Misc {
            continue;
        }
        misc_count += 1;
        for (column_index, analysis) in analyze_table(&document.records, 2).into_iter().enumerate()
        {
            for problem in analysis.problems {
                for kind in problem.kinds {
                    match kind {
                        CellProblemKind::InvalidTypeDeclaration => invalid_declarations += 1,
                        CellProblemKind::DuplicateKey => duplicate_keys += 1,
                        CellProblemKind::DeclaredTypeMismatch => type_mismatches += 1,
                        _ => {}
                    }
                    if matches!(
                        kind,
                        CellProblemKind::InvalidTypeDeclaration
                            | CellProblemKind::DuplicateKey
                            | CellProblemKind::DeclaredTypeMismatch
                    ) {
                        semantic_diagnostics.push(format!(
                            "{}:{}:{} {kind:?} {:?}",
                            path.strip_prefix(&root).unwrap_or(path).display(),
                            problem.row_index + 1,
                            column_index + 1,
                            problem.detail
                        ));
                    }
                }
            }
        }
    }

    assert_eq!(files.len(), 574);
    assert!(parse_errors.is_empty(), "{}", parse_errors.join("\n"));
    assert_eq!(misc_count, 36);
    println!(
        "misc diagnostics: invalid declarations={invalid_declarations}, duplicate keys={duplicate_keys}, type mismatches={type_mismatches}"
    );
    for diagnostic in semantic_diagnostics {
        println!("{diagnostic}");
    }
}

fn collect_csv_files(root: &Path, output: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).expect("fixture directory must be readable") {
        let path = entry.expect("fixture entry must be readable").path();
        if path.is_dir() {
            collect_csv_files(&path, output);
        } else if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("csv"))
        {
            output.push(path);
        }
    }
}
