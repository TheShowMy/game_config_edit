use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::mpsc::UnboundedSender;

use crate::workspace::CsvFileEntry;

pub const MAX_CURRENT_MATCHES: usize = 10_000;
pub const MAX_GLOBAL_MATCHES: usize = 1_000;
const GLOBAL_BATCH_SIZE: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellSearchMatch {
    pub row_index: usize,
    pub column_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextSearchMatch {
    pub start_utf16: usize,
    pub end_utf16: usize,
    pub line_number: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlobalSearchMatch {
    pub absolute_path: PathBuf,
    pub relative_path: PathBuf,
    pub line_number: usize,
    pub snippet: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GlobalSearchEvent {
    Batch(Vec<GlobalSearchMatch>),
    Finished {
        cancelled: bool,
        truncated: bool,
        warning_count: usize,
    },
}

pub fn rank_files(files: &[CsvFileEntry], query: &str, limit: usize) -> Vec<CsvFileEntry> {
    let query = query.trim().to_lowercase();
    let mut ranked = files
        .iter()
        .filter_map(|entry| {
            let file_name = entry.file_name.to_lowercase();
            let relative = entry.relative_path.to_string_lossy().to_lowercase();
            file_score(&file_name, &relative, &query).map(|score| (score, entry.clone()))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|(left_score, left), (right_score, right)| {
        left_score.cmp(right_score).then_with(|| {
            left.relative_path
                .to_string_lossy()
                .cmp(&right.relative_path.to_string_lossy())
        })
    });
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, entry)| entry)
        .collect()
}

pub fn find_text_matches(text: &str, query: &str, case_sensitive: bool) -> Vec<TextSearchMatch> {
    if query.is_empty() {
        return Vec::new();
    }
    find_match_ranges(text, query, case_sensitive)
        .take(MAX_CURRENT_MATCHES)
        .map(|(start, end)| TextSearchMatch {
            start_utf16: text[..start].encode_utf16().count(),
            end_utf16: text[..end].encode_utf16().count(),
            line_number: text[..start].bytes().filter(|byte| *byte == b'\n').count() + 1,
        })
        .collect()
}

pub fn find_cell_matches(
    records: &[Vec<String>],
    header_rows: usize,
    query: &str,
    case_sensitive: bool,
) -> Vec<CellSearchMatch> {
    if query.is_empty() {
        return Vec::new();
    }
    records
        .iter()
        .enumerate()
        .skip(header_rows)
        .flat_map(|(row_index, row)| {
            row.iter()
                .enumerate()
                .filter(move |(_, value)| contains(value, query, case_sensitive))
                .map(move |(column_index, _)| CellSearchMatch {
                    row_index,
                    column_index,
                })
        })
        .take(MAX_CURRENT_MATCHES)
        .collect()
}

pub fn stream_workspace_search(
    files: Vec<CsvFileEntry>,
    query: String,
    case_sensitive: bool,
    cancel: Arc<AtomicBool>,
    sender: UnboundedSender<GlobalSearchEvent>,
) {
    let mut batch = Vec::with_capacity(GLOBAL_BATCH_SIZE);
    let mut match_count = 0_usize;
    let mut warning_count = 0_usize;
    let mut truncated = false;

    'files: for entry in files {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let file = match File::open(&entry.absolute_path) {
            Ok(file) => file,
            Err(_) => {
                warning_count += 1;
                continue;
            }
        };
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        let mut line_number = 0_usize;
        loop {
            if cancel.load(Ordering::Relaxed) {
                break 'files;
            }
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => line_number += 1,
                Err(_) => {
                    warning_count += 1;
                    break;
                }
            }
            if !contains(&line, &query, case_sensitive) {
                continue;
            }
            batch.push(GlobalSearchMatch {
                absolute_path: entry.absolute_path.clone(),
                relative_path: entry.relative_path.clone(),
                line_number,
                snippet: compact_snippet(&line),
            });
            match_count += 1;
            if batch.len() == GLOBAL_BATCH_SIZE
                && sender
                    .send(GlobalSearchEvent::Batch(std::mem::take(&mut batch)))
                    .is_err()
            {
                return;
            }
            if match_count == MAX_GLOBAL_MATCHES {
                truncated = true;
                break 'files;
            }
        }
    }

    if !batch.is_empty() && sender.send(GlobalSearchEvent::Batch(batch)).is_err() {
        return;
    }
    let _ = sender.send(GlobalSearchEvent::Finished {
        cancelled: cancel.load(Ordering::Relaxed),
        truncated,
        warning_count,
    });
}

