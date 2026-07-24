use std::collections::BTreeMap;

use serde_json::Value;

pub const MISC_HEADER_ROWS: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TableKind {
    Standard,
    Misc,
}

pub fn table_kind(records: &[Vec<String>]) -> TableKind {
    let has_misc_header = records.get(1).is_some_and(|header| {
        header.len() == 4 && header[0] == "valueType" && header[1] == "key" && header[2] == "value"
    });
    if has_misc_header && records.iter().all(|record| record.len() == 4) {
        TableKind::Misc
    } else {
        TableKind::Standard
    }
}

pub fn effective_header_rows(records: &[Vec<String>], configured: usize) -> usize {
    match table_kind(records) {
        TableKind::Standard => configured,
        TableKind::Misc => MISC_HEADER_ROWS,
    }
}

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
    InvalidTypeDeclaration,
    DuplicateKey,
    DeclaredTypeMismatch,
    StructuralMismatch,
    DangerousInvisibleCharacter,
    UnescapedQuote,
    InvalidBackslashEscape,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProblemSeverity {
    Warning,
    Error,
}

impl CellProblemKind {
    pub const fn severity(self) -> ProblemSeverity {
        match self {
            Self::UnescapedQuote | Self::InvalidBackslashEscape => ProblemSeverity::Warning,
            Self::InvalidTypeDeclaration
            | Self::DuplicateKey
            | Self::DeclaredTypeMismatch
            | Self::StructuralMismatch
            | Self::DangerousInvisibleCharacter => ProblemSeverity::Error,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CellProblemDetail {
    DeclaredTypeMismatch { expected: String, path: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CellProblem {
    pub row_index: usize,
    pub kinds: Vec<CellProblemKind>,
    pub detail: Option<CellProblemDetail>,
}

impl CellProblem {
    pub fn severity(&self) -> Option<ProblemSeverity> {
        if self
            .kinds
            .iter()
            .any(|kind| kind.severity() == ProblemSeverity::Error)
        {
            Some(ProblemSeverity::Error)
        } else if self
            .kinds
            .iter()
            .any(|kind| kind.severity() == ProblemSeverity::Warning)
        {
            Some(ProblemSeverity::Warning)
        } else {
            None
        }
    }
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
    if table_kind(records) == TableKind::Misc {
        return analyze_misc_table(records);
    }
    analyze_standard_table(records, header_rows)
}

fn analyze_standard_table(records: &[Vec<String>], header_rows: usize) -> Vec<ColumnAnalysis> {
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
            problems.push(CellProblem {
                row_index,
                kinds,
                detail: None,
            });
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

#[derive(Clone, Debug, Eq, PartialEq)]
enum DeclaredType {
    String,
    Number,
    Bool,
    Null,
    Array(Box<DeclaredType>),
    Object(BTreeMap<String, DeclaredType>),
}

impl DeclaredType {
    fn render(&self) -> String {
        match self {
            Self::String => "string".to_owned(),
            Self::Number => "number".to_owned(),
            Self::Bool => "bool".to_owned(),
            Self::Null => "null".to_owned(),
            Self::Array(element) => format!("{}[]", element.render()),
            Self::Object(fields) => {
                let fields = fields
                    .iter()
                    .map(|(name, field_type)| {
                        format!("{}:{}", render_field_name(name), field_type.render())
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                format!("{{{fields}}}")
            }
        }
    }

    fn is_primitive(&self) -> bool {
        matches!(self, Self::String | Self::Number | Self::Bool | Self::Null)
    }

    fn validate_cell(&self, value: &str) -> Result<(), String> {
        if let Self::Array(element) = self
            && element.is_primitive()
        {
            if let Ok(json @ Value::Array(_)) = serde_json::from_str::<Value>(value) {
                return self.validate_json(&json, "$".to_owned());
            }
            for (index, item) in value.split(',').enumerate() {
                element
                    .validate_cell(item.trim())
                    .map_err(|_| format!("$[{index}]"))?;
            }
            return Ok(());
        }

        match self {
            Self::String => Ok(()),
            Self::Number => match serde_json::from_str::<Value>(value.trim()) {
                Ok(Value::Number(_)) => Ok(()),
                _ => Err("$".to_owned()),
            },
            Self::Bool if matches!(value.trim(), "true" | "false" | "0" | "1") => Ok(()),
            Self::Bool => Err("$".to_owned()),
            Self::Null if value.trim() == "null" => Ok(()),
            Self::Null => Err("$".to_owned()),
            Self::Array(_) | Self::Object(_) => serde_json::from_str::<Value>(value.trim())
                .map_err(|_| "$".to_owned())
                .and_then(|json| self.validate_json(&json, "$".to_owned())),
        }
    }

    fn validate_json(&self, value: &Value, path: String) -> Result<(), String> {
        match (self, value) {
            (Self::String, Value::String(_))
            | (Self::Number, Value::Number(_))
            | (Self::Null, Value::Null) => Ok(()),
            (Self::Bool, Value::Bool(_)) => Ok(()),
            (Self::Bool, Value::Number(number))
                if number.as_i64().is_some_and(|value| matches!(value, 0 | 1)) =>
            {
                Ok(())
            }
            (Self::Array(element), Value::Array(items)) => {
                for (index, item) in items.iter().enumerate() {
                    element.validate_json(item, format!("{path}[{index}]"))?;
                }
                Ok(())
            }
            (Self::Object(expected), Value::Object(actual)) => {
                for name in expected.keys() {
                    if !actual.contains_key(name) {
                        return Err(object_path(&path, name));
                    }
                }
                for name in actual.keys() {
                    if !expected.contains_key(name) {
                        return Err(object_path(&path, name));
                    }
                }
                for (name, field_type) in expected {
                    field_type.validate_json(
                        actual.get(name).expect("object keys were checked above"),
                        object_path(&path, name),
                    )?;
                }
                Ok(())
            }
            _ => Err(path),
        }
    }
}

fn object_path(parent: &str, name: &str) -> String {
    let mut characters = name.chars();
    let identifier = characters
        .next()
        .is_some_and(|character| character == '_' || character == '$' || character.is_alphabetic())
        && characters
            .all(|character| character == '_' || character == '$' || character.is_alphanumeric());
    if identifier {
        format!("{parent}.{name}")
    } else {
        format!(
            "{parent}[{}]",
            serde_json::to_string(name).expect("serializing a JSON object key cannot fail")
        )
    }
}

struct DeclaredTypeParser<'a> {
    source: &'a str,
    offset: usize,
}

impl<'a> DeclaredTypeParser<'a> {
    fn parse(source: &'a str) -> Result<DeclaredType, ()> {
        let mut parser = Self { source, offset: 0 };
        let declared = parser.parse_type()?;
        parser.skip_whitespace();
        if parser.offset == source.len() {
            Ok(declared)
        } else {
            Err(())
        }
    }

    fn parse_type(&mut self) -> Result<DeclaredType, ()> {
        self.skip_whitespace();
        let mut declared = if self.peek() == Some('{') {
            self.parse_object()?
        } else {
            match self.parse_identifier()?.as_str() {
                "string" => DeclaredType::String,
                "number" => DeclaredType::Number,
                "bool" => DeclaredType::Bool,
                "null" => DeclaredType::Null,
                _ => return Err(()),
            }
        };
        loop {
            self.skip_whitespace();
            if !self.consume('[') {
                break;
            }
            self.skip_whitespace();
            if !self.consume(']') {
                return Err(());
            }
            declared = DeclaredType::Array(Box::new(declared));
        }
        Ok(declared)
    }

    fn parse_object(&mut self) -> Result<DeclaredType, ()> {
        if !self.consume('{') {
            return Err(());
        }
        let mut fields = BTreeMap::new();
        self.skip_whitespace();
        if self.consume('}') {
            return Ok(DeclaredType::Object(fields));
        }
        loop {
            self.skip_whitespace();
            let name = if self.peek() == Some('"') {
                self.parse_quoted_name()?
            } else {
                self.parse_identifier()?
            };
            self.skip_whitespace();
            if !self.consume(':') {
                return Err(());
            }
            let field_type = self.parse_type()?;
            if fields.insert(name, field_type).is_some() {
                return Err(());
            }
            self.skip_whitespace();
            if self.consume('}') {
                break;
            }
            if !self.consume(',') {
                return Err(());
            }
        }
        Ok(DeclaredType::Object(fields))
    }

    fn parse_quoted_name(&mut self) -> Result<String, ()> {
        let start = self.offset;
        if !self.consume('"') {
            return Err(());
        }
        let mut escaped = false;
        while let Some(character) = self.peek() {
            self.offset += character.len_utf8();
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                return serde_json::from_str(&self.source[start..self.offset]).map_err(|_| ());
            }
        }
        Err(())
    }

    fn parse_identifier(&mut self) -> Result<String, ()> {
        let start = self.offset;
        let Some(first) = self.peek() else {
            return Err(());
        };
        if first != '_' && first != '$' && !first.is_alphabetic() {
            return Err(());
        }
        self.offset += first.len_utf8();
        while let Some(character) = self.peek() {
            if character != '_' && character != '$' && !character.is_alphanumeric() {
                break;
            }
            self.offset += character.len_utf8();
        }
        Ok(self.source[start..self.offset].to_owned())
    }

    fn skip_whitespace(&mut self) {
        while let Some(character) = self.peek() {
            if !character.is_whitespace() {
                break;
            }
            self.offset += character.len_utf8();
        }
    }

    fn peek(&self) -> Option<char> {
        self.source[self.offset..].chars().next()
    }

    fn consume(&mut self, expected: char) -> bool {
        if self.peek() != Some(expected) {
            return false;
        }
        self.offset += expected.len_utf8();
        true
    }
}

fn analyze_misc_table(records: &[Vec<String>]) -> Vec<ColumnAnalysis> {
    let mut columns = (0..4)
        .map(|column_index| {
            let values = records
                .iter()
                .enumerate()
                .skip(MISC_HEADER_ROWS)
                .map(|(row_index, row)| (row_index, row[column_index].as_str()))
                .collect::<Vec<_>>();
            analyze_misc_column(&values, column_index != 0)
        })
        .collect::<Vec<_>>();

    let mut duplicate_rows = BTreeMap::<&str, Vec<usize>>::new();
    for (row_index, row) in records.iter().enumerate().skip(MISC_HEADER_ROWS) {
        if !row[1].is_empty() {
            duplicate_rows.entry(&row[1]).or_default().push(row_index);
        }
        match DeclaredTypeParser::parse(&row[0]) {
            Ok(declared) => {
                if let Err(path) = declared.validate_cell(&row[2]) {
                    add_problem(
                        &mut columns[2],
                        row_index,
                        CellProblemKind::DeclaredTypeMismatch,
                        Some(CellProblemDetail::DeclaredTypeMismatch {
                            expected: declared.render(),
                            path,
                        }),
                    );
                }
            }
            Err(()) => add_problem(
                &mut columns[0],
                row_index,
                CellProblemKind::InvalidTypeDeclaration,
                None,
            ),
        }
    }
    for rows in duplicate_rows.values().filter(|rows| rows.len() > 1) {
        for row_index in rows {
            add_problem(
                &mut columns[1],
                *row_index,
                CellProblemKind::DuplicateKey,
                None,
            );
        }
    }
    for column in &mut columns {
        column.problems.sort_by_key(|problem| problem.row_index);
    }
    columns
}

fn analyze_misc_column(values: &[(usize, &str)], check_string_escapes: bool) -> ColumnAnalysis {
    let max_content_chars = values
        .iter()
        .flat_map(|(_, value)| value.lines())
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    let problems = values
        .iter()
        .filter_map(|(row_index, value)| {
            let mut kinds = dangerous_character_problems(value, classify_cell(value));
            if !check_string_escapes {
                kinds.retain(|kind| {
                    !matches!(
                        kind,
                        CellProblemKind::UnescapedQuote | CellProblemKind::InvalidBackslashEscape
                    )
                });
            }
            (!kinds.is_empty()).then_some(CellProblem {
                row_index: *row_index,
                kinds,
                detail: None,
            })
        })
        .collect();
    ColumnAnalysis {
        column_type: ColumnType::String,
        type_expression: "string".to_owned(),
        has_mixed_warning: false,
        problems,
        max_content_chars,
    }
}

fn add_problem(
    analysis: &mut ColumnAnalysis,
    row_index: usize,
    kind: CellProblemKind,
    detail: Option<CellProblemDetail>,
) {
    if let Some(problem) = analysis
        .problems
        .iter_mut()
        .find(|problem| problem.row_index == row_index)
    {
        if !problem.kinds.contains(&kind) {
            problem.kinds.push(kind);
            problem.kinds.sort_unstable_by_key(|kind| *kind as u8);
        }
        if detail.is_some() {
            problem.detail = detail;
        }
    } else {
        analysis.problems.push(CellProblem {
            row_index,
            kinds: vec![kind],
            detail,
        });
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
                .contains(&CellProblemKind::DangerousInvisibleCharacter)
        );
        assert_eq!(analysis.problems[0].kinds.len(), 1);
    }

    #[test]
    fn real_line_breaks_are_not_diagnostic_problems() {
        let analysis = analyze_column(&[
            (2, "line\nbreak"),
            (3, "line\rbreak"),
            (4, "line\r\nbreak"),
            (5, r"line\nbreak"),
        ]);

        assert!(analysis.problems.is_empty());
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
        assert_eq!(
            invalid.problems[0].severity(),
            Some(ProblemSeverity::Warning)
        );
    }

    #[test]
    fn character_warnings_do_not_include_curly_quotes_and_errors_take_precedence() {
        let curly_quotes = analyze_column(&[(2, "中文“引号”")]);
        let combined = analyze_column(&[(2, r#"{"id":1}"#), (3, r#"bad"quote\q"#)]);

        assert!(curly_quotes.problems.is_empty());
        assert_eq!(
            combined.problems[0].severity(),
            Some(ProblemSeverity::Error)
        );
        assert!(
            combined.problems[0]
                .kinds
                .contains(&CellProblemKind::StructuralMismatch)
        );
        assert!(
            combined.problems[0]
                .kinds
                .contains(&CellProblemKind::UnescapedQuote)
        );
        assert!(
            combined.problems[0]
                .kinds
                .contains(&CellProblemKind::InvalidBackslashEscape)
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

    #[test]
    fn misc_tables_ignore_only_the_fourth_header_name() {
        for fourth in ["fuzhu", "beizhu", "description"] {
            let records = misc_records(&[["string", "A", "value", "note"]], fourth);
            assert_eq!(table_kind(&records), TableKind::Misc);
            assert_eq!(effective_header_rows(&records, 4), MISC_HEADER_ROWS);
        }

        let mut wrong_header = misc_records(&[["string", "A", "value", "note"]], "note");
        wrong_header[1][1] = "name".to_owned();
        assert_eq!(table_kind(&wrong_header), TableKind::Standard);

        let mut five_columns = misc_records(&[["string", "A", "value", "note"]], "note");
        for record in &mut five_columns {
            record.push(String::new());
        }
        assert_eq!(table_kind(&five_columns), TableKind::Standard);
        assert_eq!(effective_header_rows(&five_columns, 3), 3);
    }

    #[test]
    fn declared_types_validate_current_scalar_array_and_object_forms() {
        let records = misc_records(
            &[
                ["string", "EMPTY", "", "note"],
                ["string", "NUMERIC_TEXT", "123", "note"],
                ["number", "COUNT", "12.5", "note"],
                ["bool", "ENABLED", "1", "note"],
                ["null", "NONE", "null", "note"],
                ["number[]", "RANGE", "1,2", "note"],
                ["number[]", "ONE", "1", "note"],
                ["number[]", "JSON_RANGE", "[1,2]", "note"],
                ["string[]", "TOPIC", "one", "note"],
                [
                    r#"{"mid":number,"name":string}"#,
                    "ITEM",
                    r#"{"mid":1,"name":"ball"}"#,
                    "note",
                ],
                [
                    r#"{mid:number,tags:string[]}"#,
                    "ITEMS",
                    r#"{"mid":1,"tags":["a","b"]}"#,
                    "note",
                ],
                [r#"{"1":number}"#, "NUMERIC_KEY", r#"{"1":55}"#, "note"],
                ["string[][]", "GRID", r#"[["a"],["b"]]"#, "note"],
            ],
            "anything",
        );

        let analyses = analyze_table(&records, 5);

        assert_eq!(analyses.len(), 4);
        assert!(analyses.iter().all(|analysis| !analysis.has_mixed_warning));
        assert!(analyses.iter().all(|analysis| analysis.problems.is_empty()));
    }

    #[test]
    fn declared_type_parser_rejects_unions_optional_fields_and_trailing_input() {
        for declaration in ["number|string", "{id?:number}", "number[] extra"] {
            assert!(
                DeclaredTypeParser::parse(declaration).is_err(),
                "{declaration}"
            );
        }
    }

    #[test]
    fn misc_diagnostics_target_duplicate_declaration_and_value_cells() {
        let records = misc_records(
            &[
                ["number", "SAME", "1", "note"],
                ["string", "SAME", "numeric-looking", "note"],
                ["not-a-type", "same", "value", "note"],
                [
                    r#"{"id":number}"#,
                    "OBJECT",
                    r#"{"id":1,"extra":2}"#,
                    "note",
                ],
                ["number", "", "not-number", "note"],
                ["number", "", "2", "note"],
            ],
            "note",
        );

        let analyses = analyze_table(&records, 2);

        assert_eq!(
            problem_rows(&analyses[1], CellProblemKind::DuplicateKey),
            vec![2, 3]
        );
        assert_eq!(
            problem_rows(&analyses[0], CellProblemKind::InvalidTypeDeclaration),
            vec![4]
        );
        assert_eq!(
            problem_rows(&analyses[2], CellProblemKind::DeclaredTypeMismatch),
            vec![5, 6]
        );
        assert_eq!(
            analyses[2].problems[0].detail,
            Some(CellProblemDetail::DeclaredTypeMismatch {
                expected: "{id:number}".to_owned(),
                path: "$.extra".to_owned(),
            })
        );
    }

    #[test]
    fn misc_type_declarations_do_not_trigger_quote_escape_warnings() {
        let records = misc_records(
            &[
                [r#"{"mid":number}"#, "ITEM", r#"{"mid":1}"#, "line\nplain"],
                ["string", "NOTE", "value", "line\n\u{200B}"],
            ],
            "note",
        );

        let analyses = analyze_table(&records, 2);

        assert!(analyses[0].problems.is_empty());
        assert!(analyses[2].problems.is_empty());
        assert_eq!(analyses[3].problems[0].row_index, 3);
        assert!(
            analyses[3].problems[0]
                .kinds
                .contains(&CellProblemKind::DangerousInvisibleCharacter)
        );
        assert_eq!(analyses[3].problems[0].kinds.len(), 1);
    }

    fn misc_records(rows: &[[&str; 4]], fourth_header: &str) -> Vec<Vec<String>> {
        let mut records = vec![
            ["类型", "键", "值", "说明"].map(str::to_owned).to_vec(),
            ["valueType", "key", "value", fourth_header]
                .map(str::to_owned)
                .to_vec(),
        ];
        records.extend(rows.iter().map(|row| {
            row.iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>()
        }));
        records
    }

    fn problem_rows(analysis: &ColumnAnalysis, kind: CellProblemKind) -> Vec<usize> {
        analysis
            .problems
            .iter()
            .filter(|problem| problem.kinds.contains(&kind))
            .map(|problem| problem.row_index)
            .collect()
    }
}
