use std::collections::BTreeMap;

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
    pub type_expression: String,
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
    let type_expression = infer_type_expression(&classified, column_type, expected_structural_type);
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
        type_expression,
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

#[derive(Clone, Debug, Eq, PartialEq)]
enum TypeShape {
    Unknown,
    String,
    Number,
    Bool,
    Null,
    Array(Box<TypeShape>),
    Object(BTreeMap<String, ObjectField>),
    Union(Vec<TypeShape>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ObjectField {
    shape: TypeShape,
    optional: bool,
}

impl TypeShape {
    fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Unknown, other) | (other, Self::Unknown) => other,
            (Self::Array(left), Self::Array(right)) => Self::Array(Box::new(left.merge(*right))),
            (Self::Object(left), Self::Object(right)) => {
                Self::Object(merge_object_fields(left, right))
            }
            (left, right) if left == right => left,
            (left, right) => merge_union_members(left, right),
        }
    }

    fn render(&self) -> String {
        match self {
            Self::Unknown => "unknown".to_owned(),
            Self::String => "string".to_owned(),
            Self::Number => "number".to_owned(),
            Self::Bool => "bool".to_owned(),
            Self::Null => "null".to_owned(),
            Self::Array(element) => {
                let expression = element.render();
                if matches!(element.as_ref(), Self::Union(_)) {
                    format!("({expression})[]")
                } else {
                    format!("{expression}[]")
                }
            }
            Self::Object(fields) => {
                let fields = fields
                    .iter()
                    .map(|(name, field)| {
                        let name = render_field_name(name);
                        let optional = if field.optional { "?" } else { "" };
                        format!("{name}{optional}:{}", field.shape.render())
                    })
                    .collect::<Vec<_>>()
                    .join(";");
                format!("{{{fields}}}")
            }
            Self::Union(members) => members
                .iter()
                .map(Self::render)
                .collect::<Vec<_>>()
                .join("|"),
        }
    }

    fn sort_key(&self) -> (u8, String) {
        let rank = match self {
            Self::Unknown => 0,
            Self::Null => 1,
            Self::Bool => 2,
            Self::Number => 3,
            Self::String => 4,
            Self::Array(_) => 5,
            Self::Object(_) => 6,
            Self::Union(_) => 7,
        };
        (rank, self.render())
    }
}

fn merge_object_fields(
    mut left: BTreeMap<String, ObjectField>,
    right: BTreeMap<String, ObjectField>,
) -> BTreeMap<String, ObjectField> {
    let right_names = right.keys().cloned().collect::<Vec<_>>();
    for (name, right_field) in right {
        if let Some(left_field) = left.get_mut(&name) {
            let left_shape = std::mem::replace(&mut left_field.shape, TypeShape::Unknown);
            left_field.shape = left_shape.merge(right_field.shape);
            left_field.optional |= right_field.optional;
        } else {
            left.insert(
                name,
                ObjectField {
                    shape: right_field.shape,
                    optional: true,
                },
            );
        }
    }
    for (name, field) in &mut left {
        if right_names.binary_search(name).is_err() {
            field.optional = true;
        }
    }
    left
}

fn merge_union_members(left: TypeShape, right: TypeShape) -> TypeShape {
    let mut members = Vec::new();
    flatten_union(left, &mut members);
    flatten_union(right, &mut members);

    let mut merged = Vec::<TypeShape>::new();
    for member in members {
        if let Some(index) = merged.iter().position(|existing| {
            matches!(
                (existing, &member),
                (TypeShape::Array(_), TypeShape::Array(_))
                    | (TypeShape::Object(_), TypeShape::Object(_))
            ) || existing == &member
        }) {
            let existing = merged.remove(index);
            merged.push(existing.merge(member));
        } else {
            merged.push(member);
        }
    }
    merged.sort_by_key(TypeShape::sort_key);
    if merged.len() == 1 {
        merged.pop().expect("one merged type")
    } else {
        TypeShape::Union(merged)
    }
}

fn flatten_union(shape: TypeShape, members: &mut Vec<TypeShape>) {
    if let TypeShape::Union(items) = shape {
        for item in items {
            flatten_union(item, members);
        }
    } else {
        members.push(shape);
    }
}

fn render_field_name(name: &str) -> String {
    let mut characters = name.chars();
    let valid_start = characters
        .next()
        .is_some_and(|character| character == '_' || character == '$' || character.is_alphabetic());
    if valid_start
        && characters
            .all(|character| character == '_' || character == '$' || character.is_alphanumeric())
    {
        name.to_owned()
    } else {
        serde_json::to_string(name).expect("serializing a JSON object key cannot fail")
    }
}

fn infer_type_expression(
    classified: &[(usize, &str, CellType)],
    column_type: ColumnType,
    expected_structural_type: Option<CellType>,
) -> String {
    let Some(expected) = expected_structural_type else {
        return column_type.label().to_owned();
    };
    classified
        .iter()
        .filter(|(_, _, cell_type)| *cell_type == expected)
        .filter_map(|(_, value, cell_type)| type_shape_for_cell(value, *cell_type))
        .reduce(TypeShape::merge)
        .map_or_else(|| column_type.label().to_owned(), |shape| shape.render())
}