fn file_score(file_name: &str, relative: &str, query: &str) -> Option<(u8, usize, usize)> {
    if query.is_empty() {
        return Some((0, 0, relative.len()));
    }
    if file_name == query {
        return Some((0, 0, file_name.len()));
    }
    if file_name.starts_with(query) {
        return Some((1, 0, file_name.len()));
    }
    if let Some(position) = file_name.find(query) {
        return Some((2, position, file_name.len()));
    }
    if let Some(gaps) = subsequence_gaps(file_name, query) {
        return Some((3, gaps, file_name.len()));
    }
    if let Some(position) = relative.find(query) {
        return Some((4, position, relative.len()));
    }
    subsequence_gaps(relative, query).map(|gaps| (5, gaps, relative.len()))
}

fn subsequence_gaps(candidate: &str, query: &str) -> Option<usize> {
    let mut candidate = candidate.chars().enumerate();
    let mut last_index = None;
    let mut gaps = 0;
    for expected in query.chars() {
        let (index, _) = candidate.find(|(_, actual)| *actual == expected)?;
        if let Some(previous) = last_index {
            gaps += index.saturating_sub(previous + 1);
        }
        last_index = Some(index);
    }
    Some(gaps)
}

fn contains(value: &str, query: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        value.contains(query)
    } else {
        find_match_ranges(value, query, false).next().is_some()
    }
}

fn find_match_ranges<'a>(
    value: &'a str,
    query: &'a str,
    case_sensitive: bool,
) -> impl Iterator<Item = (usize, usize)> + 'a {
    value.char_indices().filter_map(move |(start, _)| {
        let end = start.checked_add(query.len())?;
        let candidate = value.get(start..end)?;
        let matched = if case_sensitive {
            candidate == query
        } else {
            candidate.eq_ignore_ascii_case(query)
        };
        matched.then_some((start, end))
    })
}

fn compact_snippet(line: &str) -> String {
    let compact = line.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut characters = compact.chars();
    let mut snippet = characters.by_ref().take(160).collect::<String>();
    if characters.next().is_some() {
        snippet.push_str("...");
    }
    snippet
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> CsvFileEntry {
        CsvFileEntry {
            absolute_path: PathBuf::from(path),
            relative_path: PathBuf::from(path),
            file_name: PathBuf::from(path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        }
    }

    #[test]
    fn fuzzy_file_ranking_supports_documented_queries() {
        let files = vec![entry("test/hero_basic_test.csv"), entry("hero_basic.csv")];

        for query in ["hero_basic", "hbasic", "basic"] {
            assert_eq!(rank_files(&files, query, 10)[0].file_name, "hero_basic.csv");
        }
    }

    #[test]
    fn text_matches_report_utf16_offsets_and_physical_lines() {
        let matches = find_text_matches("one\n😀 Hero\nhero", "hero", false);

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].line_number, 2);
        assert_eq!(matches[0].start_utf16, 7);
        assert_eq!(matches[1].line_number, 3);
    }

    #[test]
    fn table_search_skips_configured_header_records() {
        let records = vec![
            vec!["name".to_owned()],
            vec!["Hero".to_owned()],
            vec!["heroine".to_owned()],
        ];

        assert_eq!(
            find_cell_matches(&records, 1, "hero", false),
            vec![
                CellSearchMatch {
                    row_index: 1,
                    column_index: 0,
                },
                CellSearchMatch {
                    row_index: 2,
                    column_index: 0,
                },
            ]
        );
    }

    #[test]
    fn global_search_streams_physical_line_results() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("heroes.csv");
        std::fs::write(&path, "id,name\n1,Arthur\n2,Merlin\n").unwrap();
        let files = vec![CsvFileEntry {
            absolute_path: path,
            relative_path: PathBuf::from("heroes.csv"),
            file_name: "heroes.csv".to_owned(),
        }];
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();

        stream_workspace_search(
            files,
            "merlin".to_owned(),
            false,
            Arc::new(AtomicBool::new(false)),
            sender,
        );

        let GlobalSearchEvent::Batch(matches) = receiver.try_recv().unwrap() else {
            panic!("expected a result batch");
        };
        assert_eq!(matches[0].line_number, 3);
        assert_eq!(matches[0].snippet, "2,Merlin");
        assert!(matches!(
            receiver.try_recv().unwrap(),
            GlobalSearchEvent::Finished {
                cancelled: false,
                truncated: false,
                warning_count: 0,
            }
        ));
    }
}
