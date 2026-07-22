use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::csv_document::CsvDocument;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CsvFileEntry {
    pub absolute_path: PathBuf,
    pub relative_path: PathBuf,
    pub file_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanWarning {
    pub path: Option<PathBuf>,
    pub message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkspaceSnapshot {
    pub files: Vec<CsvFileEntry>,
    pub warnings: Vec<ScanWarning>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CsvFileStats {
    Ready { data_rows: usize, columns: usize },
    Error { message: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceTreeRow {
    Directory {
        relative_path: PathBuf,
        name: String,
        depth: usize,
        expanded: bool,
    },
    File {
        entry: CsvFileEntry,
        depth: usize,
    },
}

#[derive(Debug, Default)]
struct TreeDirectory {
    directories: Vec<(OsString, TreeDirectory)>,
    files: Vec<CsvFileEntry>,
}

pub fn scan_workspace(root: &Path) -> WorkspaceSnapshot {
    let mut snapshot = WorkspaceSnapshot::default();

    for entry in WalkDir::new(root).follow_links(false) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                snapshot.warnings.push(ScanWarning {
                    path: error.path().map(Path::to_path_buf),
                    message: error.to_string(),
                });
                continue;
            }
        };

        if !entry.file_type().is_file() || !is_csv(entry.path()) {
            continue;
        }

        let relative_path = entry
            .path()
            .strip_prefix(root)
            .unwrap_or(entry.path())
            .to_path_buf();
        snapshot.files.push(CsvFileEntry {
            absolute_path: entry.path().to_path_buf(),
            file_name: entry.file_name().to_string_lossy().into_owned(),
            relative_path,
        });
    }

    snapshot.files.sort_by(|left, right| {
        left.relative_path
            .to_string_lossy()
            .to_lowercase()
            .cmp(&right.relative_path.to_string_lossy().to_lowercase())
    });
    snapshot
}

pub fn inspect_csv_file(
    path: &Path,
    header_rows: usize,
    delimiter_override: Option<u8>,
) -> CsvFileStats {
    match CsvDocument::open(path, delimiter_override) {
        Ok(document) => match document.dimensions(header_rows) {
            Some((data_rows, columns)) => CsvFileStats::Ready { data_rows, columns },
            None => CsvFileStats::Error {
                message: format!("file has fewer than the configured {header_rows} header records"),
            },
        },
        Err(error) => CsvFileStats::Error {
            message: error.to_string(),
        },
    }
}

pub fn visible_tree_rows(
    files: &[CsvFileEntry],
    expanded_directories: &HashSet<PathBuf>,
    expand_all: bool,
) -> Vec<WorkspaceTreeRow> {
    let mut root = TreeDirectory::default();
    for file in files {
        let mut directory = &mut root;
        if let Some(parent) = file.relative_path.parent() {
            for component in parent.components() {
                let name = component.as_os_str().to_os_string();
                let index = directory
                    .directories
                    .iter()
                    .position(|(existing, _)| existing == &name)
                    .unwrap_or_else(|| {
                        directory
                            .directories
                            .push((name.clone(), TreeDirectory::default()));
                        directory.directories.len() - 1
                    });
                directory = &mut directory.directories[index].1;
            }
        }
        directory.files.push(file.clone());
    }

    sort_tree(&mut root);
    let mut rows = Vec::new();
    flatten_tree(
        &root,
        Path::new(""),
        0,
        expanded_directories,
        expand_all,
        &mut rows,
    );
    rows
}

fn sort_tree(directory: &mut TreeDirectory) {
    directory.directories.sort_by(|(left, _), (right, _)| {
        left.to_string_lossy()
            .to_lowercase()
            .cmp(&right.to_string_lossy().to_lowercase())
            .then_with(|| left.cmp(right))
    });
    directory.files.sort_by(|left, right| {
        left.file_name
            .to_lowercase()
            .cmp(&right.file_name.to_lowercase())
            .then_with(|| left.file_name.cmp(&right.file_name))
    });
    for (_, child) in &mut directory.directories {
        sort_tree(child);
    }
}

fn flatten_tree(
    directory: &TreeDirectory,
    parent_path: &Path,
    depth: usize,
    expanded_directories: &HashSet<PathBuf>,
    expand_all: bool,
    rows: &mut Vec<WorkspaceTreeRow>,
) {
    for (name, child) in &directory.directories {
        let relative_path = parent_path.join(name);
        let expanded = expand_all || expanded_directories.contains(&relative_path);
        rows.push(WorkspaceTreeRow::Directory {
            relative_path: relative_path.clone(),
            name: name.to_string_lossy().into_owned(),
            depth,
            expanded,
        });
        if expanded {
            flatten_tree(
                child,
                &relative_path,
                depth + 1,
                expanded_directories,
                expand_all,
                rows,
            );
        }
    }
    rows.extend(
        directory
            .files
            .iter()
            .cloned()
            .map(|entry| WorkspaceTreeRow::File { entry, depth }),
    );
}

fn is_csv(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("csv"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn scans_csv_files_recursively_and_case_insensitively() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir_all(directory.path().join("nested")).unwrap();
        fs::write(directory.path().join("z.csv"), "id\n1\n").unwrap();
        fs::write(directory.path().join("nested/a.CSV"), "id\n2\n").unwrap();
        fs::write(directory.path().join("ignored.txt"), "text").unwrap();

        let snapshot = scan_workspace(directory.path());

        let relative_paths = snapshot
            .files
            .iter()
            .map(|file| file.relative_path.to_string_lossy().replace('\\', "/"))
            .collect::<Vec<_>>();
        assert_eq!(relative_paths, vec!["nested/a.CSV", "z.csv"]);
        assert!(snapshot.warnings.is_empty());
    }

    #[test]
    fn does_not_follow_directory_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("outside.csv"), "id\n1\n").unwrap();

        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), directory.path().join("linked")).unwrap();
        #[cfg(windows)]
        if std::os::windows::fs::symlink_dir(outside.path(), directory.path().join("linked"))
            .is_err()
        {
            return;
        }

        let snapshot = scan_workspace(directory.path());

        assert!(snapshot.files.is_empty());
    }

    #[test]
    fn inspects_data_rows_and_columns_after_headers() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("heroes.csv");
        fs::write(
            &path,
            "Identifier,Display name\nid,name\n1,Arthur\n2,Merlin\n",
        )
        .unwrap();

        assert_eq!(
            inspect_csv_file(&path, 2, None),
            CsvFileStats::Ready {
                data_rows: 2,
                columns: 2,
            }
        );
    }

    #[test]
    fn reports_parse_errors_instead_of_partial_dimensions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("broken.csv");
        fs::write(&path, "id,name\n1,Arthur,extra\n").unwrap();

        let result = inspect_csv_file(&path, 1, Some(b','));

        assert!(matches!(result, CsvFileStats::Error { .. }));
    }

    #[test]
    fn tree_rows_only_include_children_of_expanded_directories() {
        let files = vec![
            file_entry("root.csv"),
            file_entry("configs/hero/heroes.csv"),
            file_entry("configs/items.csv"),
        ];

        let collapsed = visible_tree_rows(&files, &HashSet::new(), false);
        assert_eq!(tree_labels(&collapsed), vec!["D:configs", "F:root.csv"]);

        let expanded = HashSet::from([PathBuf::from("configs")]);
        let rows = visible_tree_rows(&files, &expanded, false);
        assert_eq!(
            tree_labels(&rows),
            vec!["D:configs", "D:hero", "F:items.csv", "F:root.csv"]
        );
    }

    #[test]
    fn search_tree_can_expand_all_ancestors() {
        let files = vec![file_entry("configs/hero/heroes.csv")];

        let rows = visible_tree_rows(&files, &HashSet::new(), true);

        assert_eq!(
            tree_labels(&rows),
            vec!["D:configs", "D:hero", "F:heroes.csv"]
        );
    }

    fn file_entry(relative: &str) -> CsvFileEntry {
        let relative_path = PathBuf::from(relative);
        CsvFileEntry {
            absolute_path: PathBuf::from("C:/workspace").join(&relative_path),
            file_name: relative_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            relative_path,
        }
    }

    fn tree_labels(rows: &[WorkspaceTreeRow]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                WorkspaceTreeRow::Directory { name, .. } => format!("D:{name}"),
                WorkspaceTreeRow::File { entry, .. } => format!("F:{}", entry.file_name),
            })
            .collect()
    }
}