fn type_shape_for_cell(value: &str, cell_type: CellType) -> Option<TypeShape> {
    if let Ok(value) = serde_json::from_str::<Value>(value) {
        return Some(type_shape_for_json(&value));
    }
    match cell_type {
        CellType::Array => Some(TypeShape::Array(Box::new(TypeShape::String))),
        CellType::Array2d => Some(TypeShape::Array(Box::new(TypeShape::Array(Box::new(
            TypeShape::String,
        ))))),
        _ => None,
    }
}

fn type_shape_for_json(value: &Value) -> TypeShape {
    match value {
        Value::Null => TypeShape::Null,
        Value::Bool(_) => TypeShape::Bool,
        Value::Number(_) => TypeShape::Number,
        Value::String(_) => TypeShape::String,
        Value::Array(items) => TypeShape::Array(Box::new(
            items
                .iter()
                .map(type_shape_for_json)
                .reduce(TypeShape::merge)
                .unwrap_or(TypeShape::Unknown),
        )),
        Value::Object(fields) => TypeShape::Object(
            fields
                .iter()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        ObjectField {
                            shape: type_shape_for_json(value),
                            optional: false,
                        },
                    )
                })
                .collect(),
        ),
    }
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

    if classified
        .iter()
        .all(|(_, _, cell_type)| *cell_type == CellType::Number)
    {
        return (ColumnType::Number, None, false);
    }

    if classified
        .iter()
        .all(|(_, value, _)| matches!(*value, "true" | "false" | "0" | "1"))
    {
        return (ColumnType::Bool, None, false);
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
    if matches!(value, "true" | "false") {
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
    fn zero_and_one_are_numbers_when_the_whole_column_is_numeric() {
        let analysis = analyze_column(&[(2, "0"), (3, "1")]);

        assert_eq!(analysis.column_type, ColumnType::Number);
        assert!(!analysis.has_mixed_warning);
    }

    #[test]
    fn zero_one_and_other_numbers_are_numbers() {
        let analysis = analyze_column(&[(2, "0"), (3, "1"), (4, "2")]);

        assert_eq!(analysis.column_type, ColumnType::Number);
        assert!(!analysis.has_mixed_warning);
    }

    #[test]
    fn textual_bools_can_mix_with_zero_and_one() {
        let analysis = analyze_column(&[(2, "true"), (3, "false"), (4, "0"), (5, "1")]);

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
    fn non_json_floating_point_spellings_remain_strings() {
        for value in ["NaN", "inf", "+1", "01"] {
            let analysis = analyze_column(&[(2, value)]);
            assert_eq!(analysis.column_type, ColumnType::String, "{value}");
        }
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
    fn describes_json_and_separator_arrays_with_concrete_types() {
        assert_eq!(
            analyze_column(&[(2, r#"["a","b"]"#), (3, r#""c","d""#)]).type_expression,
            "string[]"
        );
        assert_eq!(
            analyze_column(&[(2, "[1,2]"), (3, "[3]")]).type_expression,
            "number[]"
        );
        assert_eq!(
            analyze_column(&[(2, "[[1],[2,3]]")]).type_expression,
            "number[][]"
        );
        assert_eq!(analyze_column(&[(2, "[]")]).type_expression, "unknown[]");
    }

    #[test]
    fn merges_object_fields_recursively_and_marks_missing_fields_optional() {
        let analysis = analyze_column(&[
            (
                2,
                r#"{"mid":1,"name":"first","meta":{"rank":1},"items":[{"id":1}]}"#,
            ),
            (
                3,
                r#"{"mid":2,"meta":{"rank":2,"tag":"x"},"items":[{"id":2,"name":"n"}]}"#,
            ),
        ]);

        assert_eq!(
            analysis.type_expression,
            "{items:{id:number;name?:string}[];meta:{rank:number;tag?:string};mid:number;name?:string}"
        );
    }

    #[test]
    fn an_optional_field_remains_optional_if_it_reappears_later() {
        let analysis = analyze_column(&[
            (2, r#"{"id":1,"name":"first"}"#),
            (3, r#"{"id":2}"#),
            (4, r#"{"id":3,"name":"third"}"#),
        ]);

        assert_eq!(analysis.type_expression, "{id:number;name?:string}");
    }

    #[test]
    fn renders_unions_and_quotes_non_identifier_object_keys() {
        let analysis = analyze_column(&[
            (2, r#"{"bad-key":1,"value":[1,"x"]}"#),
            (3, r#"{"bad-key":"x","value":[false]}"#),
        ]);

        assert_eq!(
            analysis.type_expression,
            r#"{"bad-key":number|string;value:(bool|number|string)[]}"#
        );
    }

    #[test]
    fn invalid_structural_cells_do_not_change_expression_or_problem_count() {
        let analysis = analyze_column(&[(2, r#"{"id":1}"#), (3, "broken"), (4, r#"{"id":2}"#)]);

        assert_eq!(analysis.type_expression, "{id:number}");
        assert_eq!(analysis.problems.len(), 1);
        assert_eq!(analysis.problems[0].row_index, 3);
        assert_eq!(
            analysis.problems[0].kinds,
            vec![CellProblemKind::StructuralMismatch]
        );
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
