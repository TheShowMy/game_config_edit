use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColumnType {
    String,
    Number,
    Bool,
    Json,
    Array,
    Array2d,
    Mixed,
}

impl ColumnType {
    pub fn label(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Number => "number",
            Self::Bool => "bool",
            Self::Json => "json",
            Self::Array => "array",
            Self::Array2d => "array_2d",
            Self::Mixed => "mixed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CellProblemKind {
    StructuralMismatch,
    RealLineBreak,
    DangerousInvisibleCharacter,
    UnescapedQuote,
    InvalidBackslashEscape,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CellProblem {
    pub row_index: usize,
    pub kinds: Vec<CellProblemKind>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColumnAnalysis {
    pub column_type: ColumnType,
    pub has_mixed_warning: bool,
    pub problems: Vec<CellProblem>,
    pub max_content_chars: usize,
}

pub fn analyze_table(records: &[Vec<String>], header_rows: usize) -> Vec<ColumnAnalysis> {
    let column_count = records.first().map_or(0, Vec::len);
    (0..column_count)
        .map(|column_index| {
            let values = records
                .iter()
                .enumerate()
                .skip(header_rows)
                .filter_map(|(row_index, row)| {
                    row.get(column_index)
                        .map(|value| (row_index, value.as_str()))
                })
                .collect::<Vec<_>>();
            analyze_column(&values)
        })
        .collect()
}

pub fn analyze_column(values: &[(usize, &str)]) -> ColumnAnalysis {
    let max_content_chars = values
        .iter()
        .flat_map(|(_, value)| value.lines())
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    let classified = values
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(row_index, value)| (*row_index, *value, classify_cell(value)))
        .collect::<Vec<_>>();

    let (column_type, expected_structural_type, has_mixed_warning) = infer_column_type(&classified);
    let mut problems = Vec::new();

    for (row_index, value, cell_type) in classified {
        let mut kinds = dangerous_character_problems(value, cell_type);
        if expected_structural_type.is_some_and(|expected| expected != cell_type) {
            kinds.insert(0, CellProblemKind::StructuralMismatch);
        }
        kinds.sort_unstable_by_key(|kind| *kind as u8);
        kinds.dedup();
        if !kinds.is_empty() {
            problems.push(CellProblem { row_index, kinds });
        }
    }

    ColumnAnalysis {
        column_type,
        has_mixed_warning,
        problems,
        max_content_chars,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CellType {
    String,
    Number,
    Bool,
    Json,
    Array,
    Array2d,
}

fn infer_column_type(
    classified: &[(usize, &str, CellType)],
) -> (ColumnType, Option<CellType>, bool) {
    if classified.is_empty() {
        return (ColumnType::String, None, false);
    }
    if classified
        .iter()
        .any(|(_, _, cell_type)| *cell_type == CellType::Json)
    {
        return (ColumnType::Json, Some(CellType::Json), false);
    }
    if classified
        .iter()
        .any(|(_, _, cell_type)| *cell_type == CellType::Array2d)
    {
        return (ColumnType::Array2d, Some(CellType::Array2d), false);
    }
    if classified
        .iter()
        .any(|(_, _, cell_type)| *cell_type == CellType::Array)
    {
        return (ColumnType::Array, Some(CellType::Array), false);
    }

    let has_bool = classified
        .iter()
        .any(|(_, _, cell_type)| *cell_type == CellType::Bool);
    let has_number = classified
        .iter()
        .any(|(_, _, cell_type)| *cell_type == CellType::Number);
    let has_string = classified
        .iter()
        .any(|(_, _, cell_type)| *cell_type == CellType::String);

    match (has_bool, has_number, has_string) {
        (true, false, false) => (ColumnType::Bool, None, false),
        (false, true, false) => (ColumnType::Number, None, false),
        (false, false, true) | (false, true, true) => (ColumnType::String, None, false),
        (true, true, false) => (ColumnType::Mixed, None, true),
        _ => (ColumnType::Mixed, None, true),
    }
}

fn classify_cell(value: &str) -> CellType {
    if matches!(value, "true" | "false" | "0" | "1") {
        return CellType::Bool;
    }

    if let Ok(json) = serde_json::from_str::<Value>(value) {
        match json {
            Value::Object(_) => return CellType::Json,
            Value::Array(items) => {
                return if !items.is_empty() && items.iter().all(Value::is_array) {
                    CellType::Array2d
                } else {
                    CellType::Array
                };
            }
            Value::Number(_) => return CellType::Number,
            _ => {}
        }
    }

    if let Some(array_type) = classify_separator_array(value) {
        return array_type;
    }
    CellType::String
}

fn classify_separator_array(value: &str) -> Option<CellType> {
    let bytes = value.as_bytes();
    let mut index = 0;
    let mut elements = 0;
    let mut groups = 1;

    while index < bytes.len() {
        if bytes[index] != b'"' {
            return None;
        }
        index += 1;
        loop {
            match bytes.get(index) {
                Some(b'"') if bytes.get(index + 1) == Some(&b'"') => index += 2,
                Some(b'"') => {
                    index += 1;
                    break;
                }
                Some(_) => index += 1,
                None => return None,
            }
        }
        elements += 1;

        match bytes.get(index) {
            None => break,
            Some(b',') => index += 1,
            Some(b';') => {
                groups += 1;
                index += 1;
            }
            Some(_) => return None,
        }
    }

    if groups > 1 && elements >= groups {
        Some(CellType::Array2d)
    } else if elements >= 2 {
        Some(CellType::Array)
    } else {
        None
    }
}

fn dangerous_character_problems(value: &str, cell_type: CellType) -> Vec<CellProblemKind> {
    let mut problems = Vec::new();
    if value.contains(['\r', '\n']) {
        problems.push(CellProblemKind::RealLineBreak);
    }
    if value.chars().any(|character| {
        matches!(
            character,
            '\t' | '\u{3000}' | '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}'
        )
    }) {
        problems.push(CellProblemKind::DangerousInvisibleCharacter);
    }
    if cell_type == CellType::String {
        problems.extend(string_escape_problems(value));
    }
    problems
}

fn string_escape_problems(value: &str) -> Vec<CellProblemKind> {
    let characters = value.chars().collect::<Vec<_>>();
    let mut index = 0;
    let mut problems = Vec::new();

    while index < characters.len() {
        match characters[index] {
            '"' => {
                problems.push(CellProblemKind::UnescapedQuote);
                index += 1;
            }
            '\\' => {
                let Some(escaped) = characters.get(index + 1) else {
                    problems.push(CellProblemKind::InvalidBackslashEscape);
                    break;
                };
                match escaped {
                    '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' => index += 2,
                    'u' if characters.get(index + 2..index + 6).is_some_and(|digits| {
                        digits.iter().all(|digit| digit.is_ascii_hexdigit())
                    }) =>
                    {
                        index += 6;
                    }
                    _ => {
                        problems.push(CellProblemKind::InvalidBackslashEscape);
                        index += 2;
                    }
                }
            }
            _ => index += 1,
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_empty_values_are_strings() {
        let analysis = analyze_column(&[(2, ""), (3, "")]);

        assert_eq!(analysis.column_type, ColumnType::String);
        assert!(analysis.problems.is_empty());
    }

    #[test]
    fn zero_and_one_are_bool_before_number() {
        let analysis = analyze_column(&[(2, "0"), (3, "1")]);

        assert_eq!(analysis.column_type, ColumnType::Bool);
        assert!(!analysis.has_mixed_warning);
    }

    #[test]
    fn bool_and_number_are_mixed() {
        let analysis = analyze_column(&[(2, "true"), (3, "2")]);

        assert_eq!(analysis.column_type, ColumnType::Mixed);
        assert!(analysis.has_mixed_warning);
    }

    #[test]
    fn number_and_text_become_string_without_warning() {
        let analysis = analyze_column(&[(2, "42"), (3, "unknown")]);

        assert_eq!(analysis.column_type, ColumnType::String);
        assert!(!analysis.has_mixed_warning);
    }

    #[test]
    fn a_valid_json_object_makes_other_values_structural_errors() {
        let analysis = analyze_column(&[(2, r#"{"id":1}"#), (3, "not-json"), (4, "")]);

        assert_eq!(analysis.column_type, ColumnType::Json);
        assert_eq!(analysis.problems.len(), 1);
        assert_eq!(analysis.problems[0].row_index, 3);
        assert!(
            analysis.problems[0]
                .kinds
                .contains(&CellProblemKind::StructuralMismatch)
        );
    }

    #[test]
    fn malformed_json_without_valid_structured_value_is_plain_text() {
        let analysis = analyze_column(&[(2, "{not-json")]);

        assert_eq!(analysis.column_type, ColumnType::String);
        assert!(analysis.problems.is_empty());
    }

    #[test]
    fn recognizes_json_and_separator_arrays() {
        assert_eq!(classify_cell(r#"["a","b"]"#), CellType::Array);
        assert_eq!(classify_cell(r#"[[1,2],[3,4]]"#), CellType::Array2d);
        assert_eq!(classify_cell(r#""a","b""#), CellType::Array);
        assert_eq!(classify_cell(r#""a","b";"c","d""#), CellType::Array2d);
        assert_eq!(classify_cell(r#""only one""#), CellType::String);
    }

    #[test]
    fn reports_character_compatibility_problems_once_per_cell() {
        let analysis = analyze_column(&[(2, "line\nbreak\t\u{200B}")]);

        assert_eq!(analysis.problems.len(), 1);
        assert!(
            analysis.problems[0]
                .kinds
                .contains(&CellProblemKind::RealLineBreak)
        );
        assert!(
            analysis.problems[0]
                .kinds
                .contains(&CellProblemKind::DangerousInvisibleCharacter)
        );
    }

    #[test]
    fn accepts_documented_string_escapes_and_rejects_invalid_ones() {
        let accepted = analyze_column(&[(2, r#"line\n\"quoted\"\u0041"#)]);
        let invalid = analyze_column(&[(2, r#"bad\q"quote"#)]);

        assert!(accepted.problems.is_empty());
        assert_eq!(invalid.problems.len(), 1);
        assert!(
            invalid.problems[0]
                .kinds
                .contains(&CellProblemKind::InvalidBackslashEscape)
        );
        assert!(
            invalid.problems[0]
                .kinds
                .contains(&CellProblemKind::UnescapedQuote)
        );
    }

    #[test]
    fn table_analysis_skips_header_rows_and_preserves_source_row_index() {
        let records = vec![
            vec!["description".to_owned()],
            vec!["field".to_owned()],
            vec!["true".to_owned()],
            vec!["false".to_owned()],
        ];

        let analysis = analyze_table(&records, 2);

        assert_eq!(analysis.len(), 1);
        assert_eq!(analysis[0].column_type, ColumnType::Bool);
    }
}
